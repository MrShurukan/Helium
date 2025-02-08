use std::iter::Peekable;
use unicode_segmentation::{Graphemes, UnicodeSegmentation};

/// Helper struct for holding string slice and working with graphemes
pub struct ReadOnlyGraphemeSlice<'a> {
    string: &'a str,
    grapheme_count: usize
}

impl ReadOnlyGraphemeSlice<'_> {
    pub fn new(string: &str) -> ReadOnlyGraphemeSlice {
        ReadOnlyGraphemeSlice { string, grapheme_count: string.graphemes(true).count() }
    }

    pub fn grapheme_count(&self) -> &usize {
        &self.grapheme_count
    }
    pub fn grapheme_peekable(&self) -> Peekable<Graphemes> {
        self.string.graphemes(true).peekable()
    }

    /// Gets the line from the source string. `line_num` is counting from 1, not 0
    pub fn get_line(&self, line_num: usize) -> Option<&str> {
        self.string.split("\n").nth(line_num - 1)
    }
}