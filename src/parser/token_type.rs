use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use lazy_static::lazy_static;

#[derive(Debug, Clone)]
pub enum TokenType {
    Literal(LiteralType),
    // static lifetime, as we will read from KEYWORDS and CONSTANTS HashMaps
    // see those hashmaps below in a lazy_static! block
    Keyword(&'static KeywordType),
    Constant(&'static ConstantType),
    Punctuation(PunctuationType),

    EOF
}

lazy_static! {
    pub static ref KEYWORDS: HashMap<&'static str, KeywordType> = {
        let mut m = HashMap::new();

        m.insert("and",     KeywordType::And);
        m.insert("else",    KeywordType::Else);
        m.insert("fun",     KeywordType::Fun);
        m.insert("for",     KeywordType::For);
        m.insert("if",      KeywordType::If);
        m.insert("or",      KeywordType::Or);
        m.insert("return",  KeywordType::Return);
        m.insert("var",     KeywordType::Var);
        m.insert("while",   KeywordType::While);

        m
    };

    pub static ref CONSTANTS: HashMap<&'static str, ConstantType> = {
        let mut m = HashMap::new();

        m.insert("false",   ConstantType::False);
        m.insert("true",    ConstantType::True);

        m
    };
}


#[derive(Debug, Clone)]
pub enum LiteralType {
    Identifier(String),
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
}

#[derive(Debug)]
pub enum KeywordType {
    And, Else, Fun, For, If, Or,
    Return, Var, While
}

#[derive(Debug, Clone)]
pub enum ConstantType {
    True, False
}

#[derive(Debug, Clone)]
pub enum PunctuationType {
    // Single-character tokens.
    LeftParen, RightParen, LeftBrace, RightBrace,
    Comma, Dot, Minus, Plus, Semicolon, Slash, Star,
    Question, Colon,

    // One or two character tokens.
    Bang, BangEqual,
    Equal, EqualEqual,
    Greater, GreaterEqual,
    Less, LessEqual,
}

impl Display for PunctuationType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            PunctuationType::LeftParen => "(",
            PunctuationType::RightParen => ")",
            PunctuationType::LeftBrace => "{",
            PunctuationType::RightBrace => "}",
            PunctuationType::Comma => ",",
            PunctuationType::Dot => ".",
            PunctuationType::Minus => "-",
            PunctuationType::Plus => "+",
            PunctuationType::Semicolon => ";",
            PunctuationType::Slash => "/",
            PunctuationType::Star => "*",
            PunctuationType::Question => "?",
            PunctuationType::Colon => ":",
            PunctuationType::Bang => "!",
            PunctuationType::BangEqual => "!=",
            PunctuationType::Equal => "=",
            PunctuationType::EqualEqual => "==",
            PunctuationType::Greater => ">",
            PunctuationType::GreaterEqual => ">=",
            PunctuationType::Less => "<",
            PunctuationType::LessEqual => "<=",
        };

        write!(f, "{}", str)?;
        Ok(())
    }
}