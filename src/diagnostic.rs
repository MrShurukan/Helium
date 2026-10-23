mod messages;

use crate::frontend::{
    static_type::StaticType,
    token_type::{PunctuationType, TokenType},
    utils::SourceLocation,
    ParserErrorType,
};
use std::fmt;
use unicode_segmentation::UnicodeSegmentation;

/// Stable public identifiers, independent of wording and presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCode {
    Duplicate,
    Initializer,
    ForType,
    Condition,
    Undefined,
    UnaryType,
    BinaryType,
    TernaryType,
    NumericType,
    LiteralRange,
    Type,
    Target,
    Pin,
    PinDuplicate,
    PinMode,
    Unsupported,
    Backend,
    Parse,
    Internal,
}

impl DiagnosticCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Duplicate => "E_DUPLICATE",
            Self::Initializer => "E_INITIALIZER",
            Self::ForType => "E_FOR_TYPE",
            Self::Condition => "E_CONDITION",
            Self::Undefined => "E_UNDEFINED",
            Self::UnaryType => "E_UNARY_TYPE",
            Self::BinaryType => "E_BINARY_TYPE",
            Self::TernaryType => "E_TERNARY_TYPE",
            Self::NumericType => "E_NUMERIC_TYPE",
            Self::LiteralRange => "E_LITERAL_RANGE",
            Self::Type => "E_TYPE",
            Self::Target => "E_TARGET",
            Self::Pin => "E_PIN",
            Self::PinDuplicate => "E_PIN_DUPLICATE",
            Self::PinMode => "E_PIN_MODE",
            Self::Unsupported => "E_UNSUPPORTED",
            Self::Backend => "E_BACKEND",
            Self::Parse => "E_PARSE",
            Self::Internal => "E_INTERNAL_IR",
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured facts about an error. Add translations at the presentation layer.
#[derive(Debug, Clone)]
pub enum DiagnosticKind {
    TypedIrUnsupported {
        feature: String,
    },
    InvalidIr {
        stage: &'static str,
        detail: String,
    },
    Parse(ParserErrorType),
    DuplicateName {
        name: String,
    },
    MissingInitializer,
    ForBounds,
    ConditionType {
        ty: StaticType,
    },
    UndefinedName {
        name: String,
    },
    UnaryType {
        operator: PunctuationType,
        operand: StaticType,
    },
    BinaryType {
        operator: TokenType,
        a: StaticType,
        b: StaticType,
    },
    BranchTypes {
        a: StaticType,
        b: StaticType,
    },
    NumericTypes {
        a: StaticType,
        b: StaticType,
    },
    InexactInteger {
        value: i128,
        target: StaticType,
    },
    IntegerRange {
        value: i128,
        target: StaticType,
        min: i128,
        max: i128,
    },
    LossyConversion {
        from: StaticType,
        target: StaticType,
    },
    NegatedUnsigned,
    FloatRange,
    UnsupportedTarget {
        mcu: String,
    },
    InvalidPin {
        port: String,
        number: usize,
    },
    DuplicatePin,
    PullDown,
    PullUpLowering,
    ConditionLowering,
    PinAssignmentLowering,
    StorageLowering,
    LoopLowering,
    PullResistorLowering,
    StatementLowering,
    ExpressionLowering,
    RegisterLowering,
}

impl DiagnosticKind {
    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::TypedIrUnsupported { .. } => DiagnosticCode::Unsupported,
            Self::InvalidIr { .. } => DiagnosticCode::Internal,
            Self::Parse(_) => DiagnosticCode::Parse,
            Self::DuplicateName { .. } => DiagnosticCode::Duplicate,
            Self::MissingInitializer => DiagnosticCode::Initializer,
            Self::ForBounds => DiagnosticCode::ForType,
            Self::ConditionType { .. } => DiagnosticCode::Condition,
            Self::UndefinedName { .. } => DiagnosticCode::Undefined,
            Self::UnaryType { .. } => DiagnosticCode::UnaryType,
            Self::BinaryType { .. } => DiagnosticCode::BinaryType,
            Self::BranchTypes { .. } => DiagnosticCode::TernaryType,
            Self::NumericTypes { .. } => DiagnosticCode::NumericType,
            Self::InexactInteger { .. } => DiagnosticCode::LiteralRange,
            Self::IntegerRange { .. } => DiagnosticCode::LiteralRange,
            Self::LossyConversion { .. } => DiagnosticCode::Type,
            Self::NegatedUnsigned => DiagnosticCode::Type,
            Self::FloatRange => DiagnosticCode::LiteralRange,
            Self::UnsupportedTarget { .. } => DiagnosticCode::Target,
            Self::InvalidPin { .. } => DiagnosticCode::Pin,
            Self::DuplicatePin => DiagnosticCode::PinDuplicate,
            Self::PullDown => DiagnosticCode::PinMode,
            Self::PullUpLowering => DiagnosticCode::Unsupported,
            Self::ConditionLowering => DiagnosticCode::Unsupported,
            Self::PinAssignmentLowering => DiagnosticCode::Unsupported,
            Self::StorageLowering => DiagnosticCode::Unsupported,
            Self::LoopLowering => DiagnosticCode::Unsupported,
            Self::PullResistorLowering => DiagnosticCode::Unsupported,
            Self::StatementLowering => DiagnosticCode::Unsupported,
            Self::ExpressionLowering => DiagnosticCode::Unsupported,
            Self::RegisterLowering => DiagnosticCode::Backend,
        }
    }
}

/// A diagnostic is data; rendering and process exit belong to the caller.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub kind: Box<DiagnosticKind>,
    pub location: Option<SourceLocation>,
}

impl Diagnostic {
    pub fn new(kind: DiagnosticKind, location: Option<SourceLocation>) -> Self {
        Self {
            kind: Box::new(kind),
            location,
        }
    }

    pub fn code(&self) -> DiagnosticCode {
        self.kind.code()
    }

    pub fn message(&self) -> String {
        self.kind.message()
    }

    pub fn render(&self, source: &str) -> String {
        self.render_with(source, RenderOptions::default())
    }

    pub fn render_with(&self, source: &str, options: RenderOptions) -> String {
        let label = if self.code() == DiagnosticCode::Internal {
            "internal compiler error"
        } else {
            "error"
        };
        let heading = paint(&format!("{label}[{}]", self.code()), "1;31", options.color);
        let mut result = format!("{heading}: {}", self.message());

        if let Some(span) = self.location {
            // split keeps a final empty line, so EOF errors can point after a newline.
            for (index, line) in source.split('\n').enumerate() {
                let number = index + 1;
                if number < span.line_start || number > span.line_end {
                    continue;
                }

                let line = line.strip_suffix('\r').unwrap_or(line);
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

                // Source offsets count graphemes; expand tabs consistently in both
                // the displayed line and the underline (four-column tab stops).
                let (line, columns) = expand_tabs(line);
                let start = columns[start];
                let end = columns[end];
                let width = span.line_end.to_string().len().max(4);
                let gutter = paint(&format!("{number:>width$} |"), "34", options.color);
                let marker_gutter = paint(&format!("{:width$} |", ""), "34", options.color);

                result.push_str(&format!(
                    "\n{gutter} {line}\n{marker_gutter} {}{}",
                    " ".repeat(start),
                    paint(
                        &"^".repeat(end.saturating_sub(start).max(1)),
                        "1;31",
                        options.color
                    )
                ));
            }
        }

        result
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {}", self.code(), self.message())
    }
}

impl std::error::Error for Diagnostic {}

/// The caller resolves terminal/environment policy; rendering has no global state.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    pub color: bool,
}

fn expand_tabs(line: &str) -> (String, Vec<usize>) {
    let mut text = String::new();
    let mut columns = vec![0];
    let mut column = 0;

    for grapheme in line.graphemes(true) {
        if grapheme == "\t" {
            let spaces = 4 - column % 4;
            text.push_str(&" ".repeat(spaces));
            column += spaces;
        } else {
            text.push_str(grapheme);
            column += 1;
        }

        columns.push(column);
    }

    (text, columns)
}

fn paint(text: &str, style: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{style}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}
