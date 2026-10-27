//! Source identity and checked conversion of lexer coordinates to byte ranges.
//! Editors can convert these byte ranges to their negotiated position encoding.
use crate::frontend::utils::SourceLocation;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    pub file: FileId,
    pub bytes: Range<usize>,
}

pub struct SourceFile<'a> {
    pub id: FileId,
    pub name: &'a str,
    pub text: &'a str,
}

impl SourceFile<'_> {
    pub fn span(&self, location: SourceLocation) -> Option<SourceSpan> {
        let offset = |line_number: usize, column: usize| {
            let mut base = 0;
            for (index, line) in self.text.split('\n').enumerate() {
                if index + 1 == line_number {
                    let line = if base + line.len() < self.text.len() {
                        line.strip_suffix('\r').unwrap_or(line)
                    } else {
                        line
                    };
                    let end = line
                        .grapheme_indices(true)
                        .map(|(i, _)| i)
                        .chain(std::iter::once(line.len()))
                        .nth(column)?;
                    return Some(base + end);
                }
                base += line.len() + 1;
            }
            None
        };
        let start = offset(location.line_start, location.start_offset)?;
        let end = offset(location.line_end, location.end_offset)?;
        (start <= end).then_some(SourceSpan {
            file: self.id,
            bytes: start..end,
        })
    }
}

impl super::Diagnostic {
    /// Bind a single-source compilation's primary and related locations to a file.
    /// Invalid locations are rejected rather than silently moved to other text.
    pub fn source_spans(&self, source: &SourceFile<'_>) -> Option<Vec<SourceSpan>> {
        if let Some(sources) = &self.sources {
            return self
                .location
                .into_iter()
                .chain(self.labels.iter().map(|l| l.location))
                .map(|s| sources.span(s))
                .collect();
        }
        self.location
            .into_iter()
            .chain(self.labels.iter().map(|l| l.location))
            .map(|location| source.span(location))
            .collect()
    }
}

/// Owned, stable source identities for a complete module graph.
#[derive(Debug, Clone, Default)]
pub struct SourceSet {
    pub files: Vec<SourceText>,
}
#[derive(Debug, Clone)]
pub struct SourceText {
    pub name: String,
    pub text: String,
}
impl SourceSet {
    pub fn get(&self, id: usize) -> Option<SourceFile<'_>> {
        self.files.get(id).map(|s| SourceFile {
            id: FileId(id),
            name: &s.name,
            text: &s.text,
        })
    }
    pub fn span(&self, location: SourceLocation) -> Option<SourceSpan> {
        self.get(location.file)?.span(location)
    }
}
