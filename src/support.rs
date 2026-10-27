//! Explicit AVR support boundary. Unsupported language features never reach code generation.
use crate::resources::{HardwareResource, Reservations};
use crate::{
    diagnostic::{Diagnostic, DiagnosticKind},
    frontend::{
        expression::{Expression, ExpressionType as E},
        statement::{Statement, StatementType as S},
        static_type::StaticType as T,
        token_type::PinModeConstantType as M,
    },
};

pub fn validate_pins(
    statements: &[Statement],
    target: &crate::backend::microcontroller::Microcontroller,
    reservations: &mut Reservations<crate::frontend::utils::SourceLocation>,
) -> Result<(), Vec<Diagnostic>> {
    let mut errors = vec![];

    for statement in statements {
        if let S::DefDeclaration(declarations) = &statement.statement_type {
            for declaration in declarations {
                let pin = &declaration.pin;
                let valid = target.has_digital_pin(&pin.port, pin.pin_number);

                if !valid {
                    errors.push(Diagnostic::new(
                        DiagnosticKind::InvalidPin {
                            port: pin.port.clone(),
                            number: pin.pin_number,
                        },
                        Some(declaration.pin_location),
                    ));
                }

                let key = HardwareResource::Pin {
                    port: pin.port.clone(),
                    bit: pin.pin_number,
                };
                if let Err(previous) = reservations.claim(key, declaration.pin_location) {
                    errors.push(
                        Diagnostic::new(
                            DiagnosticKind::DuplicatePin,
                            Some(declaration.pin_location),
                        )
                        .with_label(previous, crate::diagnostic::LabelKind::PreviousDeclaration),
                    );
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

    // Only statement-position constant mode expressions are executable. Modes
    // used as values (comparisons, arguments, ternaries) are not IR values.
    fn mode_expr(e: &Expression, errors: &mut Vec<Diagnostic>) {
        use crate::frontend::token_type::LiteralType;
        match &e.expression_type {
            E::Literal {
                value: LiteralType::PinMode(mode),
            } => {
                if *mode == M::InputPullDown {
                    errors.push(Diagnostic::new(
                        DiagnosticKind::PullDown,
                        Some(e.source_location),
                    ));
                }
            }
            E::Grouping { expression } | E::Assignment { expression, .. } => {
                mode_expr(expression, errors)
            }
            _ => errors.push(Diagnostic::new(
                DiagnosticKind::TypedIrUnsupported {
                    feature: "first-class or computed PinMode values".into(),
                },
                Some(e.source_location),
            )),
        }
    }

    fn expr(e: &Expression, machine: bool, errors: &mut Vec<Diagnostic>) {
        use crate::frontend::token_type::{PunctuationType as P, TokenType};
        if let E::Binary {
            operator: TokenType::Punctuation(P::Slash),
            operator_location,
            ..
        } = &e.expression_type
        {
            errors.push(Diagnostic::new(
                DiagnosticKind::TypedIrUnsupported {
                    feature: "division (rounding and zero-divisor semantics are not defined yet)"
                        .into(),
                },
                Some(*operator_location),
            ));
        }
        // Direct/grouped pins are physical reads. Selecting a first-class pin is
        // not supported, even if the whole expression will later be discarded.
        if e.static_type != T::Pin || matches!(e.expression_type, E::Ternary { .. }) {
            let before = errors.len();
            check_type(e.static_type, e.source_location, machine, errors);
            if errors.len() != before {
                return;
            }
        }
        if let E::Literal {
            value: crate::frontend::token_type::LiteralType::String(text),
        } = &e.expression_type
        {
            if text.len() > u16::MAX as usize {
                errors.push(Diagnostic::new(
                    DiagnosticKind::TypedIrUnsupported {
                        feature: "text longer than 65535 bytes".into(),
                    },
                    Some(e.source_location),
                ));
            }
        }
        if let E::Binary { left, .. } = &e.expression_type {
            if left.static_type == T::Text {
                errors.push(Diagnostic::new(
                    DiagnosticKind::TypedIrUnsupported {
                        feature: "text operators; use byte-oriented methods".into(),
                    },
                    Some(e.source_location),
                ));
            }
        }
        e.for_each_child(|child| expr(child, machine, errors));
    }

    fn stmt(s: &Statement, machine: bool, errors: &mut Vec<Diagnostic>) {
        match &s.statement_type {
            S::TimerDeclaration(timer) => {
                for statement in &timer.body {
                    stmt(statement, machine, errors);
                }
            }
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
                if *return_type == Some(T::Text) {
                    errors.push(Diagnostic::new(
                        DiagnosticKind::TypedIrUnsupported {
                            feature:
                                "returning borrowed text (lifetime analysis is not implemented)"
                                    .into(),
                        },
                        *return_location,
                    ));
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
                if *static_type == Some(T::Text) && initializer.is_none() {
                    errors.push(Diagnostic::new(
                        DiagnosticKind::TypedIrUnsupported {
                            feature:
                                "uninitialized text references; provide a literal or another &str"
                                    .into(),
                        },
                        Some(*name_location),
                    ));
                }
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
            S::Block(body)
            | S::NoInterrupt(body)
            | S::SetupDeclaration(body)
            | S::LoopDeclaration(body) => {
                for s in body {
                    stmt(s, machine, errors);
                }
            }
            S::Expression(e) if e.static_type == T::PinMode => mode_expr(e, errors),
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
        if matches!(
            &s.statement_type,
            S::VariableDeclaration {
                static_type: Some(T::Text),
                ..
            }
        ) {
            errors.push(Diagnostic::new(
                DiagnosticKind::TypedIrUnsupported {
                    feature: "global text references (escaping borrows)".into(),
                },
                Some(s.source_location),
            ));
        }
        stmt(s, machine, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
