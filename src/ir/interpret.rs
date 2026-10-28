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
    Bounds,
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

#[derive(Clone, Copy)]
enum Reference {
    Local(usize),
    Global(usize),
    Element(usize, usize),
}

pub struct Interpreter<'a> {
    module: &'a Module,
    pub globals: Vec<Constant>,
    pub interrupts_enabled: bool,
    pub events: Vec<Event>,
    /// Scripted physical readings. Writes do not change this input stream.
    pub inputs: HashMap<PinId, VecDeque<bool>>,
    /// Physical level used when the scripted readings are exhausted; defaults to LOW.
    pub levels: HashMap<PinId, bool>,
    texts: Vec<(String, TextSpace)>,
    arrays: Vec<Vec<Constant>>,
    cells: Vec<Constant>,
    references: Vec<Reference>,
    remaining: usize,
}

impl<'a> Interpreter<'a> {
    fn reference(&mut self, ty: Type, place: Reference) -> Constant {
        let id = self.references.len();
        self.references.push(place);
        Constant::new(ty, id as u64)
    }
    fn read_reference(&mut self, r: Constant) -> Result<Constant, Error> {
        if let Type::ArrayRef(e, n, _) = r.ty {
            return self.snapshot(Constant::new(Type::Array(e, n), r.bits));
        }
        match self
            .references
            .get(r.bits as usize)
            .ok_or(Error::InvalidInvocation)?
        {
            Reference::Local(i) => self.cells.get(*i),
            Reference::Global(i) => self.globals.get(*i),
            Reference::Element(a, i) => self.arrays.get(*a).and_then(|a| a.get(*i)),
        }
        .copied()
        .ok_or(Error::InvalidInvocation)
    }
    fn write_reference(&mut self, r: Constant, value: Constant) -> Result<(), Error> {
        if let Type::ArrayRef(e, n, _) = r.ty {
            self.store_value(Some(Constant::new(Type::Array(e, n), r.bits)), value)?;
            return Ok(());
        }
        let slot = match self
            .references
            .get(r.bits as usize)
            .ok_or(Error::InvalidInvocation)?
        {
            Reference::Local(i) => self.cells.get_mut(*i),
            Reference::Global(i) => self.globals.get_mut(*i),
            Reference::Element(a, i) => self.arrays.get_mut(*a).and_then(|a| a.get_mut(*i)),
        }
        .ok_or(Error::InvalidInvocation)?;
        *slot = value;
        Ok(())
    }

    fn array_value(&mut self, ty: Type, value: Vec<Constant>) -> Constant {
        let id = self.arrays.len();
        self.arrays.push(value);
        Constant::new(ty, id as u64)
    }
    fn snapshot(&mut self, value: Constant) -> Result<Constant, Error> {
        if matches!(value.ty, Type::Array(..)) {
            let a = self
                .arrays
                .get(value.bits as usize)
                .ok_or(Error::InvalidInvocation)?
                .clone();
            Ok(self.array_value(value.ty, a))
        } else {
            Ok(value)
        }
    }
    fn store_value(&mut self, old: Option<Constant>, value: Constant) -> Result<Constant, Error> {
        if matches!(value.ty, Type::Array(..)) {
            let data = self
                .arrays
                .get(value.bits as usize)
                .ok_or(Error::InvalidInvocation)?
                .clone();
            if let Some(old) = old {
                *self
                    .arrays
                    .get_mut(old.bits as usize)
                    .ok_or(Error::InvalidInvocation)? = data;
                Ok(old)
            } else {
                Ok(self.array_value(value.ty, data))
            }
        } else {
            Ok(value)
        }
    }
    pub fn new(module: &'a Module, step_limit: usize) -> Result<Self, verify::Error> {
        verify::verify(module)?;

        let mut arrays = vec![];
        let globals = module
            .globals
            .iter()
            .map(|g| {
                if let Type::Array(e, n) = g.ty {
                    let id = arrays.len();
                    arrays.push(vec![Constant::new(Type::element(e), 0); usize::from(n)]);
                    Constant::new(g.ty, id as u64)
                } else {
                    Constant::new(g.ty, 0)
                }
            })
            .collect();
        Ok(Self {
            arrays,
            cells: vec![],
            references: vec![],
            module,
            interrupts_enabled: false,
            globals,
            events: vec![],
            inputs: HashMap::new(),
            levels: HashMap::new(),
            remaining: step_limit,
            texts: vec![],
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
        let mut locals: Vec<Option<usize>> = vec![None; function.locals.len()];
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
                    Op::ReadRef(r) => Some(self.read_reference(get(r)?)?),
                    Op::WriteRef(r, v) => {
                        self.write_reference(get(r)?, get(v)?)?;
                        None
                    }
                    Op::ElementAddress(a, i, _) => {
                        let (a, i) = (get(a)?.bits as usize, get(i)?.bits as usize);
                        if self.arrays.get(a).and_then(|a| a.get(i)).is_none() {
                            return Err(Error::Bounds);
                        }
                        Some(
                            self.reference(instruction.result.unwrap().1, Reference::Element(a, i)),
                        )
                    }
                    Op::ArrayLiteral(v) => {
                        let a = v.iter().map(get).collect::<Result<Vec<_>, _>>()?;
                        Some(self.array_value(instruction.result.unwrap().1, a))
                    }
                    Op::ArrayRepeat(v, n) => Some(self.array_value(
                        instruction.result.unwrap().1,
                        vec![get(v)?; usize::from(*n)],
                    )),
                    Op::AddressLocal(l, _) => {
                        let v = locals[l.index]
                            .ok_or_else(|| Error::InvalidIr("uninitialized array".into()))?;
                        let ty = instruction.result.unwrap().1;
                        Some(if matches!(ty, Type::Ref(..)) {
                            self.reference(ty, Reference::Local(v))
                        } else {
                            Constant::new(ty, self.cells[v].bits)
                        })
                    }
                    Op::AddressGlobal(g, _) => {
                        let ty = instruction.result.unwrap().1;
                        Some(if matches!(ty, Type::Ref(..)) {
                            self.reference(ty, Reference::Global(g.0))
                        } else {
                            Constant::new(ty, self.globals[g.0].bits)
                        })
                    }
                    Op::SliceLen(v) => Some(Constant::new(
                        Type::U16,
                        self.arrays
                            .get(get(v)?.bits as usize)
                            .ok_or(Error::InvalidInvocation)?
                            .len() as u64,
                    )),
                    Op::Slice(v, _) | Op::Reborrow(v, _) => {
                        Some(Constant::new(instruction.result.unwrap().1, get(v)?.bits))
                    }
                    Op::ArrayRead(a, i) => Some(
                        *self
                            .arrays
                            .get(get(a)?.bits as usize)
                            .and_then(|a| a.get(get(i).ok()?.bits as usize))
                            .ok_or(Error::Bounds)?,
                    ),
                    Op::ArrayWrite(a, i, v) => {
                        let (a, i, v) = (get(a)?.bits as usize, get(i)?.bits as usize, get(v)?);
                        *self
                            .arrays
                            .get_mut(a)
                            .and_then(|a| a.get_mut(i))
                            .ok_or(Error::Bounds)? = v;
                        None
                    }
                    Op::InterruptDisable => {
                        self.interrupts_enabled = false;
                        None
                    }
                    Op::InterruptEnable => {
                        self.interrupts_enabled = true;
                        None
                    }
                    Op::Intrinsic { .. } => {
                        return Err(Error::InvalidIr(
                            "UART execution requires the machine simulator".into(),
                        ))
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
                    Op::TextLiteral { text, space } => {
                        let index = if let Some(i) = self
                            .texts
                            .iter()
                            .position(|s| s.0 == *text && s.1 == *space)
                        {
                            i
                        } else {
                            self.texts.push((text.clone(), *space));
                            self.texts.len() - 1
                        };
                        Some(Constant {
                            ty: instruction.result.unwrap().1,
                            bits: index as u64,
                        })
                    }
                    Op::TextIsFlash(v) => {
                        let text = self
                            .texts
                            .get(get(v)?.bits as usize)
                            .ok_or(Error::InvalidInvocation)?;
                        Some(Constant::new(
                            Type::Bool,
                            u64::from(text.1 == TextSpace::Flash),
                        ))
                    }
                    Op::TextErase(v) | Op::TextNarrow(v, _) => {
                        let mut value = get(v)?;
                        value.ty = instruction.result.unwrap().1;
                        Some(value)
                    }
                    Op::TextLen(v) => {
                        let text = self
                            .texts
                            .get(get(v)?.bits as usize)
                            .ok_or(Error::InvalidInvocation)?;
                        Some(Constant::new(Type::U16, text.0.len() as u64))
                    }
                    Op::TextByteOr {
                        text,
                        index,
                        fallback,
                    } => {
                        let text = self
                            .texts
                            .get(get(text)?.bits as usize)
                            .ok_or(Error::InvalidInvocation)?;
                        let byte = text
                            .0
                            .as_bytes()
                            .get(get(index)?.bits as usize)
                            .copied()
                            .unwrap_or(get(fallback)?.bits as u8);
                        Some(Constant::new(Type::U8, u64::from(byte)))
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
                        self.snapshot(
                            self.cells[locals[local.index]
                                .ok_or_else(|| Error::InvalidIr("uninitialized local".into()))?],
                        )?,
                    ),
                    Op::StoreLocal(local, value) => {
                        let old = locals[local.index].map(|i| self.cells[i]);
                        let value = self.store_value(old, get(value)?)?;
                        if let Some(i) = locals[local.index] {
                            self.cells[i] = value;
                        } else {
                            locals[local.index] = Some(self.cells.len());
                            self.cells.push(value);
                        }
                        None
                    }
                    Op::LoadGlobal(global) => Some(self.snapshot(self.globals[global.0])?),
                    Op::StoreGlobal(global, value) => {
                        self.globals[global.0] =
                            self.store_value(Some(self.globals[global.0]), get(value)?)?;
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
