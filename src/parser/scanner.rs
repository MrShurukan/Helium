use crate::parser::token::Token;

/// This struct is responsible for converting the source file to a list of tokens.
/// This list will later be used by other parts of the front-end (Parser) to create
/// expressions and statements
pub struct Scanner<'a> {
    source: &'a str
}

impl Scanner<'_> {
    pub fn new(source: &str) -> Scanner {
        Scanner { source }
    }

    /// Our compiler takes first steps in analyzing what's happening in the program.
    /// This function strolls throw `source` by analyzing graphemes.
    /// It's important to analyze *graphemes* and not *chars*, because Rust's char will not
    /// properly hold extended utf-8 chars, like Russian letters or emojis. Let's stay
    /// modern, shall we?
    pub fn scan_tokens() -> Result<Vec<Token>, Vec<ScannerError>> {
        todo!("Scanner.scan")
    }
}

pub struct ScannerError;