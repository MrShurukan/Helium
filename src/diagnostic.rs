use crate::frontend::utils::SourceLocation;
use std::fmt;
use unicode_segmentation::UnicodeSegmentation;

/// A diagnostic is data; rendering and process exit belong to the caller.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub location: Option<SourceLocation>,
}

impl Diagnostic {
    pub fn new(
        code: &'static str,
        message: impl Into<String>,
        location: Option<SourceLocation>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            location,
        }
    }

    pub fn render(&self, source: &str) -> String {
        let mut result = self.to_string();
        if let Some(span) = self.location {
            for (index, line) in source.lines().enumerate() {
                let number = index + 1;
                if number < span.line_start || number > span.line_end {
                    continue;
                }
                let length = line.graphemes(true).count();
                let start = if number == span.line_start {
                    span.start_offset.min(length)
                } else {
                    0
                };
                let end = if number == span.line_end {
                    span.end_offset.min(length)
                } else {
                    length
                };
                result.push_str(&format!(
                    "\n{number:>4} | {line}\n     | {}{}",
                    " ".repeat(start),
                    "^".repeat(end.saturating_sub(start).max(1))
                ));
            }
        }
        result
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {}", self.code, self.message)
    }
}
impl std::error::Error for Diagnostic {}
