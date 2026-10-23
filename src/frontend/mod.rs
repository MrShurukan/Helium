use crate::frontend::expression::{Expression, ExpressionType, PinRef, PinRefError};
use crate::frontend::scanner::{Scanner, ScannerError};
use crate::frontend::statement::{
    DefDeclarationError, Parameter, SingleDefDeclaration, Statement, StatementType,
};
use crate::frontend::token::Token;
use crate::frontend::token_type::{
    BooleanConstantType as BCT, ConstantType as CT, KeywordType as KT, LiteralType as LT,
    PunctuationType as PT, TokenType as TT,
};
use crate::frontend::utils::TokenIterator;
use std::error::Error;
use std::fmt::{Display, Formatter};

pub mod expression;
mod position_info;
mod scanner;
pub mod statement;
pub mod static_type;
mod token;
pub mod token_type;
pub mod utils;

/// Main struct for the front-end of helium. This class receives a source file,
/// starts up a scanner, gets tokens and then parses them into a list of statements.
///
/// This is a top-down parser, more specifically a Recursive Descent Parser.
///
/// Grammar is defined in `grammar.txt` file in the root of the repo
pub struct Parser<'a> {
    /// Reference to an original string to point out errors with a location
    pub source: &'a str,
    /// List of errors that will be accumulated during the call of `parse`
    errors: Vec<ParserError>,
}

/// Performs a check on the peeked token. If it matched the `enum_types` pattern then it's consumed.
/// Otherwise, an error is returned that the token was missing with the `error_message` error message
///
/// Returns the consumed token for convenience
macro_rules! check_and_consume {
    ($self:ident, $tokens:ident, $enum_types:pat_param, $error_message:literal) => {
        match $self.peek($tokens)?.token_type {
            $enum_types => $tokens.advance().unwrap(),
            _ => {
                $self
                    .errors
                    .push($self.missing_token_error($error_message, $tokens));
                return None;
            }
        }
    };

    // Returns error pointing to a previous token
    ($self:ident, $tokens:ident, $enum_types:pat_param, $error_message:literal, use_prev_token) => {
        match $self.peek($tokens)?.token_type {
            $enum_types => $tokens.advance().unwrap(),
            _ => {
                $self
                    .errors
                    .push($self.missing_token_error_previous($error_message, $tokens));
                return None;
            }
        }
    };
}

/// This macro is used to automate binary operator parsing.
/// All binary operators are left-associative with themselves (at least in the moment of
/// writing, check `grammar.txt`).
///
/// This means there is a loop to read them
macro_rules! parse_binary {
    ($self:ident, $enum_types:pat_param, $expression:ident, $tokens:ident, $next_precedence:ident) => {
        while let token @ Token {
            token_type: $enum_types,
            ..
        } = $self.peek($tokens)?
        {
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
        let current_token = $self.peek($tokens)?;
        if let $error_enum_types = current_token.token_type {
            let current_token = current_token.clone();
            $tokens.advance();
            // Discard the right part
            _ = $self.$next_precedence($tokens)?;

            // Discard the expression entirely. Report an error
            $self.errors.push(
                $self.get_error_marked_line(
                    ParserErrorType::BinaryOperatorNoLeftPart,
                    current_token,
                ),
            );
            return None;
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
///   producing instead of `$enum_types`. The latter is still used for parsing the binary operator
/// * `$tokens` - mutable reference to `TokenIterator`
/// * `$next_precedence` - function for the next precedence after current one (like `ternary` after
///   `assignment`)
macro_rules! parse_binary_with_error_produce {
    ($self:ident, $enum_types:pat_param, $tokens:ident, $next_precedence:ident) => {{
        binary_error_produce!($self, $enum_types, $tokens, $next_precedence);

        let mut expr = $self.$next_precedence($tokens)?;
        parse_binary!($self, $enum_types, expr, $tokens, $next_precedence);

        expr
    }};

    ($self:ident, $enum_types:pat_param, $error_enum_types:pat_param, $tokens:ident, $next_precedence:ident) => {{
        binary_error_produce!($self, $error_enum_types, $tokens, $next_precedence);

        let mut expr = $self.$next_precedence($tokens)?;
        parse_binary!($self, $enum_types, expr, $tokens, $next_precedence);

        expr
    }};
}

impl Parser<'_> {
    pub fn new(source: &str) -> Parser<'_> {
        Parser {
            source,
            errors: Vec::new(),
        }
    }

    /// Main function - parses the source file and gets out a list of statements (or a list of errors).
    /// This list then can be used to statically check and analyze the language before
    /// passing it to back-end (which will convert it to assembly language).
    ///
    /// This is parsed according to rules in `grammar.txt` (root of the repo)
    // TODO: A lot of things like blocks if unclosed will return an error to EOF with 'Incorrect token in this place'
    pub fn parse(self) -> Result<Vec<Statement>, Vec<ParserError>> {
        let tokens = self.scan()?;
        let mut tokens = TokenIterator::new(tokens.as_slice());
        self.read_top_level(&mut tokens)
    }

    // -------------------- THIS PART IS DECLARATIONS AND STATEMENTS -------------------- //
    fn read_top_level(
        mut self,
        tokens: &mut TokenIterator,
    ) -> Result<Vec<Statement>, Vec<ParserError>> {
        let mut statements: Vec<Statement> = Vec::new();

        let mut already_read_setup = false;
        let mut already_read_loop = false;

        while let Some(token) = tokens.advance() {
            // EOF means statements are over and the parsing is done
            if matches!(token.token_type, TT::Eof) {
                break;
            };

            let result =
                match token.token_type {
                    TT::Keyword(KT::Fn) => self.function_declaration(tokens),
                    TT::Keyword(KT::Var) if !already_read_setup && !already_read_loop => {
                        self.var_declaration(tokens)
                    }
                    TT::Keyword(KT::Def) if !already_read_setup && !already_read_loop => {
                        self.def_declaration(tokens)
                    }
                    // Can't declare global variables and ports after setup or loop
                    TT::Keyword(KT::Var | KT::Def) => {
                        let token = token.clone();
                        // Discard the declaration
                        _ = if matches!(token.token_type, TT::Keyword(KT::Var)) {
                            self.var_declaration(tokens)
                        } else {
                            self.def_declaration(tokens)
                        };

                        self.errors.push(self.get_error_marked_line(
                            ParserErrorType::DefOrVarAfterSetupOrLoop,
                            token,
                        ));
                        None
                    }
                    TT::Keyword(KT::Setup) => {
                        if already_read_loop {
                            let token = token.clone();
                            let _ = self.setup_declaration(tokens);
                            self.errors.push(
                                self.get_error_marked_line(ParserErrorType::SetupAfterLoop, token),
                            );
                            None
                        } else if already_read_setup {
                            let token = token.clone();
                            // Read the setup anyway, but discard the result
                            _ = self.setup_declaration(tokens);
                            self.errors.push(self.get_error_marked_line(
                                ParserErrorType::DuplicateSetup,
                                token.clone(),
                            ));
                            None
                        } else {
                            already_read_setup = true;
                            self.setup_declaration(tokens)
                        }
                    }
                    TT::Keyword(KT::Loop) => {
                        if already_read_loop {
                            let token = token.clone();
                            // Read the loop anyway, but discard the result
                            _ = self.loop_declaration(tokens);
                            self.errors.push(
                                self.get_error_marked_line(ParserErrorType::DuplicateLoop, token),
                            );
                            None
                        } else {
                            already_read_loop = true;
                            self.loop_declaration(tokens)
                        }
                    }

                    _ => {
                        self.errors.push(self.get_error_marked_line(
                            ParserErrorType::IncorrectGlobalToken,
                            token.clone(),
                        ));
                        None
                    }
                };

            match result {
                Some(statement) => {
                    statements.push(statement);
                }
                None => {
                    // If we got an error, we align our tokens with the next valid entry point.
                    // See `Self::synchronize` comments for more info
                    Self::synchronize(tokens);
                }
            }
        }

        if !already_read_loop {
            self.errors.push(ParserError::new_without_token(
                ParserErrorType::MissingLoopDeclaration,
            ));
        }

        if self.errors.is_empty() {
            Ok(statements)
        } else {
            Err(self.errors)
        }
    }

    fn function_declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let start = tokens.previous()?.source_location;
        let name =
            check_and_consume!(self, tokens, TT::Identifier(_), "Expected function name").clone();
        let TT::Identifier(identifier) = name.token_type else {
            unreachable!()
        };
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftParen),
            "Expected '(' after function name"
        );

        let mut parameters = Vec::new();
        if !matches!(
            self.peek(tokens)?.token_type,
            TT::Punctuation(PT::RightParen)
        ) {
            loop {
                let name =
                    check_and_consume!(self, tokens, TT::Identifier(_), "Expected parameter name")
                        .clone();
                let TT::Identifier(identifier) = name.token_type else {
                    unreachable!()
                };
                check_and_consume!(
                    self,
                    tokens,
                    TT::Punctuation(PT::Colon),
                    "Expected ':' after parameter name"
                );
                let kind = check_and_consume!(self, tokens, TT::Type(_), "Expected parameter type")
                    .clone();
                let TT::Type(static_type) = kind.token_type else {
                    unreachable!()
                };
                parameters.push(Parameter {
                    identifier,
                    static_type: *static_type,
                    source_location: name.source_location,
                });
                if !matches!(self.peek(tokens)?.token_type, TT::Punctuation(PT::Comma)) {
                    break;
                }
                tokens.advance();
            }
        }
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::RightParen),
            "Expected ')' after parameters"
        );

        let return_type = if matches!(self.peek(tokens)?.token_type, TT::Punctuation(PT::Colon)) {
            tokens.advance();
            let kind =
                check_and_consume!(self, tokens, TT::Type(_), "Expected return type").clone();
            let TT::Type(kind) = kind.token_type else {
                unreachable!()
            };
            Some(*kind)
        } else {
            None
        };
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftBrace),
            "Expected function body"
        );
        let block = self.block_statement(tokens)?;
        let StatementType::Block(body) = block.statement_type else {
            unreachable!()
        };
        Some(Statement {
            source_location: crate::frontend::utils::SourceLocation::new_from_start_end(
                start,
                block.source_location,
            ),
            statement_type: StatementType::FunctionDeclaration {
                identifier,
                parameters,
                return_type,
                body,
            },
        })
    }

    fn return_statement(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let start = self.advance(tokens)?.source_location;
        let value = if matches!(
            self.peek(tokens)?.token_type,
            TT::Punctuation(PT::Semicolon)
        ) {
            None
        } else {
            Some(self.expression(tokens)?)
        };
        let end = check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Semicolon),
            "Expected ';' after return"
        )
        .source_location;
        Some(Statement {
            source_location: crate::frontend::utils::SourceLocation::new_from_start_end(start, end),
            statement_type: StatementType::Return(value),
        })
    }

    fn declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let token = self.peek(tokens)?;

        match token.token_type {
            TT::Keyword(KT::Var) => {
                self.advance(tokens)?;
                self.var_declaration(tokens)
            }
            _ => self.statement(tokens),
        }
    }

    fn var_declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let var_keyword = tokens
            .previous()
            .expect("Expected previous token ('{') in block_statement")
            .clone();

        let ident_token = self.advance(tokens)?;
        if !matches!(ident_token.token_type, TT::Identifier(_)) {
            self.errors
                .push(self.missing_token_error("Expected a variable name", tokens));
            return None;
        }
        let ident_token = ident_token.clone();
        let mut static_type = None;

        if let TT::Punctuation(PT::Colon) = self.peek(tokens)?.token_type {
            self.advance(tokens)?;
            let token = self.peek(tokens)?;
            if let TT::Type(_) = token.token_type {
                static_type = Some(token.clone());
            } else {
                self.errors.push(self.get_error_marked_line(
                    ParserErrorType::IncorrectToken(format!(
                        "Expected type after ':', got '{:?}'",
                        token.token_type
                    )),
                    token.clone(),
                ));
                return None;
            }
            self.advance(tokens)?;
        }

        let mut initializer = None;

        if static_type.is_none() {
            check_and_consume!(
                self,
                tokens,
                TT::Punctuation(PT::Equal),
                "Expected '=' (initializer) here"
            );
            initializer = Some(self.expression(tokens)?);
        } else if let TT::Punctuation(PT::Equal) = self.peek(tokens)?.token_type {
            self.advance(tokens)?;
            initializer = Some(self.expression(tokens)?);
        }

        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Semicolon),
            "Expected a ';' after variable declaration",
            use_prev_token
        );
        Some(Statement::new_var_declaration(
            &var_keyword,
            ident_token,
            static_type,
            initializer,
        ))
    }

    fn def_declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        // There are two types of def declaration. One is a one-liner, like this:
        // def Led = PortB.2;
        // Other one is multiline:
        // def {
        //     Led    = PortB.2;
        //     Button = PortC.2;
        // }

        let token = self.peek(tokens)?;
        match token {
            // Identifier = it's a one-liner
            Token {
                token_type: TT::Identifier(_),
                ..
            } => {
                let def_declaration = self.single_def_declaration(tokens)?;
                Some(Statement::new_def_declaration(vec![def_declaration]))
            }
            // Left brace = it's a multiline version
            Token {
                token_type: TT::Punctuation(PT::LeftBrace),
                ..
            } => {
                let token = token.clone();
                tokens.advance();

                let mut def_declarations: Vec<SingleDefDeclaration> = Vec::new();
                loop {
                    let next_token = self.peek(tokens);
                    if next_token.is_none() {
                        self.errors.push(
                            self.get_error_marked_line(ParserErrorType::UnclosedDefBlock, token),
                        );
                        return None;
                    }

                    if let TT::Punctuation(PT::RightBrace) = next_token.unwrap().token_type {
                        // Consume the brace
                        tokens.advance();
                        break;
                    }

                    def_declarations.push(self.single_def_declaration(tokens)?);
                }

                if def_declarations.is_empty() {
                    self.errors
                        .push(self.get_error_marked_line(ParserErrorType::EmptyDefBlock, token));
                    None
                } else {
                    Some(Statement::new_def_declaration(def_declarations))
                }
            }
            // Error
            token => {
                self.errors.push(self.get_error_marked_line(
                    ParserErrorType::MissingToken(String::from(
                        "Expected '{' or an identifier after 'def'",
                    )),
                    token.clone(),
                ));
                None
            }
        }
    }

    fn single_def_declaration(
        &mut self,
        tokens: &mut TokenIterator,
    ) -> Option<SingleDefDeclaration> {
        let identifier = check_and_consume!(
            self,
            tokens,
            TT::Identifier(_),
            "Expected identifier after this token",
            use_prev_token
        )
        .clone();
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Equal),
            "Expected '=' after an identifier in a def statement"
        );
        let pin_ref = self.pin_ref(tokens)?;
        let pin_mode = check_and_consume!(
            self,
            tokens,
            TT::Constant(CT::PinMode(_)),
            "Expected a pin mode (for example 'INPUT') after a port reference"
        )
        .clone();
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Semicolon),
            "Expected ';' after a def statement"
        );

        SingleDefDeclaration::new(identifier, pin_ref, pin_mode)
            .map_err(|err| self.errors.push(self.convert_def_error(err)))
            .ok()
    }

    fn pin_ref(&mut self, tokens: &mut TokenIterator) -> Option<PinRef> {
        let port =
            check_and_consume!(self, tokens, TT::Port(_), "Expected a port name here").clone();
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Dot),
            "Expected a '.' after a port name"
        );
        let pin_number = check_and_consume!(
            self,
            tokens,
            TT::Literal(LT::Integer(_)),
            "Expected an integer here"
        )
        .clone();

        PinRef::new(port, pin_number)
            .map_err(|err| self.errors.push(self.convert_pin_ref_error(err)))
            .ok()
    }

    fn setup_declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftBrace),
            "Expected '{' after setup",
            use_prev_token
        );
        let block = self.block_statement(tokens)?;
        if let StatementType::Block(statements) = block.statement_type {
            Some(Statement::new_setup_declaration(
                statements,
                block.source_location,
            ))
        } else {
            unreachable!()
        }
    }

    fn loop_declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftBrace),
            "Expected '{' after loop",
            use_prev_token
        );
        let block = self.block_statement(tokens)?;
        if let StatementType::Block(statements) = block.statement_type {
            Some(Statement::new_loop_declaration(
                statements,
                block.source_location,
            ))
        } else {
            unreachable!()
        }
    }

    fn statement(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let token = self.peek(tokens)?;
        match token.token_type {
            TT::Keyword(KT::Return) => self.return_statement(tokens),
            TT::Keyword(KT::For) => {
                self.advance(tokens)?;
                self.for_statement(tokens)
            }
            TT::Keyword(KT::If) => {
                self.advance(tokens)?;
                self.if_statement(tokens)
            }
            TT::Keyword(KT::While) => {
                self.advance(tokens)?;
                self.while_loop(tokens)
            }
            TT::Punctuation(PT::LeftBrace) => {
                self.advance(tokens)?;
                self.block_statement(tokens)
            }
            _ => self.expression_statement(tokens),
        }
    }

    fn for_statement(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let ident_token = self.advance(tokens)?;
        if !matches!(ident_token.token_type, TT::Identifier(_)) {
            self.errors
                .push(self.missing_token_error("Expected identifier after 'for'", tokens));
            return None;
        }
        let ident_token = ident_token.clone();

        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Equal),
            "Expected '=' after identifier in for loop"
        );
        let from = self.expression(tokens)?;
        check_and_consume!(
            self,
            tokens,
            TT::Keyword(KT::To),
            "Expected 'to' after from-expression in a for loop"
        );
        let to = self.expression(tokens)?;
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftBrace),
            "For only supports a block body '{'"
        );
        let body = self.block_statement(tokens)?;

        Some(Statement::new_for(ident_token, from, to, Box::new(body)))
    }

    fn if_statement(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let condition = self.expression(tokens)?;
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftBrace),
            "Expected '{' after if condition"
        );
        let body = self.block_statement(tokens)?;
        let mut else_body: Option<Box<Statement>> = None;
        if let TT::Keyword(KT::Else) = self.peek(tokens)?.token_type {
            self.advance(tokens)?;
            match self.peek(tokens)?.token_type {
                TT::Punctuation(PT::LeftBrace) => {
                    self.advance(tokens)?;
                    else_body = Some(Box::new(self.block_statement(tokens)?));
                }
                TT::Keyword(KT::If) => {
                    self.advance(tokens)?;
                    else_body = Some(Box::new(self.if_statement(tokens)?));
                }
                _ => {
                    self.errors.push(self.missing_token_error(
                        "Expected a block or another 'if' after else",
                        tokens,
                    ));
                }
            }
        }

        Some(Statement::new_if(condition, Box::new(body), else_body))
    }

    fn while_loop(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let condition = self.expression(tokens)?;
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftBrace),
            "Expected '{' after while condition"
        );
        let body = self.block_statement(tokens)?;

        Some(Statement::new_while(condition, Box::new(body)))
    }

    fn block_statement(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let mut statements: Vec<Statement> = Vec::new();
        let left_brace = tokens
            .previous()
            .expect("Expected previous token ('{') in block_statement")
            .clone();

        loop {
            match self.peek(tokens) {
                // Right brace means we have reached the end of this block
                Some(Token { token_type: TT::Punctuation(PT::RightBrace), .. }) |
                // Hitting the end of tokens counts too
                None => { break; }
                // Anything else should be parsed by self.declaration
                Some(_) => {}
            }

            let position = tokens.position();
            let declaration = self.declaration(tokens);
            match declaration {
                Some(statement) => statements.push(statement),
                None => {
                    // Recovery must consume an invalid token if parsing made no progress.
                    // Otherwise a reserved but unsupported keyword can repeat forever.
                    if tokens.position() == position {
                        tokens.advance();
                    }
                    Self::synchronize(tokens);
                }
            }
        }

        let right_brace = if let Some(Token {
            token_type: TT::Punctuation(PT::RightBrace),
            ..
        }) = self.peek(tokens)
        {
            tokens.advance().unwrap()
        } else {
            self.errors
                .push(self.missing_token_error_previous("Expected '}' after a block", tokens));
            return None;
        };

        Some(Statement::new_block(&left_brace, right_brace, statements))
    }

    fn expression_statement(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let value = self.expression(tokens)?;
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Semicolon),
            "Expected ';' after expression.",
            use_prev_token
        );
        Some(Statement::new_expression(value))
    }

    // --------------------------- THIS PART IS EXPRESSIONS --------------------------- //
    /// Root of the expression tree. Always defaults to the next precedence level.
    /// This is done so that adding new precedence levels won't change other parts of the
    /// program that call `expression` to get any expression.
    fn expression(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        self.assignment(tokens)
    }

    // a = b
    fn assignment(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        // We don't know if it's the assignment yet, so we read anything of higher precedence
        // If it's not the assignment, we will just return this value
        let l_value = self.ternary(tokens)?;

        let equals = self.peek(tokens)?;
        if let TT::Punctuation(PT::Equal) = equals.token_type {
            if let ExpressionType::Identifier(_) = &l_value.expression_type {
                tokens.advance();
                // Assignments are right associative
                let r_value = self.assignment(tokens)?;
                return Some(Expression::new_assignment(&l_value, r_value));
            }

            self.errors.push(
                self.get_error_marked_line(
                    ParserErrorType::InvalidAssignmentTarget,
                    equals.clone(),
                ),
            );
            return None;
        }

        // It wasn't an assignment
        Some(l_value)
    }

    // expr ? expr : expr
    fn ternary(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        let mut expr = self.logic_or(tokens)?;

        let question_token = self.peek(tokens)?;
        if let TT::Punctuation(PT::Question) = question_token.token_type {
            let question_token = question_token.clone();
            tokens.advance();
            let second = self.logic_or(tokens)?;

            let third_token = self.peek(tokens)?;
            if let TT::Punctuation(PT::Colon) = third_token.token_type {
                tokens.advance();
                let third = self.logic_or(tokens)?;

                expr = Expression::new_ternary(expr, second, third);
            } else {
                self.errors.push(
                    self.get_error_marked_line(ParserErrorType::UnfinishedTernary, question_token),
                );
                return None;
            }
        }

        Some(expr)
    }

    fn logic_or(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        let expr = parse_binary_with_error_produce!(self, TT::Keyword(KT::Or), tokens, logic_and);

        Some(expr)
    }

    fn logic_and(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        let expr = parse_binary_with_error_produce!(self, TT::Keyword(KT::And), tokens, equality);

        Some(expr)
    }

    // != ==
    fn equality(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        let expr = parse_binary_with_error_produce!(
            self,
            TT::Punctuation(PT::BangEqual | PT::EqualEqual),
            tokens,
            comparison
        );

        Some(expr)
    }

    // > >= < <=
    fn comparison(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        let expr = parse_binary_with_error_produce!(
            self,
            TT::Punctuation(PT::Greater | PT::GreaterEqual | PT::Less | PT::LessEqual),
            tokens,
            term
        );

        Some(expr)
    }

    // + -
    fn term(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        // Only a plus should produce an error, because a minus could still be a unary operator
        let expr = parse_binary_with_error_produce!(
            self,
            TT::Punctuation(PT::Plus | PT::Minus),
            TT::Punctuation(PT::Plus),
            tokens,
            factor
        );

        Some(expr)
    }

    // * /
    fn factor(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        let expr = parse_binary_with_error_produce!(
            self,
            TT::Punctuation(PT::Star | PT::Slash),
            tokens,
            unary
        );

        Some(expr)
    }

    // ! - (as unary operator)
    fn unary(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        let token = self.peek(tokens)?;
        if let TT::Punctuation(PT::Bang | PT::Minus) = &token.token_type {
            let token = token.clone();
            tokens.advance();
            let right = self.unary(tokens)?;

            return Some(Expression::new_unary(&token, right));
        }

        self.call(tokens)
    }

    fn call(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        let callee = self.primary(tokens)?;
        if !matches!(
            self.peek(tokens)?.token_type,
            TT::Punctuation(PT::LeftParen)
        ) {
            return Some(callee);
        }
        let ExpressionType::Identifier(identifier) = callee.expression_type else {
            self.errors
                .push(self.missing_token_error("Only named functions can be called", tokens));
            return None;
        };
        tokens.advance();
        let mut arguments = Vec::new();
        if !matches!(
            self.peek(tokens)?.token_type,
            TT::Punctuation(PT::RightParen)
        ) {
            loop {
                arguments.push(self.expression(tokens)?);
                if !matches!(self.peek(tokens)?.token_type, TT::Punctuation(PT::Comma)) {
                    break;
                }
                tokens.advance();
            }
        }
        let end = check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::RightParen),
            "Expected ')' after arguments"
        )
        .source_location;
        Some(Expression {
            expression_type: ExpressionType::Call {
                identifier,
                arguments,
            },
            source_location: crate::frontend::utils::SourceLocation::new_from_start_end(
                callee.source_location,
                end,
            ),
            static_type: crate::frontend::static_type::StaticType::Unknown,
        })
    }

    // Literals, groupings
    fn primary(&mut self, tokens: &mut TokenIterator) -> Option<Expression> {
        let token = self.peek(tokens)?.clone();

        let result = match token.token_type {
            TT::Identifier(_) => Some(Expression::new_identifier(&token)),
            TT::Literal(_) => Some(Expression::new_literal(&token)),
            TT::Constant(CT::Boolean(constant)) => match constant {
                BCT::True => Some(Expression::new_literal_custom_type(
                    &token,
                    LT::Boolean(true),
                )),
                BCT::False => Some(Expression::new_literal_custom_type(
                    &token,
                    LT::Boolean(false),
                )),
            },
            TT::Constant(CT::PinMode(constant)) => Some(Expression::new_literal_custom_type(
                &token,
                LT::PinMode(*constant),
            )),
            TT::Constant(CT::PinState(constant)) => Some(Expression::new_literal_custom_type(
                &token,
                LT::PinState(*constant),
            )),
            TT::Punctuation(PT::LeftParen) => {
                tokens.advance();
                let expr = self.expression(tokens)?;
                let next_token = self.peek(tokens);
                if next_token.is_none() {
                    self.errors.push(
                        self.get_error_marked_line(ParserErrorType::UnclosedParenthesis, token),
                    );
                    return None;
                }

                let next_token = next_token.unwrap();
                if let TT::Punctuation(PT::RightParen) = next_token.token_type {
                } else {
                    self.errors.push(self.get_error_marked_line(
                        ParserErrorType::UnclosedParenthesis,
                        (*next_token).clone(),
                    ));
                    return None;
                }

                Some(Expression {
                    source_location: crate::frontend::utils::SourceLocation::new_from_start_end(
                        token.source_location,
                        next_token.source_location,
                    ),
                    expression_type: ExpressionType::Grouping {
                        expression: Box::new(expr),
                    },
                    static_type: crate::frontend::static_type::StaticType::Unknown,
                })
            }
            _ => None,
        };

        if result.is_none() {
            self.errors.push(self.get_error_marked_line(
                ParserErrorType::IncorrectToken(String::new()),
                token.clone(),
            ));
        }

        tokens.advance();
        result
    }

    // ------------------------------- HELPER FUNCTIONS ------------------------------- //

    /// A helper wrapper around `tokens.peek()` function.
    /// Appends an error if peeking yielded no results
    fn peek<'a>(&mut self, tokens: &'a mut TokenIterator) -> Option<&'a Token> {
        // Infinite loop prevention
        // This shouldn't happen in production, but it helps in development as it spits out errors
        // instead of hanging forever
        if tokens.repeated_peek_count > 1000 {
            let token = tokens.peek();
            if let Some(token) = token {
                self.errors.push(self.get_error_marked_line(
                    ParserErrorType::InfiniteLoopDetected,
                    (*token).clone(),
                ));
            } else {
                self.errors.push(ParserError::new_without_token(
                    ParserErrorType::InfiniteLoopDetected,
                ));
            }

            tokens.invalidate();
            return None;
        }

        let token = tokens.peek();

        if let Some(token) = token {
            Some(token)
        } else {
            self.errors.push(ParserError::new_without_token(
                ParserErrorType::RanOutOfTokens,
            ));
            None
        }
    }

    /// A helper wrapper around `tokens.advance()` function.
    /// Appends an error if advancing yielded no results
    fn advance<'a>(&mut self, tokens: &'a mut TokenIterator) -> Option<&'a Token> {
        let token = tokens.advance();
        if let Some(token) = token {
            Some(token)
        } else {
            self.errors.push(ParserError::new_without_token(
                ParserErrorType::RanOutOfTokens,
            ));
            None
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

            match token.token_type {
                // Keyword is a nice way to know we are aligned
                TT::Keyword(_) |
                // Finding the end of blocks should help parser correctly close them
                TT::Punctuation(PT::RightBrace) => return,
                _ => {}
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

    /// Retain the token span; source excerpts are rendered by the diagnostic layer.
    fn get_error_marked_line(&self, error_type: ParserErrorType, token: Token) -> ParserError {
        ParserError {
            location: Some(token.source_location),
            error_type,
            token: Some(token),
        }
    }

    /// Creates a Missing Token error while pointing to the current token in the iterator
    fn missing_token_error(
        &self,
        error_message: &'static str,
        tokens: &mut TokenIterator,
    ) -> ParserError {
        let current_token = tokens
            .peek()
            .map(|token| (*token).clone())
            .or_else(|| tokens.previous().cloned());
        let error_type = ParserErrorType::MissingToken(String::from(error_message));
        match current_token {
            Some(token) => self.get_error_marked_line(error_type, token),
            None => ParserError::new_without_token(error_type),
        }
    }

    /// Creates a Missing Token error while pointing to the previous token in the iterator
    fn missing_token_error_previous(
        &self,
        error_message: &'static str,
        tokens: &TokenIterator,
    ) -> ParserError {
        let error_type = ParserErrorType::MissingToken(String::from(error_message));
        match tokens.previous() {
            Some(token) => self.get_error_marked_line(error_type, token.clone()),
            None => ParserError::new_without_token(error_type),
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

#[derive(Debug, Clone)]
pub struct ParserError {
    error_type: ParserErrorType,
    pub token: Option<Token>,
    location: Option<crate::frontend::utils::SourceLocation>,
}

impl ParserError {
    pub fn new_without_token(error_type: ParserErrorType) -> ParserError {
        ParserError {
            error_type,
            token: None,
            location: None,
        }
    }
}

impl Error for ParserError {}
impl Display for ParserError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error_type)
    }
}

impl From<ParserError> for crate::diagnostic::Diagnostic {
    fn from(error: ParserError) -> Self {
        Self::new(
            crate::diagnostic::DiagnosticKind::Parse(error.error_type),
            error.location,
        )
    }
}

impl Display for ParserErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let output = match self {
            ParserErrorType::ScannerError(scanner_error) => &format!("{scanner_error}"),
            ParserErrorType::UnclosedParenthesis => "Unclosed parenthesis",
            ParserErrorType::RanOutOfTokens => "No more tokens to parse",
            ParserErrorType::IncorrectToken(message) if message.is_empty() => {
                "Incorrect token in this place."
            }
            ParserErrorType::IncorrectToken(message) => {
                &format!("Incorrect token in this place. {message}")
            }
            ParserErrorType::UnfinishedTernary => "Incorrect ternary operator",
            ParserErrorType::BinaryOperatorNoLeftPart => "Binary operator without a left part",
            ParserErrorType::MissingToken(message) => &format!("Missing token. {message}"),
            ParserErrorType::InvalidAssignmentTarget => "Invalid assignment target",
            ParserErrorType::InvalidDefDeclaration(def_error) => &format!("{def_error}"),
            ParserErrorType::InvalidPinRef(pin_ref_error) => &format!("{pin_ref_error}"),
            ParserErrorType::UnclosedDefBlock => "Def block was unclosed (missing '}')",
            ParserErrorType::EmptyDefBlock => "Def block can't be empty",
            ParserErrorType::DefOrVarAfterSetupOrLoop => {
                "'def' and 'var' statements must go before setup and loop statements"
            }
            ParserErrorType::DuplicateSetup => "Can't declare 'setup' more than once",
            ParserErrorType::SetupAfterLoop => "Declare 'setup' before 'loop'",
            ParserErrorType::DuplicateLoop => "Can't declare 'loop' more than once",
            ParserErrorType::IncorrectGlobalToken => {
                "This token is invalid here (expected 'fn', 'def', 'var', 'setup', 'loop')"
            }
            ParserErrorType::MissingLoopDeclaration => "'loop' must be defined",
            ParserErrorType::InfiniteLoopDetected => {
                "Parser got stuck in an infinite loop here (internal error)"
            }
        };

        f.write_str(output)
    }
}

#[derive(Debug, Clone)]
pub enum ParserErrorType {
    ScannerError(ScannerError),
    UnclosedParenthesis,
    RanOutOfTokens,
    IncorrectToken(String),
    UnfinishedTernary,
    BinaryOperatorNoLeftPart,
    MissingToken(String),
    InvalidAssignmentTarget,
    InvalidDefDeclaration(DefDeclarationError),
    InvalidPinRef(PinRefError),
    UnclosedDefBlock,
    EmptyDefBlock,
    DefOrVarAfterSetupOrLoop,
    DuplicateSetup,
    SetupAfterLoop,
    DuplicateLoop,
    IncorrectGlobalToken,
    MissingLoopDeclaration,
    InfiniteLoopDetected,
}

impl From<ScannerError> for ParserError {
    fn from(value: ScannerError) -> Self {
        Self {
            location: Some(value.location),
            error_type: ParserErrorType::ScannerError(value),
            token: None,
        }
    }
}
