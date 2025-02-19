use crate::frontend::expression::{Expression, PinRef};
use crate::frontend::static_type::StaticType;
use crate::frontend::token::Token;
use crate::frontend::token_type::{ConstantType, LiteralType, PinModeConstantType, TokenType};
use crate::frontend::utils::{LocatableInSource, SourceLocation};
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

/// Statements are the backbone of the entire language.
/// Source file represents a list of statements when parsed
#[derive(Debug)]
#[allow(clippy::struct_field_names)]
pub struct Statement {
    pub statement_type: StatementType,
    pub source_location: SourceLocation,
}

impl LocatableInSource for Statement {
    fn get_location(&self) -> &SourceLocation {
        &self.source_location
    }
}

#[derive(Debug)]
pub enum StatementType {
    Expression(Expression),
    /// Static type will always be filled later during semantic analysis
    /// Same with initializer. If it's missing it will be filled with a default init value
    VariableDeclaration { identifier: String, static_type: Option<StaticType>, initializer: Option<Expression> },
    DefDeclaration(Vec<SingleDefDeclaration>),
    SetupDeclaration(Vec<Statement>),
    LoopDeclaration(Vec<Statement>),
    Block(Vec<Statement>),
    If { condition: Expression, then_body: Box<Statement>, else_body: Option<Box<Statement>> },
    While { condition: Expression, body: Box<Statement> },
    For { identifier: String, from: Expression, to: Expression, body: Box<Statement> }
}

impl Statement {
    pub fn new_expression(expression: Expression) -> Self {
        Self {
            source_location: expression.source_location,
            statement_type: StatementType::Expression(expression)
        }
    }

    pub fn new_var_declaration(var_keyword: Token, identifier: Token, static_type: Option<Token>, initializer: Option<Expression>) -> Self {
        let TokenType::Identifier(ident) = identifier.token_type
        else { unreachable!("Non-identifier passed to var statement") };
        
        let mut st_t: Option<StaticType> = None;
        if let Some(token) = &static_type {
            let TokenType::Type(t) = token.token_type
            else { unreachable!("Non-type passed to var statement") };
            
            st_t = Some(*t);
        }

        let end = if let Some(init) = &initializer {
            init.source_location
        }
        else if st_t.is_some() {
            static_type.unwrap().source_location
        }
        else {
            identifier.source_location
        };
        
        Self {
            source_location: SourceLocation::new_from_start_end(var_keyword.source_location, end),
            statement_type: StatementType::VariableDeclaration { identifier: ident, static_type: st_t, initializer }
        }
    }

    pub fn new_def_declaration(def_declarations: Vec<SingleDefDeclaration>) -> Self {
        if def_declarations.is_empty() {
            unreachable!("No SingleDefDeclaration was passed to new_def_declaration")
        }
        let first = def_declarations.first().unwrap();
        let last = def_declarations.last().unwrap();

        Self {
            source_location: SourceLocation::new_from_start_end(first.source_location, last.source_location),
            statement_type: StatementType::DefDeclaration(def_declarations),
        }
    }

    pub fn new_setup_declaration(body: Vec<Statement>, source_location: SourceLocation) -> Self {
        Self {
            source_location,
            statement_type: StatementType::SetupDeclaration(body),
        }
    }

    pub fn new_loop_declaration(body: Vec<Statement>, source_location: SourceLocation) -> Self {
        Self {
            source_location,
            statement_type: StatementType::LoopDeclaration(body),
        }
    }

    /// `opening_brace` and `closing_brace` are used only to calculate the size of the block
    pub fn new_block(opening_brace: &Token, closing_brace: &Token, statements: Vec<Statement>) -> Self {
        Self {
            source_location: SourceLocation::new_from_start_end(opening_brace.source_location, closing_brace.source_location),
            statement_type: StatementType::Block(statements),
        }
    }

    pub fn new_if(condition: Expression, then_body: Box<Statement>, else_body: Option<Box<Statement>>) -> Self {
        let (line_end, end_offset) = if let Some(else_body) = &else_body {
            (else_body.source_location.line_end, else_body.source_location.end_offset)
        }
        else {
            (then_body.source_location.line_end, then_body.source_location.end_offset)
        };

        Self {
            source_location: SourceLocation {
                line_start: condition.source_location.line_start,
                line_end,
                start_offset: condition.source_location.start_offset,
                end_offset,
            },
            statement_type: StatementType::If { condition, then_body, else_body },
        }
    }

    pub fn new_while(condition: Expression, body: Box<Statement>) -> Self {
        Self {
            source_location: SourceLocation::new_from_start_end(condition.source_location, body.source_location),
            statement_type: StatementType::While { condition, body },
        }
    }

    pub fn new_for(identifier: Token, from: Expression, to: Expression, body: Box<Statement>) -> Self {
        let TokenType::Identifier(ident) = identifier.token_type
        else { unreachable!("Non-identifier passed to for statement") };

        Self {
            source_location: SourceLocation::new_from_start_end(identifier.source_location, body.source_location),
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
            StatementType::VariableDeclaration { identifier, static_type, initializer } =>
            {
                match (static_type, initializer) {
                    (Some(st_type), Some(init)) =>
                        write!(f, "var {identifier}: {} = {init};", st_type.to_string().yellow()),
                    (Some(st_type), None) =>
                        write!(f, "var {identifier}: {};", st_type.to_string().yellow()),
                    (None, Some(init)) =>
                        write!(f, "{}var {identifier} = {init};", "[no type!]".red()),
                    (None, None) =>
                        write!(f, "{}var {identifier};", "[no type!]".red()),
                }   
            },
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
                Self::write_block(f, statements)
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
            StatementType::SetupDeclaration(statements) =>
                { write!(f, "setup ")?; Self::write_block(f, statements) },
            StatementType::LoopDeclaration(statements) =>
                { write!(f, "loop ")?; Self::write_block(f, statements) }
        }
    }
}

impl StatementType {
    fn write_block(f: &mut Formatter, statements: &Vec<Statement>) -> std::fmt::Result {
        writeln!(f, "{{")?;
        for statement in statements.iter() {
            writeln!(f, "{statement}")?;
        }
        write!(f, "}}")
    }
}

#[derive(Debug)]
pub struct SingleDefDeclaration {
    pub identifier: String,
    pub port: PinRef,
    pub pin_mode: PinModeConstantType,
    pub source_location: SourceLocation,
}

impl LocatableInSource for SingleDefDeclaration {
    fn get_location(&self) -> &SourceLocation {
        &self.source_location
    }
}

impl SingleDefDeclaration {
    // TODO: Figure out a cleaner way with these if lets...
    pub fn new(identifier: Token, port: PinRef, pin_mode: Token) -> Result<SingleDefDeclaration, DefDeclarationError> {
        if let TokenType::Identifier(ident) = identifier.token_type {
            if let TokenType::Constant(ConstantType::PinMode(pin)) = pin_mode.token_type {
                Ok(SingleDefDeclaration {
                    source_location: SourceLocation::new_from_start_end(identifier.source_location, pin_mode.source_location),
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