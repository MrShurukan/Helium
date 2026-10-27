use super::*;
use crate::diagnostic::{Diagnostic, DiagnosticKind};
use crate::frontend::{
    expression::{Expression, ExpressionType as E},
    statement::{Statement, StatementType as S},
    static_type::{
        IntegerType as I, SignedIntegerType as SI, StaticType as T, UnsignedIntegerType as UI,
    },
    token_type::{
        KeywordType as K, LiteralType as L, PinModeConstantType as Mode,
        PinStateConstantType as State, PunctuationType as P, TokenType as Token,
    },
};
use crate::semantic::{AnalyzedProgram, SymbolId};
use std::collections::HashMap;

fn unsupported(feature: &str, location: SourceLocation) -> Diagnostic {
    Diagnostic::new(
        DiagnosticKind::TypedIrUnsupported {
            feature: feature.into(),
        },
        Some(location),
    )
}

fn invalid(detail: &str, location: SourceLocation) -> Diagnostic {
    Diagnostic::new(
        DiagnosticKind::InvalidIr {
            stage: "typed lowering",
            detail: detail.into(),
        },
        Some(location),
    )
}

fn ty(ty: T, location: SourceLocation) -> Result<Type, Diagnostic> {
    Ok(match ty {
        T::Text => Type::Text,
        T::Boolean => Type::Bool,
        T::PinState => Type::PinState,
        T::Integer(integer) => match integer {
            I::Literal => return Err(invalid("unresolved integer literal type", location)),
            I::Signed(SI::Int32) => Type::I32,
            I::Signed(SI::Int8) => Type::I8,
            I::Signed(SI::Int16) => Type::I16,
            I::Signed(SI::Int64) => Type::I64,
            I::Unsigned(UI::Uns8) => Type::U8,
            I::Unsigned(UI::Uns16) => Type::U16,
            I::Unsigned(UI::Uns32) => Type::U32,
            I::Unsigned(UI::Uns64) => Type::U64,
        },
        _ => return Err(unsupported(&format!("type {ty}"), location)),
    })
}

fn mode(mode: &Mode, location: SourceLocation) -> Result<PinMode, Diagnostic> {
    match mode {
        Mode::Input => Ok(PinMode::Input),
        Mode::Output => Ok(PinMode::Output),
        Mode::InputPullUp => Ok(PinMode::InputPullUp),
        Mode::InputPullDown => Err(unsupported("internal pull-down", location)),
    }
}

// The analyzer checks the whole signed literal, including -128:i8. Its positive
// token need not fit the final type and must not become a narrowing runtime cast.
fn integer_literal(expr: &Expression) -> Option<i128> {
    match &expr.expression_type {
        E::Literal {
            value: L::Integer(value),
        } => Some(i128::from(*value)),
        E::Grouping { expression } => integer_literal(expression),
        E::Unary {
            operator: P::Minus,
            right,
            ..
        } => integer_literal(right)?.checked_neg(),
        _ => None,
    }
}

fn checked_integer(
    value: i128,
    expected: Type,
    location: SourceLocation,
) -> Result<Constant, Diagnostic> {
    let (width, signed) = expected.integer().expect("integer context");
    let (min, max) = if signed {
        (-(1i128 << (width - 1)), (1i128 << (width - 1)) - 1)
    } else {
        (0, (1i128 << width) - 1)
    };

    if !(min..=max).contains(&value) {
        let integer = match expected {
            Type::U8 => I::Unsigned(UI::Uns8),
            Type::U16 => I::Unsigned(UI::Uns16),
            Type::U32 => I::Unsigned(UI::Uns32),
            Type::U64 => I::Unsigned(UI::Uns64),
            Type::I8 => I::Signed(SI::Int8),
            Type::I16 => I::Signed(SI::Int16),
            Type::I32 => I::Signed(SI::Int32),
            Type::I64 => I::Signed(SI::Int64),
            _ => unreachable!("integer context"),
        };
        return Err(Diagnostic::new(
            DiagnosticKind::IntegerRange {
                value,
                target: T::Integer(integer),
                min,
                max,
            },
            Some(location),
        ));
    }

    Ok(Constant::new(expected, value as u64))
}

pub(crate) fn lower(
    program: &AnalyzedProgram,
    target: &'static crate::backend::microcontroller::Microcontroller,
    timer_config: Option<crate::timer::TimerConfig>,
    uarts: Vec<(SymbolId, crate::uart::UartConfig, SourceLocation)>,
) -> Result<Module, Diagnostic> {
    let statements = program.statements();
    let mut module = Module {
        warnings: program.warnings().to_vec(),
        lifecycle: None,
        devices: Vec::new(),
        interrupts: Vec::new(),
        interrupt_globals: Default::default(),
        target,
        globals: vec![],
        pins: vec![],
        functions: vec![],
        entry: FunctionId(0),
    };
    let mut devices = HashMap::new();
    for (symbol, config, location) in uarts {
        devices.insert(symbol, DeviceId(module.devices.len()));
        module.devices.push(Device {
            config: DeviceConfig::UartTx(config),
            phase: StartupPhase::BeforeSetup,
            origin: Origin::Source(location),
        });
    }
    let intrinsics = program
        .symbols()
        .iter()
        .filter_map(|(id, symbol)| {
            if let crate::semantic::SymbolKind::Intrinsic { device, operation } = symbol.kind {
                devices
                    .get(&device)
                    .map(|device| (id, (*device, operation)))
            } else {
                None
            }
        })
        .collect::<HashMap<_, _>>();
    let mut globals = HashMap::new();
    let mut pins = HashMap::new();
    let mut setup = &[][..];
    let mut loop_body = &[][..];
    let mut functions = HashMap::new();

    for statement in statements {
        if let S::FunctionDeclaration { return_type, .. } = &statement.statement_type {
            let id = FunctionId(functions.len() + 3);
            let result = return_type
                .map(|t| ty(t, statement.source_location))
                .transpose()?;
            functions.insert(statement.symbol.unwrap(), (id, result));
        }
    }

    for statement in statements {
        match &statement.statement_type {
            S::VariableDeclaration {
                identifier,
                static_type,
                ..
            } => {
                let global = GlobalId(module.globals.len());
                let ty = ty(static_type.unwrap_or(T::Unknown), statement.source_location)?;
                globals.insert(statement.symbol.unwrap(), (global, ty));
                module.globals.push(Storage {
                    name: identifier.clone(),
                    ty,
                    origin: Origin::Source(statement.source_location),
                });
            }
            S::DefDeclaration(declarations) => {
                for declaration in declarations {
                    let pin = PinId(module.pins.len());
                    pins.insert(declaration.symbol.unwrap(), pin);
                    module.pins.push(Pin {
                        name: declaration.identifier.clone(),
                        port: declaration.pin.port.clone(),
                        bit: declaration.pin.pin_number,
                        origin: Origin::Source(declaration.source_location),
                    });
                }
            }
            S::SetupDeclaration(body) => setup = body,
            S::LoopDeclaration(body) => loop_body = body,
            _ => {}
        }
    }

    let lifecycle = Lifecycle {
        initialize: FunctionId(functions.len() + 3),
        setup: FunctionId(1),
        loop_body: FunctionId(2),
    };
    let mut entry = Builder::new(
        module.entry,
        "$entry",
        &globals,
        &pins,
        &functions,
        &intrinsics,
    );
    let mut initialize = Builder::new(
        lifecycle.initialize,
        "$initialize",
        &globals,
        &pins,
        &functions,
        &intrinsics,
    );
    for statement in statements {
        if let S::VariableDeclaration {
            initializer: Some(expr),
            ..
        } = &statement.statement_type
        {
            let value = initialize.value(expr)?;
            initialize.effect(
                Op::StoreGlobal(globals[&statement.symbol.unwrap()].0, value),
                Origin::Source(statement.source_location),
            );
        }
    }
    for statement in statements {
        if let S::DefDeclaration(declarations) = &statement.statement_type {
            for declaration in declarations {
                initialize.effect(
                    Op::SetPinMode(
                        pins[&declaration.symbol.unwrap()],
                        mode(&declaration.pin_mode, declaration.source_location)?,
                    ),
                    Origin::Source(declaration.source_location),
                );
            }
        }
    }

    initialize.terminate(Terminator::Return(None));
    // Reserve the entry slot; fill its startup sequence after device bindings exist.
    entry.terminate(Terminator::Return(None));
    module.functions.push(entry.finish()?);

    for (id, name, body) in [
        (lifecycle.setup, "$setup", setup),
        (lifecycle.loop_body, "$loop", loop_body),
    ] {
        let mut builder = Builder::new(id, name, &globals, &pins, &functions, &intrinsics);
        if id == lifecycle.setup && !body.is_empty() {
            builder.protected_body(body, Origin::Generated("setup interrupt protection"))?;
        } else {
            builder.body(body)?;
        }
        builder.terminate(Terminator::Return(None));
        module.functions.push(builder.finish()?);
    }

    for statement in statements {
        let S::FunctionDeclaration {
            identifier,
            parameters,
            body,
            ..
        } = &statement.statement_type
        else {
            continue;
        };
        let (id, result) = functions[&statement.symbol.unwrap()];
        let mut builder = Builder::new(id, identifier, &globals, &pins, &functions, &intrinsics);
        builder.return_type = result;
        for parameter in parameters {
            let kind = ty(parameter.static_type, parameter.source_location)?;
            let value = ValueId {
                function: id,
                index: builder.next_value,
            };
            builder.next_value += 1;
            builder.parameters.push((value, kind));
            builder.value_types.insert(value, kind);

            let origin = Origin::Source(parameter.source_location);
            let local = builder.local(&parameter.identifier, kind, origin.clone());
            builder.bindings.insert(parameter.symbol.unwrap(), local);
            builder.effect(Op::StoreLocal(local, value), origin);
        }
        builder.body(body)?;
        if !builder.terminated() {
            if result.is_some() {
                return Err(invalid(
                    "function falling through without a return",
                    statement.source_location,
                ));
            }
            builder.terminate(Terminator::Return(None));
        }
        let mut function = builder.finish()?;
        if let S::FunctionDeclaration { no_interrupt, .. } = &statement.statement_type {
            function.requires_no_interrupt = no_interrupt.is_some();
        }
        function.origin = Origin::Source(statement.source_location);
        module.functions.push(function);
    }
    module.functions.push(initialize.finish()?);
    for statement in statements {
        if let S::TimerDeclaration(timer) = &statement.statement_type {
            let id = FunctionId(module.functions.len());
            let mut builder =
                Builder::new(id, "$interrupt", &globals, &pins, &functions, &intrinsics);
            builder.body(&timer.body)?;
            builder.terminate(Terminator::Return(None));
            let mut handler = builder.finish()?;
            handler.requires_no_interrupt = true;
            handler.origin = Origin::Source(timer.handler_location);
            let config = timer_config
                .as_ref()
                .ok_or_else(|| invalid("missing timer configuration", statement.source_location))?;
            let device = DeviceId(module.devices.len());
            module.devices.push(Device {
                config: DeviceConfig::PeriodicTimer(config.clone()),
                phase: StartupPhase::AfterSetup,
                origin: Origin::Source(statement.source_location),
            });
            module.interrupts.push(InterruptBinding {
                device,
                vector: config.hardware.vector,
                handler: id,
            });
            module.functions.push(handler);
        }
    }
    for symbol in program.shared_analysis().interrupt_globals() {
        module.interrupt_globals.insert(globals[symbol].0);
    }
    let mut entry = Builder::new(
        module.entry,
        "$entry",
        &globals,
        &pins,
        &functions,
        &intrinsics,
    );
    for op in module.startup_operations(&lifecycle) {
        let origin = match &op {
            Op::DeviceStart(id) => module.devices[id.0].origin.clone(),
            _ => Origin::Generated("hardware lifecycle"),
        };
        entry.effect(op, origin);
    }
    let repeat = entry.new_block(Origin::Generated("main loop"));
    entry.terminate(Terminator::Jump(repeat));
    entry.current = repeat.index;
    entry.effect(
        Op::Call(lifecycle.loop_body, vec![]),
        Origin::Generated("loop invocation"),
    );
    entry.terminate(Terminator::Jump(repeat));
    module.functions[module.entry.0] = entry.finish()?;
    module.lifecycle = Some(lifecycle);
    Ok(module)
}

struct DraftBlock {
    instructions: Vec<Instruction>,
    terminator: Option<Terminator>,
    origin: Origin,
}

struct Builder<'a> {
    id: FunctionId,
    name: String,
    blocks: Vec<DraftBlock>,
    current: usize,
    next_value: usize,
    value_types: HashMap<ValueId, Type>,
    locals: Vec<Storage>,
    bindings: HashMap<SymbolId, LocalId>,
    globals: &'a HashMap<SymbolId, (GlobalId, Type)>,
    pins: &'a HashMap<SymbolId, PinId>,
    functions: &'a HashMap<SymbolId, (FunctionId, Option<Type>)>,
    intrinsics: &'a HashMap<SymbolId, (DeviceId, crate::intrinsics::Intrinsic)>,
    parameters: Vec<(ValueId, Type)>,
    return_type: Option<Type>,
    interrupt_states: Vec<ValueId>,
}

impl<'a> Builder<'a> {
    fn new(
        id: FunctionId,
        name: &str,
        globals: &'a HashMap<SymbolId, (GlobalId, Type)>,
        pins: &'a HashMap<SymbolId, PinId>,
        functions: &'a HashMap<SymbolId, (FunctionId, Option<Type>)>,
        intrinsics: &'a HashMap<SymbolId, (DeviceId, crate::intrinsics::Intrinsic)>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            blocks: vec![DraftBlock {
                instructions: vec![],
                terminator: None,
                origin: Origin::Generated("function entry"),
            }],
            current: 0,
            next_value: 0,
            value_types: HashMap::new(),
            locals: vec![],
            bindings: HashMap::new(),
            globals,
            pins,
            functions,
            intrinsics,
            parameters: vec![],
            return_type: None,
            interrupt_states: vec![],
        }
    }

    fn finish(self) -> Result<Function, Diagnostic> {
        let mut blocks = vec![];
        for (index, block) in self.blocks.into_iter().enumerate() {
            let terminator = block.terminator.ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticKind::InvalidIr {
                        stage: "construction",
                        detail: "builder left a block without a terminator".into(),
                    },
                    None,
                )
            })?;
            blocks.push(Block {
                id: BlockId {
                    function: self.id,
                    index,
                },
                instructions: block.instructions,
                terminator,
                origin: block.origin,
            });
        }

        Ok(Function {
            requires_no_interrupt: false,
            id: self.id,
            name: self.name,
            parameters: self.parameters,
            return_type: self.return_type,
            locals: self.locals,
            blocks,
            entry: BlockId {
                function: self.id,
                index: 0,
            },
            origin: Origin::Generated("program lifecycle"),
        })
    }

    fn new_block(&mut self, origin: Origin) -> BlockId {
        let id = BlockId {
            function: self.id,
            index: self.blocks.len(),
        };
        self.blocks.push(DraftBlock {
            instructions: vec![],
            terminator: None,
            origin,
        });
        id
    }

    fn terminate(&mut self, term: Terminator) {
        debug_assert!(self.blocks[self.current].terminator.is_none());
        self.blocks[self.current].terminator = Some(term);
    }

    fn terminated(&self) -> bool {
        self.blocks[self.current].terminator.is_some()
    }

    fn emit(&mut self, op: Op, ty: Type, origin: Origin) -> ValueId {
        let id = ValueId {
            function: self.id,
            index: self.next_value,
        };
        self.next_value += 1;
        self.value_types.insert(id, ty);
        self.blocks[self.current].instructions.push(Instruction {
            result: Some((id, ty)),
            op,
            origin,
        });
        id
    }

    fn effect(&mut self, op: Op, origin: Origin) {
        self.blocks[self.current].instructions.push(Instruction {
            result: None,
            op,
            origin,
        });
    }

    fn local(&mut self, name: &str, ty: Type, origin: Origin) -> LocalId {
        let id = LocalId {
            function: self.id,
            index: self.locals.len(),
        };
        self.locals.push(Storage {
            name: name.into(),
            ty,
            origin,
        });
        id
    }

    fn protected_body(
        &mut self,
        statements: &[Statement],
        origin: Origin,
    ) -> Result<(), Diagnostic> {
        let saved = self.emit(Op::InterruptSave, Type::U8, origin.clone());
        self.interrupt_states.push(saved);
        self.body(statements)?;
        self.interrupt_states.pop();

        if !self.terminated() {
            self.effect(Op::InterruptRestore(saved), origin);
        }
        Ok(())
    }

    fn body(&mut self, statements: &[Statement]) -> Result<(), Diagnostic> {
        for statement in statements {
            if self.terminated() {
                break;
            }
            self.statement(statement)?;
        }
        Ok(())
    }

    fn statement(&mut self, statement: &Statement) -> Result<(), Diagnostic> {
        let origin = Origin::Source(statement.source_location);
        match &statement.statement_type {
            S::Return(value) => {
                let value = value.as_ref().map(|v| self.value(v)).transpose()?;
                for saved in self.interrupt_states.clone().into_iter().rev() {
                    self.effect(Op::InterruptRestore(saved), origin.clone());
                }
                self.terminate(Terminator::Return(value));
            }
            S::Expression(Expression {
                symbol: Some(symbol),
                expression_type: E::Call { arguments, .. },
                ..
            }) => {
                self.call(*symbol, arguments, origin)?;
            }
            S::Expression(expr) if expr.static_type == T::PinMode => {
                self.mode_expression(expr)?;
            }
            S::Expression(expr) => {
                self.value(expr)?;
            }
            S::VariableDeclaration {
                identifier,
                static_type,
                initializer,
                ..
            } => {
                let expr = initializer
                    .as_ref()
                    .ok_or_else(|| invalid("missing initializer", statement.source_location))?;
                let value = self.value(expr)?;
                let local = self.local(
                    identifier,
                    ty(static_type.unwrap_or(T::Unknown), statement.source_location)?,
                    origin.clone(),
                );
                self.effect(Op::StoreLocal(local, value), origin);
                self.bindings.insert(statement.symbol.unwrap(), local);
            }
            S::Block(body) => self.body(body)?,
            S::NoInterrupt(body) => self.protected_body(body, origin)?,
            S::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition = self.value(condition)?;
                let yes = self.new_block(origin.clone());
                let no = self.new_block(origin.clone());
                self.terminate(Terminator::Branch { condition, yes, no });

                self.current = yes.index;
                self.statement(then_body)?;
                let yes_end = self.current;
                let yes_closed = self.terminated();

                self.current = no.index;
                if let Some(body) = else_body {
                    self.statement(body)?;
                }
                let no_end = self.current;
                let no_closed = self.terminated();
                if !yes_closed || !no_closed {
                    let end = self.new_block(origin);
                    if !yes_closed {
                        self.current = yes_end;
                        self.terminate(Terminator::Jump(end));
                    }
                    if !no_closed {
                        self.current = no_end;
                        self.terminate(Terminator::Jump(end));
                    }
                    self.current = end.index;
                }
            }
            S::While { condition, body } => {
                let test = self.new_block(origin.clone());
                let yes = self.new_block(origin.clone());
                let end = self.new_block(origin);
                self.terminate(Terminator::Jump(test));

                self.current = test.index;
                let condition = self.value(condition)?;
                self.terminate(Terminator::Branch {
                    condition,
                    yes,
                    no: end,
                });
                self.current = yes.index;
                self.statement(body)?;
                if !self.terminated() {
                    self.terminate(Terminator::Jump(test));
                }
                self.current = end.index;
            }
            S::For { .. } => {
                return Err(unsupported(
                    "for ranges (endpoint and overflow rules are not specified yet)",
                    statement.source_location,
                ))
            }
            _ => {
                return Err(invalid(
                    "nested program declaration",
                    statement.source_location,
                ))
            }
        }
        Ok(())
    }

    fn mode_expression(&mut self, expr: &Expression) -> Result<PinMode, Diagnostic> {
        match &expr.expression_type {
            E::Literal {
                value: L::PinMode(value),
            } => mode(value, expr.source_location),
            E::Grouping { expression } => self.mode_expression(expression),
            E::Assignment {
                identifier,
                expression,
            } if self.pins.contains_key(&expr.symbol.unwrap()) => {
                let mode = self.mode_expression(expression)?;
                self.effect(
                    Op::SetPinMode(self.pins[&expr.symbol.unwrap()], mode),
                    Origin::Source(expr.source_location),
                );
                Ok(mode)
            }
            _ => Err(unsupported(
                "first-class or computed PinMode values",
                expr.source_location,
            )),
        }
    }

    fn call(
        &mut self,
        symbol: SymbolId,
        arguments: &[Expression],
        origin: Origin,
    ) -> Result<Option<ValueId>, Diagnostic> {
        // Evaluate and snapshot each argument before evaluating the next one.
        let arguments = arguments
            .iter()
            .map(|arg| self.value(arg))
            .collect::<Result<Vec<_>, _>>()?;
        let (operation, result) = if let Some(&(device, operation)) = self.intrinsics.get(&symbol) {
            (
                Op::Intrinsic {
                    device,
                    operation,
                    arguments,
                },
                if operation == crate::intrinsics::Intrinsic::UartTryWrite {
                    Some(Type::Bool)
                } else {
                    None
                },
            )
        } else {
            let (id, result) = self.functions[&symbol];
            (Op::Call(id, arguments), result)
        };
        if let Some(kind) = result {
            Ok(Some(self.emit(operation, kind, origin)))
        } else {
            self.effect(operation, origin);
            Ok(None)
        }
    }

    fn value(&mut self, expr: &Expression) -> Result<ValueId, Diagnostic> {
        let origin = Origin::Source(expr.source_location);
        let expected = if expr.static_type == T::Pin {
            Type::PinState
        } else {
            ty(expr.static_type, expr.source_location)?
        };
        if expected.integer().is_some() {
            if let Some(value) = integer_literal(expr) {
                return Ok(self.emit(
                    Op::Const(checked_integer(value, expected, expr.source_location)?),
                    expected,
                    origin,
                ));
            }
        }

        let value = match &expr.expression_type {
            E::Literal {
                value: L::String(text),
            } => self.emit(
                Op::TextLiteral {
                    text: text.clone(),
                    space: TextSpace::Flash,
                },
                Type::Text,
                origin.clone(),
            ),
            E::Text {
                operation,
                arguments,
            } => {
                let values = arguments
                    .iter()
                    .map(|a| self.value(a))
                    .collect::<Result<Vec<_>, _>>()?;
                let op = match operation {
                    crate::frontend::expression::TextOperation::Len => Op::TextLen(values[0]),
                    crate::frontend::expression::TextOperation::ByteOr => Op::TextByteOr {
                        text: values[0],
                        index: values[1],
                        fallback: values[2],
                    },
                };
                self.emit(op, expected, origin.clone())
            }
            E::Call { arguments, .. } => self
                .call(expr.symbol.unwrap(), arguments, origin.clone())?
                .ok_or_else(|| invalid("procedure used as a value", expr.source_location))?,
            E::Literal { value } => {
                let bits = match value {
                    L::Integer(value) => *value,
                    L::Boolean(value) => u64::from(*value),
                    L::PinState(State::High) => 1,
                    L::PinState(State::Low) => 0,
                    _ => return Err(unsupported("literal", expr.source_location)),
                };
                self.emit(
                    Op::Const(Constant::new(expected, bits)),
                    expected,
                    origin.clone(),
                )
            }
            E::Identifier(_) => {
                let symbol = expr.symbol.unwrap();
                if let Some(&pin) = self.pins.get(&symbol) {
                    self.emit(Op::ReadPin(pin), Type::PinState, origin.clone())
                } else if let Some(local) = self.bindings.get(&symbol).copied() {
                    self.emit(
                        Op::LoadLocal(local),
                        self.locals[local.index].ty,
                        origin.clone(),
                    )
                } else if let Some(&(global, ty)) = self.globals.get(&symbol) {
                    self.emit(Op::LoadGlobal(global), ty, origin.clone())
                } else {
                    return Err(invalid("unresolved name", expr.source_location));
                }
            }
            E::Grouping { expression } => self.value(expression)?,
            E::Unary {
                operator, right, ..
            } => {
                let value = self.value(right)?;
                let op = match operator {
                    P::Minus => Unary::Negate,
                    P::Bang => Unary::Not,
                    _ => return Err(invalid("unary operator", expr.source_location)),
                };
                self.emit(
                    Op::Unary(op, value),
                    self.value_types[&value],
                    origin.clone(),
                )
            }
            E::Binary {
                left,
                operator: Token::Keyword(K::And | K::Or),
                right,
                ..
            } => {
                let condition = self.value(left)?;
                let E::Binary {
                    operator: Token::Keyword(keyword),
                    ..
                } = &expr.expression_type
                else {
                    unreachable!()
                };
                let short = matches!(keyword, K::Or);
                let slot = self.local("$short_circuit", Type::Bool, origin.clone());
                let constant = self.emit(
                    Op::Const(Constant::new(Type::Bool, u64::from(short))),
                    Type::Bool,
                    origin.clone(),
                );
                self.effect(Op::StoreLocal(slot, constant), origin.clone());
                let evaluate = self.new_block(origin.clone());
                let end = self.new_block(origin.clone());
                let (yes, no) = if short {
                    (end, evaluate)
                } else {
                    (evaluate, end)
                };
                self.terminate(Terminator::Branch { condition, yes, no });

                self.current = evaluate.index;
                let value = self.value(right)?;
                self.effect(Op::StoreLocal(slot, value), origin.clone());
                self.terminate(Terminator::Jump(end));
                self.current = end.index;
                self.emit(Op::LoadLocal(slot), Type::Bool, origin.clone())
            }
            E::Binary {
                left,
                operator,
                right,
                ..
            } => {
                let left = self.value(left)?;
                let right = self.value(right)?;
                let op = match operator {
                    Token::Punctuation(punctuation) => match punctuation {
                        P::Plus => Binary::Add,
                        P::Minus => Binary::Subtract,
                        P::Star => Binary::Multiply,
                        P::EqualEqual => Binary::Equal,
                        P::BangEqual => Binary::NotEqual,
                        P::Less => Binary::Less,
                        P::LessEqual => Binary::LessEqual,
                        P::Greater => Binary::Greater,
                        P::GreaterEqual => Binary::GreaterEqual,
                        _ => {
                            return Err(unsupported(
                                "division, remainder or shifts",
                                expr.source_location,
                            ))
                        }
                    },
                    _ => return Err(invalid("binary operator", expr.source_location)),
                };
                let result_type = if op.comparison() {
                    Type::Bool
                } else {
                    self.value_types[&left]
                };
                self.emit(Op::Binary(op, left, right), result_type, origin.clone())
            }
            E::Ternary {
                first,
                second,
                third,
            } => {
                let condition = self.value(first)?;
                let branch_type = ty(second.static_type, second.source_location)?;
                let slot = self.local("$ternary", branch_type, origin.clone());
                let yes = self.new_block(origin.clone());
                let no = self.new_block(origin.clone());
                let end = self.new_block(origin.clone());
                self.terminate(Terminator::Branch { condition, yes, no });

                for (block, expression) in [(yes, second), (no, third)] {
                    self.current = block.index;
                    let value = self.value(expression)?;
                    self.effect(Op::StoreLocal(slot, value), origin.clone());
                    self.terminate(Terminator::Jump(end));
                }
                self.current = end.index;
                self.emit(Op::LoadLocal(slot), branch_type, origin.clone())
            }
            E::Assignment { expression, .. } => {
                let value = self.value(expression)?;
                let symbol = expr.symbol.unwrap();
                let op = if let Some(&pin) = self.pins.get(&symbol) {
                    Op::WritePin(pin, value)
                } else if let Some(local) = self.bindings.get(&symbol).copied() {
                    Op::StoreLocal(local, value)
                } else if let Some(&(global, _)) = self.globals.get(&symbol) {
                    Op::StoreGlobal(global, value)
                } else {
                    return Err(invalid("unresolved assignment", expr.source_location));
                };
                self.effect(op, origin.clone());
                value
            }
        };

        if self.value_types[&value] == expected {
            Ok(value)
        } else {
            Ok(self.emit(Op::Widen(value, expected), expected, origin))
        }
    }
}
