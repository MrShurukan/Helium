//! Explicit AVR support boundary. Unsupported language features never reach code generation.
use crate::{
    diagnostic::{Diagnostic, DiagnosticKind},
    frontend::{
        expression::{Expression, ExpressionType as E},
        statement::{Statement, StatementType as S},
        static_type::StaticType as T,
        token_type::PinModeConstantType as M,
    },
};
use std::collections::HashMap;

pub fn validate_pins(statements: &[Statement], mcu: &str) -> Result<(), Vec<Diagnostic>> {
    if crate::backend::microcontroller::get(mcu).is_none() {
        return Err(vec![Diagnostic::new(
            DiagnosticKind::UnsupportedTarget {
                mcu: mcu.to_owned(),
            },
            None,
        )]);
    }

    let mut errors = vec![];
    let mut pins = HashMap::new();

    for statement in statements {
        if let S::DefDeclaration(declarations) = &statement.statement_type {
            for declaration in declarations {
                let pin = &declaration.pin;
                let valid = crate::backend::microcontroller::ATMEGA328P
                    .has_digital_pin(&pin.port, pin.pin_number);

                if !valid {
                    errors.push(Diagnostic::new(
                        DiagnosticKind::InvalidPin {
                            port: pin.port.clone(),
                            number: pin.pin_number,
                        },
                        Some(declaration.pin_location),
                    ));
                }

                let key = (pin.port.clone(), pin.pin_number);
                if let Some(previous) = pins.get(&key) {
                    errors.push(
                        Diagnostic::new(
                            DiagnosticKind::DuplicatePin,
                            Some(declaration.pin_location),
                        )
                        .with_label(*previous, crate::diagnostic::LabelKind::PreviousDeclaration),
                    );
                } else {
                    pins.insert(key, declaration.pin_location);
                }

                if declaration.pin_mode == M::InputPullDown {
                    errors.push(Diagnostic::new(
                        DiagnosticKind::PullDown,
                        Some(declaration.mode_location),
                    ));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Catch source limitations before lowering discards source syntax or dead code.
pub fn validate_features(statements: &[Statement], machine: bool) -> Result<(), Vec<Diagnostic>> {
    use crate::frontend::{
        static_type::{IntegerType as I, SignedIntegerType as SI, UnsignedIntegerType as UI},
        utils::SourceLocation,
    };
    fn check_type(ty: T, span: SourceLocation, machine: bool, errors: &mut Vec<Diagnostic>) {
        let unsupported =
            match ty {
                T::Integer(
                    I::Signed(SI::Int32 | SI::Int64) | I::Unsigned(UI::Uns32 | UI::Uns64),
                ) if machine => Some(true),
                T::Float(_) | T::String | T::Array(_) | T::PinMode | T::Pin => Some(false),
                _ => None,
            };
        if let Some(machine) = unsupported {
            errors.push(Diagnostic::new(
                DiagnosticKind::UnsupportedType { ty, machine },
                Some(span),
            ));
        }
    }

    fn expr(e: &Expression, machine: bool, errors: &mut Vec<Diagnostic>) {
        use crate::frontend::token_type::{PunctuationType as P, TokenType};
        match &e.expression_type {
            E::Binary {
                left,
                operator,
                right,
                operator_location,
                ..
            } => {
                if matches!(operator, TokenType::Punctuation(P::Slash)) {
                    errors.push(Diagnostic::new(
                        DiagnosticKind::TypedIrUnsupported {
                            feature:
                                "division (rounding and zero-divisor semantics are not defined yet)"
                                    .into(),
                        },
                        Some(*operator_location),
                    ));
                }
                expr(left, machine, errors);
                expr(right, machine, errors);
            }
            E::Unary { right, .. } => expr(right, machine, errors),
            E::Grouping { expression } | E::Assignment { expression, .. } => {
                expr(expression, machine, errors)
            }
            E::Ternary {
                first,
                second,
                third,
            } => {
                expr(first, machine, errors);
                expr(second, machine, errors);
                expr(third, machine, errors);
            }
            E::Call { arguments, .. } => {
                for a in arguments {
                    expr(a, machine, errors);
                }
            }
            E::Literal {
                value: crate::frontend::token_type::LiteralType::PinMode(mode),
            } => {
                if *mode == M::InputPullDown {
                    errors.push(Diagnostic::new(
                        DiagnosticKind::PullDown,
                        Some(e.source_location),
                    ));
                }
            }
            E::Identifier(_) => {}
            E::Literal { .. } => check_type(e.static_type, e.source_location, machine, errors),
        }
    }

    fn stmt(s: &Statement, machine: bool, errors: &mut Vec<Diagnostic>) {
        match &s.statement_type {
            S::FunctionDeclaration {
                parameters,
                return_type,
                return_location,
                body,
                ..
            } => {
                for p in parameters {
                    check_type(p.static_type, p.type_location, machine, errors);
                }
                if let Some(t) = return_type {
                    check_type(*t, return_location.unwrap(), machine, errors);
                }
                for s in body {
                    stmt(s, machine, errors);
                }
            }
            S::VariableDeclaration {
                static_type,
                type_location,
                name_location,
                initializer,
                ..
            } => {
                let before = errors.len();
                check_type(
                    static_type.unwrap(),
                    type_location.unwrap_or(*name_location),
                    machine,
                    errors,
                );
                if errors.len() == before {
                    if let Some(e) = initializer {
                        expr(e, machine, errors);
                    }
                }
            }
            S::Block(body) | S::SetupDeclaration(body) | S::LoopDeclaration(body) => {
                for s in body {
                    stmt(s, machine, errors);
                }
            }
            S::Expression(e) | S::Return(Some(e)) => expr(e, machine, errors),
            S::If {
                condition,
                then_body,
                else_body,
            } => {
                expr(condition, machine, errors);
                stmt(then_body, machine, errors);
                if let Some(s) = else_body {
                    stmt(s, machine, errors);
                }
            }
            S::While { condition, body } => {
                expr(condition, machine, errors);
                stmt(body, machine, errors);
            }
            S::For { from, to, .. } => errors.push(Diagnostic::new(
                DiagnosticKind::TypedIrUnsupported {
                    feature: "for ranges (endpoint and overflow rules are not specified yet)"
                        .into(),
                },
                Some(SourceLocation::new_from_start_end(
                    from.source_location,
                    to.source_location,
                )),
            )),
            _ => {}
        }
    }

    let mut errors = Vec::new();
    for s in statements {
        stmt(s, machine, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
