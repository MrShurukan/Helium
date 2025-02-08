use std::error::Error;
use std::fmt::{Display, Formatter};
use std::iter::Peekable;
use unicode_segmentation::Graphemes;
use crate::parser::position_info::PositionInfo;
use crate::parser::token::Token;
use crate::parser::token_type::TokenType;
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
            Token { token_type: TokenType::EOF, line: self.position_info.line,
                lexeme_size: 1, line_offset: self.position_info.current_from_line_start + 1 }
        );

        if errors.len() > 0 {
            Err(errors)
        }
        else {
            Ok(tokens)
        }
    }

    fn scan_token(position_info: &mut PositionInfo,
                      peekable: &mut Peekable<Graphemes>,
                      source: &ReadOnlyGraphemeSlice) -> Result<Option<Token>, ScannerError> {

        let grapheme = match Self::advance(peekable, position_info) {
            Some(grapheme) => grapheme,
            None => return Err(SE::new_with_location(SET::NoMoreTokens, position_info, source))
        };

        use TokenType::Punctuation as P;
        use crate::parser::token_type::PunctuationType as PT;

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
            " " => None,
            "\r" => None,
            "\t" => None,

            "\n" | "\r\n" => { position_info.advance_line(); None },

            // Literals
            "\"" => Some(Self::parse_string_literal(peekable, position_info, source)?),

            // It's okay to use chars here, as numbers will always be expected to be regular chars,
            // not graphemes
            number if number.chars().all(|x| x.is_ascii_digit()) => {
                Some(Self::parse_number_literal(peekable, position_info, source)?)
            },

            // Identifiers and reserve words are a different story. Could be pretty complex.
            alpha if alpha.chars().all(|x| x.is_alphabetic() || x == '_') => {
                Some(Self::parse_alpha(peekable, position_info, source?))
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
        let multiline_comment_start = position_info.current;

        let mut nesting_level = 1;

        while let Some(current_char) = peekable.next() {
            position_info.advance();
            
            if Self::is_newline(current_char) {
                position_info.advance_line()
            }
            // I support nested multiline comments. We need to account for that
            else if current_char == "/" {
                if Self::consume_if_matches("*", peekable, position_info) {
                    // Nesting located, increase the nesting_level
                    nesting_level += 1;
                }
            }
            // If we encounter a star, that could mean we are at the end of the comment
            else if current_char == "*" {
                if Self::consume_if_matches("/", peekable, position_info) {
                    // We found the end of a block! Decrease the nesting_level
                    // Check if we are out of nesting by making sure nesting_level == 0, 
                    // otherwise continue

                    nesting_level -= 1;
                    if nesting_level == 0 {
                        return Ok(());
                    }
                }
            }
        }

        // If we finished reading the source but never reached the end of a multiline - it's an error!
        if nesting_level != 0 {
            Err(SE::new_with_location(SET::UnterminatedMultilineComment, position_info, source))
        } else {
            unreachable!();
        }
    }
    
    /// Checks if the next grapheme in `peekable` matches `grapheme`. If it does - produces 
    /// token with `if_matches` token_type and advances position_info. Otherwise, produces
    /// token with `id_doesnt_match` token_type.
    fn next_grapheme_check(grapheme: &'static str, if_matches: TokenType, if_doesnt_match: TokenType,
        peekable: &mut Peekable<Graphemes>, position_info: &mut PositionInfo) -> Token {
        
        // TODO: Potentially doesn't work?
        // It seems this compares references to str, not str slices themselves. Gotta check for that.
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
        match grapheme {
            "\r\n" | "\n" => true,
            _ => false
        }
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

        // Something went horribly wrong, not supposed to happen
        if location.is_none() {
            panic!("Couldn't get the line #{}", position_info.line);
        }

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

        Some(format!("{line_number_string}{line}\n{spaces}^"))
    }
}

#[derive(Debug)]
pub enum ScannerErrorType {
    UnknownToken(String),
    UnterminatedString,
    IncorrectNumberToken { reason: String },
    TrailingNumber,
    NoMoreTokens,
    UnterminatedMultilineComment
}

impl Error for ScannerError {}
impl Display for ScannerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let error_header =
            format!("[line {}] Scanner Error", self.line);

        let output = match &self.error_type {
            ScannerErrorType::UnknownToken(lexeme) => &format!("Unknown token ('{}')", lexeme),
            ScannerErrorType::UnterminatedString => "Unterminated string",
            ScannerErrorType::IncorrectNumberToken { reason }
                => &format!("The number was written in an incorrect format: {}", reason),
            ScannerErrorType::NoMoreTokens => "There were no more tokens to parse",
            ScannerErrorType::UnterminatedMultilineComment => "Multiline comment was not terminated",
            ScannerErrorType::TrailingNumber => "Trailing literals are not supported"
        };

        write!(f, "{error_header}: {}", output)?;
        if let Some(location) = &self.location {
            write!(f, "\n\nHappened here:\n{}", location)?;
        }

        Ok(())
    }
}