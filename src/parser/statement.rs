use crate::parser::expression::{Expression, PinRef};
use crate::parser::token::Token;
use crate::parser::token_type::{ConstantType, LiteralType, PinModeConstantType, TokenType};
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

/// Statements are the backbone of the entire language.
/// Source file represents a list of statements when parsed
#[derive(Debug)]
pub enum Statement {
    Expression(Expression),
    VariableDeclaration { identifier: String, initializer: Option<Expression> },
    DefDeclaration(Rc<[SingleDefDeclaration]>),
    SetupDeclaration(Box<Statement>),
    LoopDeclaration(Box<Statement>),
    Block(Rc<[Statement]>),
    If { condition: Expression, then_body: Box<Statement>, else_body: Option<Box<Statement>> },
    While { condition: Expression, body: Box<Statement> },
    For { identifier: String, from: Expression, to: Expression, body: Box<Statement> }
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
                    write!(f, "{} = {}.{} {:?};\n",
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
            Statement::SetupDeclaration(block) =>
                write!(f, "setup {block}"),
            Statement::LoopDeclaration(block) =>
                write!(f, "loop {block}"),             
        }
    }
}

#[derive(Debug)]
pub struct SingleDefDeclaration {
    identifier: String,
    port: PinRef,
    pin_mode: PinModeConstantType
}

impl SingleDefDeclaration {
    // TODO: Figure out a cleaner way with these if lets...
    pub fn new(identifier: Token, port: PinRef, pin_mode: Token) -> Result<SingleDefDeclaration, DefDeclarationError> {
        if let TokenType::Literal(LiteralType::Identifier(ident)) = identifier.token_type {
            if let TokenType::Constant(ConstantType::PinMode(pin)) = pin_mode.token_type {
                Ok(SingleDefDeclaration { identifier: ident, port, pin_mode: *pin })
            } else {
                Err(DefDeclarationError { token: pin_mode, message: "Expected pin mode (for example 'INPUT) in this place" })             
            }         
        } else {             
            Err(DefDeclarationError { token: identifier, message: "Expected identifier in this place" })         
        }
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