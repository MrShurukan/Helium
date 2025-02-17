use lazy_static::lazy_static;
use std::collections::HashMap;
use std::fmt::{write, Display, Formatter};
use crate::frontend::static_type::StaticType;

#[derive(Debug, Clone)]
pub enum TokenType {
    Literal(LiteralType),
    Port(String),
    // static lifetime, as we will read from KEYWORDS and CONSTANTS HashMaps
    // see those hashmaps below in a lazy_static! block
    Keyword(&'static KeywordType),
    Constant(&'static ConstantType),
    Type(&'static StaticType),
    Punctuation(PunctuationType),

    Eof
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
        m.insert("def",     KeywordType::Def);
        m.insert("while",   KeywordType::While);
        m.insert("alias",   KeywordType::Alias);
        m.insert("setup",   KeywordType::Setup);
        m.insert("loop",    KeywordType::Loop);
        m.insert("not",     KeywordType::Not);
        m.insert("to",      KeywordType::To);

        m
    };

    pub static ref CONSTANTS: HashMap<&'static str, ConstantType> = {
        let mut m = HashMap::new();

        m.insert("false",           ConstantType::Boolean(BooleanConstantType::False));
        m.insert("true",            ConstantType::Boolean(BooleanConstantType::True));
        m.insert("LOW",             ConstantType::PinState(PinStateConstantType::Low));
        m.insert("HIGH",            ConstantType::PinState(PinStateConstantType::High));
        m.insert("INPUT",           ConstantType::PinMode(PinModeConstantType::Input));
        m.insert("INPUT_PULLUP",    ConstantType::PinMode(PinModeConstantType::InputPullUp));
        m.insert("INPUT_PULLDOWN",  ConstantType::PinMode(PinModeConstantType::InputPullDown));
        m.insert("OUTPUT",          ConstantType::PinMode(PinModeConstantType::Output));

        m
    };
}


#[derive(Debug, Clone)]
pub enum LiteralType {
    Identifier(String),
    String(String),
    // You can't read negative integers, this would be read as unary minus + positive integer
    Integer(u64),
    Float(f64),
    Boolean(bool),
    PinMode(PinModeConstantType),
    PinState(PinStateConstantType)
}

#[derive(Debug)]
pub enum KeywordType {
    And, Else, Fun, For, If, Or,
    Return, Var, Def, While, Alias, 
    Setup, Loop, Not, To
}

#[derive(Debug, Clone)]
pub enum ConstantType {
    Boolean(BooleanConstantType),
    PinState(PinStateConstantType),
    PinMode(PinModeConstantType)
}

#[derive(Debug, Clone)]
pub enum BooleanConstantType {
    True, False,
}

#[derive(Debug, Clone, Copy)]
pub enum PinStateConstantType {
    Low, High,
}

#[derive(Debug, Clone, Copy)]
pub enum PinModeConstantType {
    Input, InputPullDown, InputPullUp,
    Output
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
            PunctuationType::LessEqual => "<="
        };

        write!(f, "{str}")?;
        Ok(())
    }
}