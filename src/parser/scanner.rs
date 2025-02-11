use std::error::Error;
use std::fmt::{Display, Formatter};
use std::iter::Peekable;
use colored::Colorize;
use unicode_segmentation::Graphemes;
use crate::parser::position_info::PositionInfo;
use crate::parser::token::Token;
use crate::parser::token_type::{LiteralType, TokenType, CONSTANTS, KEYWORDS, TYPES};
use crate::parser::utils::ReadOnlyGraphemeSlice;

use ScannerErrorType as SET;
use ScannerError as SE;

/// This struct is responsible for converting the source file to a list of tokens.
/// This list will later be used by other parts of the front-end (like `Parser`) to create
/// expressions and statements
pub struct Scanner<'a> {
    /// Source code (single file)
    source: ReadOnlyGraphemeSlice<'a>,
    position_info: PositionInfo
}

impl Scanner<'_> {
    pub fn new(source: &str) -> Scanner {
        Scanner { source: ReadOnlyGraphemeSlice::new(source), position_info: PositionInfo::new() }
    }

    /// Our compiler takes first steps in analyzing what's happening in the program.
    /// This function strolls through `source` by analyzing graphemes.
    /// It's important to analyze *graphemes* and not *chars*, because Rust's char will not
    /// properly hold extended utf-8 chars, like Russian letters or emojis. Let's stay
    /// modern, shall we?
    pub fn scan_tokens(&mut self) -> Result<Vec<Token>, Vec<ScannerError>> {
        let mut errors: Vec<ScannerError> = Vec::new();
        let mut tokens: Vec<Token> = Vec::new();
        
        let mut peekable = self.source.grapheme_peekable();

        loop {
            // We are at the beginning of the next lexeme.
            self.position_info.start = self.position_info.current;

            match Self::scan_token(&mut self.position_info, &mut peekable, &self.source) {
                Ok(Some(token)) => tokens.push(token),
                // No errors found, but no token was produced, i.e. whitespace, comments, etc.
                Ok(None) => { },
                Err(err) => {
                    if let ScannerErrorType::NoMoreTokens = err.error_type {
                        break;
                    }

                    errors.push(err);
                }
            }
        }

        tokens.push(
            Token { token_type: TokenType::Eof, line: self.position_info.line,
                lexeme_size: 1, line_offset: self.position_info.current_from_line_start + 1 }
        );

        if errors.is_empty() {
            Ok(tokens)
        }
        else {
            Err(errors)
        }
    }

    fn scan_token(position_info: &mut PositionInfo,
                      peekable: &mut Peekable<Graphemes>,
                      source: &ReadOnlyGraphemeSlice) -> Result<Option<Token>, ScannerError> {
        
        use TokenType::Punctuation as P;
        use crate::parser::token_type::PunctuationType as PT;
        
        let grapheme = Self::advance(peekable, position_info).ok_or_else(|| {
            SE::new_with_location(SET::NoMoreTokens, position_info, source)
        })?;

        let token = match grapheme {
            // Single char punctuation
            "(" => Some(Self::produce_token(P(PT::LeftParen), position_info)),
            ")" => Some(Self::produce_token(P(PT::RightParen), position_info)),
            "{" => Some(Self::produce_token(P(PT::LeftBrace), position_info)),
            "}" => Some(Self::produce_token(P(PT::RightBrace), position_info)),
            "," => Some(Self::produce_token(P(PT::Comma), position_info)),
            "." => Some(Self::produce_token(P(PT::Dot), position_info)),
            "-" => Some(Self::produce_token(P(PT::Minus), position_info)),
            "+" => Some(Self::produce_token(P(PT::Plus), position_info)),
            ";" => Some(Self::produce_token(P(PT::Semicolon), position_info)),
            "*" => Some(Self::produce_token(P(PT::Star), position_info)),
            "?" => Some(Self::produce_token(P(PT::Question), position_info)),
            ":" => Some(Self::produce_token(P(PT::Colon), position_info)),

            // Check for multichar punctuation (by checking the next char)
            "!" => Some(Self::next_grapheme_check("=", P(PT::BangEqual), P(PT::Bang), peekable, position_info)),
            "=" => Some(Self::next_grapheme_check("=", P(PT::EqualEqual), P(PT::Equal), peekable, position_info)),
            "<" => Some(Self::next_grapheme_check("=", P(PT::LessEqual), P(PT::Less), peekable, position_info)),
            ">" => Some(Self::next_grapheme_check("=", P(PT::GreaterEqual), P(PT::Greater), peekable, position_info)),

            // A special case: '/', as it's division and comments
            "/" => {
                // Comment
                if Self::consume_if_matches("/", peekable, position_info) {
                    // Read until the end of the line
                    while let Some(next_grapheme) = peekable.peek() {
                        if Self::is_newline(next_grapheme) { break; }
                        Self::advance(peekable, position_info);
                    }
                    
                    None
                }
                // Multiline comment
                else if Self::consume_if_matches("*", peekable, position_info) {
                    Self::multiline_comment(peekable, position_info, source)?;
                    
                    None
                }
                else {
                    Some(Self::produce_token(P(PT::Slash), position_info))
                }
            },

            // Stuff to skip
            // \u{FEFF} - BOM
            " " | "\r" | "\t" | "\u{FEFF}" => None,

            "\n" | "\r\n" => { position_info.advance_line(); None },

            // Literals
            "\"" => Some(Self::parse_string_literal(peekable, position_info, source)?),

            // It's okay to use chars here, as numbers will always be expected to be regular chars,
            // not graphemes
            number if number.chars().all(|x| x.is_ascii_digit()) => {
                Some(Self::parse_number_literal(peekable, position_info, source)?)
            },

            // Identifiers and reserve words are a different story than numbers. But we still limit
            // this to alphabetic characters
            alpha if alpha.chars().all(|x| x.is_alphabetic() || x == '_') => {
                Some(Self::parse_alpha(peekable, position_info, source)?)
            },
            
            _ => {
                let error_type = ScannerErrorType::UnknownToken(String::from(grapheme));
                return Err(SE::new_with_location(error_type, position_info, source))
            }
        };
        
        Ok(token)
    }
    
    fn multiline_comment(peekable: &mut Peekable<Graphemes>, 
                         position_info: &mut PositionInfo, 
                         source: &ReadOnlyGraphemeSlice) -> Result<(), ScannerError> {
        // Remember where the multiline comment started
        let multiline_comment_start = position_info.clone();

        let mut nesting_level = 1;

        while let Some(current_char) = peekable.next() {
            position_info.advance();
            
            if Self::is_newline(current_char) {
                position_info.advance_line();
            }
            // I support nested multiline comments. We need to account for that
            else if current_char == "/" {
                if Self::consume_if_matches("*", peekable, position_info) {
                    // Nesting located, increase the nesting_level
                    nesting_level += 1;
                }
            }
            // If we encounter a star, that could mean we are at the end of the comment
            else if current_char == "*" && Self::consume_if_matches("/", peekable, position_info) {
                // We found the end of a block! Decrease the nesting_level
                // Check if we are out of nesting by making sure nesting_level == 0, 
                // otherwise continue

                nesting_level -= 1;
                if nesting_level == 0 {
                    return Ok(());
                }
            }
        }

        // If we finished reading the source but never reached the end of a multiline - it's an error!
        if nesting_level != 0 {
            Err(SE::new_with_location(SET::UnterminatedMultilineComment, &multiline_comment_start, source))
        } else {
            unreachable!();
        }
    }
    
    fn parse_string_literal(peekable: &mut Peekable<Graphemes>,
                            position_info: &mut PositionInfo,
                            source: &ReadOnlyGraphemeSlice) -> Result<Token, ScannerError> {
        let start_string_position = position_info.clone();
        while let Some(current_char) = peekable.next() {
            position_info.advance();
            match current_char {
                "\n" | "\r\n" => return Err(SE::new_with_location(SET::MultilineStringNotSupported, position_info, source)),
                "\"" => break,
                _ => {}
            }
        }

        if peekable.peek().is_none() {
            return Err(
                SE::new_with_location(SET::UnterminatedString, &start_string_position, source)
            );
        }

        // Get the string value
        let str = source.grapheme_peekable()
            .skip(position_info.start + 1)
            .take(position_info.current - position_info.start - 2)
            .collect();

        Ok(Self::produce_token(TokenType::Literal(LiteralType::String(str)), position_info))
    }
    
    fn parse_number_literal(peekable: &mut Peekable<Graphemes>,
                            position_info: &mut PositionInfo,
                            source: &ReadOnlyGraphemeSlice) -> Result<Token, ScannerError> {
        // Advance until there are no more digits
        Self::advance_while_digit(peekable, position_info);

        // Look for a fractional part.
        let current_grapheme = peekable.peek();
        match current_grapheme {
            // Fractional number
            Some(current_grapheme) if *current_grapheme == "." => {
                Self::advance(peekable, position_info);
                // Using peek_no_borrow here because it's an arbitrary peek ahead function
                let grapheme_after_that = peekable.peek()
                    .ok_or_else(||
                        SE::new_with_location(SET::TrailingNumber, position_info, source)
                    )?;

                if grapheme_after_that.chars().all(|c| c.is_ascii_digit()) {
                    // Consume the "."
                    Self::advance(peekable, position_info);
                    // Advance until there are no more digits
                    Self::advance_while_digit(peekable, position_info);
                }
                else {
                    let position_info_copy = PositionInfo { current: position_info.current + 1, ..*position_info };
                    return Err(SE::new_with_location(SET::TrailingNumber, &position_info_copy, source));
                }

                // Get the number itself
                let str: String = Self::get_current_lexeme(position_info, source);
                let value: f64 = str.parse().map_err(|_| SE::new_with_location(
                    SET::IncorrectNumberToken { reason: "Couldn't convert to a double".parse().unwrap() },
                    position_info, source))?;
                Ok(Self::produce_token(TokenType::Literal(LiteralType::Float(value)), position_info))
            },
            // Integer
            _ => {
                Self::produce_integer_token(position_info, source)
            }
        }
    }

    fn produce_integer_token(position_info: &mut PositionInfo, source: &ReadOnlyGraphemeSlice) -> Result<Token, ScannerError> {
        // Get the number itself
        let str: String = Self::get_current_lexeme(position_info, source);
        let value: i64 = str.parse().map_err(|_| SE::new_with_location(
            SET::IncorrectNumberToken { reason: "Couldn't convert to an integer".parse().unwrap() },
            position_info, source))?;
        
        Ok(Self::produce_token(TokenType::Literal(LiteralType::Integer(value)), position_info))
    }

    fn parse_alpha(peekable: &mut Peekable<Graphemes>,
                   position_info: &mut PositionInfo,
                   source: &ReadOnlyGraphemeSlice) -> Result<Token, ScannerError> {
        while peekable.next_if(|grapheme| 
                               grapheme.chars().all(|x| x.is_alphanumeric() || x == '_')).is_some() {
            position_info.advance();
        }

        let lexeme = Self::get_current_lexeme(position_info, source);

        // Check if it's reserved
        if let Some(keyword_type) = KEYWORDS.get(lexeme.as_str()) {
            Ok(Self::produce_token(TokenType::Keyword(keyword_type), position_info))
        }
        else if let Some(constant_type) = CONSTANTS.get(lexeme.as_str()) {
            Ok(Self::produce_token(TokenType::Constant(constant_type), position_info))
        }
        else if let Some(static_type) = TYPES.get(lexeme.as_str()) {
            Ok(Self::produce_token(TokenType::Type(static_type), position_info))
        }
        // Otherwise it's just a user defined lexeme / port ref
        else if lexeme.starts_with("Port") {
            let port_name = lexeme.split("Port").nth(1);
            if port_name.is_none() || port_name.unwrap().is_empty() {
                return Err(SE::new_with_location(SET::MissingPortName, position_info, source));
            }
            
            Ok(Self::produce_token(TokenType::Port(port_name.unwrap().to_owned()), position_info))
        }
        else {
            Ok(Self::produce_token(TokenType::Literal(LiteralType::Identifier(lexeme)), position_info))
        }
    }

    fn get_current_lexeme(position_info: &mut PositionInfo, source: &ReadOnlyGraphemeSlice) -> String {
        let take_count = position_info.current - position_info.start;
        source.grapheme_peekable().skip(position_info.start).take(take_count).collect()
    }

    fn advance_while_digit(peekable: &mut Peekable<Graphemes>, position_info: &mut PositionInfo) {
        while peekable.next_if(|grapheme| {
            grapheme.chars().any(|c| c.is_ascii_digit())
        }).is_some() {
            position_info.advance();
        }
    }
    
    /// Checks if the next grapheme in `peekable` matches `grapheme`. If it does - produces 
    /// token with `if_matches` token type and advances `position_info`. Otherwise, produces
    /// token with `id_doesnt_match` token type.
    fn next_grapheme_check(grapheme: &'static str, if_matches: TokenType, if_doesnt_match: TokenType,
        peekable: &mut Peekable<Graphemes>, position_info: &mut PositionInfo) -> Token {
        
        match peekable.next_if_eq(&grapheme) {
            Some(_) => { position_info.advance(); Self::produce_token(if_matches, position_info) },
            None => Self::produce_token(if_doesnt_match, position_info)
        }
    }

    /// Checks if the next grapheme in `peekable` matches `grapheme`. If it does - 
    /// it consumes it (by advancing) and returns true. Otherwise, false is returned.
    fn consume_if_matches(grapheme: &'static str, 
                          peekable: &mut Peekable<Graphemes>, 
                          position_info: &mut PositionInfo) -> bool {
        
        match peekable.next_if_eq(&grapheme) {
            Some(_) => { position_info.advance(); true }
            None => false
        }
    }
    
    fn produce_token(token_type: TokenType, position_info: &PositionInfo) -> Token {
        let lexeme_size = position_info.current - position_info.start;
        Token { token_type, line: position_info.line,
            line_offset: position_info.current_from_line_start - lexeme_size + 1, lexeme_size }
    }

    fn advance<'a>(peekable: &'a mut Peekable<Graphemes>, position_info: &mut PositionInfo) -> Option<&'a str> {
        let current_grapheme = peekable.next()?;
        position_info.advance();

        Some(current_grapheme)
    }
    
    fn is_newline(grapheme: &str) -> bool {
        matches!(grapheme, "\r\n" | "\n")
    }
}

#[derive(Debug)]
pub struct ScannerError {
    error_type: ScannerErrorType,
    line: usize,
    location: Option<String>
}

impl ScannerError {
    fn new(error_type: ScannerErrorType, line: usize) -> Self {
        ScannerError { error_type, line, location: None }
    }

    fn new_with_location(error_type: ScannerErrorType, position_info: &PositionInfo, source: &ReadOnlyGraphemeSlice) -> Self {
        let location = ScannerError::get_line_with_marker(
            source,
            position_info.line,
            position_info.current_from_line_start);

        // Something went horribly wrong if we didn't get the line, not supposed to happen
        assert!(location.is_some(), "Couldn't get the line #{}", position_info.line);

        ScannerError { error_type, line: position_info.line, location: Some(location.unwrap()) }
    }

    /// Gets a helper string for error displaying with a marker on the second line.
    /// ### Example
    /// This function is used to create a string that looks something like this:
    /// ```
    /// 14. var test = (hello + 1
    ///                ^
    /// ```
    /// Resulting string consists of two lines and has a marker denoted by
    /// `^` on the second line. The line is automatically inferred by the `marker_position` from
    /// the `self.source` string. See "Arguments" below.
    ///
    /// ### Arguments
    /// * `line_number` - Line number (counting from 1) where the error is located
    /// * `marker_position` - Index from a start of a line (`line_number`) into the `source.grapheme_peekable` string.
    fn get_line_with_marker(source: &ReadOnlyGraphemeSlice, line_number: usize, marker_position: usize) -> Option<String> {
        let line = source.get_line(line_number)?;

        let line_number_string = format!("{line_number}. ");
        let spaces = " ".repeat(line_number_string.len() + marker_position - 1);

        Some(format!("{line_number_string}{line}\n{spaces}{}", "^".red()))
    }
}

#[derive(Debug)]
pub enum ScannerErrorType {
    UnknownToken(String),
    UnterminatedString,
    IncorrectNumberToken { reason: String },
    TrailingNumber,
    NoMoreTokens,
    UnterminatedMultilineComment,
    MultilineStringNotSupported,
    MissingPortName
}

impl Error for ScannerError {}
impl Display for ScannerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let error_header =
            format!("[line {}] Scanner Error", self.line);

        let output = match &self.error_type {
            ScannerErrorType::UnknownToken(lexeme) => &format!("Unknown token ('{lexeme}')"),
            ScannerErrorType::UnterminatedString => "Unterminated string",
            ScannerErrorType::IncorrectNumberToken { reason }
                => &format!("The number was written in an incorrect format: {reason}"),
            ScannerErrorType::NoMoreTokens => "There were no more tokens to parse",
            ScannerErrorType::UnterminatedMultilineComment => "Multiline comment was not terminated",
            ScannerErrorType::TrailingNumber => "Trailing literals are not supported",
            ScannerErrorType::MultilineStringNotSupported => "Multiline strings are not supported",
            ScannerErrorType::MissingPortName => "Port name was not specified"
        };

        write!(f, "{error_header}: {}", output.red())?;
        if let Some(location) = &self.location {
            write!(f, "\n\n{}\n{location}", "Happened here:".green())?;
        }

        Ok(())
    }
}