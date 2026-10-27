mod functions;
pub mod shared;

use crate::diagnostic::{Diagnostic, DiagnosticKind, LabelKind};
use crate::frontend::{
    expression::{Expression, ExpressionType as E},
    statement::{Statement, StatementType as S},
    static_type::{
        FloatType as F, IntegerType as I, SignedIntegerType as SI, StaticType as T,
        UnsignedIntegerType as UI,
    },
    token_type::{KeywordType as K, LiteralType as L, PunctuationType as P, TokenType as Token},
    utils::SourceLocation,
};
use crate::semantic::{AnalyzedProgram, BodyId, Symbol, SymbolId, SymbolKind, SymbolTable};
use functions::Signature;
use std::collections::{BTreeMap, HashMap};

/// Resolves names and types. A declaration may not shadow any visible name.
pub struct Analyzer {
    scopes: Vec<HashMap<String, SymbolId>>,
    symbols: SymbolTable,
    current_body: BodyId,
    errors: Vec<Diagnostic>,
    functions: BTreeMap<String, Signature>,
    calls: BTreeMap<SymbolId, Vec<(SymbolId, SourceLocation)>>,
    current_function: Option<String>,
    return_type: Option<Option<T>>,
    return_location: Option<SourceLocation>,
    global_initializer: bool,
    interrupts_disabled: bool,
}

impl Analyzer {
    pub fn new(_source: &str) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            symbols: SymbolTable::default(),
            current_body: BodyId::Initialization,
            errors: vec![],
            functions: BTreeMap::new(),
            calls: BTreeMap::new(),
            current_function: None,
            return_type: None,
            return_location: None,
            global_initializer: false,
            interrupts_disabled: false,
        }
    }

    pub fn analyze(
        mut self,
        mut statements: Vec<Statement>,
    ) -> Result<AnalyzedProgram, Vec<Diagnostic>> {
        self.collect_signatures(&mut statements);
        self.collect_devices(&mut statements);

        // Global initialization keeps source order. Function bodies see all globals,
        // but calls in global initializers are deferred until setup is available.
        self.global_initializer = true;
        for statement in statements.iter_mut() {
            if matches!(
                statement.statement_type,
                S::VariableDeclaration { .. } | S::DefDeclaration(_)
            ) {
                self.statement(statement);
            }
        }
        self.global_initializer = false;
        // Hardware names occupy the same namespace, but are not runtime values.
        for statement in statements.iter_mut() {
            if let S::TimerDeclaration(timer) = &statement.statement_type {
                statement.symbol = self.declare(
                    &timer.identifier,
                    T::NoValue,
                    timer.name_location,
                    SymbolKind::Timer,
                );
            }
        }

        for statement in statements.iter_mut() {
            if !matches!(
                statement.statement_type,
                S::VariableDeclaration { .. } | S::DefDeclaration(_)
            ) {
                self.statement(statement);
            }
        }
        self.check_recursion();

        if self.errors.is_empty() {
            for statement in &mut statements {
                statement.for_each_expression_mut(&mut |e| self.finalize_expression(e));
            }
        }

        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        let shared = shared::Program::collect(&statements, &self.symbols);
        let errors = shared.check(&self.symbols);
        if !errors.is_empty() {
            return Err(errors);
        }
        let contracts = self
            .symbols
            .iter()
            .filter_map(|(id, symbol)| match symbol.kind {
                SymbolKind::Intrinsic { operation, .. } => Some((id, operation.contract())),
                _ => None,
            })
            .collect();
        let runtime_irqs = statements
            .iter()
            .any(|s| matches!(s.statement_type, S::TimerDeclaration(_)));
        let (effects, diagnostics) =
            crate::effects::analyze(shared.bodies(), &self.symbols, &contracts, runtime_irqs);
        if diagnostics
            .iter()
            .any(|d| d.severity() == crate::diagnostic::Severity::Error)
        {
            return Err(diagnostics);
        }
        Ok(AnalyzedProgram {
            warnings: diagnostics,
            effects,
            statements,
            symbols: self.symbols,
            shared,
        })
    }

    fn error(&mut self, kind: DiagnosticKind, span: SourceLocation) {
        self.errors.push(Diagnostic::new(kind, Some(span)));
    }

    fn finalize_expression(&mut self, expr: &mut Expression) {
        let target = default_type(expr.static_type);
        if integer_literal(expr).is_some() {
            // A signed literal is one semantic atom: the positive token in -128:i8
            // need not independently fit i8. Lowering consumes the complete atom.
            self.coerce(expr, target);
            fn stamp(expr: &mut Expression, target: T) {
                expr.static_type = target;
                expr.for_each_child_mut(|child| stamp(child, target));
            }
            stamp(expr, target);
            return;
        }
        if literal_type(expr.static_type) {
            self.coerce(expr, target);
        }
        expr.for_each_child_mut(|child| self.finalize_expression(child));
    }

    fn resolve(&self, name: &str) -> Option<SymbolId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn lookup(&self, name: &str) -> Option<T> {
        self.resolve(name).map(|id| self.symbols.get(id).ty)
    }

    fn declare(
        &mut self,
        name: &str,
        ty: T,
        span: SourceLocation,
        kind: SymbolKind,
    ) -> Option<SymbolId> {
        let previous = self
            .resolve(name)
            .or_else(|| self.functions.get(name).map(|s| s.id));
        if let Some(previous) = previous {
            self.errors.push(
                Diagnostic::new(
                    DiagnosticKind::DuplicateName { name: name.into() },
                    Some(span),
                )
                .with_label(
                    self.symbols.get(previous).declaration,
                    LabelKind::PreviousDeclaration,
                ),
            );
            None
        } else {
            let owner = (!self.global_initializer && kind != SymbolKind::Timer)
                .then_some(self.current_body);
            let id = self.symbols.add(Symbol {
                name: name.into(),
                kind,
                ty,
                declaration: span,
                owner,
            });
            self.scopes.last_mut().unwrap().insert(name.into(), id);
            Some(id)
        }
    }

    fn coerce_with_origin(&mut self, expr: &mut Expression, target: T, origin: SourceLocation) {
        let before = self.errors.len();
        self.coerce(expr, target);
        for error in &mut self.errors[before..] {
            error.labels.push(crate::diagnostic::Label {
                location: origin,
                kind: LabelKind::ExpectedType(target),
            });
        }
    }

    fn block(&mut self, body: &mut [Statement]) {
        self.scopes.push(HashMap::new());

        for statement in body {
            self.statement(statement);
        }

        self.scopes.pop();
    }

    fn statement(&mut self, statement: &mut Statement) {
        let span = statement.source_location;

        match &mut statement.statement_type {
            S::DeviceDeclaration(_) => {}
            S::TimerDeclaration(timer) => {
                if let Some(id) = statement.symbol {
                    self.current_body = BodyId::Interrupt(id);
                }
                self.interrupts_disabled = true;
                self.block(&mut timer.body);
                self.interrupts_disabled = false;
            }
            S::FunctionDeclaration {
                no_interrupt,
                identifier,
                parameters,
                return_type,
                return_location,
                body,
                ..
            } => {
                self.interrupts_disabled = no_interrupt.is_some();
                self.current_function = Some(identifier.clone());
                self.current_body = BodyId::Function(self.functions[identifier].id);
                self.return_type = Some(*return_type);
                self.return_location = *return_location;
                self.scopes.push(HashMap::new());
                for parameter in parameters {
                    parameter.symbol = self.declare(
                        &parameter.identifier,
                        parameter.static_type,
                        parameter.source_location,
                        SymbolKind::Parameter,
                    );
                }
                self.block(body);
                self.scopes.pop();
                if let Some(target) = return_type {
                    let exits = functions::fallthrough(body, span.closing());
                    if let Some((location, _)) = exits.first() {
                        let mut error = Diagnostic::new(
                            DiagnosticKind::MissingReturn {
                                name: identifier.clone(),
                            },
                            Some(*location),
                        );
                        for (location, kind) in exits {
                            error = error.with_label(location, kind);
                        }
                        error = error
                            .with_label(return_location.unwrap(), LabelKind::ExpectedType(*target));
                        self.errors.push(error);
                    }
                }
                self.return_location = None;
                self.current_function = None;
                self.return_type = None;
                self.interrupts_disabled = false;
            }
            S::Return(value) => {
                if let Some(value) = value {
                    self.expression(value);
                }
                match (self.return_type, value) {
                    (None, _) => self.error(DiagnosticKind::ReturnOutsideFunction, span),
                    (Some(Some(target)), Some(value)) => {
                        self.coerce_with_origin(value, target, self.return_location.unwrap())
                    }
                    (Some(Some(target)), None) => self.errors.push(
                        Diagnostic::new(DiagnosticKind::ReturnValueRequired { target }, Some(span))
                            .with_label(
                                self.return_location.unwrap(),
                                LabelKind::ExpectedType(target),
                            ),
                    ),
                    (Some(None), Some(value)) => {
                        let declaration =
                            self.functions[self.current_function.as_ref().unwrap()].name_location;
                        self.errors.push(
                            Diagnostic::new(
                                DiagnosticKind::UnexpectedReturnValue,
                                Some(value.source_location),
                            )
                            .with_label(declaration, LabelKind::FunctionDeclaration),
                        );
                    }
                    (Some(None), None) => {}
                }
            }
            S::VariableDeclaration {
                identifier,
                static_type,
                initializer,
                name_location,
                type_location,
            } => {
                if initializer.is_none() {
                    let value = match static_type {
                        Some(T::Integer(_)) => Some(L::Integer(0)),
                        Some(T::Boolean) => Some(L::Boolean(false)),
                        Some(T::Float(_)) => Some(L::Float(0.0)),
                        Some(T::String) => Some(L::String(String::new())),
                        _ => None,
                    };
                    if let Some(value) = value {
                        *initializer = Some(Expression {
                            symbol: None,
                            expression_type: E::Literal { value },
                            source_location: span,
                            static_type: T::Unknown,
                        });
                    } else {
                        self.error(DiagnosticKind::MissingInitializer, *name_location);
                    }
                }

                // Analyze before declaration: an initializer cannot read its own variable.
                let inferred = initializer
                    .as_mut()
                    .map(|expr| self.expression(expr))
                    .unwrap_or(T::Unknown);

                let target = static_type.unwrap_or_else(|| default_type(inferred));

                if let Some(expr) = initializer {
                    self.coerce_with_origin(expr, target, type_location.unwrap_or(*name_location));
                }

                *static_type = Some(target);
                statement.symbol = self.declare(
                    identifier,
                    target,
                    *name_location,
                    if self.global_initializer {
                        SymbolKind::Global
                    } else {
                        SymbolKind::Local
                    },
                );
            }
            S::DefDeclaration(declarations) => {
                for pin in declarations {
                    let port = self.symbols.port(&pin.pin.port);
                    pin.symbol = self.declare(
                        &pin.identifier,
                        T::Pin,
                        pin.name_location,
                        SymbolKind::Pin {
                            port,
                            bit: pin.pin.pin_number,
                        },
                    );
                }
            }
            S::SetupDeclaration(body) => {
                self.current_body = BodyId::Setup;
                self.interrupts_disabled = true;
                self.block(body);
                self.interrupts_disabled = false;
            }
            S::NoInterrupt(body) => {
                let previous = self.interrupts_disabled;
                self.interrupts_disabled = true;
                self.block(body);
                self.interrupts_disabled = previous;
            }
            S::LoopDeclaration(body) => {
                self.current_body = BodyId::MainLoop;
                self.block(body);
            }
            S::Block(body) => self.block(body),
            S::Expression(expr) => {
                if let E::Call {
                    identifier,
                    arguments,
                } = &mut expr.expression_type
                {
                    expr.symbol = self.functions.get(identifier).map(|s| s.id);
                    expr.static_type = self
                        .call(identifier, arguments, expr.source_location)
                        .unwrap_or(T::NoValue);
                } else {
                    self.expression(expr);
                }
            }
            S::If {
                condition,
                then_body,
                else_body,
            } => {
                self.condition(condition);
                self.statement(then_body);

                if let Some(body) = else_body {
                    self.statement(body);
                }
            }
            S::While { condition, body } => {
                self.condition(condition);
                self.statement(body);
            }
            S::For {
                identifier,
                from,
                to,
                body,
                name_location,
            } => {
                let a = self.expression(from);
                let b = self.expression(to);
                let ty = if matches!((a, b), (T::Integer(_), T::Integer(_))) {
                    self.common_numeric(from, to)
                } else {
                    if a != T::Unknown && b != T::Unknown {
                        let bad = if !matches!(a, T::Integer(_)) {
                            from.source_location
                        } else {
                            to.source_location
                        };
                        self.error(DiagnosticKind::ForBounds, bad);
                    }
                    T::Unknown
                };

                let ty = default_type(ty);
                self.coerce(from, ty);
                self.coerce(to, ty);

                self.scopes.push(HashMap::new());
                statement.symbol = self.declare(identifier, ty, *name_location, SymbolKind::Local);
                self.statement(body);
                self.scopes.pop();
            }
        }
    }

    fn condition(&mut self, expr: &mut Expression) {
        let ty = self.expression(expr);

        if ty != T::Boolean && ty != T::Unknown {
            self.error(DiagnosticKind::ConditionType { ty }, expr.source_location);
        }
    }

    fn expression(&mut self, expr: &mut Expression) -> T {
        let span = expr.source_location;

        expr.symbol = match &expr.expression_type {
            E::Call { identifier, .. } => self.functions.get(identifier).map(|s| s.id),
            E::Identifier(name)
            | E::Assignment {
                identifier: name, ..
            } => self.resolve(name),
            _ => None,
        };
        let error_start = self.errors.len();
        let ty = match &mut expr.expression_type {
            E::Call {
                identifier,
                arguments,
            } => self.call(identifier, arguments, span).unwrap_or_else(|| {
                self.error(
                    DiagnosticKind::ProcedureValue {
                        name: identifier.clone(),
                    },
                    span,
                );
                T::Unknown
            }),
            E::Literal { value } => match value {
                L::Integer(_) => T::Integer(I::Literal),
                L::Float(_) => T::Float(F::Literal),
                L::Boolean(_) => T::Boolean,
                L::String(_) => T::String,
                L::PinState(_) => T::PinState,
                L::PinMode(_) => T::PinMode,
            },
            E::Identifier(name) if self.functions.contains_key(name) => {
                self.error(DiagnosticKind::FunctionValue { name: name.clone() }, span);
                T::Unknown
            }
            E::Identifier(name) if self.lookup(name) == Some(T::NoValue) => {
                self.error(
                    if self
                        .resolve(name)
                        .is_some_and(|id| matches!(self.symbols.get(id).kind, SymbolKind::Device))
                    {
                        DiagnosticKind::Device {
                            detail: format!(
                                "Device '{name}' is a static resource, not a runtime value"
                            ),
                        }
                    } else {
                        DiagnosticKind::Timer {
                            error: crate::timer::TimerError::NotValue { name: name.clone() },
                        }
                    },
                    span,
                );
                T::Unknown
            }
            E::Identifier(name) => self.lookup(name).unwrap_or_else(|| {
                self.error(
                    DiagnosticKind::UndefinedName {
                        name: name.to_owned(),
                    },
                    span,
                );
                T::Unknown
            }),
            E::Grouping { expression } => self.expression(expression),
            E::Unary {
                operator, right, ..
            } => {
                let operand = self.expression(right);

                match (operator, operand) {
                    (P::Bang, T::Boolean) => T::Boolean,
                    (P::Bang, T::Pin | T::PinState) => T::PinState,
                    (P::Minus, T::Integer(I::Signed(_) | I::Literal) | T::Float(_)) => operand,
                    (_, T::Unknown) => T::Unknown,
                    (operator, _) => {
                        self.error(
                            DiagnosticKind::UnaryType {
                                operator: operator.clone(),
                                operand,
                            },
                            span,
                        );
                        T::Unknown
                    }
                }
            }
            E::Binary {
                left,
                operator,
                right,
                ..
            } => {
                let a = self.expression(left);
                let b = self.expression(right);
                if a == T::Unknown || b == T::Unknown {
                    T::Unknown
                } else {
                    match operator {
                        Token::Keyword(K::And | K::Or) if a == T::Boolean && b == T::Boolean => {
                            T::Boolean
                        }
                        Token::Punctuation(P::Plus | P::Minus | P::Star | P::Slash)
                            if numeric(a) && numeric(b) =>
                        {
                            self.common_numeric(left, right)
                        }
                        Token::Punctuation(P::EqualEqual | P::BangEqual)
                            if numeric(a) && numeric(b) =>
                        {
                            self.common_numeric(left, right);
                            T::Boolean
                        }
                        Token::Punctuation(P::EqualEqual | P::BangEqual)
                            if (a == b && matches!(a, T::Boolean | T::PinState | T::PinMode))
                                || matches!(
                                    (a, b),
                                    (T::Pin, T::PinState) | (T::PinState, T::Pin)
                                ) =>
                        {
                            T::Boolean
                        }
                        Token::Punctuation(
                            P::Less | P::LessEqual | P::Greater | P::GreaterEqual,
                        ) if numeric(a) && numeric(b) => {
                            self.common_numeric(left, right);
                            T::Boolean
                        }
                        _ => {
                            self.error(
                                DiagnosticKind::BinaryType {
                                    operator: operator.clone(),
                                    a,
                                    b,
                                },
                                span,
                            );
                            T::Unknown
                        }
                    }
                }
            }
            E::Ternary {
                first,
                second,
                third,
            } => {
                self.condition(first);
                let a = self.expression(second);
                let b = self.expression(third);

                if a == T::Unknown || b == T::Unknown {
                    T::Unknown
                } else if numeric(a) && numeric(b) {
                    self.common_numeric(second, third)
                } else if a == b {
                    a
                } else {
                    self.error(DiagnosticKind::BranchTypes { a, b }, span);
                    T::Unknown
                }
            }
            E::Assignment {
                identifier,
                expression,
            } => {
                let value = self.expression(expression);

                match self.lookup(identifier) {
                    Some(T::Pin) if matches!(value, T::PinState | T::PinMode) => value,
                    Some(target) => {
                        let origin = self
                            .scopes
                            .iter()
                            .rev()
                            .find_map(|scope| {
                                scope
                                    .get(identifier)
                                    .map(|id| self.symbols.get(*id).declaration)
                            })
                            .unwrap();
                        self.coerce_with_origin(expression, target, origin);
                        target
                    }
                    None => {
                        self.error(
                            DiagnosticKind::UndefinedName {
                                name: identifier.clone(),
                            },
                            span.name(identifier),
                        );
                        T::Unknown
                    }
                }
            }
        };

        // Attach both sides of type disagreements without creating extra errors.
        for error in &mut self.errors[error_start..] {
            if error.location != Some(span) {
                continue;
            }
            match (&*error.kind, &expr.expression_type) {
                (
                    DiagnosticKind::BinaryType { a, b, .. },
                    E::Binary {
                        left,
                        right,
                        operator_location,
                        ..
                    },
                ) => {
                    error.location = Some(*operator_location);
                    error.labels = vec![
                        crate::diagnostic::Label {
                            location: left.source_location,
                            kind: LabelKind::OperandType(*a),
                        },
                        crate::diagnostic::Label {
                            location: right.source_location,
                            kind: LabelKind::OperandType(*b),
                        },
                    ];
                }
                (
                    DiagnosticKind::UnaryType { operand, .. },
                    E::Unary {
                        right,
                        operator_location,
                        ..
                    },
                ) => {
                    error.location = Some(*operator_location);
                    error.labels.push(crate::diagnostic::Label {
                        location: right.source_location,
                        kind: LabelKind::OperandType(*operand),
                    });
                }
                (DiagnosticKind::BranchTypes { a, b }, E::Ternary { second, third, .. }) => {
                    error.labels = vec![
                        crate::diagnostic::Label {
                            location: second.source_location,
                            kind: LabelKind::OperandType(*a),
                        },
                        crate::diagnostic::Label {
                            location: third.source_location,
                            kind: LabelKind::OperandType(*b),
                        },
                    ];
                }
                (
                    DiagnosticKind::ProcedureValue { name }
                    | DiagnosticKind::FunctionValue { name },
                    _,
                ) => {
                    if let Some(signature) = self.functions.get(name) {
                        error.labels.push(crate::diagnostic::Label {
                            location: signature.return_location.unwrap_or(signature.name_location),
                            kind: LabelKind::FunctionDeclaration,
                        });
                    }
                }
                _ => {}
            }
        }
        expr.static_type = ty;

        ty
    }

    fn common_numeric(&mut self, left: &mut Expression, right: &mut Expression) -> T {
        let a = left.static_type;
        let b = right.static_type;

        let target = if a == b {
            a
        } else if literal_type(a) && !literal_type(b) {
            b
        } else if literal_type(b) && !literal_type(a) {
            a
        } else if widening(a, b) {
            b
        } else if widening(b, a) {
            a
        } else {
            self.errors.push(
                Diagnostic::new(
                    DiagnosticKind::NumericTypes { a, b },
                    Some(left.source_location),
                )
                .with_label(right.source_location, LabelKind::OperandType(b)),
            );
            return T::Unknown;
        };

        self.coerce(left, target);
        self.coerce(right, target);

        target
    }

    fn coerce(&mut self, expr: &mut Expression, target: T) {
        let from = expr.static_type;

        if from == T::Unknown || target == T::Unknown {
            return;
        }

        if matches!(target, T::Float(F::Float32 | F::Float64)) {
            if let Some(value) = integer_literal(expr) {
                let restored = if target == T::Float(F::Float32) {
                    (value as f32) as i128
                } else {
                    (value as f64) as i128
                };

                if restored != value {
                    self.error(
                        DiagnosticKind::InexactInteger { value, target },
                        expr.source_location,
                    );
                } else {
                    expr.static_type = target;
                }

                return;
            }
        }

        if let Some((min, max)) = integer_bounds(target) {
            if let Some(value) = integer_literal(expr) {
                if !(min..=max).contains(&value) {
                    self.error(
                        DiagnosticKind::IntegerRange {
                            value,
                            target,
                            min,
                            max,
                        },
                        expr.source_location,
                    );
                    return;
                }

                expr.static_type = target;

                return;
            }
        }

        if from != target
            && !widening(from, target)
            && !(from == T::Integer(I::Literal) && matches!(target, T::Integer(_)))
        {
            self.error(
                DiagnosticKind::LossyConversion { from, target },
                expr.source_location,
            );
            return;
        }

        if literal_type(from) && !literal_type(target) {
            match &mut expr.expression_type {
                E::Grouping { expression } => self.coerce(expression, target),
                E::Unary {
                    operator: P::Minus,
                    right,
                    ..
                } => {
                    if matches!(target, T::Integer(I::Unsigned(_))) {
                        self.error(DiagnosticKind::NegatedUnsigned, expr.source_location);
                    } else {
                        self.coerce(right, target);
                    }
                }
                E::Binary { left, right, .. } => {
                    self.coerce(left, target);
                    self.coerce(right, target);
                }
                E::Ternary { second, third, .. } => {
                    self.coerce(second, target);
                    self.coerce(third, target);
                }
                E::Literal {
                    value: L::Float(value),
                } if target == T::Float(F::Float32) && !(*value as f32).is_finite() => {
                    self.error(DiagnosticKind::FloatRange, expr.source_location);
                }
                _ => {}
            }
        }

        expr.static_type = target;
    }
}

fn numeric(ty: T) -> bool {
    matches!(ty, T::Integer(_) | T::Float(_))
}

fn literal_type(ty: T) -> bool {
    matches!(ty, T::Integer(I::Literal) | T::Float(F::Literal))
}

fn default_type(ty: T) -> T {
    match ty {
        T::Integer(I::Literal) => T::Integer(I::Signed(SI::Int32)),
        T::Float(F::Literal) => T::Float(F::Float32),
        _ => ty,
    }
}

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

fn integer_bounds(ty: T) -> Option<(i128, i128)> {
    let (signed, bits) = match ty {
        T::Integer(I::Signed(s)) => (
            true,
            match s {
                SI::Int8 => 8,
                SI::Int16 => 16,
                SI::Int32 => 32,
                SI::Int64 => 64,
            },
        ),
        T::Integer(I::Unsigned(s)) => (
            false,
            match s {
                UI::Uns8 => 8,
                UI::Uns16 => 16,
                UI::Uns32 => 32,
                UI::Uns64 => 64,
            },
        ),
        _ => return None,
    };

    Some(if signed {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    })
}

fn widening(from: T, to: T) -> bool {
    if from == to {
        return true;
    }

    if let (Some((a, b)), Some((c, d))) = (integer_bounds(from), integer_bounds(to)) {
        return c <= a && b <= d;
    }

    match (from, to) {
        (T::Float(F::Literal), T::Float(_)) | (T::Float(F::Float32), T::Float(F::Float64)) => true,
        (T::Integer(_), T::Float(F::Float32 | F::Float64)) => {
            let precision = if to == T::Float(F::Float32) { 24 } else { 53 };
            integer_bounds(from)
                .is_some_and(|(a, b)| a >= -(1i128 << precision) && b <= (1i128 << precision))
        }
        _ => false,
    }
}
