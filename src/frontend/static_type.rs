use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use lazy_static::lazy_static;

lazy_static! {
    pub static ref TYPES: HashMap<&'static str, StaticType> = {
        let mut m = HashMap::new();

        m.insert("i8",      StaticType::Integer(IntegerType::Signed(SignedIntegerType::Int8))); 
        m.insert("i16",     StaticType::Integer(IntegerType::Signed(SignedIntegerType::Int16)));
        m.insert("i32",     StaticType::Integer(IntegerType::Signed(SignedIntegerType::Int32)));
        m.insert("i64",     StaticType::Integer(IntegerType::Signed(SignedIntegerType::Int64)));
        m.insert("u8",      StaticType::Integer(IntegerType::Unsigned(UnsignedIntegerType::Uns8))); 
        m.insert("u16",     StaticType::Integer(IntegerType::Unsigned(UnsignedIntegerType::Uns16)));
        m.insert("u32",     StaticType::Integer(IntegerType::Unsigned(UnsignedIntegerType::Uns32)));
        m.insert("u64",     StaticType::Integer(IntegerType::Unsigned(UnsignedIntegerType::Uns64)));
        
        m.insert("f32",     StaticType::Float(FloatType::Float32));
        m.insert("f64",     StaticType::Float(FloatType::Float64));

        m.insert("PinState",    StaticType::PinState);
        m.insert("PinMode",     StaticType::PinMode);

        m.insert("bool",    StaticType::Boolean);
        m.insert("string",  StaticType::String);

        m
    };
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum StaticType {
    Integer(IntegerType),

    Float(FloatType),

    /// `LOW`, `HIGH`
    PinState,
    /// `INPUT`, `OUTPUT`, `INPUT_PULLUP`, `INPUT_PULLDOWN`
    PinMode,

    Boolean,
    String,

    Array(&'static StaticType),
    // TODO: Function would need argument types + return type
    Function,
    /// Actual pin, like PortB.2
    Pin,
    
    /// Internal type for expressions that are not yet resolved or failed to resolve.
    /// This is useful, for instance, in variable definitions.
    /// ```
    /// var a = b + 123;
    /// ```
    /// We will get an analyzer error that b is not defined, but it will still evaluate to "Unknown",
    /// so that "a" is defined. Other operations are expected to ignore this value and return "Unknown"
    /// when working with it 
    Unknown
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum IntegerType {
    Signed(SignedIntegerType),
    Unsigned(UnsignedIntegerType),
    /// Represents literal of yet unknown type. Will be converted to an appropriate `IntegerType`
    /// when analyzed by Semantic Analyzer
    Literal
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SignedIntegerType {
    Int8, Int16, Int32, Int64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UnsignedIntegerType {
    Uns8, Uns16, Uns32, Uns64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FloatType {
    Float32, Float64,
    /// Represents literal of yet unknown type. Will be converted to an appropriate `FloatType`
    /// when analyzed by Semantic Analyzer
    Literal
}

impl Display for StaticType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            StaticType::Integer(IntegerType::Signed(SignedIntegerType::Int8)) => write!(f, "i8"),
            StaticType::Integer(IntegerType::Signed(SignedIntegerType::Int16)) => write!(f, "i16"),
            StaticType::Integer(IntegerType::Signed(SignedIntegerType::Int32)) => write!(f, "i32"),
            StaticType::Integer(IntegerType::Signed(SignedIntegerType::Int64) )=> write!(f, "i64"),
            StaticType::Integer(IntegerType::Unsigned(UnsignedIntegerType::Uns8)) => write!(f, "u8"),
            StaticType::Integer(IntegerType::Unsigned(UnsignedIntegerType::Uns16)) => write!(f, "u16"),
            StaticType::Integer(IntegerType::Unsigned(UnsignedIntegerType::Uns32)) => write!(f, "u32"),
            StaticType::Integer(IntegerType::Unsigned(UnsignedIntegerType::Uns64)) => write!(f, "u64"),
            StaticType::Integer(IntegerType::Literal) => write!(f, "literal int"),
            StaticType::Float(FloatType::Float32) => write!(f, "f32"),
            StaticType::Float(FloatType::Float64) => write!(f, "f64"),
            StaticType::Float(FloatType::Literal) => write!(f, "literal float"),
            StaticType::PinState => write!(f, "PinState"),
            StaticType::PinMode => write!(f, "PinMode"),
            StaticType::Boolean => write!(f, "bool"),
            StaticType::String => write!(f, "string"),
            StaticType::Array(array_type) => write!(f, "Arr<{array_type}>"),
            StaticType::Function => write!(f, "Func"),
            StaticType::Pin => write!(f, "Pin"),
            StaticType::Unknown => write!(f, "<unknown>"),
        }
    }
}