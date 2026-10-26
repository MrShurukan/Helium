mod messages;
pub mod source;

use crate::frontend::{
    static_type::StaticType,
    token_type::{PunctuationType, TokenType},
    utils::SourceLocation,
    ParserErrorType,
};
use std::fmt;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Stable public identifiers, independent of wording and presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCode {
    Call,
    Return,
    Recursion,
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
    Parse,
    Internal,
}

impl DiagnosticCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Call => "E_CALL",
            Self::Return => "E_RETURN",
            Self::Recursion => "E_RECURSION",
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
    NotCallable {
        name: String,
    },
    ArgumentCount {
        name: String,
        expected: usize,
        actual: usize,
    },
    ProcedureValue {
        name: String,
    },
    FunctionValue {
        name: String,
    },
    GlobalInitializerCall,
    ReturnOutsideFunction,
    MissingReturn {
        name: String,
    },
    ReturnValueRequired {
        target: StaticType,
    },
    UnexpectedReturnValue,
    RecursiveCalls {
        chain: Vec<String>,
    },
    FunctionSignatureType {
        ty: StaticType,
    },
    UnsupportedType {
        ty: StaticType,
        machine: bool,
    },
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
}

impl DiagnosticKind {
    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::NotCallable { .. }
            | Self::ArgumentCount { .. }
            | Self::ProcedureValue { .. }
            | Self::FunctionValue { .. }
            | Self::GlobalInitializerCall => DiagnosticCode::Call,
            Self::ReturnOutsideFunction
            | Self::MissingReturn { .. }
            | Self::ReturnValueRequired { .. }
            | Self::UnexpectedReturnValue => DiagnosticCode::Return,
            Self::RecursiveCalls { .. } => DiagnosticCode::Recursion,
            Self::FunctionSignatureType { .. } => DiagnosticCode::Unsupported,
            Self::TypedIrUnsupported { .. } | Self::UnsupportedType { .. } => {
                DiagnosticCode::Unsupported
            }
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
        }
    }
}

/// Semantic labels stay independent of wording and terminal/editor rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelKind {
    OpeningDelimiter,
    PreviousDeclaration,
    ExpectedType(StaticType),
    OperandType(StaticType),
    FunctionDeclaration,
    Fallthrough,
    FalseBranch,
    LoopMayNotRun,
    RecursiveCall,
}

#[derive(Debug, Clone)]
pub struct Label {
    pub location: SourceLocation,
    pub kind: LabelKind,
}

/// A diagnostic is data; rendering and process exit belong to the caller.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub kind: Box<DiagnosticKind>,
    pub location: Option<SourceLocation>,
    pub labels: Vec<Label>,
}

impl Diagnostic {
    pub fn new(kind: DiagnosticKind, location: Option<SourceLocation>) -> Self {
        Self {
            kind: Box::new(kind),
            location,
            labels: Vec::new(),
        }
    }

    pub fn with_label(mut self, location: SourceLocation, kind: LabelKind) -> Self {
        self.labels.push(Label { location, kind });
        self
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

    pub fn render_in(&self, source: &source::SourceFile<'_>, options: RenderOptions) -> String {
        let position = self
            .location
            .map(|s| format!("{}:{}:{}", source.name, s.line_start, s.start_offset + 1))
            .unwrap_or_else(|| source.name.to_owned());
        format!("--> {position}\n{}", self.render_with(source.text, options))
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
            render_span(&mut result, source, span, options);
        }

        for label in &self.labels {
            result.push_str(&format!("\n  = {}", label.kind.message()));
            if Some(label.location) != self.location {
                render_span(&mut result, source, label.location, options);
            }
        }

        if let Some(help) = self.kind.help() {
            result.push_str(&format!("\n  help: {help}"));
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

fn render_span(result: &mut String, source: &str, span: SourceLocation, options: RenderOptions) {
    // split keeps a final empty line, so EOF errors can point after a newline.
    for (index, line) in source.split('\n').enumerate() {
        let number = index + 1;
        if number < span.line_start || number > span.line_end {
            continue;
        }

        // Bound excerpts while retaining both ends of a multiline range.
        if span.line_end.saturating_sub(span.line_start) > 4
            && number > span.line_start + 1
            && number < span.line_end - 1
        {
            if number == span.line_start + 2 {
                result.push_str("\n     | ...");
            }
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
            column += UnicodeWidthStr::width(grapheme);
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
