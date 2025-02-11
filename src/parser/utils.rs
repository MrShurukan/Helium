use crate::parser::token::Token;
use std::iter::Peekable;
use std::slice::Iter;
use unicode_segmentation::{Graphemes, UnicodeSegmentation};

/// Helper struct for holding string slice and working with graphemes
pub struct ReadOnlyGraphemeSlice<'a> {
    string: &'a str
}

impl ReadOnlyGraphemeSlice<'_> {
    pub fn new(string: &str) -> ReadOnlyGraphemeSlice {
        ReadOnlyGraphemeSlice { string }
    }

    pub fn grapheme_peekable(&self) -> Peekable<Graphemes> {
        self.string.graphemes(true).peekable()
    }

    /// Gets the line from the source string. `line_num` is counting from 1, not 0
    pub fn get_line(&self, line_num: usize) -> Option<&str> {
        self.string.split('\n').nth(line_num - 1)
    }
}

/// Helper struct for temporarily hold a token slice + create a peekable iterator from it.
/// It's useful for token marching in the parser as well as keeping track of index (for error
/// producing)
pub struct TokenIterator<'a> {
    tokens: &'a [Token],
    iter: Peekable<Iter<'a, Token>>,
    index: usize
}

impl TokenIterator<'_> {
    pub fn new(tokens: &[Token]) -> TokenIterator {
        TokenIterator { tokens, iter: tokens.iter().peekable(), index: 0 }
    }
    
    pub fn advance(&mut self) -> Option<&Token> {
        let next = self.iter.next();
        if next.is_some() {
            self.index += 1;
        }
        
        next
    }
    
    pub fn peek(&mut self) -> Option<&&Token> {
        self.iter.peek()
    }
    
    pub fn previous(&self) -> Option<&Token> {
        self.tokens.get(self.index - 1)
    }
}