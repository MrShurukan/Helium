use crate::parser::expression::{Expression, PinRef};
use crate::parser::token::Token;
use crate::parser::token_type::{ConstantType, LiteralType, TokenType};
use colored::Colorize;
use std::error::Error;
use std::fmt::{write, Display, Formatter};
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
    While { condition: Expression, body: Box<Statement> },
    For { identifier: Token, from: Expression, to: Expression, body: Box<Statement> }
}

impl Display for Statement {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Statement::Expression(exp) => 
                write!(f, "{exp};"),
            Statement::VariableDeclaration { identifier, initializer } => 
                if let Some(init) = initializer { 
                    write!(f, "var {identifier} = {init};") 
                } else { 
                    write!(f, "var {identifier};")
                }
            Statement::DefDeclaration(declarations) => {
                write!(f, "def {{\n")?;
                for declaration in declarations.iter() {
                    write!(f, "{} = {}.{} {};\n",
                        declaration.identifier, declaration.port.port,
                        declaration.port.pin_number , declaration.pin_mode)?;
                }
                write!(f, "}}")
            },
            Statement::Block(statements) => {
                write!(f, "{{\n")?;
                for statement in statements.iter() {
                    write!(f, "{}\n", statement)?;
                }
                write!(f, "}}")
            },
            Statement::If { condition, then_body, else_body } => {
                write!(f, "if {condition} {then_body}")?;
                if let Some(else_body) = else_body {
                    write!(f, " else {else_body}")?;
                }
                
                Ok(())
            }
            Statement::While { condition, body } =>
                write!(f, "while {condition} {body}"),
            Statement::For { identifier, from, to, body } =>
                write!(f, "for {identifier} = {from} to {to} {body}"),
        }
    }
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