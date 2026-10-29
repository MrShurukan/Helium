//! Internal consistency checks. Not a substitute for source-language validation.
use super::*;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidId,
    DuplicateDefinition,
    TypeMismatch,
    UnavailableValue,
    UninitializedLocal,
    InvalidEntry,
    InvalidPin,
    RecursiveCall,
    InterruptState,
}

#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub function: Option<FunctionId>,
    pub block: Option<BlockId>,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} in {:?}/{:?}: {}",
            self.kind, self.function, self.block, self.message
        )
    }
}

impl std::error::Error for Error {}

fn failure(
    kind: ErrorKind,
    function: Option<FunctionId>,
    block: Option<BlockId>,
    message: &str,
) -> Error {
    Error {
        kind,
        function,
        block,
        message: message.into(),
    }
}

pub fn verify(module: &Module) -> Result<(), Error> {
    let all_types = module
        .globals
        .iter()
        .map(|s| s.ty)
        .chain(module.functions.iter().flat_map(|f| {
            f.parameters
                .iter()
                .map(|(_, t)| *t)
                .chain(f.return_type)
                .chain(f.locals.iter().map(|s| s.ty))
                .chain(
                    f.blocks
                        .iter()
                        .flat_map(|b| &b.instructions)
                        .filter_map(|i| i.result.map(|(_, t)| t)),
                )
        }));
    for t in all_types {
        if matches!(
            t,
            Type::Ref(
                ElementType::Integer(crate::frontend::static_type::IntegerType::Literal),
                _
            )
        ) {
            return Err(failure(
                ErrorKind::TypeMismatch,
                None,
                None,
                "unresolved reference element type",
            ));
        }
        if let Some((e, n)) = t.sequence() {
            if n.is_some_and(|n| n == 0 || n > 1024)
                || matches!(
                    e,
                    ElementType::Integer(crate::frontend::static_type::IntegerType::Literal)
                )
            {
                return Err(failure(
                    ErrorKind::TypeMismatch,
                    None,
                    None,
                    "invalid array type",
                ));
            }
        }
    }
    if module.globals.iter().any(|g| {
        matches!(
            g.ty,
            Type::Ref(..) | Type::ArrayRef(..) | Type::SliceRef(..)
        )
    }) || module.functions.iter().any(|f| {
        matches!(
            f.return_type,
            Some(Type::Ref(..) | Type::ArrayRef(..) | Type::SliceRef(..))
        )
    }) {
        return Err(failure(
            ErrorKind::TypeMismatch,
            None,
            None,
            "references cannot escape through globals or returns",
        ));
    }
    verify_devices(module)?;
    verify_lifecycle(module)?;
    let entry = module
        .functions
        .get(module.entry.0)
        .ok_or_else(|| failure(ErrorKind::InvalidEntry, None, None, "missing module entry"))?;
    if !entry.parameters.is_empty() || entry.return_type.is_some() || entry.requires_no_interrupt {
        return Err(failure(
            ErrorKind::InvalidEntry,
            None,
            None,
            "entry must have no parameters and return void",
        ));
    }

    let target = module.target;
    let mut pins = HashSet::new();
    for pin in &module.pins {
        if !target.has_digital_pin(&pin.port, pin.bit) || !pins.insert((&pin.port, pin.bit)) {
            return Err(failure(
                ErrorKind::InvalidPin,
                None,
                None,
                "invalid or duplicate physical pin",
            ));
        }
    }

    for (index, function) in module.functions.iter().enumerate() {
        if function.id != FunctionId(index) {
            return Err(failure(
                ErrorKind::InvalidId,
                Some(function.id),
                None,
                "function ID does not match module slot",
            ));
        }
        verify_function(module, function)?;
        verify_interrupts(module, function)?;
    }

    // Recursion is deliberately outside v1's contract, including mutually recursive calls.
    fn visit(
        id: FunctionId,
        module: &Module,
        active: &mut HashSet<FunctionId>,
        done: &mut HashSet<FunctionId>,
    ) -> Result<(), Error> {
        if done.contains(&id) {
            return Ok(());
        }
        if !active.insert(id) {
            return Err(failure(
                ErrorKind::RecursiveCall,
                Some(id),
                None,
                "recursive calls are not supported in IR v1",
            ));
        }

        for block in &module.functions[id.0].blocks {
            for instruction in &block.instructions {
                if let Op::Call(callee, _) = instruction.op {
                    visit(callee, module, active, done)?;
                }
            }
        }

        active.remove(&id);
        done.insert(id);
        Ok(())
    }

    let mut done = HashSet::new();
    for function in &module.functions {
        visit(function.id, module, &mut HashSet::new(), &mut done)?;
    }

    if let Some(roles) = &module.lifecycle {
        let mut pending = vec![roles.initialize];
        let mut visited = HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            for i in module.functions[id.0]
                .blocks
                .iter()
                .flat_map(|b| &b.instructions)
            {
                match &i.op {
                    Op::Call(callee, _) => pending.push(*callee),
                    Op::Intrinsic { .. } => {
                        return Err(failure(
                            ErrorKind::InvalidEntry,
                            Some(id),
                            None,
                            "device used before startup",
                        ))
                    }
                    _ => {}
                }
            }
        }
    }

    if !module.interrupts.is_empty() {
        let mut pending: Vec<_> = module
            .interrupts
            .iter()
            .map(|binding| binding.handler)
            .collect();
        let mut visited = HashSet::new();

        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }

            for block in &module.functions[id.0].blocks {
                for instruction in &block.instructions {
                    match instruction.op {
                        Op::Call(callee, _) => pending.push(callee),
                        Op::LoadGlobal(global) | Op::StoreGlobal(global, _)
                            if !module.interrupt_globals.contains(&global) =>
                        {
                            return Err(failure(
                                ErrorKind::InterruptState,
                                Some(id),
                                Some(block.id),
                                "ISR global is missing volatile visibility metadata",
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

/// Source programs have a small, explicit startup wrapper. Initializer CFG lives
/// in its own function, so branching expressions cannot bypass lifecycle checks.
fn verify_lifecycle(module: &Module) -> Result<(), Error> {
    let Some(roles) = &module.lifecycle else {
        return Ok(());
    };
    let error = |message| failure(ErrorKind::InvalidEntry, None, None, message);
    let ids = [module.entry, roles.initialize, roles.setup, roles.loop_body];
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(error("lifecycle roles must be distinct"));
    }
    for id in ids {
        let function = module
            .functions
            .get(id.0)
            .ok_or_else(|| error("missing lifecycle function"))?;
        if !function.parameters.is_empty()
            || function.return_type.is_some()
            || function.requires_no_interrupt
        {
            return Err(error("lifecycle functions must be ordinary void functions"));
        }
    }
    let entry = &module.functions[module.entry.0];
    let first = entry
        .blocks
        .iter()
        .find(|b| b.id == entry.entry)
        .ok_or_else(|| error("missing startup block"))?;
    let Terminator::Jump(repeat_id) = first.terminator else {
        return Err(error("startup must lead to the main loop"));
    };
    let repeat = entry
        .blocks
        .iter()
        .find(|b| b.id == repeat_id)
        .ok_or_else(|| error("missing main loop block"))?;
    if entry.blocks.len() != 2
        || repeat_id == entry.entry
        || !matches!(repeat.terminator, Terminator::Jump(id) if id == repeat_id)
    {
        return Err(error("main loop must repeat without reentering startup"));
    }
    let is_call = |i: &Instruction, id| {
        i.result.is_none()
            && matches!(&i.op, Op::Call(callee, args) if *callee == id && args.is_empty())
    };
    let startup = &first.instructions;
    let expected = module.startup_operations(roles);
    if startup.len() != expected.len()
        || !startup.iter().zip(&expected).all(|(actual, expected)| {
            actual.result.is_none()
                && match (&actual.op, expected) {
                    (Op::InterruptDisable, Op::InterruptDisable)
                    | (Op::InterruptEnable, Op::InterruptEnable) => true,
                    (Op::DeviceStart(a), Op::DeviceStart(b)) => a == b,
                    (_, Op::Call(id, _)) => is_call(actual, *id),
                    _ => false,
                }
        })
        || repeat.instructions.len() != 1
        || !is_call(&repeat.instructions[0], roles.loop_body)
    {
        return Err(error(
            "expected interrupt mask, initialization, phased device starts, setup, IRQ enable, then loop",
        ));
    }
    // Lifecycle routines must not be invoked again by helpers or handlers.
    for function in &module.functions {
        for instruction in function.blocks.iter().flat_map(|b| &b.instructions) {
            if let Op::Call(id, _) = instruction.op {
                if id == module.entry || (ids.contains(&id) && function.id != module.entry) {
                    return Err(error("lifecycle call outside startup wrapper"));
                }
            }
        }
    }
    Ok(())
}

fn verify_devices(module: &Module) -> Result<(), Error> {
    use crate::resources::{HardwareResource, Reservations};
    let error = |message| failure(ErrorKind::InterruptState, None, None, message);
    if !module.devices.is_empty() && module.lifecycle.is_none() {
        return Err(error("devices require lifecycle roles"));
    }
    if module.interrupts.is_empty() && !module.interrupt_globals.is_empty() {
        return Err(error("interrupt globals without a handler"));
    }
    if module
        .interrupt_globals
        .iter()
        .any(|id| id.0 >= module.globals.len())
    {
        return Err(error("unknown interrupt global"));
    }
    let mut reservations = Reservations::default();
    for pin in &module.pins {
        reservations
            .claim(
                HardwareResource::Pin {
                    port: pin.port.clone(),
                    bit: pin.bit,
                },
                (),
            )
            .map_err(|_| error("duplicate physical pin"))?;
    }
    for (index, device) in module.devices.iter().enumerate() {
        match &device.config {
            DeviceConfig::UartTx(config) => {
                if device.phase != StartupPhase::BeforeSetup
                    || !crate::uart::plan(module.target, config.cpu_hz, u64::from(config.baud))
                        .is_ok_and(|expected| expected == *config)
                    || module
                        .interrupts
                        .iter()
                        .any(|b| b.device == DeviceId(index))
                {
                    return Err(error(
                        "invalid polling UART configuration, phase or interrupt binding",
                    ));
                }
                reservations
                    .claim(
                        HardwareResource::Peripheral(config.hardware.name.into()),
                        (),
                    )
                    .map_err(|_| error("hardware peripheral is already reserved"))?;
                reservations
                    .claim(
                        HardwareResource::Pin {
                            port: config.hardware.tx_port.into(),
                            bit: config.hardware.tx_bit,
                        },
                        (),
                    )
                    .map_err(|_| error("UART TX pin is already reserved"))?;
            }
            DeviceConfig::PeriodicTimer(timer) => {
                let valid = crate::timer::plan_with_direction(
                    module.target,
                    timer.cpu_hz,
                    timer.requested_ns,
                    timer.tolerance_ns,
                    timer.direction,
                )
                .is_ok_and(|expected| expected == *timer);
                if !valid || device.phase != StartupPhase::AfterSetup {
                    return Err(error(
                        "invalid periodic timer configuration or startup phase",
                    ));
                }
                reservations
                    .claim(HardwareResource::Peripheral(timer.hardware.name.into()), ())
                    .map_err(|_| error("hardware peripheral is already reserved"))?;
                let bindings: Vec<_> = module
                    .interrupts
                    .iter()
                    .filter(|binding| binding.device == DeviceId(index))
                    .collect();
                if bindings.len() != 1 || bindings[0].vector != timer.hardware.vector {
                    return Err(error(
                        "periodic timer requires its compare interrupt binding",
                    ));
                }
            }
        }
    }
    let mut handlers = HashSet::new();
    for binding in &module.interrupts {
        if binding.device.0 >= module.devices.len() {
            return Err(error("unknown interrupt device"));
        }
        reservations
            .claim(HardwareResource::InterruptVector(binding.vector.into()), ())
            .map_err(|_| error("interrupt vector is already reserved"))?;
        if !handlers.insert(binding.handler) {
            return Err(error("interrupt handler is bound more than once"));
        }
        let handler = module
            .functions
            .get(binding.handler.0)
            .ok_or_else(|| error("missing interrupt handler"))?;
        if binding.handler == module.entry
            || !handler.requires_no_interrupt
            || !handler.parameters.is_empty()
            || handler.return_type.is_some()
        {
            return Err(error(
                "interrupt handler must be a protected void function with no parameters",
            ));
        }
    }
    Ok(())
}
type Definitions = HashMap<ValueId, (Type, Option<(BlockId, usize)>)>;

fn verify_function(module: &Module, function: &Function) -> Result<(), Error> {
    let error = |kind, block, message| failure(kind, Some(function.id), block, message);
    let mut blocks = HashMap::new();
    let mut definitions: Definitions = HashMap::new();

    for &(value, ty) in &function.parameters {
        if value.function != function.id || definitions.insert(value, (ty, None)).is_some() {
            return Err(error(
                ErrorKind::DuplicateDefinition,
                None,
                "invalid or duplicate parameter ID",
            ));
        }
    }

    for block in &function.blocks {
        if block.id.function != function.id || blocks.insert(block.id, block).is_some() {
            return Err(error(
                ErrorKind::InvalidId,
                Some(block.id),
                "invalid or duplicate block ID",
            ));
        }
        for (position, instruction) in block.instructions.iter().enumerate() {
            if let Some((value, ty)) = instruction.result {
                if value.function != function.id
                    || definitions
                        .insert(value, (ty, Some((block.id, position))))
                        .is_some()
                {
                    return Err(error(
                        ErrorKind::DuplicateDefinition,
                        Some(block.id),
                        "invalid or duplicate value definition",
                    ));
                }
            }
        }
    }

    if !blocks.contains_key(&function.entry) {
        return Err(error(
            ErrorKind::InvalidEntry,
            None,
            "missing function entry block",
        ));
    }

    let mut predecessors: HashMap<BlockId, Vec<BlockId>> =
        blocks.keys().map(|&id| (id, vec![])).collect();
    for block in &function.blocks {
        for successor in block.terminator.successors() {
            let incoming = predecessors.get_mut(&successor).ok_or_else(|| {
                error(
                    ErrorKind::InvalidId,
                    Some(block.id),
                    "branch targets a missing or foreign block",
                )
            })?;
            incoming.push(block.id);
        }
    }

    let reachable = reachable(function);
    let mut dominators: HashMap<_, HashSet<_>> = reachable
        .iter()
        .map(|&id| {
            (
                id,
                if id == function.entry {
                    HashSet::from([id])
                } else {
                    reachable.clone()
                },
            )
        })
        .collect();

    loop {
        let mut changed = false;
        for block in &function.blocks {
            if block.id == function.entry || !reachable.contains(&block.id) {
                continue;
            }

            let mut incoming = predecessors[&block.id]
                .iter()
                .filter(|p| reachable.contains(p));
            let mut intersection =
                dominators[incoming.next().expect("reachable block has predecessor")].clone();
            for previous in incoming {
                intersection.retain(|id| dominators[previous].contains(id));
            }
            intersection.insert(block.id);
            if dominators[&block.id] != intersection {
                dominators.insert(block.id, intersection);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for block in &function.blocks {
        let value_type = |value: ValueId| -> Result<Type, Error> {
            definitions.get(&value).map(|(ty, _)| *ty).ok_or_else(|| {
                error(
                    ErrorKind::InvalidId,
                    Some(block.id),
                    "undefined or foreign value",
                )
            })
        };
        let local_type = |local: LocalId| -> Result<Type, Error> {
            if local.function != function.id {
                return Err(error(
                    ErrorKind::InvalidId,
                    Some(block.id),
                    "local belongs to another function",
                ));
            }
            function
                .locals
                .get(local.index)
                .map(|local| local.ty)
                .ok_or_else(|| error(ErrorKind::InvalidId, Some(block.id), "missing local"))
        };
        let global_type = |global: GlobalId| -> Result<Type, Error> {
            module
                .globals
                .get(global.0)
                .map(|global| global.ty)
                .ok_or_else(|| error(ErrorKind::InvalidId, Some(block.id), "missing global"))
        };
        let require = |valid, message| {
            if valid {
                Ok(())
            } else {
                Err(error(ErrorKind::TypeMismatch, Some(block.id), message))
            }
        };
        let available = |value: ValueId, position: usize| -> Result<(), Error> {
            value_type(value)?;
            if let Some((defined_block, defined_at)) = definitions[&value].1 {
                let valid = if defined_block == block.id {
                    defined_at < position
                } else {
                    !reachable.contains(&block.id) || dominators[&block.id].contains(&defined_block)
                };
                if !valid {
                    return Err(error(
                        ErrorKind::UnavailableValue,
                        Some(block.id),
                        "value definition does not dominate its use",
                    ));
                }
            }
            Ok(())
        };

        for (position, instruction) in block.instructions.iter().enumerate() {
            for operand in instruction.op.operands() {
                available(operand, position)?;
            }
            let expected = match &instruction.op {
                Op::ArrayLiteral(values) => {
                    let t = instruction.result.map(|(_, t)| t);
                    let Some(Type::Array(e, n)) = t else {
                        return Err(error(
                            ErrorKind::TypeMismatch,
                            Some(block.id),
                            "array literal result type",
                        ));
                    };
                    require(
                        values.len() == usize::from(n),
                        "array literal length mismatch",
                    )?;
                    for v in values {
                        require(
                            value_type(*v)? == Type::element(e),
                            "array element type mismatch",
                        )?;
                    }
                    t
                }
                Op::ArrayRepeat(v, n) => {
                    let t = instruction.result.map(|(_, t)| t);
                    let Some(Type::Array(e, m)) = t else {
                        return Err(error(
                            ErrorKind::TypeMismatch,
                            Some(block.id),
                            "array repeat result type",
                        ));
                    };
                    require(
                        *n == m && *n > 0 && value_type(*v)? == Type::element(e),
                        "array repeat mismatch",
                    )?;
                    t
                }
                Op::AddressLocal(l, m) => Some(local_type(*l)?.address(*m).ok_or_else(|| {
                    error(
                        ErrorKind::TypeMismatch,
                        Some(block.id),
                        "address requires sized value storage",
                    )
                })?),
                Op::AddressGlobal(g, m) => Some(global_type(*g)?.address(*m).ok_or_else(|| {
                    error(
                        ErrorKind::TypeMismatch,
                        Some(block.id),
                        "address requires sized value storage",
                    )
                })?),
                Op::ReadRef(r) | Op::WriteRef(r, _) => {
                    let (t, mutable) = value_type(*r)?.reference().ok_or_else(|| {
                        error(
                            ErrorKind::TypeMismatch,
                            Some(block.id),
                            "reference operation requires sized reference",
                        )
                    })?;
                    if let Op::WriteRef(_, v) = instruction.op {
                        require(
                            mutable && value_type(v)? == t,
                            "reference write requires mutable reference and matching value",
                        )?;
                        None
                    } else {
                        Some(t)
                    }
                }
                Op::ElementAddress(a, i, mutable) => {
                    let (Type::ArrayRef(e, _, old) | Type::SliceRef(e, old)) = value_type(*a)?
                    else {
                        return Err(error(
                            ErrorKind::TypeMismatch,
                            Some(block.id),
                            "element address requires sequence reference",
                        ));
                    };
                    require(!*mutable || old, "cannot upgrade shared reference")?;
                    require(value_type(*i)? == Type::U16, "element index must be u16")?;
                    Some(Type::Ref(e, *mutable))
                }
                Op::SliceLen(v) => {
                    require(
                        matches!(value_type(*v)?, Type::SliceRef(..)),
                        "slice length requires slice",
                    )?;
                    Some(Type::U16)
                }
                Op::Slice(v, m) => {
                    let (e, old) = match value_type(*v)? {
                        Type::ArrayRef(e, _, old) | Type::SliceRef(e, old) => (e, old),
                        _ => {
                            return Err(error(
                                ErrorKind::TypeMismatch,
                                Some(block.id),
                                "slice requires reference",
                            ))
                        }
                    };
                    require(!*m || old, "cannot upgrade shared reference")?;
                    Some(Type::SliceRef(e, *m))
                }
                Op::Reborrow(v, m) => {
                    let (t, old) = value_type(*v)?.reference().ok_or_else(|| {
                        error(
                            ErrorKind::TypeMismatch,
                            Some(block.id),
                            "reborrow requires sized reference",
                        )
                    })?;
                    require(!*m || old, "cannot upgrade shared reference")?;
                    t.address(*m)
                }
                Op::ArrayRead(a, i) | Op::ArrayWrite(a, i, _) => {
                    let (Type::ArrayRef(e, _, mutable) | Type::SliceRef(e, mutable)) =
                        value_type(*a)?
                    else {
                        return Err(error(
                            ErrorKind::TypeMismatch,
                            Some(block.id),
                            "array access requires reference",
                        ));
                    };
                    require(value_type(*i)? == Type::U16, "array index must be u16")?;
                    if let Op::ArrayWrite(_, _, v) = instruction.op {
                        require(
                            mutable && value_type(v)? == Type::element(e),
                            "array write requires mutable reference and matching element",
                        )?;
                        None
                    } else {
                        Some(Type::element(e))
                    }
                }
                Op::Intrinsic {
                    device,
                    operation,
                    arguments,
                } => {
                    require(
                        module
                            .devices
                            .get(device.0)
                            .is_some_and(|d| matches!(d.config, DeviceConfig::UartTx(_))),
                        "UART intrinsic requires a UART device",
                    )?;
                    let count = if *operation == crate::intrinsics::Intrinsic::UartFlush {
                        0
                    } else {
                        1
                    };
                    require(arguments.len() == count, "invalid intrinsic argument count")?;
                    for argument in arguments {
                        require(
                            value_type(*argument)? == Type::U8,
                            "UART byte must have type u8",
                        )?;
                    }
                    if *operation == crate::intrinsics::Intrinsic::UartTryWrite {
                        Some(Type::Bool)
                    } else {
                        None
                    }
                }
                Op::InterruptDisable | Op::InterruptEnable | Op::DeviceStart(_) => {
                    require(
                        function.id == module.entry && !module.devices.is_empty(),
                        "timer lifecycle effect outside entry",
                    )?;
                    None
                }
                Op::InterruptSave => Some(Type::U8),
                Op::InterruptRestore(value) => {
                    require(
                        value_type(*value)? == Type::U8,
                        "interrupt state must have type u8",
                    )?;
                    None
                }
                Op::TextLiteral { text, .. } => {
                    require(
                        text.len() <= u16::MAX as usize,
                        "text exceeds u16 byte length",
                    )?;
                    let ty = instruction.result.map(|(_, t)| t).unwrap_or(Type::Text);
                    let Op::TextLiteral { space, .. } = instruction.op else {
                        unreachable!()
                    };
                    require(
                        ty == Type::Text || ty == Type::TextIn(space),
                        "literal placement/type mismatch",
                    )?;
                    Some(ty)
                }
                Op::TextIsFlash(v) => {
                    require(
                        value_type(*v)? == Type::Text,
                        "text dispatch requires universal text",
                    )?;
                    Some(Type::Bool)
                }
                Op::TextErase(v) => {
                    require(
                        matches!(value_type(*v)?, Type::TextIn(_)),
                        "text erase requires concrete placement",
                    )?;
                    Some(Type::Text)
                }
                Op::TextNarrow(v, space) => {
                    require(
                        value_type(*v)? == Type::Text,
                        "text narrow requires universal text",
                    )?;
                    let guarded = function.blocks.iter().any(|test| {
                        let Terminator::Branch { condition, yes, no } = test.terminator else {
                            return false;
                        };
                        let target = if *space == TextSpace::Flash { yes } else { no };
                        yes != no
                            && target != function.entry
                            && predecessors[&target] == [test.id]
                            && dominators
                                .get(&block.id)
                                .is_some_and(|ds| ds.contains(&target))
                            && test.instructions.iter().any(|i| {
                                i.result == Some((condition, Type::Bool))
                                    && matches!(i.op, Op::TextIsFlash(arg) if arg == *v)
                            })
                    });
                    require(guarded, "text narrowing lacks a dominating placement guard")?;
                    Some(Type::TextIn(*space))
                }
                Op::TextLen(v) => {
                    require(value_type(*v)?.is_text(), "text_len requires text")?;
                    Some(Type::U16)
                }
                Op::TextByteOr {
                    text,
                    index,
                    fallback,
                } => {
                    require(
                        value_type(*text)?.is_text()
                            && value_type(*index)? == Type::U16
                            && value_type(*fallback)? == Type::U8,
                        "invalid text_byte_or operands",
                    )?;
                    Some(Type::U8)
                }
                Op::Const(value) => {
                    require(
                        value.ty.sequence().is_none() && value.ty.reference().is_none(),
                        "arrays and references cannot be scalar constants",
                    )?;
                    require(
                        !value.ty.is_text(),
                        "text cannot be forged from scalar bits",
                    )?;
                    require(
                        value.bits & !value.ty.mask() == 0,
                        "constant has a non-canonical bit pattern",
                    )?;
                    Some(value.ty)
                }
                Op::Unary(op, value) => {
                    let ty = value_type(*value)?;
                    require(
                        match op {
                            Unary::BitNot => ty.integer().is_some(),
                            Unary::Negate => ty.integer().is_some_and(|(_, signed)| signed),
                            Unary::Not => matches!(ty, Type::Bool | Type::PinState),
                        },
                        "invalid unary operand type",
                    )?;
                    Some(ty)
                }
                Op::Binary(op, left, right) => {
                    let ty = value_type(*left)?;
                    require(
                        if matches!(op, Binary::ShiftLeft | Binary::ShiftRight) {
                            value_type(*right)?.integer().is_some()
                        } else {
                            ty == value_type(*right)?
                        },
                        "binary operands must have identical types",
                    )?;
                    require(
                        !ty.is_text()
                            && ty.sequence().is_none()
                            && ty.reference().is_none()
                            && (ty.integer().is_some()
                                || matches!(op, Binary::Equal | Binary::NotEqual)),
                        "invalid binary operand type",
                    )?;
                    Some(if op.comparison() { Type::Bool } else { ty })
                }
                Op::NumericCast(value, target) => {
                    require(
                        value_type(*value)?.integer().is_some() && target.integer().is_some(),
                        "numeric cast requires integers",
                    )?;
                    Some(*target)
                }
                Op::Widen(value, target) => {
                    require(
                        !target.is_text()
                            && target.sequence().is_none()
                            && target.reference().is_none()
                            && value_type(*value)?.can_widen_to(*target),
                        "conversion is not lossless widening",
                    )?;
                    Some(*target)
                }
                Op::LoadLocal(local) => Some(local_type(*local)?),
                Op::StoreLocal(local, value) => {
                    require(
                        local_type(*local)? == value_type(*value)?,
                        "local store type mismatch",
                    )?;
                    None
                }
                Op::LoadGlobal(global) => Some(global_type(*global)?),
                Op::StoreGlobal(global, value) => {
                    require(
                        global_type(*global)? == value_type(*value)?,
                        "global store type mismatch",
                    )?;
                    None
                }
                Op::ReadPin(pin) | Op::WritePin(pin, _) | Op::SetPinMode(pin, _) => {
                    if module.pins.get(pin.0).is_none() {
                        return Err(error(ErrorKind::InvalidPin, Some(block.id), "unknown pin"));
                    }
                    if let Op::WritePin(_, value) = instruction.op {
                        require(
                            value_type(value)? == Type::PinState,
                            "pin write requires PinState",
                        )?;
                    }
                    if matches!(instruction.op, Op::ReadPin(_)) {
                        Some(Type::PinState)
                    } else {
                        None
                    }
                }
                Op::Call(callee, args) => {
                    require(
                        !module.is_interrupt_handler(*callee),
                        "interrupt handler cannot be called as a function",
                    )?;
                    let callee = module.functions.get(callee.0).ok_or_else(|| {
                        error(ErrorKind::InvalidId, Some(block.id), "unknown callee")
                    })?;
                    require(
                        args.len() == callee.parameters.len(),
                        "incorrect argument count",
                    )?;
                    for (arg, (_, ty)) in args.iter().zip(&callee.parameters) {
                        require(value_type(*arg)? == *ty, "argument type mismatch")?;
                    }
                    callee.return_type
                }
            };
            require(
                instruction.result.map(|(_, ty)| ty) == expected,
                "instruction result type mismatch",
            )?;
        }

        for value in block.terminator.operands() {
            available(value, block.instructions.len())?;
        }
        match block.terminator {
            Terminator::Branch { condition, .. } => require(
                value_type(condition)? == Type::Bool,
                "branch condition must be bool",
            )?,
            Terminator::Return(value) => require(
                value.map(value_type).transpose()? == function.return_type,
                "return type mismatch",
            )?,
            Terminator::Jump(_) => {}
        }
    }

    verify_initialization(function, &predecessors, &reachable)
}

/// Saved states are internal tokens: they may only restore their matching scope.
/// Every CFG edge must agree on the active stack, including loop back edges.
fn verify_interrupts(module: &Module, function: &Function) -> Result<(), Error> {
    let blocks: HashMap<_, _> = function.blocks.iter().map(|b| (b.id, b)).collect();
    let tokens: HashSet<_> = function
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter(|i| matches!(i.op, Op::InterruptSave))
        .filter_map(|i| i.result.map(|(id, _)| id))
        .collect();
    let error = |block, message| {
        failure(
            ErrorKind::InterruptState,
            Some(function.id),
            Some(block),
            message,
        )
    };

    for block in &function.blocks {
        for instruction in &block.instructions {
            if !matches!(instruction.op, Op::InterruptRestore(_))
                && instruction.op.operands().iter().any(|v| tokens.contains(v))
            {
                return Err(error(
                    block.id,
                    "saved interrupt state cannot be used as an ordinary value",
                ));
            }
        }
        if block
            .terminator
            .operands()
            .iter()
            .any(|v| tokens.contains(v))
        {
            return Err(error(
                block.id,
                "saved interrupt state cannot escape its function",
            ));
        }
    }

    let mut incoming = HashMap::from([(function.entry, Vec::<ValueId>::new())]);
    let mut pending = vec![function.entry];
    while let Some(id) = pending.pop() {
        let block = blocks[&id];
        let mut active = incoming[&id].clone();
        for instruction in &block.instructions {
            match instruction.op {
                Op::DeviceStart(_) | Op::InterruptEnable | Op::InterruptDisable
                    if !active.is_empty() =>
                {
                    return Err(error(
                        id,
                        "lifecycle interrupt operation inside a protected scope",
                    ));
                }
                Op::InterruptSave => active.push(instruction.result.unwrap().0),
                Op::InterruptRestore(saved) => {
                    if active.pop() != Some(saved) {
                        return Err(error(
                            id,
                            "interrupt restore must match the innermost active save",
                        ));
                    }
                }
                Op::Call(callee, _)
                    if module.functions[callee.0].requires_no_interrupt
                        && !function.requires_no_interrupt
                        && active.is_empty() =>
                {
                    return Err(error(id, "callee requires disabled interrupts"));
                }
                _ => {}
            }
        }
        if matches!(block.terminator, Terminator::Return(_)) && !active.is_empty() {
            return Err(error(id, "return leaves an interrupt scope unrestored"));
        }
        for successor in block.terminator.successors() {
            if let Some(previous) = incoming.get(&successor) {
                if *previous != active {
                    return Err(error(
                        successor,
                        "control-flow paths have different interrupt scopes",
                    ));
                }
            } else {
                incoming.insert(successor, active.clone());
                pending.push(successor);
            }
        }
    }
    Ok(())
}

pub(crate) fn reachable(function: &Function) -> HashSet<BlockId> {
    let blocks: HashMap<_, _> = function.blocks.iter().map(|b| (b.id, b)).collect();
    let mut found = HashSet::new();
    let mut pending = vec![function.entry];
    while let Some(id) = pending.pop() {
        if found.insert(id) {
            if let Some(block) = blocks.get(&id) {
                pending.extend(block.terminator.successors());
            }
        }
    }
    found
}

fn verify_initialization(
    function: &Function,
    predecessors: &HashMap<BlockId, Vec<BlockId>>,
    reachable: &HashSet<BlockId>,
) -> Result<(), Error> {
    let all: HashSet<usize> = (0..function.locals.len()).collect();
    let mut incoming: HashMap<_, _> = reachable.iter().map(|&id| (id, all.clone())).collect();
    let mut outgoing = incoming.clone();

    loop {
        let mut changed = false;
        for block in &function.blocks {
            if !reachable.contains(&block.id) {
                continue;
            }

            let mut initialized = if block.id == function.entry {
                HashSet::new()
            } else {
                all.clone()
            };
            for pred in &predecessors[&block.id] {
                if reachable.contains(pred) {
                    initialized.retain(|id| outgoing[pred].contains(id));
                }
            }
            incoming.insert(block.id, initialized.clone());
            for instruction in &block.instructions {
                if let Op::StoreLocal(local, _) = instruction.op {
                    initialized.insert(local.index);
                }
            }
            if outgoing[&block.id] != initialized {
                outgoing.insert(block.id, initialized);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for block in &function.blocks {
        if !reachable.contains(&block.id) {
            continue;
        }
        let mut initialized = incoming[&block.id].clone();
        for instruction in &block.instructions {
            match instruction.op {
                Op::LoadLocal(local) | Op::AddressLocal(local, _)
                    if !initialized.contains(&local.index) =>
                {
                    return Err(failure(
                        ErrorKind::UninitializedLocal,
                        Some(function.id),
                        Some(block.id),
                        "local may be read before initialization",
                    ));
                }
                Op::StoreLocal(local, _) => {
                    initialized.insert(local.index);
                }
                _ => {}
            }
        }
    }
    Ok(())
}
