use crate::parser::statement::Statement;
use std::fmt::{Display, Formatter};

mod scanner;
mod statement;
mod token;
mod token_type;

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
}

pub struct ParserError;

impl Display for ParserError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        todo!("ParserError.Display")
    }
}