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
    repeated_peek_count: usize,
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
    
    pub fn previous(&self) -> Option<&Token> {
        self.tokens.get(self.index - 1)
    }
}