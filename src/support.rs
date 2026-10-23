//! Explicit AVR support boundary. Unsupported language features never reach code generation.
use crate::{
    diagnostic::Diagnostic,
    frontend::{
        expression::{Expression, ExpressionType as E},
        statement::{Statement, StatementType as S},
        static_type::StaticType as T,
        token_type::{LiteralType as L, PinModeConstantType as M, PunctuationType as P, TokenType},
    },
};
use std::collections::HashSet;

pub fn validate_pins(statements: &[Statement], mcu: &str) -> Result<(), Vec<Diagnostic>> {
    if crate::backend::microcontroller::get(mcu).is_none() {
        return Err(vec![Diagnostic::new(
            "E_TARGET",
            format!("Unsupported MCU '{mcu}'; supported: atmega328p"),
            None,
        )]);
    }
    let mut errors = vec![];
    let mut pins = HashSet::new();
    for statement in statements {
        if let S::DefDeclaration(declarations) = &statement.statement_type {
            for declaration in declarations {
                let pin = &declaration.pin;
                let valid = crate::backend::microcontroller::ATMEGA328P
                    .has_digital_pin(&pin.port, pin.pin_number);
                if !valid {
                    errors.push(Diagnostic::new(
                        "E_PIN",
                        format!(
                            "Port{}.{} is not a digital pin on ATmega328P",
                            pin.port, pin.pin_number
                        ),
                        Some(declaration.source_location),
                    ));
                }
                if !pins.insert((pin.port.clone(), pin.pin_number)) {
                    errors.push(Diagnostic::new(
                        "E_PIN_DUPLICATE",
                        "The same physical pin is declared more than once",
                        Some(declaration.source_location),
                    ));
                }
                if declaration.pin_mode == M::InputPullDown {
                    errors.push(Diagnostic::new(
                        "E_PIN_MODE",
                        "ATmega328P has no internal input pull-down resistor",
                        Some(declaration.source_location),
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

pub fn validate(statements: &[Statement], mcu: &str) -> Result<(), Vec<Diagnostic>> {
    validate_pins(statements, mcu)?;
    let mut errors = vec![];
    for statement in statements {
        validate_statement(statement, &mut errors);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
fn validate_statement(statement: &Statement, errors: &mut Vec<Diagnostic>) {
    let unsupported =
        |message| Diagnostic::new("E_UNSUPPORTED", message, Some(statement.source_location));
    match &statement.statement_type {
        S::DefDeclaration(declarations) => {
            for pin in declarations {
                if pin.pin_mode == M::InputPullUp { errors.push(Diagnostic::new("E_UNSUPPORTED", "INPUT_PULLUP code generation is not implemented yet",Some(pin.source_location))); }
            }
        }
        S::SetupDeclaration(body) | S::LoopDeclaration(body) | S::Block(body) => for s in body { validate_statement(s,errors); },
        S::If { condition,then_body,else_body } => {
            if !pin_condition(condition) { errors.push(Diagnostic::new("E_UNSUPPORTED", "AVR conditions currently require a pin == HIGH/LOW comparison; boolean operators (short-circuit), ! and other conditions are not lowered yet",Some(condition.source_location))); }
            validate_statement(then_body,errors);
            if let Some(body) = else_body { validate_statement(body,errors); }
        }
        S::Expression(expr) if pin_assignment(expr) => {},
        S::Expression(expr) => errors.push(Diagnostic::new("E_UNSUPPORTED", "AVR currently supports pin assignments of constant HIGH/LOW or INPUT/OUTPUT; !Pin is typed but its single-read inversion is not lowered yet",Some(expr.source_location))),
        S::VariableDeclaration { .. } => errors.push(unsupported("Variable storage is not implemented by the AVR backend yet; use --emit ir to inspect supported legacy IR")),
        _ => errors.push(unsupported("Loop code generation is not implemented yet (the top-level loop block is supported)")),
    }
}
fn ungroup(mut expr: &Expression) -> &Expression {
    while let E::Grouping { expression } = &expr.expression_type {
        expr = expression;
    }
    expr
}
fn pin_condition(expr: &Expression) -> bool {
    if let E::Binary {
        left,
        operator: TokenType::Punctuation(P::EqualEqual),
        right,
    } = &ungroup(expr).expression_type
    {
        let (pin, state) = if left.static_type == T::Pin {
            (left, right)
        } else {
            (right, left)
        };
        return pin.static_type == T::Pin
            && matches!(ungroup(pin).expression_type, E::Identifier(_))
            && matches!(
                ungroup(state).expression_type,
                E::Literal {
                    value: L::PinState(_)
                }
            );
    }
    false
}
fn pin_assignment(expr: &Expression) -> bool {
    match &ungroup(expr).expression_type {
        E::Assignment { expression, .. }
            if matches!(expr.static_type, T::PinState | T::PinMode) =>
        {
            constant_pin_value(expression)
        }
        _ => false,
    }
}
fn constant_pin_value(expr: &Expression) -> bool {
    match &ungroup(expr).expression_type {
        E::Literal {
            value: L::PinState(_) | L::PinMode(M::Input | M::Output),
        } => true,
        E::Assignment { .. } => pin_assignment(expr),
        _ => false,
    }
}
