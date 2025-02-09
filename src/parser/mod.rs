use crate::parser::scanner::{Scanner, ScannerError};
use crate::parser::statement::Statement;
use crate::parser::token::Token;
use std::error::Error;
use std::fmt::{Display, Formatter};

mod scanner;
mod statement;
mod token;
mod token_type;
mod utils;
mod position_info;

/// Main struct for the front-end of helium. This class receives a source file,
/// starts up a scanner, gets tokens and then parses them into a list of statements
pub struct Parser;

impl Parser {
    pub fn new() -> Self {
        Self
    }

    /// Main function - parses the source file and gets out a list of statements (or a list of errors).
    /// This list then can be used to statically check and analyze the language before
    /// passing it to back-end (which will convert it to assembly language)
    pub fn parse(&self, source: &str) -> Result<Vec<Statement>, Vec<ParserError>> {
        todo!("Parser.parse")
    }
    
    /// Temporary function just to test scanner out
    pub fn scan(&self, source: &str) -> Result<Vec<Token>, Vec<ParserError>> {
        let mut scanner = Scanner::new(source);
        scanner.scan_tokens().map_err(|errors| {
            let mut new_vec: Vec<ParserError> = Vec::with_capacity(errors.len());
            for error in errors {
                new_vec.push(error.into());
            }
            new_vec
        })
    }
}

#[derive(Debug)]
pub struct ParserError {
    error_type: ParserErrorType
}

impl Error for ParserError {}
impl Display for ParserError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.error_type {
            ParserErrorType::ScannerError(scanner_error) => write!(f, "{scanner_error}")
        }
    }
}

#[derive(Debug)]
pub enum ParserErrorType {
    ScannerError(ScannerError)
}

impl From<ScannerError> for ParserError {
    fn from(value: ScannerError) -> Self {
        Self { error_type: ParserErrorType::ScannerError(value) }
    }
}