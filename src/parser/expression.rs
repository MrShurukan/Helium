use std::error::Error;
use std::fmt::{Display, Formatter};
use std::pin::Pin;
use colored::Colorize;
use crate::parser::token::Token;
use crate::parser::token_type::{LiteralType, PunctuationType, TokenType};

#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct Expression {
    pub expression_type: ExpressionType,
    pub line_start: usize,
    pub line_end: usize,
    /// Offset in grapheme count from the start of the first line
    pub start_offset: usize,
    /// Offset in grapheme count from the start of the last file
    pub end_offset: usize
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
    pub fn new_assignment(identifier: Expression, expression: Expression) -> Self {
        if let ExpressionType::Literal { value: LiteralType::Identifier(name) } = &identifier.expression_type {
            Self {
                line_start: identifier.line_start,
                line_end: expression.line_end,
                start_offset: identifier.start_offset,
                end_offset: expression.end_offset,
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
            line_start: first.line_start,
            line_end: third.line_end,
            start_offset: first.start_offset,
            end_offset: third.end_offset,
            expression_type: ExpressionType::Ternary {
                first: Box::new(first),
                second: Box::new(second),
                third: Box::new(third)
            },
        }
    }

    pub fn new_binary(left: Expression, operator: &Token, right: Expression) -> Self {
        Self {
            line_start: left.line_start,
            line_end: right.line_end,
            start_offset: left.start_offset,
            end_offset: right.end_offset,
            expression_type: ExpressionType::Binary {
                left: Box::new(left),
                operator: operator.token_type.to_owned(),
                right: Box::new(right)
            },
        }
    }

    pub fn new_unary(operator: &Token, expression: Expression) -> Self {
        if let Token { token_type: TokenType::Punctuation(op), .. } = operator {
            Self {
                line_start: operator.line,
                line_end: expression.line_end,
                start_offset: operator.line_offset,
                end_offset: expression.end_offset,
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
                expression_type: ExpressionType::Literal { value: literal_type.to_owned() },
                line_start: token.line,
                line_end: token.line,
                start_offset: token.line_offset,
                end_offset: token.line_offset + token.lexeme_size,
            }
        }
        else {
            unreachable!("Non-literal token passed to new_literal expression function");
        }
    }

    pub fn new_literal_custom_type(token: &Token, literal_type: LiteralType) -> Self {
        Self {
            expression_type: ExpressionType::Literal { value: literal_type },
            line_start: token.line,
            line_end: token.line,
            start_offset: token.line_offset,
            end_offset: token.line_offset + token.lexeme_size,
        }
    }

    pub fn new_grouping(expression: Expression) -> Self {
        Self {
            line_start: expression.line_start,
            line_end: expression.line_end,
            start_offset: expression.start_offset - 1,
            end_offset: expression.end_offset + 1,
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
                Ok(PinRef { port: port_name, pin_number: pin_int as usize })
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