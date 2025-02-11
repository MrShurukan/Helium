use crate::parser::token_type::{LiteralType, PunctuationType, TokenType};

#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct Expression {
    pub expression_type: ExpressionType,
    pub line_start: usize,
    pub line_end: usize,
    /// Offset in grapheme count from the start of the first line
    pub start_offset: usize,
    /// Offset in grapheme count from the start of the last file
    pub end_offset: usize
}

#[derive(Debug, Clone)]
pub enum ExpressionType {
    Binary { left: Box<Expression>, operator: TokenType, right: Box<Expression> },
    Ternary { first: Box<Expression>, second: Box<Expression>, third: Box<Expression> },
    Grouping { expression: Box<Expression> },
    Literal { value: LiteralType },
    Unary { operator: PunctuationType, right: Box<Expression> },
    Assignment { identifier: String, expression: Box<Expression> }
}