use crate::parser::scanner::{Scanner, ScannerError};
use crate::parser::statement::Statement;
use crate::parser::token::Token;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::iter::Peekable;
use std::slice::Iter;
use colored::Colorize;
use crate::parser::token_type::{TokenType as TT, KeywordType as KT, 
                                PunctuationType as PT, LiteralType as LT};

mod scanner;
mod statement;
mod token;
mod token_type;
mod utils;
mod position_info;

/// Main struct for the front-end of helium. This class receives a source file,
/// starts up a scanner, gets tokens and then parses them into a list of statements.
///
/// This is a top-down parser, more specifically a Recursive Descent Parser.
///
/// Grammar is defined in `grammar.txt` file in the root of the repo
pub struct Parser<'a> {
    /// Reference to an original string to point out errors with a location
    pub source: &'a str,
    /// Current index in tokens peekable iterator
    index: usize
}

/// Performs a check on the peeked token. If it matched the `enum_types` pattern then it's consumed.
/// Otherwise, an error is thrown that the token was missing with the `error_message` error message
macro_rules! check_and_consume {
    ($self:ident, $tokens:ident, $enum_types:pat_param, $error_message:literal) => {
        let current_token = &Self::peek($tokens)?;
        if let $enum_types = current_token.token_type {
            advance!($self, $tokens);
        }
        else {
            let previous_token = $self.tokens.iter().nth($self.index - 1).expect("No previous token available while parsing");
            return Err(Self::get_error_marked_line(
                ParserErrorType::MissingToken(String::from($error_message)), (*previous_token).clone(), &$self.source
            ));
        }
    };
}

/// A version of the `check_and_consume` but this one actually clones and returns the parsed
/// token. Unless you need the token use the `check_and_consume` instead
macro_rules! check_and_consume_return {
    ($self:ident, $tokens:ident, $enum_types:pat_param, $error_message:literal) => {
        {
            let current_token = &self.peek($tokens)?.to_owned();
            if let $enum_types = current_token.token_type {
                advance!($self, $tokens);
            }
            else {
                let previous_token = $self.tokens.iter().nth($self.index - 1).expect("No previous token available while parsing");
                return Err(Self::get_error_marked_line(
                    ParserErrorType::MissingToken(String::from($error_message)), (*previous_token).clone(), &$self.source
                ));
            }

            current_token
        }
    };
}

impl Parser<'_> {
    pub fn new(source: &str) -> Parser {
        Parser { source, index: 0 }
    }

    /// Main function - parses the source file and gets out a list of statements (or a list of errors).
    /// This list then can be used to statically check and analyze the language before
    /// passing it to back-end (which will convert it to assembly language).
    ///
    /// This is parsed according to rules in `grammar.txt` (root of the repo)
    pub fn parse(&mut self) -> Result<Vec<Statement>, Vec<ParserError>> {
        let mut statements: Vec<Statement> = Vec::new();
        let mut errors: Vec<ParserError> = Vec::new();

        let tokens = self.scan()?;
        let mut token_peekable = tokens.iter().peekable();

        while let Some(Token { token_type, ..}) = &token_peekable.peek() {
            match token_type {
                // EOF means statements are over and the parsing is done
                TT::Eof => { break; },
                _ => {}
            };

            // Program is a list of declarations (see grammar.txt)
            let result = self.declaration(&mut token_peekable);
            if let Ok(statement) = result {
                statements.push(statement);
            }
            else {
                errors.push(result.err().unwrap())
            }
        }

        if !errors.is_empty() {
            Err(errors)
        }
        else {
            Ok(statements)
        }
    }

    fn declaration(&mut self, tokens: &mut Peekable<Iter<Token>>) -> Result<Statement, ParserError> {
        let token = &Self::peek(tokens)?;
        let result = match token.token_type {
            TT::Keyword(KT::Var) => { self.advance(tokens); self.var_declaration(tokens) },
            TT::Keyword(KT::Def) => { self.advance(tokens); self.def_declaration(tokens) },
            _ => self.statement(tokens)
        };

        // If we got an error, we align our tokens with the next valid entry point.
        // See `self.synchronize` comments for more info
        if result.is_err() {
            self.synchronize(tokens);
        }

        result
    }

    fn var_declaration(&mut self, tokens: &mut Peekable<Iter<Token>>) -> Result<Statement, ParserError> {
        let name = check_and_consume_return!(self, tokens,TT::Literal(LT::Identifier(_)), 
            "Expected a variable name");

        let mut initializer: Option<Expression> = None;
        if let TT::Punctuation(PT::Equal) = &Self::peek(tokens)?.token_type {
            advance!(self, tokens);
            initializer = Some(self.expression(tokens)?);
        }

        check_and_consume!(self, tokens, TT::Punctuation(PT::Semicolon), "Expected a ';' after variable declaration");
        Ok(Statement::VariableDeclaration { identifier: name, initializer })
    }

    fn def_declaration(&mut self, tokens: &mut Peekable<Iter<Token>>) -> Result<Statement, ParserError> {

    }

    fn statement(&mut self, tokens: &mut Peekable<Iter<Token>>) -> Result<Statement, ParserError> {

    }

    fn advance<'a>(&mut self, peekable: &'a mut Peekable<Iter<Token>>) -> Option<&'a Token> {
        let next = peekable.next();
        if next.is_some() {
            self.index += 1;
        }

        next
    }

    fn peek<'a>(tokens: &mut Peekable<Iter<'a, Token>>) -> Result<&'a Token, ParserError> {
        let token = tokens.peek();
        if token.is_none() {
            Err(ParserError::new_without_token(ParserErrorType::RanOutOfTokens))
        }
        else {
            Ok(token.unwrap())
        }
    }

    /// The goal of this method is to align parser with a next statement
    /// after error was found somewhere.
    ///
    /// Say, an incorrect ternary `1 ? : 2` - this won't parse very well.
    /// If we just continue marching forward from encountering that ":" - we will report
    /// a lot of errors down the line that may not be truthful (phantom errors).
    ///
    /// This method attempts to minimize those by aligning with the next statement.
    /// It's not 100% correct, but it's good enough
    fn synchronize(&mut self, tokens: &mut Peekable<Iter<Token>>) {
        let current_token = tokens.peek();
        // If we ran out of tokens - we're done already
        if current_token.is_none() {
            return;
        }

        let mut old_token_type = current_token.unwrap().token_type.to_owned();
        while let Some(token) = self.advance(tokens) {
            match old_token_type {
                // A semicolon is a very clear statement boundary
                TT::Punctuation(PT::Semicolon) => { return; },
                // Continue otherwise
                _ => {}
            }

            match token.token_type {
                // Keyword is a nice way to know we are aligned
                TT::Keyword(_) => { return; }
                // Just keep on marching otherwise
                _ => {}
            }

            old_token_type = token.token_type.to_owned();
        }
    }

    fn scan(&self) -> Result<Vec<Token>, Vec<ParserError>> {
        let mut scanner = Scanner::new(self.source);
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
    error_type: ParserErrorType,
    token: Option<Token>,
    location: Option<String>
}

impl ParserError {
    pub fn new_without_token<'a>(error_type: ParserErrorType) -> ParserError {
        ParserError {
            error_type,
            token: None,
            location: None
        }
    }
}

impl Error for ParserError {}
impl Display for ParserError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let error_header = match &self.error_type {
            ParserErrorType::ScannerError(_) => "",
            _ => {
                if let Some(token) = &self.token {
                    &format!("[line {}] Parser Error", token.line)
                }
                else {
                    "Parser Error"
                }
            }
        };

        let output = match &self.error_type {
            ParserErrorType::ScannerError(scanner_error) =>
                &format!("{scanner_error}"),
            ParserErrorType::UnclosedParenthesis => "Unclosed parenthesis",
            ParserErrorType::RanOutOfTokens => "No more tokens to parse",
            ParserErrorType::IncorrectToken => "Incorrect token in this place",
            ParserErrorType::UnfinishedTernary => "Incorrect ternary operator",
            ParserErrorType::BinaryOperatorNoLeftPart => "Binary operator without a left part",
            ParserErrorType::MissingToken(message) =>
                &format!("Missing token. {}", message),
            ParserErrorType::InvalidAssignmentTarget => "Invalid assignment target",
        };

        write!(f, "{error_header}: {}", output)?;
        if let Some(location) = &self.location {
            write!(f, "\n\n{}\n{location}", "Happened here:".green())?;
        }

        if self.token.is_none() {
            write!(f, "{}", "(internal parser error, something went wrong)".yellow())?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum ParserErrorType {
    ScannerError(ScannerError),
    UnclosedParenthesis,
    RanOutOfTokens,
    IncorrectToken,
    UnfinishedTernary,
    BinaryOperatorNoLeftPart,
    MissingToken(String),
    InvalidAssignmentTarget
}

impl From<ScannerError> for ParserError {
    fn from(value: ScannerError) -> Self {
        Self {
            error_type: ParserErrorType::ScannerError(value),
            token: None,
            location: None,
        }
    }
}