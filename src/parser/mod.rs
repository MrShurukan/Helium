use crate::parser::expression::Expression;
use crate::parser::scanner::{Scanner, ScannerError};
use crate::parser::statement::{DefDeclarationError, SingleDefDeclaration, Statement};
use crate::parser::token::Token;
use crate::parser::token_type::{KeywordType as KT, LiteralType as LT, PunctuationType as PT, TokenType as TT};
use crate::parser::utils::TokenIterator;
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

mod scanner;
mod statement;
mod token;
mod token_type;
mod utils;
mod position_info;
mod expression;

/// Main struct for the front-end of helium. This class receives a source file,
/// starts up a scanner, gets tokens and then parses them into a list of statements.
///
/// This is a top-down parser, more specifically a Recursive Descent Parser.
///
/// Grammar is defined in `grammar.txt` file in the root of the repo
pub struct Parser<'a> {
    /// Reference to an original string to point out errors with a location
    pub source: &'a str
}

/// Performs a check on the peeked token. If it matched the `enum_types` pattern then it's consumed.
/// Otherwise, an error is returned that the token was missing with the `error_message` error message
/// 
/// Returns the consumed token for convenience
macro_rules! check_and_consume {
    ($self:ident, $tokens:ident, $enum_types:pat_param, $error_message:literal) => {
        match Self::peek($tokens)?.token_type {
            $enum_types => $tokens.advance().unwrap(),
            _ => return Err($self.missing_token_error($error_message, $tokens))
        }
    };
}

impl Parser<'_> {
    pub fn new(source: &str) -> Parser {
        Parser { source }
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
        let mut tokens = TokenIterator::new(tokens.as_slice());

        while let Some(Token { token_type, ..}) = tokens.peek() {
            // EOF means statements are over and the parsing is done
            if let TT::Eof = token_type {
                break;
            };

            // Program is a list of declarations (see grammar.txt)
            let result = self.declaration(&mut tokens);
            if let Ok(statement) = result {
                statements.push(statement);
            }
            else {
                errors.push(result.err().unwrap());
            }
        }
        
        if errors.is_empty() {
            Ok(statements)
        }
        else {
            Err(errors)
        }
    }

    // -------------------- THIS PART IS DECLARATIONS AND STATEMENTS -------------------- //
    fn declaration(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        let token = Self::peek(tokens)?;
        let result = match token.token_type {
            TT::Keyword(KT::Var) => { tokens.advance(); self.var_declaration(tokens) },
            TT::Keyword(KT::Def) => { tokens.advance(); self.def_declaration(tokens) },
            _ => self.statement(tokens)
        };

        // If we got an error, we align our tokens with the next valid entry point.
        // See `Self::synchronize` comments for more info
        if result.is_err() {
            Self::synchronize(tokens);
        }

        result
    }

    fn var_declaration(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        let name = check_and_consume!(self, tokens, TT::Literal(LT::Identifier(_)), "Expected a variable name").clone();

        let mut initializer: Option<Expression> = None;
        if let TT::Punctuation(PT::Equal) = Self::peek(tokens)?.token_type {
            tokens.advance();
            initializer = Some(self.expression(tokens)?);
        }

        check_and_consume!(self, tokens, TT::Punctuation(PT::Semicolon), "Expected a ';' after variable declaration");
        Ok(Statement::VariableDeclaration { identifier: name, initializer })
    }

    fn def_declaration(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        // There are two types of def declaration. One is a one-liner, like this:
        // def Led = PortB.2;
        // Other one is multiline:
        // def {
        //     Led    = PortB.2;
        //     Button = PortC.2;
        // }
        
        match Self::peek(tokens)? {
            // Identifier = it's a one-liner
            Token { token_type: TT::Literal(LT::Identifier(_)), .. } => {
                let def_declaration = self.single_def_declaration(tokens)?;
                Ok(Statement::DefDeclaration(Rc::new([def_declaration])))
            },
            // Left brace = it's a multiline version
            Token { token_type: TT::Punctuation(PT::LeftBrace), .. } => {
                let mut def_declarations: Vec<SingleDefDeclaration> = Vec::new();
                loop {
                    let next_token = Self::peek(tokens)?;
                    if let TT::Punctuation(PT::RightBrace) = next_token.token_type { break; }
                    
                    def_declarations.push(self.single_def_declaration(tokens)?);
                }
                
                Ok(Statement::DefDeclaration(def_declarations.into()))
            },
            // Error
            token => {
                Err(self.get_error_marked_line(
                    ParserErrorType::MissingToken(String::from("Expected '{' or an identifier after 'def'")),
                    token.clone()
                ))
            }
        }
    }
    
    fn single_def_declaration(&mut self, tokens: &mut TokenIterator) -> Result<SingleDefDeclaration, ParserError> {
        let identifier = check_and_consume!(self, tokens, TT::Literal(LT::Identifier(_)), "Expected identifier after this token").clone();
        check_and_consume!(self, tokens, TT::Punctuation(PT::Equal), "Expected '=' after an identifier in a def statement");
        let port = check_and_consume!(self, tokens, TT::Port(_), "Expected a port name after an '='").clone();
        check_and_consume!(self, tokens, TT::Punctuation(PT::Semicolon), "Expected ';' after a port in a def statement");

        SingleDefDeclaration::new(identifier, port).map_err(|err| self.convert_def_error(err))
    }

    fn statement(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        let token = Self::peek(tokens)?;
        match token.token_type {
            TT::Keyword(KT::For) => { Self::advance(tokens)?; self.for_statement(tokens) }
            TT::Keyword(KT::If) => { Self::advance(tokens)?; self.if_statement(tokens) }
            TT::Keyword(KT::While) => { Self::advance(tokens)?; self.while_loop(tokens) }
            TT::Punctuation(PT::LeftBrace) => { Self::advance(tokens)?; self.block_statement(tokens) }
            _ => self.expression_statement(tokens)
        }
    }
    
    fn for_statement(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        todo!("for_statement")
    }

    fn if_statement(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        todo!("if_statement")
    }

    fn while_loop(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        todo!("while_loop")
    }

    fn block_statement(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        todo!("block_statement")
    }

    fn expression_statement(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        let value = self.expression(tokens)?;
        check_and_consume!(self, tokens, TT::Punctuation(PT::Semicolon), "Expected ';' after expression.");
        Ok(Statement::Expression(value))
    }

    // --------------------------- THIS PART IS EXPRESSIONS --------------------------- //
    fn expression(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        todo!("expression")
    }

    // ------------------------------- HELPER FUNCTIONS ------------------------------- //
    
    /// A helper wrapper around `tokens.peek()` function. 
    /// Returns an error if peeking yielded no results
    fn peek<'a>(tokens: &'a mut TokenIterator) -> Result<&'a Token, ParserError> {
        let token = tokens.peek();
        if let Some(token) = token {
            Ok(token)
        }
        else {
            Err(ParserError::new_without_token(ParserErrorType::RanOutOfTokens))
        }
    }

    /// A helper wrapper around `tokens.advance()` function. 
    /// Returns an error if advancing yielded no results
    fn advance<'a>(tokens: &'a mut TokenIterator) -> Result<&'a Token, ParserError> {
        let token = tokens.advance();
        if let Some(token) = token {
            Ok(token)
        }
        else {
            Err(ParserError::new_without_token(ParserErrorType::RanOutOfTokens))
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
    fn synchronize(tokens: &mut TokenIterator) {
        let current_token = tokens.peek();
        // If we ran out of tokens - we're done already
        if current_token.is_none() {
            return;
        }
        
        tokens.advance();

        while let Some(token) = tokens.peek() {
            // A semicolon is a very clear statement boundary
            if let TT::Punctuation(PT::Semicolon) = token.token_type {
                // Consume the semicolon
                tokens.advance();
                return;
            }

            // Keyword is a nice way to know we are aligned
            if let TT::Keyword(_) = token.token_type {
                return;
            }
        }
    }
    
    fn convert_def_error(&self, error: DefDeclarationError) -> ParserError {
        let token = error.token.clone();
        self.get_error_marked_line(ParserErrorType::InvalidDefDeclaration(error), token)
    }

    /// Gets a helper string for error displaying with a marker on the second line.
    /// ### Example
    /// This function is used to create a string that looks something like this:
    /// ```
    /// 14. var stuff = (1 + class) * 3
    ///                      ^^^^^
    /// ```
    /// Resulting string consists of two lines and has a marker denoted by a sequence of
    /// `^` on the second line. The line is parsed from `token` by the `token.line_offset` and
    /// the `token.lexeme_size` string. Line is fetched from `source` string.
    fn get_error_marked_line(&self, error_type: ParserErrorType, token: Token) -> ParserError {
        let line = self.source.split('\n').nth(token.line - 1);
        if line.is_none() {
            unreachable!("Incorrect line provided to get_error_marked_line");
        }

        let line = line.unwrap();
        let line_number_string = format!("{}. ", token.line);
        let spaces = " ".repeat(line_number_string.len() + token.line_offset - 1);
        let marker = "^".repeat(token.lexeme_size).red();

        let location = format!("{line_number_string}{line}\n{spaces}{marker}");
        ParserError { location: Some(location), error_type, token: Some(token) }
    }
    
    /// Creates a Missing Token error while pointing to the previous token in the iterator
    fn missing_token_error(&self, error_message: &'static str, tokens: &TokenIterator) -> ParserError {
        let previous_token = tokens.previous().expect("No previous token available while parsing");
        
        self.get_error_marked_line(
            ParserErrorType::MissingToken(String::from(error_message)), (*previous_token).clone()
        )
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
    pub fn new_without_token(error_type: ParserErrorType) -> ParserError {
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
                &format!("Missing token. {message}"),
            ParserErrorType::InvalidAssignmentTarget => "Invalid assignment target",
            ParserErrorType::InvalidDefDeclaration(def_error) =>
                &format!("{def_error}")
        };

        write!(f, "{error_header}: {output}")?;
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
    InvalidAssignmentTarget,
    InvalidDefDeclaration(DefDeclarationError)
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