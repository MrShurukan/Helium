use std::fmt;
use crate::parser::token_type::TokenType;

/// After a character inside the source file, this is the smallest meaningful piece of
/// information about a program. Could represent punctuation, a keyword, identifier, etc.
///
/// Doesn't actually store the representing lexeme inside the source code, instead a `line` number
/// and a `line_offset` + `lexeme_size` are used to reconstruct the lexeme if necessary.
/// I decided it was a little bit excessive to store strings or construct str slices (it's rarely
/// required, just for error printing)
pub struct Token {
    pub token_type: TokenType,
    pub line: usize,
    /// Offset in grapheme count from the start of the line
    pub line_offset: usize,
    /// Size of the token lexeme
    pub lexeme_size: usize
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Token {{ type: {:?}, line: {} }}",
               self.token_type, self.line)
    }
}