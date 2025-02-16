use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use lazy_static::lazy_static;
use crate::frontend::statement::StatementType;

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

    pub static ref TYPES: HashMap<&'static str, StaticType> = {
        let mut m = HashMap::new();

        m.insert("i8",      StaticType::Int8);
        m.insert("i16",     StaticType::Int16);
        m.insert("i32",     StaticType::Int32);
        m.insert("i64",     StaticType::Int64);
        m.insert("u8",      StaticType::Uns8);
        m.insert("u16",     StaticType::Uns16);
        m.insert("u32",     StaticType::Uns32);
        m.insert("u64",     StaticType::Uns64);

        m.insert("f32",     StaticType::Float32);
        m.insert("f64",     StaticType::Float64);

        m.insert("PinState",    StaticType::PinState);
        m.insert("PinMode",     StaticType::PinMode);

        m.insert("bool",    StaticType::Boolean);
        m.insert("string",  StaticType::String);

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

#[derive(Debug, Clone, Copy)]
pub enum StaticType {
    Int8, Int16, Int32, Int64,
    Uns8, Uns16, Uns32, Uns64,

    Float32, Float64,

    // LOW, HIGH
    PinState,
    // INPUT, OUTPUT, INPUT_PULLUP, INPUT_PULLDOWN
    PinMode,

    Boolean,
    String,

    Array(&'static StaticType),
    // TODO: Function would need argument types + return type
    Function
}

impl Display for StaticType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            StaticType::Int8 => write!(f, "i8"),
            StaticType::Int16 => write!(f, "i16"),
            StaticType::Int32 => write!(f, "i32"),
            StaticType::Int64 => write!(f, "i64"),
            StaticType::Uns8 => write!(f, "u8"),
            StaticType::Uns16 => write!(f, "u16"),
            StaticType::Uns32 => write!(f, "u32"), 
            StaticType::Uns64 => write!(f, "u64"),
            StaticType::Float32 => write!(f, "f32"),
            StaticType::Float64 => write!(f, "f64"),
            StaticType::PinState => write!(f, "PinState"),
            StaticType::PinMode => write!(f, "PinMode"),
            StaticType::Boolean => write!(f, "bool"),
            StaticType::String => write!(f, "string"),
            StaticType::Array(array_type) => write!(f, "Arr<{array_type}>"),
            StaticType::Function => write!(f, "func"),
        }
    }
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