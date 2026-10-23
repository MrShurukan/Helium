//! English presentation. Future locales match the same structured diagnostic kinds.
use super::DiagnosticKind;

impl DiagnosticKind {
    pub fn message(&self) -> String {
        match self {
            Self::LlvmUnsupported { detail } => format!("LLVM backend: {detail}"),
            Self::TypedIrUnsupported { feature } => {
                format!("Typed IR v1 does not support {feature}")
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

            Self::PullUpLowering => "INPUT_PULLUP code generation is not implemented yet".into(),
            Self::ConditionLowering => {
                "AVR conditions currently require a pin == HIGH/LOW comparison; \
                 boolean operators (short-circuit), ! and other conditions are not lowered yet"
                    .into()
            }
            Self::PinAssignmentLowering => {
                "AVR currently supports pin assignments of constant HIGH/LOW or INPUT/OUTPUT; \
                 !Pin is typed but its single-read inversion is not lowered yet"
                    .into()
            }
            Self::StorageLowering => "Variable storage is not implemented by the AVR backend yet; \
                 use --emit ir to inspect typed IR"
                .into(),
            Self::LoopLowering => "Loop code generation is not implemented yet \
                 (the top-level loop block is supported)"
                .into(),
            Self::PullResistorLowering => "Pull resistor configuration is not implemented".into(),
            Self::StatementLowering => {
                "This statement is not supported by the legacy IR yet".into()
            }
            Self::ExpressionLowering => {
                "This expression is not supported by the legacy IR yet".into()
            }
            Self::RegisterLowering => "Instruction requires general register/storage lowering, \
                 which is not implemented yet"
                .into(),
        }
    }
}
