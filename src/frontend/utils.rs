use crate::frontend::token::Token;
use std::iter::Peekable;
use std::slice::Iter;
use colored::Colorize;
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
    index: usize,
    /// Holds how many times current token was peeked
    pub repeated_peek_count: usize,
    last_peeked_index: Option<usize>
}

impl TokenIterator<'_> {
    pub fn new(tokens: &[Token]) -> TokenIterator {
        TokenIterator { tokens, iter: tokens.iter().peekable(), index: 0, 
            last_peeked_index: None, repeated_peek_count: 0 }
    }
    
    pub fn advance(&mut self) -> Option<&Token> {
        let next = self.iter.next();
        if next.is_some() {
            self.index += 1;
            if self.repeated_peek_count != 0 {
                println!();
                self.repeated_peek_count = 0;
            }
            println!("[TokenIterator] Advancing to {} (read: {:?})", self.index.to_string().yellow(), next);
        }
        
        next
    }
    
    pub fn peek(&mut self) -> Option<&&Token> {
        let peeked = self.iter.peek();
        
        if let Some(last_peeked) = self.last_peeked_index {
            if last_peeked != self.index {
                if self.repeated_peek_count != 0 {
                    println!();
                    self.repeated_peek_count = 0;
                }
                println!("[TokenIterator] Peeking at {} (read: {:?})", self.index.to_string().green(), peeked);
            }
            else {
                if self.repeated_peek_count == 0 {
                    print!("{} {} (1 time)", "[TokenIterator] Repeated peek at".yellow(),
                           self.index.to_string().green());
                }
                else {
                    print!("\r{} {} ({} times)", "[TokenIterator] Repeated peek at".yellow(),
                           self.index.to_string().green(), self.repeated_peek_count+1);
                }
                self.repeated_peek_count += 1;
            }
        }
        else {
            println!("[TokenIterator] Peeking at {} (read: {:?})", self.index.to_string().green(), peeked);
        }

        self.last_peeked_index = Some(self.index);
        peeked
    }
    
    /// This is the nuke button. If called token iterator will march until the end, effectively
    /// running out of tokens immediately. Useful as a "get-me-out-of-parser" measure (i.e. infinite
    /// loop detection)
    pub fn invalidate(&mut self) {
        while let Some(_) = self.iter.next() {}
    }
    
    pub fn previous(&self) -> Option<&Token> {
        self.tokens.get(self.index - 1)
    }
}

/// Anything that can be located in source via `SourceLocation` (i.e. `Expression`s or `Statement`s)
/// may implement this. Just makes it a bit easier to handle error printing in later stages of
/// the compiler
pub trait LocatableInSource {
    fn get_location(&self) -> &SourceLocation;
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub struct SourceLocation {
    pub line_start: usize,
    pub line_end: usize,
    /// Offset in grapheme count from the start of the first line
    pub start_offset: usize,
    /// Offset in grapheme count from the start of the last file
    pub end_offset: usize
}

impl SourceLocation {
    pub fn new_from_start_end(start: SourceLocation, end: SourceLocation) -> SourceLocation {
        SourceLocation {
            line_start: start.line_start,
            line_end: end.line_end,
            start_offset: start.start_offset,
            end_offset: end.end_offset,
        }
    }
}