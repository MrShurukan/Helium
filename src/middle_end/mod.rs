pub mod linear_ir;

use crate::diagnostic::Diagnostic;
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
use std::collections::HashMap;

/// Resolves names and types. A declaration may not shadow any visible name.
pub struct Analyzer {
    scopes: Vec<HashMap<String, T>>,
    errors: Vec<Diagnostic>,
}

impl Analyzer {
    pub fn new(_source: &str) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            errors: vec![],
        }
    }
    pub fn analyze(mut self, statements: &mut [Statement]) -> Result<(), Vec<Diagnostic>> {
        for statement in statements {
            self.statement(statement);
        }
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors)
        }
    }
    fn error(&mut self, code: &'static str, message: impl Into<String>, span: SourceLocation) {
        self.errors.push(Diagnostic::new(code, message, Some(span)));
    }
    fn lookup(&self, name: &str) -> Option<T> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }
    fn declare(&mut self, name: &str, ty: T, span: SourceLocation) {
        if self.lookup(name).is_some() {
            self.error(
                "E_DUPLICATE",
                format!("'{name}' is already defined; shadowing is not allowed"),
                span,
            );
        } else {
            self.scopes
                .last_mut()
                .expect("root scope exists")
                .insert(name.to_owned(), ty);
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
            S::VariableDeclaration {
                identifier,
                static_type,
                initializer,
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
                            expression_type: E::Literal { value },
                            source_location: span,
                            static_type: T::Unknown,
                        });
                    } else {
                        self.error(
                            "E_INITIALIZER",
                            "Provide a type with a default value or an initializer",
                            span,
                        );
                    }
                }
                // Analyze before declaration: an initializer cannot read its own variable.
                let inferred = initializer
                    .as_mut()
                    .map(|expr| self.expression(expr))
                    .unwrap_or(T::Unknown);
                let target = static_type.unwrap_or_else(|| default_type(inferred));
                if let Some(expr) = initializer {
                    self.coerce(expr, target);
                }
                *static_type = Some(target);
                self.declare(identifier, target, span);
            }
            S::DefDeclaration(declarations) => {
                for pin in declarations {
                    self.declare(&pin.identifier, T::Pin, pin.source_location);
                }
            }
            S::SetupDeclaration(body) | S::LoopDeclaration(body) | S::Block(body) => {
                self.block(body)
            }
            S::Expression(expr) => {
                self.expression(expr);
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
            } => {
                let a = self.expression(from);
                let b = self.expression(to);
                let ty = if matches!((a, b), (T::Integer(_), T::Integer(_))) {
                    self.common_numeric(from, to)
                } else {
                    self.error("E_FOR_TYPE", "For bounds must be integers", span);
                    T::Unknown
                };
                let ty = default_type(ty);
                self.coerce(from, ty);
                self.coerce(to, ty);
                self.scopes.push(HashMap::new());
                self.declare(identifier, ty, span);
                self.statement(body);
                self.scopes.pop();
            }
        }
    }
    fn condition(&mut self, expr: &mut Expression) {
        let ty = self.expression(expr);
        if ty != T::Boolean && ty != T::Unknown {
            self.error(
                "E_CONDITION",
                format!("Expected bool condition, found {ty}"),
                expr.source_location,
            );
        }
    }
    fn expression(&mut self, expr: &mut Expression) -> T {
        let span = expr.source_location;
        let ty = match &mut expr.expression_type {
            E::Literal { value } => match value {
                L::Integer(_) => T::Integer(I::Literal),
                L::Float(_) => T::Float(F::Literal),
                L::Boolean(_) => T::Boolean,
                L::String(_) => T::String,
                L::PinState(_) => T::PinState,
                L::PinMode(_) => T::PinMode,
            },
            E::Identifier(name) => self.lookup(name).unwrap_or_else(|| {
                self.error(
                    "E_UNDEFINED",
                    format!("'{name}' is undefined in this scope"),
                    span,
                );
                T::Unknown
            }),
            E::Grouping { expression } => self.expression(expression),
            E::Unary { operator, right } => {
                let operand = self.expression(right);
                match (operator, operand) {
                    (P::Bang, T::Boolean) => T::Boolean,
                    (P::Bang, T::Pin | T::PinState) => T::PinState,
                    (P::Minus, T::Integer(I::Signed(_) | I::Literal) | T::Float(_)) => operand,
                    (_, T::Unknown) => T::Unknown,
                    (operator, _) => {
                        self.error(
                            "E_UNARY_TYPE",
                            format!("Unary {operator} cannot be used with {operand}"),
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
                                "E_BINARY_TYPE",
                                format!("Operator {operator:?} does not accept {a} and {b}"),
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
                if numeric(a) && numeric(b) {
                    self.common_numeric(second, third)
                } else if a == b {
                    a
                } else {
                    self.error(
                        "E_TERNARY_TYPE",
                        format!("Branches have incompatible types {a} and {b}"),
                        span,
                    );
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
                        self.coerce(expression, target);
                        target
                    }
                    None => {
                        self.error(
                            "E_UNDEFINED",
                            format!("'{identifier}' is undefined in this scope"),
                            span,
                        );
                        T::Unknown
                    }
                }
            }
        };
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
            self.error(
                "E_NUMERIC_TYPE",
                format!("No lossless common type for {a} and {b}"),
                left.source_location,
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
                        "E_LITERAL_RANGE",
                        format!(
                            "Integer literal {value} cannot be represented exactly as {target}"
                        ),
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
                        "E_LITERAL_RANGE",
                        format!("Integer literal {value} is outside {target} range {min}..={max}"),
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
                "E_TYPE",
                format!("Cannot implicitly convert {from} to {target} without loss"),
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
                } => {
                    if matches!(target, T::Integer(I::Unsigned(_))) {
                        self.error(
                            "E_TYPE",
                            "A negated expression cannot implicitly become unsigned",
                            expr.source_location,
                        );
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
                    self.error(
                        "E_LITERAL_RANGE",
                        "Floating-point literal is outside f32 range",
                        expr.source_location,
                    );
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
