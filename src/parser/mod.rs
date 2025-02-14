use crate::parser::expression::{Expression, ExpressionType, PinRef, PinRefError};
use crate::parser::scanner::{Scanner, ScannerError};
use crate::parser::statement::{DefDeclarationError, SingleDefDeclaration, Statement};
use crate::parser::token::Token;
use crate::parser::token_type::{KeywordType as KT, LiteralType as LT, PunctuationType as PT,
                                TokenType as TT, ConstantType as CT, BooleanConstantType as BCT};
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
    
    // Returns error pointing to a previous token
    ($self:ident, $tokens:ident, $enum_types:pat_param, $error_message:literal, use_prev_token) => {
        match Self::peek($tokens)?.token_type {
            $enum_types => $tokens.advance().unwrap(),
            _ => return Err($self.missing_token_error_previous($error_message, $tokens))
        }
    };
}

/// This macro is used to automate binary operator parsing.
/// All binary operators are right-associative with themselves (at least in the moment of
/// writing, check `grammar.txt`).
///
/// This means there is a loop to read them
macro_rules! parse_binary {
    ($self:ident, $enum_types:pat_param, $expression:ident, $tokens:ident, $next_precedence:ident) => {
        while let token @ Token { token_type: $enum_types, .. } = Self::peek($tokens)? {
            let token = token.clone();
            $tokens.advance();
            let right = $self.$next_precedence($tokens)?;

            $expression = Expression::new_binary($expression, &token, right);
        }
    };
}

/// This macro is used to create error producing.
///
/// It's used to capture erroneous tokens to alert the user of a potential problem.
/// For instance, catching something like `* 3 + 1` and warning that the user didn't
/// provide a value on the left side of the star.
///
/// Otherwise, this error will go all the way to the bottom of precedence and just get
/// reported as "Incorrect token in this place"
macro_rules! binary_error_produce {
    ($self:ident, $error_enum_types:pat_param, $tokens:ident, $next_precedence:ident) => {
        let current_token = Self::peek($tokens)?;
        if let $error_enum_types = current_token.token_type {
            let current_token = current_token.clone();
            $tokens.advance();
            // Discard the right part
            _ = $self.$next_precedence($tokens)?;

            // Discard the expression entirely. Report an error
            return Err($self.get_error_marked_line(ParserErrorType::BinaryOperatorNoLeftPart,
                current_token));
        }
    };
}

/// A combination of two macros - `parse_binary` and `binary_error_produce`.
///
/// Read their comments for more info. There are two versions of the macro - simplified and extended.
/// ## Arguments
/// * `$self` - reference to self
/// * `$enum_types` - token type pattern to match for error producing and binary parsing
/// * `$error_enum_types` - if this argument is passed (extended version) then it's used for binary
///    producing instead of `$enum_types`. The latter is still used for parsing the binary operator
/// * `$tokens` - mutable reference to `TokenIterator`
/// * `$next_precedence` - function for the next precedence after current one (like `ternary` after
///    `assignment`)
macro_rules! parse_binary_with_error_produce {
    ($self:ident, $enum_types:pat_param, $tokens:ident, $next_precedence:ident) => {
        {
            binary_error_produce!($self, $enum_types, $tokens, $next_precedence);

            let mut expr = $self.$next_precedence($tokens)?;
            parse_binary!($self, $enum_types, expr, $tokens, $next_precedence);

            expr
        }
    };

    ($self:ident, $enum_types:pat_param, $error_enum_types:pat_param, $tokens:ident, $next_precedence:ident) => {
        {
            binary_error_produce!($self, $error_enum_types, $tokens, $next_precedence);

            let mut expr = $self.$next_precedence($tokens)?;
            parse_binary!($self, $enum_types, expr, $tokens, $next_precedence);

            expr
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
    // TODO: A lot of things like blocks if unclosed will return an error to EOF with 'Incorrect token in this place'
    pub fn parse(&mut self) -> Result<Vec<Statement>, Vec<ParserError>> {
        let mut statements: Vec<Statement> = Vec::new();
        let mut errors: Vec<ParserError> = Vec::new();

        let tokens = self.scan()?;
        println!("Tokens:");
        for (i, token) in tokens.iter().enumerate() {
            println!("{i}. {token}");
        }

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
            TT::Keyword(KT::Var) => { Self::advance(tokens)?; self.var_declaration(tokens) },
            TT::Keyword(KT::Def) => { Self::advance(tokens)?; self.def_declaration(tokens) },
            _ => self.statement(tokens)
        };

        // If we got an error, we align our tokens with the next valid entry point.
        // See `Self::synchronize` comments for more info
        if result.is_err() {
            Self::synchronize(tokens);
        }

        result
    }

    // TODO: Read the type after the identifier (: Type)
    fn var_declaration(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        let name = match &Self::advance(tokens)?.token_type {
            TT::Literal(LT::Identifier(var_name)) => var_name.clone(),
            _ => return Err(self.missing_token_error("Expected a variable name", tokens))
        };

        let mut initializer: Option<Expression> = None;
        if let TT::Punctuation(PT::Equal) = Self::peek(tokens)?.token_type {
            tokens.advance();
            initializer = Some(self.expression(tokens)?);
        }

        check_and_consume!(self, tokens, TT::Punctuation(PT::Semicolon), "Expected a ';' after variable declaration", use_prev_token);
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

        let token = Self::peek(tokens)?;
        match token {
            // Identifier = it's a one-liner
            Token { token_type: TT::Literal(LT::Identifier(_)), .. } => {
                let def_declaration = self.single_def_declaration(tokens)?;
                Ok(Statement::DefDeclaration(Rc::new([def_declaration])))
            },
            // Left brace = it's a multiline version
            Token { token_type: TT::Punctuation(PT::LeftBrace), .. } => {
                let token = token.clone();
                tokens.advance();
                
                let mut def_declarations: Vec<SingleDefDeclaration> = Vec::new();
                loop {
                    let next_token = tokens.peek();
                    if next_token.is_none() {
                        return Err(self.get_error_marked_line(
                            ParserErrorType::UnclosedDefBlock,
                            token
                        ));
                    }

                    if let TT::Punctuation(PT::RightBrace) = next_token.unwrap().token_type {
                        // Consume the brace
                        tokens.advance();
                        break;
                    }

                    def_declarations.push(self.single_def_declaration(tokens)?);
                }

                if def_declarations.is_empty() {
                    Err(self.get_error_marked_line(ParserErrorType::EmptyDefBlock, token))
                }
                else { 
                    Ok(Statement::DefDeclaration(def_declarations.into())) 
                }
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
        let identifier = check_and_consume!(self, tokens, TT::Literal(LT::Identifier(_)), "Expected identifier after this token", use_prev_token).clone();
        check_and_consume!(self, tokens, TT::Punctuation(PT::Equal), "Expected '=' after an identifier in a def statement");
        let pin_ref = self.pin_ref(tokens)?;
        let pin_mode = check_and_consume!(self, tokens, TT::Constant(CT::PinMode(_)),
            "Expected a pin mode (for example 'INPUT') after a port reference").clone();
        check_and_consume!(self, tokens, TT::Punctuation(PT::Semicolon), "Expected ';' after a def statement");

        SingleDefDeclaration::new(identifier, pin_ref, pin_mode).map_err(|err| self.convert_def_error(err))
    }

    fn pin_ref(&mut self, tokens: &mut TokenIterator) -> Result<PinRef, ParserError> {
        let port = check_and_consume!(self, tokens, TT::Port(_), "Expected a port name here").clone();
        check_and_consume!(self, tokens, TT::Punctuation(PT::Dot), "Expected a '.' after a port name");
        let pin_number = check_and_consume!(self, tokens, TT::Literal(LT::Integer(_)), "Expected an integer here").clone();

        PinRef::new(port, pin_number).map_err(|err| self.convert_pin_ref_error(err))
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
        let identifier = match &Self::peek(tokens)?.token_type {
            TT::Literal(LT::Identifier(name)) => name.clone(),
            _ => return Err(self.missing_token_error("Expected identifier after 'for'", tokens))
        };
        check_and_consume!(self, tokens, TT::Punctuation(PT::Equal), "Expected '=' after identifier in for loop");
        let from = self.expression(tokens)?;
        check_and_consume!(self, tokens, TT::Keyword(KT::To), "Expected 'to' after from-expression in a for loop");
        let to = self.expression(tokens)?;
        check_and_consume!(self, tokens, TT::Punctuation(PT::LeftBrace), "For only supports a block body '{'");
        let body = self.block_statement(tokens)?; 
        
        Ok(Statement::For { identifier, from, to, body: Box::new(body) })
    }

    fn if_statement(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        let condition = self.expression(tokens)?;
        check_and_consume!(self, tokens, TT::Punctuation(PT::LeftBrace), "Expected '{' after if condition");
        let body = self.block_statement(tokens)?;
        let mut else_body: Option<Box<Statement>> = None;
        if let TT::Keyword(KT::Else) = Self::peek(tokens)?.token_type {
            else_body = Some(Box::new(self.block_statement(tokens)?));
        }

        Ok(Statement::If { condition, then_body: Box::new(body), else_body })
    }

    fn while_loop(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        let condition = self.expression(tokens)?;
        check_and_consume!(self, tokens, TT::Punctuation(PT::LeftBrace), "Expected '{' after while condition");
        let body = self.block_statement(tokens)?;

        Ok(Statement::While { body: Box::new(body), condition })
    }

    fn block_statement(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        let mut statements: Vec<Statement> = Vec::new();

        loop {
            if let TT::Punctuation(PT::RightBrace) = Self::peek(tokens)?.token_type { break; }

            statements.push(self.declaration(tokens)?);
        }

        check_and_consume!(self, tokens, TT::Punctuation(PT::RightBrace), "Expected '}' after a block", use_prev_token);
        Ok(Statement::Block(statements.into()))
    }

    fn expression_statement(&mut self, tokens: &mut TokenIterator) -> Result<Statement, ParserError> {
        let value = self.expression(tokens)?;
        check_and_consume!(self, tokens, TT::Punctuation(PT::Semicolon), "Expected ';' after expression.", use_prev_token);
        Ok(Statement::Expression(value))
    }

    // --------------------------- THIS PART IS EXPRESSIONS --------------------------- //
    /// Root of the expression tree. Always defaults to the next precedence level.
    /// This is done so that adding new precedence levels won't change other parts of the
    /// program that call `expression` to get any expression.
    fn expression(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        self.assignment(tokens)
    }

    // a = b
    fn assignment(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        // We don't know if it's the assignment yet, so we read anything of higher precedence
        // If it's not the assignment, we will just return this value
        let l_value = self.ternary(tokens)?;

        let equals = Self::peek(tokens)?;
        if let TT::Punctuation(PT::Equal) = equals.token_type {
            if let ExpressionType::Literal { value: LT::Identifier(_) } = &l_value.expression_type {
                tokens.advance();
                // Assignments are right associative
                let r_value = self.assignment(tokens)?;
                return Ok(Expression::new_assignment(l_value, r_value));
            }

            return Err(self.get_error_marked_line(ParserErrorType::InvalidAssignmentTarget, equals.clone()))
        }

        // It wasn't an assignment
        Ok(l_value)
    }

    // expr ? expr : expr
    fn ternary<'a>(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        let mut expr = self.logic_or(tokens)?;

        let question_token = Self::peek(tokens)?;
        if let TT::Punctuation(PT::Question) = question_token.token_type {
            let question_token = question_token.clone();
            tokens.advance();
            let second = self.logic_or(tokens)?;

            let third_token = Self::peek(tokens)?;
            if let TT::Punctuation(PT::Colon) = third_token.token_type {
                tokens.advance();
                let third = self.logic_or(tokens)?;

                expr = Expression::new_ternary(expr, second, third);
            }
            else {
                return Err(
                    self.get_error_marked_line(ParserErrorType::UnfinishedTernary, question_token)
                );
            }
        }


        Ok(expr)
    }

    fn logic_or(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        let expr = parse_binary_with_error_produce!(self, TT::Keyword(KT::Or), tokens, logic_and);

        Ok(expr)
    }

    fn logic_and(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        let expr = parse_binary_with_error_produce!(self, TT::Keyword(KT::And), tokens, equality);

        Ok(expr)
    }

    // != ==
    fn equality<'a>(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        let expr = parse_binary_with_error_produce!(self, TT::Punctuation(PT::BangEqual | PT::EqualEqual), tokens, comparison);

        Ok(expr)
    }

    // > >= < <=
    fn comparison<'a>(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        let expr = parse_binary_with_error_produce!(self,
            TT::Punctuation(PT::Greater | PT::GreaterEqual | PT::Less | PT::LessEqual), tokens, term);

        Ok(expr)
    }

    // + -
    fn term<'a>(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        // Only a plus should produce an error, because a minus could still be a unary operator
        let expr = parse_binary_with_error_produce!(self,
            TT::Punctuation(PT::Plus | PT::Minus), TT::Punctuation(PT::Plus), tokens, factor);

        Ok(expr)
    }

    // * /
    fn factor<'a>(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        let expr = parse_binary_with_error_produce!(self, TT::Punctuation(PT::Star | PT::Slash), tokens, unary);

        Ok(expr)
    }

    // ! - (as unary operator)
    fn unary<'a>(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        let token = Self::peek(tokens)?;
        if let TT::Punctuation(PT::Bang | PT::Minus) = &token.token_type {
            let token = token.clone();
            tokens.advance();
            let right = self.unary(tokens)?;

            return Ok(Expression::new_unary(&token, right));
        }

        self.primary(tokens)
    }

    // Literals, groupings
    fn primary<'a>(&mut self, tokens: &mut TokenIterator) -> Result<Expression, ParserError> {
        let token = Self::peek(tokens)?.clone();

        let result = match token.token_type {
            TT::Literal(_) => { Some(Expression::new_literal(&token)) },
            TT::Constant(CT::Boolean(constant)) => {
                match constant {
                    BCT::True => Some(Expression::new_literal_custom_type(&token, LT::Boolean(true))),
                    BCT::False => Some(Expression::new_literal_custom_type(&token, LT::Boolean(false))),
                }
            },
            TT::Punctuation(PT::LeftParen) => {
                tokens.advance();
                let expr = self.expression(tokens)?;
                let next_token = tokens.peek();
                if next_token.is_none() {
                    return Err(self.get_error_marked_line(
                        ParserErrorType::UnclosedParenthesis, token
                    ));
                }
                else {
                    let next_token = next_token.unwrap();
                    match next_token.token_type {
                        TT::Punctuation(PT::RightParen) => {},

                        _ => {
                            return Err(self.get_error_marked_line(
                                ParserErrorType::UnclosedParenthesis, (*next_token).clone()
                            ));
                        }
                    }
                }

                Some(Expression::new_grouping(expr))
            },
            _ => { None }
        };

        if result.is_some() { tokens.advance(); }
        result.ok_or_else(|| self.get_error_marked_line(ParserErrorType::IncorrectToken, token.clone()))
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

            tokens.advance();
        }
    }

    fn convert_def_error(&self, error: DefDeclarationError) -> ParserError {
        let token = error.token.clone();
        self.get_error_marked_line(ParserErrorType::InvalidDefDeclaration(error), token)
    }

    fn convert_pin_ref_error(&self, error: PinRefError) -> ParserError {
        let token = error.token.clone();
        self.get_error_marked_line(ParserErrorType::InvalidPinRef(error), token)
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

    /// Creates a Missing Token error while pointing to the current token in the iterator
    fn missing_token_error(&self, error_message: &'static str, tokens: &mut TokenIterator) -> ParserError {
        let current_token = tokens.peek()
            .expect("No previous token available while reporting error");
        
        self.get_error_marked_line(
            ParserErrorType::MissingToken(String::from(error_message)), (*current_token).clone()
        )
    }
    
    /// Creates a Missing Token error while pointing to the previous token in the iterator
    fn missing_token_error_previous(&self, error_message: &'static str, tokens: &TokenIterator) -> ParserError {
        let previous_token = tokens.previous()
            .expect("No previous token available while reporting error");
        
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
        let mut is_scanner_error = false;
        let output = match &self.error_type {
            ParserErrorType::ScannerError(scanner_error) =>
                { is_scanner_error = true; &format!("{scanner_error}") },
            ParserErrorType::UnclosedParenthesis => "Unclosed parenthesis",
            ParserErrorType::RanOutOfTokens => "No more tokens to parse",
            ParserErrorType::IncorrectToken => "Incorrect token in this place",
            ParserErrorType::UnfinishedTernary => "Incorrect ternary operator",
            ParserErrorType::BinaryOperatorNoLeftPart => "Binary operator without a left part",
            ParserErrorType::MissingToken(message) =>
                &format!("Missing token. {message}"),
            ParserErrorType::InvalidAssignmentTarget => "Invalid assignment target",
            ParserErrorType::InvalidDefDeclaration(def_error) =>
                &format!("{def_error}"),
            ParserErrorType::InvalidPinRef(pin_ref_error) =>
                &format!("{pin_ref_error}"),
            ParserErrorType::UnclosedDefBlock => "Def block was unclosed (missing '}')",
            ParserErrorType::EmptyDefBlock => "Def block can't be empty"
        };

        if is_scanner_error {
            write!(f, "{}", output)?;
        }
        else {
            write!(f, "{}", output.red())?;
        }
        
        // TODO: Move common error logic somewhere
        if let Some(location) = &self.location {
            // Add tabs to each line of the location so it shifts to the right
            let location: String = location.split('\n')
                .map(|line| format!("\t{line}\n"))
                .collect();
            write!(f, "\n\t{}\n{}\n", "Happened here:".green(), location.trim_end())?;
        }

        if self.token.is_none() && !is_scanner_error {
            write!(f, "{}", "(internal parser error, something went wrong)".yellow())?;
        }

        Ok(())
    }
}

// TODO: Test every parser error type
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
    InvalidDefDeclaration(DefDeclarationError),
    InvalidPinRef(PinRefError),
    UnclosedDefBlock,
    EmptyDefBlock
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