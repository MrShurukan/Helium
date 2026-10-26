//! English presentation. Future locales match the same structured diagnostic kinds.
use super::DiagnosticKind;

impl DiagnosticKind {
    pub fn message(&self) -> String {
        match self {
            Self::NotCallable { name } => format!("'{name}' is not a function"),
            Self::ArgumentCount {
                name,
                expected,
                actual,
            } => {
                format!("Function '{name}' expects {expected} arguments, but received {actual}")
            }
            Self::ProcedureValue { name } => {
                format!("Function '{name}' has no return value; call it as a statement")
            }
            Self::FunctionValue { name } => {
                format!("Function '{name}' cannot be used as a value; call it with parentheses")
            }
            Self::GlobalInitializerCall => {
                "Function calls are not allowed in global initializers; move the call to setup"
                    .into()
            }
            Self::ReturnOutsideFunction => "'return' is only allowed inside a function".into(),
            Self::MissingReturn { name } => {
                format!(
                    "Function '{name}' must return a value on every path; add an explicit return"
                )
            }
            Self::ReturnValueRequired { target } => {
                format!("This function must return a value of type {target}")
            }
            Self::UnexpectedReturnValue => {
                "This function has no return type; use 'return;' without a value".into()
            }
            Self::RecursiveCalls { chain } => {
                format!("Recursion is not supported: {}", chain.join(" -> "))
            }
            Self::FunctionSignatureType { ty } => {
                format!(
                    "Function signatures currently support integers, bool and PinState, not {ty}"
                )
            }
            Self::UnsupportedType { ty, machine } => {
                if *machine {
                    format!("Machine code for {ty} is not implemented yet; supported integer widths are 8 and 16 bits")
                } else {
                    format!("Values of type {ty} are not implemented yet")
                }
            }
            Self::TypedIrUnsupported { feature } => {
                format!("Not implemented: {feature}")
            }
            Self::InvalidIr { stage, detail } => {
                format!("IR validation failed after {stage}: {detail}")
            }
            Self::Parse(error) => error.to_string(),
            Self::DuplicateName { name } => {
                format!("'{name}' is already defined; shadowing is not allowed")
            }
            Self::MissingInitializer => {
                "Provide a type with a default value or an initializer".into()
            }
            Self::ForBounds => "For bounds must be integers".into(),
            Self::ConditionType { ty } => format!("Expected bool condition, found {ty}"),
            Self::UndefinedName { name } => format!("'{name}' is undefined in this scope"),
            Self::UnaryType { operator, operand } => {
                format!("Unary {operator} cannot be used with {operand}")
            }
            Self::BinaryType { operator, a, b } => {
                format!("Operator {operator:?} does not accept {a} and {b}")
            }
            Self::BranchTypes { a, b } => format!("Branches have incompatible types {a} and {b}"),
            Self::NumericTypes { a, b } => format!("No lossless common type for {a} and {b}"),
            Self::InexactInteger { value, target } => {
                format!("Integer literal {value} cannot be represented exactly as {target}")
            }
            Self::IntegerRange {
                value,
                target,
                min,
                max,
            } => {
                format!("Integer literal {value} is outside {target} range {min}..={max}")
            }
            Self::LossyConversion { from, target } => {
                format!("Cannot implicitly convert {from} to {target} without loss")
            }
            Self::NegatedUnsigned => {
                "A negated expression cannot implicitly become unsigned".into()
            }
            Self::FloatRange => "Floating-point literal is outside f32 range".into(),

            Self::UnsupportedTarget { mcu } => {
                format!("Unsupported MCU '{mcu}'; supported: atmega328p")
            }
            Self::InvalidPin { port, number } => {
                format!("Port{port}.{number} is not a digital pin on ATmega328P")
            }
            Self::DuplicatePin => "The same physical pin is declared more than once".into(),
            Self::PullDown => "ATmega328P has no internal input pull-down resistor".into(),
        }
    }
}

impl super::LabelKind {
    pub fn message(&self) -> String {
        match self {
            Self::OpeningDelimiter => "block opened here".into(),
            Self::PreviousDeclaration => "first declared here".into(),
            Self::ExpectedType(ty) => format!("expected type {ty} comes from here"),
            Self::OperandType(ty) => format!("this expression has type {ty}"),
            Self::FunctionDeclaration => "function declared here".into(),
            Self::Fallthrough => "this path reaches the end without returning a value".into(),
            Self::FalseBranch => {
                "when this condition is false, execution can reach the end without a return".into()
            }
            Self::LoopMayNotRun => {
                "this loop may execute zero times; a return inside it is not sufficient".into()
            }
            Self::RecursiveCall => "this call participates in the recursion cycle".into(),
        }
    }
}

impl DiagnosticKind {
    /// Advice is presentation, not an automatic source edit.
    pub fn help(&self) -> Option<&'static str> {
        match self {
            Self::MissingReturn { .. } => Some("Return a value on each highlighted path, or add a common return after the conditional."),
            Self::UnsupportedType { machine: true, .. } => Some("Use an explicit 8/16-bit type only if it can represent all required values; wider machine integers are planned separately."),
            Self::GlobalInitializerCall => Some("Initialize the variable in setup. Startup ordering across libraries is not defined yet; the compiler does not move calls automatically."),
            Self::PullDown => Some("Use an external pull-down resistor, or INPUT_PULLUP with inverted input logic."),
            _ => None,
        }
    }
}
