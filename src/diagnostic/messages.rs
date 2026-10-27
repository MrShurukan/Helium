//! English presentation. Future locales match the same structured diagnostic kinds.
use super::DiagnosticKind;

impl crate::timer::TimerError {
    pub fn message(&self) -> String {
        match self {
            Self::InvalidClock { hz, target, maximum } => format!("CPU frequency {hz} Hz is outside the supported {target} range 1..={maximum} Hz; this does not configure the oscillator or fuses"),
            Self::ZeroPeriod => "Timer period must be greater than zero".into(),
            Self::MissingClock => "Timer declarations require an explicit --cpu-hz (for example --cpu-hz 16000000); the compiler does not change oscillator settings".into(),
            Self::Unsupported { hardware, supported } => format!("Unsupported timer '{hardware}'; this target currently supports periodic {supported} only"),
            Self::AlreadyReserved { hardware } => format!("{hardware} is already reserved by another declaration"),
            Self::NotValue { name } => format!("Timer '{name}' names a hardware resource, not a runtime value; it cannot be copied or assigned"),
            Self::Unrepresentable { cpu_hz, requested_ns, tolerance_ns, shorter, longer, direction, minimum_tolerance_ns } => {
                let show = |c: &Option<Box<crate::timer::TimerConfig>>| c.as_deref().map(crate::timer::TimerConfig::report).unwrap_or_else(|| "not available".into());
                let requirement = match minimum_tolerance_ns {
                    Some(minimum) => format!("Nearest configuration in the allowed direction needs tolerance of at least {minimum}ns."),
                    None => "No representable period exists in the allowed direction; increasing tolerance cannot help.".into(),
                };
                format!("Cannot represent every: {requested_ns}ns within tolerance: {}{tolerance_ns}ns at CPU={cpu_hz} Hz.\nNearest shorter: {}\nNearest longer: {}\n{requirement} Use helium timer-options to inspect alternatives.", direction.sign(), show(shorter), show(longer))
            }
        }
    }
}

impl DiagnosticKind {
    pub fn message(&self) -> String {
        match self {
            Self::Device { detail } => detail.clone(),
            Self::Module { detail } => detail.clone(),
            Self::IrqRequirement { name, reason } => format!("Call to '{name}' requires enabled interrupts: {}", reason.message()),
            Self::IsrBlocking { name, reason } => format!("Call to '{name}' may block inside an interrupt handler: {}. Waiting can cause long delays in main code and other interrupt handlers", reason.message()),
            Self::InterruptContext { name } => {
                format!("Function '{name}' requires interrupts to be disabled")
            }
            Self::Timer { error } => error.message(),
            Self::SharedAccess { name, reason } => {
                use super::SharedAccessReason as R;
                let detail = match reason {
                    R::Device => "shared device operations require a protected transaction",
                    R::SharedMemory => "shared memory operations, including read-modify-write, require a protected transaction",
                    R::MultiByteMemory => "multi-byte memory access can be interrupted between bytes",
                    R::SamePin => "both contexts use the same pin; atomic bit writes do not protect a complete read/compute/write operation",
                    R::PinConfiguration => "pin configuration changes direction and/or pull-up state and conflicts with use of the same pin",
                };
                format!("Access to '{name}' conflicts with an interrupt handler and requires noInterrupt protection: {detail}")
            }
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
                    "Function signatures support integers, bool and PinState; parameters also support &str, not {ty}"
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
            Self::MainAccess => "accessed by the main loop or a function it calls".into(),
            Self::MainCall => "this call is reachable from the main loop".into(),
            Self::InterruptCall => "this call is reachable from the interrupt handler".into(),
            Self::EffectCall => "effect propagated through this call".into(),
            Self::InterruptAccess => {
                "accessed by the interrupt handler or a function it calls".into()
            }
            Self::InterruptRequirement => "interrupt requirement declared here".into(),
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
            Self::InterruptContext { .. } => Some("Wrap the call in noInterrupt { ... }, or declare the caller as noInterrupt fn to require protection from its callers."),
            Self::SharedAccess { .. } => Some("Use noInterrupt { ... } for the complete operation, or declare the helper as noInterrupt fn. Protect a multi-field snapshot in one block."),
            Self::GlobalInitializerCall => Some("Initialize the variable in setup. Startup ordering across libraries is not defined yet; the compiler does not move calls automatically."),
            Self::PullDown => Some("Use an external pull-down resistor, or INPUT_PULLUP with inverted input logic."),
            _ => None,
        }
    }
}
