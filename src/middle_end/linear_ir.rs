use crate::frontend::{
    expression::{Expression, ExpressionType as E, PinRef},
    statement::{Statement, StatementType as S},
    static_type::StaticType as T,
    token_type::{
        LiteralType as L, PinModeConstantType as Mode, PinStateConstantType as State,
        PunctuationType as P, TokenType as Token,
    },
};
use crate::{diagnostic::Diagnostic, Program};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};

type RegisterNumber = u32;
#[derive(Clone, Copy)]
enum Value {
    Constant(u64),
    Register(RegisterNumber),
}

/// The existing linear instruction set, without structural expression caching.
/// This inspection IR still lacks widths and is deliberately not used for general AVR arithmetic.
pub(crate) struct IRBuilder {
    instructions: Vec<IntermediateRepresentation>,
    locals: Vec<HashMap<String, RegisterNumber>>,
    pins: HashMap<String, PinRef>,
    next_register: u32,
    next_label: u32,
}
impl IRBuilder {
    pub fn new(_source: &str) -> Self {
        Self {
            instructions: vec![],
            locals: vec![],
            pins: HashMap::new(),
            next_register: 0,
            next_label: 0,
        }
    }
    fn push(&mut self, ir_type: IRType) {
        self.instructions
            .push(IntermediateRepresentation { ir_type });
    }
    fn register(&mut self) -> u32 {
        self.next_register += 1;
        self.next_register
    }
    fn label(&mut self, prefix: &str) -> String {
        self.next_label += 1;
        format!("{prefix}_{}", self.next_label)
    }
    fn materialize(&mut self, value: Value) -> u32 {
        match value {
            Value::Register(reg) => reg,
            Value::Constant(value) => {
                let reg = self.register();
                self.push(IRType::Load(LoadType::ImmediateInt(value), reg));
                reg
            }
        }
    }
    pub fn produce_ir(
        mut self,
        program: Program,
    ) -> Result<Vec<IntermediateRepresentation>, Diagnostic> {
        self.push(IRType::Label("setup".into()));
        for declaration in program.def_declarations {
            let instruction = match declaration.pin_mode {
                Mode::Input => IRType::SetInputPinMode(declaration.pin.clone()),
                Mode::Output => IRType::SetOutputPinMode(declaration.pin.clone()),
                _ => {
                    return Err(Diagnostic::new(
                        "E_UNSUPPORTED",
                        "Pull resistor configuration is not implemented",
                        Some(declaration.source_location),
                    ))
                }
            };
            self.push(instruction);
            self.pins.insert(declaration.identifier, declaration.pin);
        }
        for statement in program.global_var_declarations {
            if let S::VariableDeclaration {
                identifier,
                initializer: Some(expr),
                ..
            } = statement.statement_type
            {
                let value = self.value(&expr)?;
                let reg = self.materialize(value);
                self.push(IRType::Store(reg, identifier));
            }
        }
        self.block(&program.setup_block)?;
        self.push(IRType::Label("loop".into()));
        self.block(&program.loop_block)?;
        self.push(IRType::Jump("loop".into()));
        Ok(self.instructions)
    }
    fn block(&mut self, body: &[Statement]) -> Result<(), Diagnostic> {
        self.locals.push(HashMap::new());
        for statement in body {
            self.statement(statement)?;
        }
        self.locals.pop();
        Ok(())
    }
    fn statement(&mut self, statement: &Statement) -> Result<(), Diagnostic> {
        match &statement.statement_type {
            S::Expression(expr) => {
                self.value(expr)?;
            }
            S::VariableDeclaration {
                identifier,
                initializer: Some(expr),
                ..
            } => {
                let value = self.value(expr)?;
                let reg = self.materialize(value);
                self.locals
                    .last_mut()
                    .expect("block scope exists")
                    .insert(identifier.clone(), reg);
            }
            S::Block(body) => self.block(body)?,
            S::If {
                condition,
                then_body,
                else_body,
            } => {
                let value = self.value(condition)?;
                let reg = self.materialize(value);
                let otherwise = self.label("else");
                self.push(IRType::JumpIfZero(reg, otherwise.clone()));
                self.statement(then_body)?;
                if let Some(body) = else_body {
                    let end = self.label("end_if");
                    self.push(IRType::Jump(end.clone()));
                    self.push(IRType::Label(otherwise));
                    self.statement(body)?;
                    self.push(IRType::Label(end));
                } else {
                    self.push(IRType::Label(otherwise));
                }
            }
            _ => {
                return Err(Diagnostic::new(
                    "E_UNSUPPORTED",
                    "This statement is not supported by the legacy IR yet",
                    Some(statement.source_location),
                ))
            }
        }
        Ok(())
    }
    fn value(&mut self, expr: &Expression) -> Result<Value, Diagnostic> {
        let unsupported = || {
            Diagnostic::new(
                "E_UNSUPPORTED",
                "This expression is not supported by the legacy IR yet",
                Some(expr.source_location),
            )
        };
        Ok(match &expr.expression_type {
            E::Literal { value } => Value::Constant(match value {
                L::Integer(value) => *value,
                L::Boolean(value) => u64::from(*value),
                L::PinState(State::Low) | L::PinMode(Mode::Input) => 0,
                L::PinState(State::High) | L::PinMode(Mode::Output) => 1,
                _ => return Err(unsupported()),
            }),
            E::Grouping { expression } => return self.value(expression),
            E::Identifier(name) if expr.static_type != T::Pin => {
                let reg = self.register();
                if let Some(previous) = self
                    .locals
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(name))
                    .copied()
                {
                    // Each read is a value snapshot, even if a later operand writes this variable.
                    self.push(IRType::Move {
                        reg_from: previous,
                        reg_to: reg,
                    });
                } else {
                    self.push(IRType::Load(LoadType::Memory(name.clone()), reg));
                }
                Value::Register(reg)
            }
            E::Binary {
                left,
                operator: Token::Punctuation(P::Plus),
                right,
            } if matches!(expr.static_type, T::Integer(_)) => {
                let left = self.value(left)?;
                let a = self.materialize(left);
                let right = self.value(right)?;
                let b = self.materialize(right);
                let output = self.register();
                self.push(IRType::IntegerAdd { a, b, output });
                Value::Register(output)
            }
            E::Binary {
                left,
                operator: Token::Punctuation(P::EqualEqual),
                right,
            } => {
                let (pin_expr, state_expr) = if left.static_type == T::Pin {
                    (left, right)
                } else {
                    (right, left)
                };
                let name = pin_name(pin_expr).ok_or_else(unsupported)?;
                let pin = self.pins.get(name).cloned().ok_or_else(unsupported)?;
                let state = match &ungroup(state_expr).expression_type {
                    E::Literal {
                        value: L::PinState(state),
                    } => *state,
                    _ => return Err(unsupported()),
                };
                // Never reuse this read across expressions or branches.
                let reg = self.register();
                let yes = self.label("true");
                let end = self.label("end");
                self.push(if state == State::Low {
                    IRType::SkipIfPinHigh(pin)
                } else {
                    IRType::SkipIfPinLow(pin)
                });
                self.push(IRType::Jump(yes.clone()));
                self.push(IRType::Load(LoadType::ImmediateInt(0), reg));
                self.push(IRType::Jump(end.clone()));
                self.push(IRType::Label(yes));
                self.push(IRType::Load(LoadType::ImmediateInt(1), reg));
                self.push(IRType::Label(end));
                Value::Register(reg)
            }
            E::Assignment {
                identifier,
                expression,
            } => {
                let value = self.value(expression)?;
                if let Some(pin) = self.pins.get(identifier).cloned() {
                    let Value::Constant(constant) = value else {
                        return Err(unsupported());
                    };
                    let instruction = match (expr.static_type, constant) {
                        (T::PinState, 0) => IRType::SetPinLow(pin),
                        (T::PinState, 1) => IRType::SetPinHigh(pin),
                        (T::PinMode, 0) => IRType::SetInputPinMode(pin),
                        (T::PinMode, 1) => IRType::SetOutputPinMode(pin),
                        _ => return Err(unsupported()),
                    };
                    self.push(instruction);
                } else {
                    let reg = self.materialize(value);
                    if let Some(target) = self
                        .locals
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(identifier))
                        .copied()
                    {
                        self.push(IRType::Move {
                            reg_from: reg,
                            reg_to: target,
                        });
                    } else {
                        self.push(IRType::Store(reg, identifier.clone()));
                    }
                    return Ok(Value::Register(reg));
                }
                value
            }
            _ => return Err(unsupported()),
        })
    }
}
fn ungroup(mut expression: &Expression) -> &Expression {
    while let E::Grouping { expression: inner } = &expression.expression_type {
        expression = inner;
    }
    expression
}
fn pin_name(expression: &Expression) -> Option<&str> {
    match &ungroup(expression).expression_type {
        E::Identifier(name) => Some(name),
        _ => None,
    }
}
pub struct IntermediateRepresentation {
    pub ir_type: IRType,
}

impl Display for IntermediateRepresentation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.ir_type {
            IRType::Load(t, reg) => {
                match t {
                    LoadType::ImmediateInt(int) => write!(f, "LOAD ${int}")?,
                    LoadType::Memory(name) => write!(f, "LOAD {name}")?,
                    LoadType::DDR(port) => write!(f, "IN DDR{port}")?,
                    LoadType::Port(port) => write!(f, "IN Port{port}")?,
                }
                write!(f, " -> %r{reg}")
            }
            IRType::Store(reg, name) => write!(f, "STOR %r{reg} -> {name}"),
            IRType::Move { reg_from, reg_to } => write!(f, "MOV %r{reg_from} -> %r{reg_to}"),
            IRType::IntegerAdd { a, b, output } => write!(f, "IADD %r{a}, %r{b} -> %r{output}"),
            IRType::Label(label) => write!(f, "{}:", label),
            IRType::Jump(label) => write!(f, "JMP {}", label),
            IRType::AndImmediate(reg, num) => write!(f, "ANDI %r{reg}, {num:#010b} -> %r{reg}"),
            IRType::OrImmediate(reg, num) => write!(f, "ORI %r{reg}, {num:#010b} -> %r{reg}"),
            IRType::LogicalShiftLeft(reg) => write!(f, "LSL %r{reg} -> %r{reg}"),
            IRType::LogicalShiftRight(reg) => write!(f, "LSR %r{reg} -> %r{reg}"),
            IRType::JumpIfZero(reg, label) => write!(f, "JMPZero %r{reg}, {}", label),
            IRType::BranchIfEqual(label) => write!(f, "BREQ {}", label),
            IRType::SkipIfPinLow(pin_ref) => write!(f, "SkipIfPortBitClear {pin_ref}"),
            IRType::SkipIfPinHigh(pin_ref) => write!(f, "SkipIfPortBitSet {pin_ref}"),
            IRType::SetPinLow(pin_ref) => write!(f, "SetPortLow {pin_ref}"),
            IRType::SetPinHigh(pin_ref) => write!(f, "SetPortHigh {pin_ref}"),
            IRType::SetInputPinMode(pin_ref) => write!(f, "SetInputPortMode {pin_ref}"),
            IRType::SetOutputPinMode(pin_ref) => write!(f, "SetOutputPortMode {pin_ref}"),
        }
    }
}

pub enum IRType {
    Label(String),
    Jump(String),
    Load(LoadType, RegisterNumber),
    Store(RegisterNumber, String),
    Move {
        reg_from: RegisterNumber,
        reg_to: RegisterNumber,
    },
    IntegerAdd {
        a: RegisterNumber,
        b: RegisterNumber,
        output: RegisterNumber,
    },
    LogicalShiftLeft(RegisterNumber),
    LogicalShiftRight(RegisterNumber),
    SetPinLow(PinRef),
    SetPinHigh(PinRef),
    SetInputPinMode(PinRef),
    SetOutputPinMode(PinRef),
    AndImmediate(RegisterNumber, i8),
    OrImmediate(RegisterNumber, i8),
    JumpIfZero(RegisterNumber, String),
    BranchIfEqual(String),
    SkipIfPinLow(PinRef),
    SkipIfPinHigh(PinRef),
}

pub enum LoadType {
    ImmediateInt(u64),
    Memory(String),
    DDR(String),
    Port(String),
}
