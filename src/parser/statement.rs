use crate::parser::expression::{Expression, PinRef};
use crate::parser::token::Token;
use crate::parser::token_type::{ConstantType, LiteralType, TokenType};
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

/// Statements are the backbone of the entire language.
/// Source file represents a list of statements when parsed
#[derive(Debug)]
pub enum Statement {
    Expression(Expression),
    VariableDeclaration { identifier: Token, initializer: Option<Expression> },
    DefDeclaration(Rc<[SingleDefDeclaration]>),
    Block(Rc<[Statement]>),
    If { condition: Expression, then_body: Box<Statement>, else_body: Option<Box<Statement>> },
    While { condition: Expression, body: Box<Statement> }
}

#[derive(Debug)]
pub struct SingleDefDeclaration {
    /// TokenType::Literal(LiteralType::Identifier)
    identifier: Token,
    port: PinRef,
    /// TokenType::Constant(ConstantType::PinMode)
    pin_mode: Token
}

impl SingleDefDeclaration {
    pub fn new(identifier: Token, port: PinRef, pin_mode: Token) -> Result<SingleDefDeclaration, DefDeclarationError> {
        if let TokenType::Literal(LiteralType::Identifier(_)) = identifier.token_type {}
        else {
            return Err(DefDeclarationError { token: identifier, message: "Expected identifier in this place" })
        }

        if let TokenType::Constant(ConstantType::PinMode(_)) = pin_mode.token_type {}
        else {
            return Err(DefDeclarationError { token: pin_mode, message: "Expected pin mode (for example 'INPUT) in this place" })
        }
        
        Ok(SingleDefDeclaration { identifier, port, pin_mode })
    }
}

#[derive(Debug)]
pub struct DefDeclarationError {
    pub token: Token,
    message: &'static str
}

impl Error for DefDeclarationError {}
impl Display for DefDeclarationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message.red())
    }
}