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

/// A reference to a pin
/// Like `PortB.2`
#[derive(Debug, Clone)]
pub struct PinRef {
    /// TokenType::Port
    port: Token,
    /// TokenType::Literal(LiteralType::Integer)
    pin_number: Token
}

impl PinRef {
    pub fn new(port: Token, pin_number: Token) -> Result<PinRef, PinRefError> {
        if let TokenType::Port(_) = port.token_type {}
        else {
            return Err(PinRefError {
                token: port,
                message: "Incorrect port token"
            });
        }
        
        if let TokenType::Literal(LiteralType::Integer(_)) = pin_number.token_type {}
        else {
            return Err(PinRefError { token: pin_number, message: "Expected integer as pin number" });
        }
        
        Ok(PinRef { port, pin_number })
    }
}

#[derive(Debug)]
pub struct PinRefError {
    pub(crate) token: Token,
    message: &'static str
}

impl Error for PinRefError {}
impl Display for PinRefError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message.red())
    }
}