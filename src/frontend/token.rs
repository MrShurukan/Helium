use std::fmt;
use crate::frontend::token_type::TokenType;
use crate::frontend::utils::{LocatableInSource, SourceLocation};

// TODO: Comment is inaccurate due to migration to source_location
/// After a character inside the source file, this is the smallest meaningful piece of
/// information about a program. Could represent punctuation, a keyword, identifier, etc.
///
/// Doesn't actually store the representing lexeme inside the source code, instead a `line` number
/// and a `line_offset` + `lexeme_size` are used to reconstruct the lexeme if necessary.
/// I decided it was a little bit excessive to store strings or construct str slices (it's rarely
/// required, just for error printing)
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct Token {
    pub token_type: TokenType,
    pub source_location: SourceLocation
}

impl LocatableInSource for Token {
    fn get_location(&self) -> &SourceLocation {
        &self.source_location
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Token {{ type: {:?}, line: {} }}",
               self.token_type, self.source_location.line_start)
    }
}