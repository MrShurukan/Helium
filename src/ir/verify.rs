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
    let entry = module
        .functions
        .get(module.entry.0)
        .ok_or_else(|| failure(ErrorKind::InvalidEntry, None, None, "missing module entry"))?;
    if !entry.parameters.is_empty() || entry.return_type.is_some() {
        return Err(failure(
            ErrorKind::InvalidEntry,
            None,
            None,
            "entry must have no parameters and return void",
        ));
    }

    let target = crate::backend::microcontroller::get(&module.target)
        .ok_or_else(|| failure(ErrorKind::InvalidPin, None, None, "unknown target"))?;
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
                Op::Const(value) => {
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
                        ty == value_type(*right)?,
                        "binary operands must have identical types",
                    )?;
                    require(
                        ty.integer().is_some() || matches!(op, Binary::Equal | Binary::NotEqual),
                        "invalid binary operand type",
                    )?;
                    Some(if op.comparison() { Type::Bool } else { ty })
                }
                Op::Widen(value, target) => {
                    require(
                        value_type(*value)?.can_widen_to(*target),
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
                Op::LoadLocal(local) if !initialized.contains(&local.index) => {
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
