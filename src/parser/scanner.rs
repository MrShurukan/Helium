use std::error::Error;
use std::fmt::{Display, Formatter};
use std::iter::Peekable;
use unicode_segmentation::Graphemes;
use crate::parser::token::Token;
use crate::parser::token_type::TokenType;
use crate::parser::utils::ReadOnlyGraphemeSlice;

/// This struct is responsible for converting the source file to a list of tokens.
/// This list will later be used by other parts of the front-end (like `Parser`) to create
/// expressions and statements
pub struct Scanner<'a> {
    /// Source code (single file)
    source: ReadOnlyGraphemeSlice<'a>,
    position_info: PositionInfo
}

struct PositionInfo {
    /// Beginning position of a lexeme
    start: usize,
    /// Where scanner is currently in the file
    current: usize,
    /// Current line being processed by scanner
    line: usize,
    /// Similar to `current`, but from the start of the line.
    /// This variable is reset each time a newline encountered
    current_from_line_start: usize
}

impl Scanner<'_> {
    pub fn new(source: &str) -> Scanner {
        Scanner { source: ReadOnlyGraphemeSlice::new(source),
            position_info: PositionInfo { start: 0, current: 0, line: 1, current_from_line_start: 0 } }
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
                Ok(token) => tokens.push(token),
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
                      source: &ReadOnlyGraphemeSlice) -> Result<Token, ScannerError> {
        todo!("Scanner.scan_token")
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