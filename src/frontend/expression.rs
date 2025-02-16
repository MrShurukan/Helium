use std::error::Error;
use std::fmt::{Display, Formatter};
use colored::Colorize;
use crate::frontend::token::Token;
use crate::frontend::token_type::{LiteralType, PunctuationType, TokenType};
use crate::frontend::utils::{LocatableInSource, SourceLocation};

#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct Expression {
    pub expression_type: ExpressionType,
    pub source_location: SourceLocation,
}

impl LocatableInSource for Expression {
    fn get_location(&self) -> &SourceLocation {
        &self.source_location
    }
}

#[derive(Debug, Clone)]
pub enum ExpressionType {
    Binary { left: Box<Expression>, operator: TokenType, right: Box<Expression> },
    Ternary { first: Box<Expression>, second: Box<Expression>, third: Box<Expression> },
    Grouping { expression: Box<Expression> },
    Literal { value: LiteralType },
    Unary { operator: PunctuationType, right: Box<Expression> },
    Assignment { identifier: String, expression: Box<Expression> }
}

// TODO: Code inconsistency! These functions panic instead of returning a result like similar parts
// TODO: (like `PinRef`) 
// I'm not sure of the solution as of right now
impl Expression {
    pub fn new_assignment(identifier: &Expression, expression: Expression) -> Self {
        if let ExpressionType::Literal { value: LiteralType::Identifier(name) } = &identifier.expression_type {
            Self {
                source_location: SourceLocation::new_from_start_end(identifier.source_location, expression.source_location),
                expression_type: ExpressionType::Assignment {
                    identifier: name.to_owned(),
                    expression: Box::new(expression),
                },
            }
        }
        else {
            unreachable!("Non-identifier token passed to new_assignment expression function");
        }
    }

    pub fn new_ternary(first: Expression, second: Expression, third: Expression) -> Self {
        Self {
            source_location: SourceLocation::new_from_start_end(first.source_location, third.source_location),
            expression_type: ExpressionType::Ternary {
                first: Box::new(first),
                second: Box::new(second),
                third: Box::new(third)
            },
        }
    }

    pub fn new_binary(left: Expression, operator: &Token, right: Expression) -> Self {
        Self {
            source_location: SourceLocation::new_from_start_end(left.source_location, right.source_location),
            expression_type: ExpressionType::Binary {
                left: Box::new(left),
                operator: operator.token_type.clone(),
                right: Box::new(right)
            },
        }
    }

    pub fn new_unary(operator: &Token, expression: Expression) -> Self {
        if let Token { token_type: TokenType::Punctuation(op), .. } = operator {
            Self {
                source_location: SourceLocation::new_from_start_end(operator.source_location, expression.source_location),
                expression_type: ExpressionType::Unary { operator: op.to_owned(), right: Box::new(expression) },
            }
        }
        else {
            unreachable!("Non-punctuation token passed to new_unary expression function");
        }
    }

    pub fn new_literal(token: &Token) -> Self {
        if let Token { token_type: TokenType::Literal(literal_type), .. } = token {
            Self {
                source_location: token.source_location,
                expression_type: ExpressionType::Literal { value: literal_type.clone() },
            }
        }
        else {
            unreachable!("Non-literal token passed to new_literal expression function");
        }
    }

    pub fn new_literal_custom_type(token: &Token, literal_type: LiteralType) -> Self {
        Self {
            source_location: token.source_location,
            expression_type: ExpressionType::Literal { value: literal_type },
        }
    }

    pub fn new_grouping(expression: Expression) -> Self {
        Self {
            source_location: SourceLocation {
                line_start: expression.source_location.line_start,
                line_end: expression.source_location.line_end,
                start_offset: expression.source_location.start_offset - 1,
                end_offset: expression.source_location.end_offset + 1,
            },
            expression_type: ExpressionType::Grouping { expression: Box::new(expression) },
        }
    }
}

impl Display for Expression {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.expression_type {
            ExpressionType::Binary { left, operator, right } =>
                write!(f, "{left} {operator:?} {right}"),
            ExpressionType::Ternary { first, second, third } =>
                write!(f, "{first} ? {second} : {third}"),
            ExpressionType::Grouping { expression } => 
                write!(f, "({expression})"),
            ExpressionType::Literal { value } =>
                write!(f, "{value:?}"),
            ExpressionType::Unary { operator, right } =>
                write!(f, "({operator:?} {right})"),
            ExpressionType::Assignment { identifier, expression } =>
                write!(f, "{identifier} = {expression}")
        }
    }
}

/// A reference to a pin
/// Like `PortB.2`
#[derive(Debug, Clone)]
pub struct PinRef {
    pub port: String,
    pub pin_number: usize
}

impl PinRef {
    pub fn new(port: Token, pin_number: Token) -> Result<PinRef, PinRefError> {
        if let TokenType::Port(port_name) = port.token_type {
            if let TokenType::Literal(LiteralType::Integer(pin_int)) = pin_number.token_type {
                let pin_number_convert = usize::try_from(pin_int)
                    .map_err(|_| PinRefError { 
                        token: pin_number, 
                        message: "Invalid number for a pin (maybe it's too large?)"
                    });
                
                Ok(PinRef { port: port_name, pin_number: pin_number_convert? })
            }
            else {
                Err(PinRefError { token: pin_number, message: "Expected integer as pin number" })
            }
        }
        else {
            Err(PinRefError {
                token: port,
                message: "Incorrect port token"
            })
        }
    }
}

#[derive(Debug)]
pub struct PinRefError {
    pub token: Token,
    message: &'static str
}

impl Error for PinRefError {}
impl Display for PinRefError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message.red())
    }
}