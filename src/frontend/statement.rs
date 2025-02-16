use crate::frontend::expression::{Expression, PinRef};
use crate::frontend::token::Token;
use crate::frontend::token_type::{ConstantType, LiteralType, PinModeConstantType, TokenType};
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

/// Statements are the backbone of the entire language.
/// Source file represents a list of statements when parsed
#[derive(Debug)]
#[allow(clippy::struct_field_names)]
pub struct Statement {
    statement_type: StatementType,
    pub line_start: usize,
    pub line_end: usize,
    /// Offset in grapheme count from the start of the first line
    pub start_offset: usize,
    /// Offset in grapheme count from the start of the last file
    pub end_offset: usize
}

#[derive(Debug)]
pub enum StatementType {
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

impl Statement {
    pub fn new_expression(expression: Expression) -> Self {
        Self {
            line_start: expression.line_start,
            line_end: expression.line_end,
            start_offset: expression.start_offset,
            end_offset: expression.end_offset,
            statement_type: StatementType::Expression(expression)
        }
    }

    pub fn new_var_declaration(identifier: Token, initializer: Option<Expression>) -> Self {
        let (line_end, end_offset) = if let Some(initializer) = &initializer {
            (initializer.line_end, initializer.end_offset)
        }
        else {
            (identifier.line, identifier.line_offset + identifier.lexeme_size)
        };
        
        let TokenType::Literal(LiteralType::Identifier(ident)) = identifier.token_type
        else { unreachable!("Non-identifier passed to var statement") };
        
        Self {
            line_start: identifier.line,
            line_end,
            start_offset: identifier.line_offset,
            end_offset,
            statement_type: StatementType::VariableDeclaration { identifier: ident, initializer } 
        }
    }
    
    pub fn new_def_declaration(def_declarations: Rc<[SingleDefDeclaration]>) -> Self {
        if def_declarations.is_empty() {
            unreachable!("No SingleDefDeclaration was passed to new_def_declaration")
        }
        let first = def_declarations.first().unwrap();
        let last = def_declarations.last().unwrap();
        
        Self {
            line_start: first.line_start,
            line_end: last.line_end,
            start_offset: first.start_offset,
            end_offset: last.end_offset,
            statement_type: StatementType::DefDeclaration(def_declarations),
        }
    }
    
    pub fn new_setup_declaration(body: Box<Statement>) -> Self {
        Self {
            line_start: body.line_start,
            line_end: body.line_end,
            start_offset: body.start_offset,
            end_offset: body.end_offset,
            statement_type: StatementType::SetupDeclaration(body),
        }
    }

    pub fn new_loop_declaration(body: Box<Statement>) -> Self {
        Self {
            line_start: body.line_start,
            line_end: body.line_end,
            start_offset: body.start_offset,
            end_offset: body.end_offset,
            statement_type: StatementType::LoopDeclaration(body),
        }
    }

    /// `opening_brace` and `closing_brace` are used only to calculate the size of the block
    pub fn new_block(opening_brace: &Token, closing_brace: &Token, statements: Rc<[Statement]>) -> Self {
        Self {
            line_start: opening_brace.line,
            line_end: closing_brace.line,
            start_offset: opening_brace.line_offset,
            end_offset: closing_brace.line_offset,
            statement_type: StatementType::Block(statements),
        }
    }
    
    pub fn new_if(condition: Expression, then_body: Box<Statement>, else_body: Option<Box<Statement>>) -> Self {
        let (line_end, end_offset) = if let Some(else_body) = &else_body {
            (else_body.line_end, else_body.end_offset)
        }
        else {
            (then_body.line_end, then_body.end_offset)
        };
        
        Self {
            line_start: condition.line_start,
            line_end,
            start_offset: condition.start_offset,
            end_offset,
            statement_type: StatementType::If { condition, then_body, else_body },
        }
    }
    
    pub fn new_while(condition: Expression, body: Box<Statement>) -> Self {
        Self {
            line_start: condition.line_start,
            line_end: body.line_end,
            start_offset: condition.start_offset,
            end_offset: body.end_offset,
            statement_type: StatementType::While { condition, body },
        }
    }
    
    pub fn new_for(identifier: Token, from: Expression, to: Expression, body: Box<Statement>) -> Self {
        let TokenType::Literal(LiteralType::Identifier(ident)) = identifier.token_type
        else { unreachable!("Non-identifier passed to for statement") };
        
        Self {
            line_start: identifier.line,
            line_end: body.line_end,
            start_offset: identifier.line_offset,
            end_offset: body.end_offset,
            statement_type: StatementType::For { identifier: ident, from, to, body },
        }
    }
}

impl Display for Statement {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.statement_type)
    }
}

impl Display for StatementType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            StatementType::Expression(exp) => 
                write!(f, "{exp};"),
            StatementType::VariableDeclaration { identifier, initializer } => 
                if let Some(init) = initializer { 
                    write!(f, "var {identifier} = {init};") 
                } else { 
                    write!(f, "var {identifier};")
                }
            StatementType::DefDeclaration(declarations) => {
                writeln!(f, "def {{")?;
                for declaration in declarations.iter() {
                    writeln!(f, "{} = {}.{} {:?};",
                        declaration.identifier, declaration.port.port,
                        declaration.port.pin_number , declaration.pin_mode)?;
                }
                write!(f, "}}")
            },
            StatementType::Block(statements) => {
                writeln!(f, "{{")?;
                for statement in statements.iter() {
                    writeln!(f, "{statement}")?;
                }
                write!(f, "}}")
            },
            StatementType::If { condition, then_body, else_body } => {
                write!(f, "if {condition} {then_body}")?;
                if let Some(else_body) = else_body {
                    write!(f, " else {else_body}")?;
                }
                
                Ok(())
            }
            StatementType::While { condition, body } =>
                write!(f, "while {condition} {body}"),
            StatementType::For { identifier, from, to, body } =>
                write!(f, "for {identifier} = {from} to {to} {body}"),
            StatementType::SetupDeclaration(block) =>
                write!(f, "setup {block}"),
            StatementType::LoopDeclaration(block) =>
                write!(f, "loop {block}"),             
        }
    }
}

#[derive(Debug)]
pub struct SingleDefDeclaration {
    identifier: String,
    port: PinRef,
    pin_mode: PinModeConstantType,
    pub line_start: usize,
    pub line_end: usize,
    /// Offset in grapheme count from the start of the first line
    pub start_offset: usize,
    /// Offset in grapheme count from the start of the last file
    pub end_offset: usize
}

impl SingleDefDeclaration {
    // TODO: Figure out a cleaner way with these if lets...
    pub fn new(identifier: Token, port: PinRef, pin_mode: Token) -> Result<SingleDefDeclaration, DefDeclarationError> {
        if let TokenType::Literal(LiteralType::Identifier(ident)) = identifier.token_type {
            if let TokenType::Constant(ConstantType::PinMode(pin)) = pin_mode.token_type {
                Ok(SingleDefDeclaration {
                    line_start: identifier.line,
                    line_end: pin_mode.line,
                    start_offset: identifier.line_offset,
                    end_offset: pin_mode.line_offset + pin_mode.lexeme_size,
                    
                    identifier: ident, 
                    port, 
                    pin_mode: *pin,
                })
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