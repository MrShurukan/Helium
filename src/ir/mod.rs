//! Typed, target-independent IR. IDs denote values and storage, never AVR registers.
pub mod interpret;
mod lower;
pub mod optimize;
pub mod verify;

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

#[derive(Debug, Clone)]
pub enum Op {
    /// Lifecycle-only effects, not exposed as source-language operations.
    InterruptDisable,
    TimerStart,
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
pub struct Module {
    pub timer: Option<crate::timer::TimerConfig>,
    pub interrupt_handler: Option<FunctionId>,
    pub interrupt_globals: std::collections::HashSet<GlobalId>,
    pub target: String,
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
            verify: cfg!(debug_assertions),
            optimize: false,
        }
    }
}

pub(crate) use lower::lower;

impl Module {
    /// Stable ordering, no addresses or hash-map iteration in the inspection output.
    pub fn dump(&self) -> String {
        let mut text = format!(
            "helium-ir v1 target={} entry=f{}\n",
            self.target, self.entry.0
        );
        if let Some(timer) = &self.timer {
            writeln!(
                text,
                "{} handler=f{}",
                timer.report(),
                self.interrupt_handler.unwrap().0
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
            Self::InterruptDisable => f.write_str("interrupt_disable"),
            Self::TimerStart => f.write_str("timer_start"),
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
