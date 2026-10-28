//! Typed, target-independent IR. IDs denote values and storage, never AVR registers.
pub mod interpret;
mod lower;
pub mod optimize;
pub mod text;
pub mod verify;

pub use crate::frontend::static_type::ElementType;
use crate::frontend::utils::SourceLocation;
use std::fmt::{self, Write};

macro_rules! module_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub usize);
    };
}

module_id!(FunctionId);
module_id!(GlobalId);
module_id!(PinId);
module_id!(DeviceId);

macro_rules! function_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name {
            pub function: FunctionId,
            pub index: usize,
        }
    };
}

function_id!(BlockId);
function_id!(LocalId);
function_id!(ValueId);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Ref(ElementType, bool),
    Array(ElementType, u16),
    ArrayRef(ElementType, u16, bool),
    SliceRef(ElementType, bool),
    Text,
    TextIn(TextSpace),
    Bool,
    PinState,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
}

impl Type {
    pub fn reference(self) -> Option<(Self, bool)> {
        match self {
            Self::Ref(e, m) => Some((Self::element(e), m)),
            Self::ArrayRef(e, n, m) => Some((Self::Array(e, n), m)),
            _ => None,
        }
    }
    pub fn address(self, mutable: bool) -> Option<Self> {
        if let Self::Array(e, n) = self {
            return Some(Self::ArrayRef(e, n, mutable));
        }
        use crate::frontend::static_type::{
            IntegerType as I, SignedIntegerType as S, UnsignedIntegerType as U,
        };
        let e = match self {
            Self::Bool => ElementType::Boolean,
            Self::PinState => ElementType::PinState,
            Self::U8 => ElementType::Integer(I::Unsigned(U::Uns8)),
            Self::U16 => ElementType::Integer(I::Unsigned(U::Uns16)),
            Self::U32 => ElementType::Integer(I::Unsigned(U::Uns32)),
            Self::U64 => ElementType::Integer(I::Unsigned(U::Uns64)),
            Self::I8 => ElementType::Integer(I::Signed(S::Int8)),
            Self::I16 => ElementType::Integer(I::Signed(S::Int16)),
            Self::I32 => ElementType::Integer(I::Signed(S::Int32)),
            Self::I64 => ElementType::Integer(I::Signed(S::Int64)),
            _ => return None,
        };
        Some(Self::Ref(e, mutable))
    }

    pub fn sequence(self) -> Option<(ElementType, Option<u16>)> {
        if let Self::SliceRef(e, _) = self {
            Some((e, None))
        } else {
            self.array().map(|(e, n)| (e, Some(n)))
        }
    }
    pub fn array(self) -> Option<(ElementType, u16)> {
        match self {
            Self::Array(e, n) | Self::ArrayRef(e, n, _) => Some((e, n)),
            _ => None,
        }
    }
    pub fn element(e: ElementType) -> Self {
        use crate::frontend::static_type::{
            IntegerType as I, SignedIntegerType as S, UnsignedIntegerType as U,
        };
        match e {
            ElementType::Boolean => Self::Bool,
            ElementType::PinState => Self::PinState,
            ElementType::Integer(i) => match i {
                I::Signed(S::Int8) => Self::I8,
                I::Signed(S::Int16) => Self::I16,
                I::Signed(S::Int32) => Self::I32,
                I::Signed(S::Int64) => Self::I64,
                I::Unsigned(U::Uns8) => Self::U8,
                I::Unsigned(U::Uns16) => Self::U16,
                I::Unsigned(U::Uns32) => Self::U32,
                I::Unsigned(U::Uns64) => Self::U64,
                I::Literal => Self::I32,
            },
        }
    }
    pub fn is_text(self) -> bool {
        matches!(self, Self::Text | Self::TextIn(_))
    }

    pub fn integer(self) -> Option<(u32, bool)> {
        match self {
            Self::U8 => Some((8, false)),
            Self::U16 => Some((16, false)),
            Self::U32 => Some((32, false)),
            Self::U64 => Some((64, false)),
            Self::I8 => Some((8, true)),
            Self::I16 => Some((16, true)),
            Self::I32 => Some((32, true)),
            Self::I64 => Some((64, true)),
            _ => None,
        }
    }

    pub fn mask(self) -> u64 {
        match self.integer() {
            Some((64, _)) => u64::MAX,
            Some((bits, _)) => (1u64 << bits) - 1,
            None if self.is_text() || self.sequence().is_some() || self.reference().is_some() => {
                u64::MAX
            }
            None => 1,
        }
    }

    pub fn numeric_value(self, bits: u64) -> i128 {
        match self.integer() {
            Some((width, true)) if bits & (1u64 << (width - 1)) != 0 => {
                i128::from(bits) - (1i128 << width)
            }
            _ => i128::from(bits),
        }
    }

    pub fn can_widen_to(self, target: Self) -> bool {
        if self == target {
            return true;
        }

        match (self.integer(), target.integer()) {
            (Some((a, signed_a)), Some((b, signed_b))) => {
                (signed_a == signed_b && a < b) || (!signed_a && signed_b && a < b)
            }
            _ => false,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Ref(e, m) => return write!(f, "&{}{}", if *m { "mut " } else { "" }, e.ty()),
            Self::Array(e, n) => return write!(f, "[{}; {n}]", e.ty()),
            Self::SliceRef(e, m) => {
                return write!(f, "&{}[{}]", if *m { "mut " } else { "" }, e.ty())
            }
            Self::ArrayRef(e, n, m) => {
                return write!(f, "&{}[{}; {n}]", if *m { "mut " } else { "" }, e.ty())
            }
            Self::Text => "&str",
            Self::TextIn(TextSpace::Flash) => "&str[Flash]",
            Self::TextIn(TextSpace::Sram) => "&str[SRAM]",
            Self::Bool => "bool",
            Self::PinState => "PinState",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
        };
        f.write_str(name)
    }
}

/// Canonical bit pattern; signed integers use two's complement. No host overflow semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constant {
    pub ty: Type,
    pub bits: u64,
}

impl Constant {
    pub fn new(ty: Type, bits: u64) -> Self {
        Self {
            ty,
            bits: bits & ty.mask(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binary {
    Add,
    Subtract,
    Multiply,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl Binary {
    pub fn comparison(self) -> bool {
        !matches!(self, Self::Add | Self::Subtract | Self::Multiply)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unary {
    Negate,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMode {
    Input,
    Output,
    InputPullUp,
}

#[derive(Debug, Clone)]
pub enum Origin {
    Source(SourceLocation),
    Generated(&'static str),
}

#[derive(Debug, Clone)]
pub struct Storage {
    pub name: String,
    pub ty: Type,
    pub origin: Origin,
}

#[derive(Debug, Clone)]
pub struct Pin {
    pub name: String,
    pub port: String,
    pub bit: usize,
    pub origin: Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextSpace {
    Flash,
    Sram,
}

#[derive(Debug, Clone)]
pub enum Op {
    ReadRef(ValueId),
    WriteRef(ValueId, ValueId),
    ElementAddress(ValueId, ValueId, bool),
    ArrayLiteral(Vec<ValueId>),
    ArrayRepeat(ValueId, u16),
    AddressLocal(LocalId, bool),
    AddressGlobal(GlobalId, bool),
    Reborrow(ValueId, bool),
    Slice(ValueId, bool),
    SliceLen(ValueId),
    ArrayRead(ValueId, ValueId),
    ArrayWrite(ValueId, ValueId, ValueId),
    /// Immutable UTF-8 storage. Source literals always use Flash; SRAM is also
    /// available to IR producers and machine fixtures. No raw pointers are exposed.
    TextLiteral {
        text: String,
        space: TextSpace,
    },
    TextLen(ValueId),
    TextIsFlash(ValueId),
    TextNarrow(ValueId, TextSpace),
    TextErase(ValueId),
    TextByteOr {
        text: ValueId,
        index: ValueId,
        fallback: ValueId,
    },
    Intrinsic {
        device: DeviceId,
        operation: crate::intrinsics::Intrinsic,
        arguments: Vec<ValueId>,
    },
    /// Lifecycle-only effects, not exposed as source-language operations.
    InterruptDisable,
    DeviceStart(DeviceId),
    InterruptEnable,
    /// Save target interrupt state and disable interrupts; an ordered memory effect.
    InterruptSave,
    /// Restore the matching saved state, not unconditionally enable interrupts.
    InterruptRestore(ValueId),
    Const(Constant),
    Unary(Unary, ValueId),
    Binary(Binary, ValueId, ValueId),
    Widen(ValueId, Type),
    LoadLocal(LocalId),
    StoreLocal(LocalId, ValueId),
    LoadGlobal(GlobalId),
    StoreGlobal(GlobalId, ValueId),
    ReadPin(PinId),
    WritePin(PinId, ValueId),
    SetPinMode(PinId, PinMode),
    Call(FunctionId, Vec<ValueId>),
}

impl Op {
    pub fn operands(&self) -> Vec<ValueId> {
        match self {
            Self::ArrayLiteral(elements) => elements.clone(),
            Self::ArrayRepeat(v, _)
            | Self::Reborrow(v, _)
            | Self::Slice(v, _)
            | Self::SliceLen(v) => vec![*v],
            Self::ReadRef(v) => vec![*v],
            Self::WriteRef(a, b) | Self::ElementAddress(a, b, _) => vec![*a, *b],
            Self::ArrayRead(a, b) => vec![*a, *b],
            Self::ArrayWrite(a, b, c) => vec![*a, *b, *c],
            Self::TextLen(v)
            | Self::TextIsFlash(v)
            | Self::TextNarrow(v, _)
            | Self::TextErase(v) => vec![*v],
            Self::TextByteOr {
                text,
                index,
                fallback,
            } => vec![*text, *index, *fallback],
            Self::Intrinsic { arguments, .. } => arguments.clone(),
            Self::InterruptRestore(a)
            | Self::Unary(_, a)
            | Self::Widen(a, _)
            | Self::StoreLocal(_, a)
            | Self::StoreGlobal(_, a)
            | Self::WritePin(_, a) => vec![*a],
            Self::Binary(_, a, b) => vec![*a, *b],
            Self::Call(_, args) => args.clone(),
            _ => vec![],
        }
    }

    /// Reads of hardware and every call are effects even when their result is unused.
    pub fn is_pure(&self) -> bool {
        matches!(
            self,
            Self::Const(_)
                | Self::Unary(..)
                | Self::Binary(..)
                | Self::Widen(..)
                | Self::LoadLocal(_)
        )
    }
}

#[derive(Debug, Clone)]
pub struct Instruction {
    pub result: Option<(ValueId, Type)>,
    pub op: Op,
    pub origin: Origin,
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Jump(BlockId),
    Branch {
        condition: ValueId,
        yes: BlockId,
        no: BlockId,
    },
    Return(Option<ValueId>),
}

impl Terminator {
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Self::Jump(block) => vec![*block],
            Self::Branch { yes, no, .. } => vec![*yes, *no],
            Self::Return(_) => vec![],
        }
    }

    pub fn operands(&self) -> Vec<ValueId> {
        match self {
            Self::Branch { condition, .. } => vec![*condition],
            Self::Return(Some(value)) => vec![*value],
            _ => vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub id: BlockId,
    pub instructions: Vec<Instruction>,
    pub terminator: Terminator,
    pub origin: Origin,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub requires_no_interrupt: bool,
    pub id: FunctionId,
    pub name: String,
    pub parameters: Vec<(ValueId, Type)>,
    /// None is void; void is never a value or a storage type.
    pub return_type: Option<Type>,
    pub locals: Vec<Storage>,
    pub blocks: Vec<Block>,
    pub entry: BlockId,
    pub origin: Origin,
}

#[derive(Debug, Clone)]
pub struct Lifecycle {
    pub initialize: FunctionId,
    pub setup: FunctionId,
    pub loop_body: FunctionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPhase {
    BeforeSetup,
    AfterSetup,
}

#[derive(Debug, Clone)]
pub enum DeviceConfig {
    PeriodicTimer(crate::timer::TimerConfig),
    UartTx(crate::uart::UartConfig),
}

#[derive(Debug, Clone)]
pub struct Device {
    pub config: DeviceConfig,
    pub phase: StartupPhase,
    pub origin: Origin,
}

impl Device {
    pub fn timer(&self) -> Option<&crate::timer::TimerConfig> {
        match &self.config {
            DeviceConfig::PeriodicTimer(config) => Some(config),
            _ => None,
        }
    }
    pub fn timer_mut(&mut self) -> Option<&mut crate::timer::TimerConfig> {
        match &mut self.config {
            DeviceConfig::PeriodicTimer(config) => Some(config),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InterruptBinding {
    pub device: DeviceId,
    pub vector: &'static str,
    pub handler: FunctionId,
}

#[derive(Debug, Clone)]
pub struct Module {
    pub warnings: Vec<crate::diagnostic::Diagnostic>,
    /// Present for source programs; absent for standalone IR test functions.
    pub lifecycle: Option<Lifecycle>,
    pub devices: Vec<Device>,
    pub interrupts: Vec<InterruptBinding>,
    pub interrupt_globals: std::collections::HashSet<GlobalId>,
    pub target: &'static crate::backend::microcontroller::Microcontroller,
    pub globals: Vec<Storage>,
    pub pins: Vec<Pin>,
    pub functions: Vec<Function>,
    pub entry: FunctionId,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub cpu_hz: Option<u32>,
    pub verify: bool,
    pub optimize: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cpu_hz: None,
            verify: true,
            optimize: false,
        }
    }
}

pub(crate) use lower::lower;

impl Module {
    pub fn is_interrupt_handler(&self, id: FunctionId) -> bool {
        self.interrupts.iter().any(|binding| binding.handler == id)
    }

    /// One explicit startup sequence for lowering and verification. Device
    /// configuration never changes the CPU interrupt-enable state itself.
    pub(crate) fn startup_operations(&self, roles: &Lifecycle) -> Vec<Op> {
        let mut operations = Vec::new();
        if !self.devices.is_empty() {
            operations.push(Op::InterruptDisable);
        }
        operations.push(Op::Call(roles.initialize, vec![]));
        for phase in [StartupPhase::BeforeSetup, StartupPhase::AfterSetup] {
            if phase == StartupPhase::AfterSetup {
                operations.push(Op::Call(roles.setup, vec![]));
            }
            for (id, device) in self.devices.iter().enumerate() {
                if device.phase == phase {
                    operations.push(Op::DeviceStart(DeviceId(id)));
                }
            }
        }
        if !self.interrupts.is_empty() {
            operations.push(Op::InterruptEnable);
        }
        operations
    }

    /// Stable ordering, no addresses or hash-map iteration in the inspection output.
    pub fn dump(&self) -> String {
        let mut text = format!(
            "helium-ir v1 target={} entry=f{}\n",
            self.target.name, self.entry.0
        );
        for (id, device) in self.devices.iter().enumerate() {
            writeln!(
                text,
                "device d{id} {:?}: {}",
                device.phase,
                match &device.config {
                    DeviceConfig::PeriodicTimer(c) => c.report(),
                    DeviceConfig::UartTx(c) => c.report(),
                }
            )
            .unwrap();
        }
        for binding in &self.interrupts {
            writeln!(
                text,
                "interrupt {} device=d{} handler=f{}",
                binding.vector, binding.device.0, binding.handler.0
            )
            .unwrap();
        }
        for (index, global) in self.globals.iter().enumerate() {
            writeln!(text, "global g{index}: {} {}", global.ty, global.name).unwrap();
        }
        for (index, pin) in self.pins.iter().enumerate() {
            writeln!(
                text,
                "pin p{index}: {} = Port{}.{}",
                pin.name, pin.port, pin.bit
            )
            .unwrap();
        }

        for function in &self.functions {
            if function.requires_no_interrupt {
                writeln!(text, "requires noInterrupt").unwrap();
            }
            let parameters = function
                .parameters
                .iter()
                .map(|(id, ty)| format!("v{}: {ty}", id.index))
                .collect::<Vec<_>>()
                .join(", ");
            let result = function
                .return_type
                .map(|ty| ty.to_string())
                .unwrap_or_else(|| "void".into());
            writeln!(
                text,
                "fn f{} {}({parameters}) -> {result} entry=b{} {{",
                function.id.0, function.name, function.entry.index
            )
            .unwrap();
            for (index, local) in function.locals.iter().enumerate() {
                writeln!(text, "  local l{index}: {} {}", local.ty, local.name).unwrap();
            }
            for block in &function.blocks {
                writeln!(text, "  b{}:", block.id.index).unwrap();
                for instruction in &block.instructions {
                    match instruction.result {
                        Some((value, ty)) => write!(text, "    v{}: {ty} = ", value.index).unwrap(),
                        None => text.push_str("    "),
                    }
                    writeln!(text, "{}", instruction.op).unwrap();
                }
                writeln!(text, "    {}", block.terminator).unwrap();
            }
            text.push_str("}\n");
        }
        text
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArrayLiteral(v) => write!(f, "array {v:?}"),
            Self::ArrayRepeat(v, n) => write!(f, "array_repeat {v:?}; {n}"),
            Self::AddressLocal(l, m) => write!(f, "address_local {l:?} mutable={m}"),
            Self::AddressGlobal(g, m) => write!(f, "address_global {g:?} mutable={m}"),
            Self::Slice(v, m) => write!(f, "slice {v:?} mutable={m}"),
            Self::SliceLen(v) => write!(f, "slice_len {v:?}"),
            Self::Reborrow(v, m) => write!(f, "reborrow {v:?} mutable={m}"),
            Self::ReadRef(v) => write!(f, "read_ref {v:?}"),
            Self::WriteRef(r, v) => write!(f, "write_ref {r:?}, {v:?}"),
            Self::ElementAddress(a, i, m) => write!(f, "element_address {a:?}, {i:?}, {m}"),
            Self::ArrayRead(a, i) => write!(f, "array_read {a:?}, {i:?}"),
            Self::ArrayWrite(a, i, v) => write!(f, "array_write {a:?}, {i:?}, {v:?}"),
            Self::TextLiteral { text, space } => write!(f, "text {space:?} {text:?}"),
            Self::TextIsFlash(v) => write!(f, "text_is_flash v{}", v.index),
            Self::TextNarrow(v, space) => write!(f, "text_narrow v{} {space:?}", v.index),
            Self::TextErase(v) => write!(f, "text_erase v{}", v.index),
            Self::TextLen(v) => write!(f, "text_len v{}", v.index),
            Self::TextByteOr {
                text,
                index,
                fallback,
            } => write!(
                f,
                "text_byte_or v{}, v{}, v{}",
                text.index, index.index, fallback.index
            ),
            Self::InterruptDisable => f.write_str("interrupt_disable"),
            Self::Intrinsic {
                device,
                operation,
                arguments,
            } => write!(f, "intrinsic d{} {:?} {:?}", device.0, operation, arguments),
            Self::DeviceStart(id) => write!(f, "device_start d{}", id.0),
            Self::InterruptEnable => f.write_str("interrupt_enable"),
            Self::InterruptSave => f.write_str("interrupt_save"),
            Self::InterruptRestore(value) => write!(f, "interrupt_restore v{}", value.index),
            Self::Const(value) => match value.ty {
                Type::Bool => write!(f, "const {}", value.bits != 0),
                Type::PinState => {
                    write!(f, "const {}", if value.bits == 0 { "LOW" } else { "HIGH" })
                }
                _ => write!(f, "const {}", value.ty.numeric_value(value.bits)),
            },
            Self::Unary(op, value) => write!(
                f,
                "{} v{}",
                match op {
                    Unary::Negate => "neg",
                    Unary::Not => "not",
                },
                value.index
            ),
            Self::Binary(op, left, right) => {
                let name = match op {
                    Binary::Add => "add",
                    Binary::Subtract => "sub",
                    Binary::Multiply => "mul",
                    Binary::Equal => "eq",
                    Binary::NotEqual => "ne",
                    Binary::Less => "lt",
                    Binary::LessEqual => "le",
                    Binary::Greater => "gt",
                    Binary::GreaterEqual => "ge",
                };
                write!(f, "{name} v{}, v{}", left.index, right.index)
            }
            Self::Widen(value, target) => write!(f, "widen v{} to {target}", value.index),
            Self::LoadLocal(local) => write!(f, "load_local l{}", local.index),
            Self::StoreLocal(local, value) => {
                write!(f, "store_local l{}, v{}", local.index, value.index)
            }
            Self::LoadGlobal(global) => write!(f, "load_global g{}", global.0),
            Self::StoreGlobal(global, value) => {
                write!(f, "store_global g{}, v{}", global.0, value.index)
            }
            Self::ReadPin(pin) => write!(f, "read_pin p{}", pin.0),
            Self::WritePin(pin, value) => write!(f, "write_pin p{}, v{}", pin.0, value.index),
            Self::SetPinMode(pin, mode) => write!(
                f,
                "set_pin_mode p{}, {}",
                pin.0,
                match mode {
                    PinMode::Input => "INPUT",
                    PinMode::Output => "OUTPUT",
                    PinMode::InputPullUp => "INPUT_PULLUP",
                }
            ),
            Self::Call(function, args) => {
                let args = args
                    .iter()
                    .map(|v| format!("v{}", v.index))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "call f{}({args})", function.0)
            }
        }
    }
}

impl fmt::Display for Terminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Jump(block) => write!(f, "jump b{}", block.index),
            Self::Branch { condition, yes, no } => write!(
                f,
                "branch v{}, b{}, b{}",
                condition.index, yes.index, no.index
            ),
            Self::Return(Some(value)) => write!(f, "return v{}", value.index),
            Self::Return(None) => f.write_str("return"),
        }
    }
}
