//! Deterministic semantic test runner. It does not model AVR timing or electrical behavior.
use super::*;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Read(PinId, bool),
    Write(PinId, bool),
    Mode(PinId, PinMode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    StepLimit,
    CallDepthLimit,
    InvalidInvocation,
    InvalidIr(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

pub struct Interpreter<'a> {
    module: &'a Module,
    pub globals: Vec<Constant>,
    pub interrupts_enabled: bool,
    pub events: Vec<Event>,
    /// Scripted physical readings. Writes do not change this input stream.
    pub inputs: HashMap<PinId, VecDeque<bool>>,
    /// Physical level used when the scripted readings are exhausted; defaults to LOW.
    pub levels: HashMap<PinId, bool>,
    remaining: usize,
}

impl<'a> Interpreter<'a> {
    pub fn new(module: &'a Module, step_limit: usize) -> Result<Self, verify::Error> {
        verify::verify(module)?;

        Ok(Self {
            module,
            interrupts_enabled: false,
            globals: module
                .globals
                .iter()
                .map(|g| Constant::new(g.ty, 0))
                .collect(),
            events: vec![],
            inputs: HashMap::new(),
            levels: HashMap::new(),
            remaining: step_limit,
        })
    }

    pub fn run_entry(&mut self) -> Result<Option<Constant>, Error> {
        self.invoke(self.module.entry, &[])
    }

    pub fn invoke(
        &mut self,
        function: FunctionId,
        args: &[Constant],
    ) -> Result<Option<Constant>, Error> {
        self.call(function, args, 0)
    }

    fn tick(&mut self) -> Result<(), Error> {
        if self.remaining == 0 {
            return Err(Error::StepLimit);
        }

        self.remaining -= 1;
        Ok(())
    }

    fn call(
        &mut self,
        function: FunctionId,
        args: &[Constant],
        depth: usize,
    ) -> Result<Option<Constant>, Error> {
        if depth >= 64 {
            return Err(Error::CallDepthLimit);
        }

        let function = self
            .module
            .functions
            .get(function.0)
            .ok_or(Error::InvalidInvocation)?;
        if function.requires_no_interrupt && self.interrupts_enabled {
            return Err(Error::InvalidInvocation);
        }
        if args.len() != function.parameters.len()
            || args
                .iter()
                .zip(&function.parameters)
                .any(|(value, (_, ty))| value.ty != *ty || value.bits & !ty.mask() != 0)
        {
            return Err(Error::InvalidInvocation);
        }

        let mut values: HashMap<ValueId, Constant> = function
            .parameters
            .iter()
            .zip(args)
            .map(|((id, _), value)| (*id, *value))
            .collect();
        let mut locals = vec![None; function.locals.len()];
        let mut block_id = function.entry;

        loop {
            let block = function
                .blocks
                .iter()
                .find(|block| block.id == block_id)
                .ok_or_else(|| Error::InvalidIr("missing block".into()))?;
            for instruction in &block.instructions {
                self.tick()?;
                let get = |id: &ValueId| {
                    values
                        .get(id)
                        .copied()
                        .ok_or_else(|| Error::InvalidIr("unavailable value".into()))
                };
                let result = match &instruction.op {
                    Op::InterruptDisable => {
                        self.interrupts_enabled = false;
                        None
                    }
                    Op::InterruptEnable => {
                        self.interrupts_enabled = true;
                        None
                    }
                    Op::DeviceStart(_) => None, // Peripherals are modeled by machine tests.
                    Op::InterruptSave => {
                        let previous = self.interrupts_enabled;
                        self.interrupts_enabled = false;
                        Some(Constant::new(Type::U8, u64::from(previous)))
                    }
                    Op::InterruptRestore(value) => {
                        self.interrupts_enabled = get(value)?.bits != 0;
                        None
                    }
                    Op::Const(value) => Some(*value),
                    Op::Unary(op, value) => Some(eval_unary(*op, get(value)?)),
                    Op::Binary(op, left, right) => Some(eval_binary(*op, get(left)?, get(right)?)),
                    Op::Widen(value, target) => {
                        let value = get(value)?;
                        Some(Constant::new(
                            *target,
                            value.ty.numeric_value(value.bits) as u64,
                        ))
                    }
                    Op::LoadLocal(local) => Some(
                        locals[local.index]
                            .ok_or_else(|| Error::InvalidIr("uninitialized local".into()))?,
                    ),
                    Op::StoreLocal(local, value) => {
                        locals[local.index] = Some(get(value)?);
                        None
                    }
                    Op::LoadGlobal(global) => Some(self.globals[global.0]),
                    Op::StoreGlobal(global, value) => {
                        self.globals[global.0] = get(value)?;
                        None
                    }
                    Op::ReadPin(pin) => {
                        let level = self
                            .inputs
                            .get_mut(pin)
                            .and_then(VecDeque::pop_front)
                            .unwrap_or_else(|| self.levels.get(pin).copied().unwrap_or(false));
                        self.events.push(Event::Read(*pin, level));
                        Some(Constant::new(Type::PinState, u64::from(level)))
                    }
                    Op::WritePin(pin, value) => {
                        self.events.push(Event::Write(*pin, get(value)?.bits != 0));
                        None
                    }
                    Op::SetPinMode(pin, mode) => {
                        self.events.push(Event::Mode(*pin, *mode));
                        None
                    }
                    Op::Call(callee, args) => {
                        let args = args.iter().map(get).collect::<Result<Vec<_>, _>>()?;
                        self.call(*callee, &args, depth + 1)?
                    }
                };
                if let (Some((id, _)), Some(value)) = (instruction.result, result) {
                    values.insert(id, value);
                }
            }

            self.tick()?;
            match block.terminator {
                Terminator::Jump(next) => block_id = next,
                Terminator::Branch { condition, yes, no } => {
                    block_id = if values[&condition].bits != 0 {
                        yes
                    } else {
                        no
                    };
                }
                Terminator::Return(value) => return Ok(value.map(|id| values[&id])),
            }
        }
    }
}

pub(crate) fn eval_unary(op: Unary, value: Constant) -> Constant {
    let bits = match op {
        Unary::Negate => value.bits.wrapping_neg(),
        Unary::Not => u64::from(value.bits == 0),
    };
    Constant::new(value.ty, bits)
}

pub(crate) fn eval_binary(op: Binary, left: Constant, right: Constant) -> Constant {
    let a = left.ty.numeric_value(left.bits);
    let b = right.ty.numeric_value(right.bits);
    let bits = match op {
        Binary::Add => left.bits.wrapping_add(right.bits),
        Binary::Subtract => left.bits.wrapping_sub(right.bits),
        Binary::Multiply => left.bits.wrapping_mul(right.bits),
        Binary::Equal => u64::from(a == b),
        Binary::NotEqual => u64::from(a != b),
        Binary::Less => u64::from(a < b),
        Binary::LessEqual => u64::from(a <= b),
        Binary::Greater => u64::from(a > b),
        Binary::GreaterEqual => u64::from(a >= b),
    };
    Constant::new(if op.comparison() { Type::Bool } else { left.ty }, bits)
}
