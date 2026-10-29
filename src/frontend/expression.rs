use crate::frontend::static_type::StaticType;
use crate::frontend::token::Token;
use crate::frontend::token_type::{LiteralType, PunctuationType, TokenType};
use crate::frontend::utils::{LocatableInSource, SourceLocation};
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct Expression {
    pub symbol: Option<crate::semantic::SymbolId>,
    pub expression_type: ExpressionType,
    pub source_location: SourceLocation,
    // TODO: Comment this
    pub static_type: StaticType,
}

impl LocatableInSource for Expression {
    fn get_location(&self) -> &SourceLocation {
        &self.source_location
    }
}

#[derive(Debug, Clone)]
pub enum ExpressionType {
    Cast {
        target: StaticType,
        expression: Box<Expression>,
    },
    Deref(Box<Expression>),
    DerefAssignment {
        reference: Box<Expression>,
        value: Box<Expression>,
    },
    CapabilityCall {
        capability: crate::values::Capability,
        arguments: Vec<Expression>,
        implementation: Option<crate::values::Implementation>,
    },
    Array {
        elements: Vec<Expression>,
        repeat: Option<u16>,
    },
    Index {
        array: Box<Expression>,
        index: Box<Expression>,
    },
    IndexAssignment {
        array: Box<Expression>,
        index: Box<Expression>,
        value: Box<Expression>,
    },
    Borrow {
        expression: Box<Expression>,
        mutable: bool,
    },
    Text {
        operation: TextOperation,
        arguments: Vec<Expression>,
    },
    Call {
        identifier: String,
        arguments: Vec<Expression>,
    },
    Identifier(String),
    Binary {
        operator_location: SourceLocation,
        left: Box<Expression>,
        operator: TokenType,
        right: Box<Expression>,
    },
    Ternary {
        first: Box<Expression>,
        second: Box<Expression>,
        third: Box<Expression>,
    },
    // TODO: Pack that into tuple enum type
    Grouping {
        expression: Box<Expression>,
    },
    Literal {
        value: LiteralType,
    },
    Unary {
        operator_location: SourceLocation,
        operator: PunctuationType,
        right: Box<Expression>,
    },
    Assignment {
        identifier: String,
        expression: Box<Expression>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOperation {
    Len,
    ByteOr,
}

// TODO: Code inconsistency! These functions panic instead of returning a result like similar parts
// TODO: (like `PinRef`)
// I'm not sure of the solution as of right now
impl Expression {
    pub fn writable_place(&self) -> bool {
        use crate::frontend::static_type::StaticType as T;
        use ExpressionType as E;
        match &self.expression_type {
            E::Grouping { expression } => expression.writable_place(),
            E::Deref(r) => r.static_type.reference().is_some_and(|(_, m)| m),
            E::Index { array, .. } => !matches!(
                array.static_type,
                T::ArrayRef(_, _, false) | T::SliceRef(_, false)
            ),
            E::Identifier(_) => !matches!(
                self.static_type,
                T::Ref(_, false) | T::ArrayRef(_, _, false) | T::SliceRef(_, false)
            ),
            _ => false,
        }
    }
    /// Root storage identity for whole-object loans, including element projections.
    pub fn place_root(&self) -> Option<crate::semantic::SymbolId> {
        match &self.expression_type {
            ExpressionType::Identifier(_) => self.symbol,
            ExpressionType::Deref(e)
            | ExpressionType::Borrow { expression: e, .. }
            | ExpressionType::Grouping { expression: e } => e.place_root(),
            ExpressionType::Index { array, .. } => array.place_root(),
            _ => None,
        }
    }

    /// Visit immediate operands in source evaluation order (not recursively).
    pub fn for_each_child(&self, mut visit: impl FnMut(&Expression)) {
        match &self.expression_type {
            ExpressionType::Cast { expression: e, .. } | ExpressionType::Deref(e) => visit(e),
            ExpressionType::DerefAssignment { reference, value } => {
                visit(reference);
                visit(value);
            }
            ExpressionType::Array { elements, .. } => elements.iter().for_each(visit),
            ExpressionType::Index { array, index } => {
                visit(array);
                visit(index);
            }
            ExpressionType::IndexAssignment {
                array,
                index,
                value,
            } => {
                visit(array);
                visit(index);
                visit(value);
            }
            ExpressionType::Borrow { expression, .. } => visit(expression),
            ExpressionType::CapabilityCall { arguments, .. }
            | ExpressionType::Call { arguments, .. }
            | ExpressionType::Text { arguments, .. } => arguments.iter().for_each(visit),
            ExpressionType::Binary { left, right, .. } => {
                visit(left);
                visit(right);
            }
            ExpressionType::Ternary {
                first,
                second,
                third,
            } => {
                visit(first);
                visit(second);
                visit(third);
            }
            ExpressionType::Grouping { expression }
            | ExpressionType::Assignment { expression, .. } => visit(expression),
            ExpressionType::Unary { right, .. } => visit(right),
            ExpressionType::Identifier(_) | ExpressionType::Literal { .. } => {}
        }
    }

    /// Mutable counterpart for type finalization.
    pub fn for_each_child_mut(&mut self, mut visit: impl FnMut(&mut Expression)) {
        match &mut self.expression_type {
            ExpressionType::Cast { expression: e, .. } | ExpressionType::Deref(e) => visit(e),
            ExpressionType::DerefAssignment { reference, value } => {
                visit(reference);
                visit(value);
            }
            ExpressionType::Array { elements, .. } => elements.iter_mut().for_each(visit),
            ExpressionType::Index { array, index } => {
                visit(array);
                visit(index);
            }
            ExpressionType::IndexAssignment {
                array,
                index,
                value,
            } => {
                visit(array);
                visit(index);
                visit(value);
            }
            ExpressionType::Borrow { expression, .. } => visit(expression),
            ExpressionType::CapabilityCall { arguments, .. }
            | ExpressionType::Call { arguments, .. }
            | ExpressionType::Text { arguments, .. } => arguments.iter_mut().for_each(visit),
            ExpressionType::Binary { left, right, .. } => {
                visit(left);
                visit(right);
            }
            ExpressionType::Ternary {
                first,
                second,
                third,
            } => {
                visit(first);
                visit(second);
                visit(third);
            }
            ExpressionType::Grouping { expression }
            | ExpressionType::Assignment { expression, .. } => visit(expression),
            ExpressionType::Unary { right, .. } => visit(right),
            ExpressionType::Identifier(_) | ExpressionType::Literal { .. } => {}
        }
    }
}

impl Expression {
    pub fn new_assignment(identifier: &Expression, expression: Expression) -> Self {
        if let ExpressionType::Identifier(name) = &identifier.expression_type {
            Self {
                symbol: None,
                source_location: SourceLocation::new_from_start_end(
                    identifier.source_location,
                    expression.source_location,
                ),
                expression_type: ExpressionType::Assignment {
                    identifier: name.to_owned(),
                    expression: Box::new(expression),
                },
                static_type: StaticType::Unknown,
            }
        } else {
            unreachable!("Non-identifier token passed to new_assignment expression function");
        }
    }

    pub fn new_ternary(first: Expression, second: Expression, third: Expression) -> Self {
        Self {
            symbol: None,
            source_location: SourceLocation::new_from_start_end(
                first.source_location,
                third.source_location,
            ),
            expression_type: ExpressionType::Ternary {
                first: Box::new(first),
                second: Box::new(second),
                third: Box::new(third),
            },
            static_type: StaticType::Unknown,
        }
    }

    pub fn new_binary(left: Expression, operator: &Token, right: Expression) -> Self {
        Self {
            symbol: None,
            source_location: SourceLocation::new_from_start_end(
                left.source_location,
                right.source_location,
            ),
            expression_type: ExpressionType::Binary {
                operator_location: operator.source_location,
                left: Box::new(left),
                operator: operator.token_type.clone(),
                right: Box::new(right),
            },
            static_type: StaticType::Unknown,
        }
    }

    pub fn new_unary(operator: &Token, expression: Expression) -> Self {
        if let Token {
            token_type: TokenType::Punctuation(op),
            ..
        } = operator
        {
            Self {
                symbol: None,
                source_location: SourceLocation::new_from_start_end(
                    operator.source_location,
                    expression.source_location,
                ),
                expression_type: ExpressionType::Unary {
                    operator_location: operator.source_location,
                    operator: op.to_owned(),
                    right: Box::new(expression),
                },
                static_type: StaticType::Unknown,
            }
        } else {
            unreachable!("Non-punctuation token passed to new_unary expression function");
        }
    }

    pub fn new_literal(token: &Token) -> Self {
        if let Token {
            token_type: TokenType::Literal(literal_type),
            ..
        } = token
        {
            Self {
                symbol: None,
                source_location: token.source_location,
                expression_type: ExpressionType::Literal {
                    value: literal_type.clone(),
                },
                static_type: StaticType::Unknown,
            }
        } else {
            unreachable!("Non-literal token passed to new_literal expression function");
        }
    }

    pub fn new_literal_custom_type(token: &Token, literal_type: LiteralType) -> Self {
        Self {
            symbol: None,
            source_location: token.source_location,
            expression_type: ExpressionType::Literal {
                value: literal_type,
            },
            static_type: StaticType::Unknown,
        }
    }

    /// This should be used only during Static Analysis as this will create a "fake" expression
    /// that doesn't point to any real source
    pub fn new_literal_tokenless(literal_type: LiteralType) -> Self {
        Self {
            symbol: None,
            source_location: SourceLocation::default(),
            expression_type: ExpressionType::Literal {
                value: literal_type,
            },
            static_type: StaticType::Unknown,
        }
    }

    pub fn new_identifier(token: &Token) -> Self {
        if let Token {
            token_type: TokenType::Identifier(name),
            ..
        } = token
        {
            Self {
                symbol: None,
                source_location: token.source_location,
                expression_type: ExpressionType::Identifier(name.clone()),
                static_type: StaticType::Unknown,
            }
        } else {
            unreachable!("Non-identifier token passed to new_identifier expression function");
        }
    }
}

impl Display for Expression {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.static_type {
            StaticType::Unknown => write!(f, "{}", format!("{}", self.static_type).red())?,
            _ => write!(f, "{}", format!("<{}>", self.static_type).yellow())?,
        }

        match &self.expression_type {
            ExpressionType::Cast { target, expression } => write!(f, "({target}){expression}"),
            ExpressionType::Deref(e) => write!(f, "*({e})"),
            ExpressionType::DerefAssignment { reference, value } => {
                write!(f, "*({reference}) = {value}")
            }
            ExpressionType::Array { elements, repeat } => write!(
                f,
                "[{}{}]",
                elements
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                repeat.map_or(String::new(), |n| format!("; {n}"))
            ),
            ExpressionType::Index { array, index } => write!(f, "{array}[{index}]"),
            ExpressionType::IndexAssignment {
                array,
                index,
                value,
            } => write!(f, "{array}[{index}] = {value}"),
            ExpressionType::Borrow {
                expression,
                mutable,
            } => write!(f, "&{}{expression}", if *mutable { "mut " } else { "" }),
            ExpressionType::Call {
                identifier,
                arguments,
            } => {
                write!(f, "{identifier}(")?;
                for (index, argument) in arguments.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{argument}")?;
                }
                write!(f, ")")
            }
            ExpressionType::Binary {
                left,
                operator,
                right,
                ..
            } => write!(f, "{left} {} {right}", format!("{operator:?}").green()),
            ExpressionType::Ternary {
                first,
                second,
                third,
            } => write!(f, "{first} ? {second} : {third}"),
            ExpressionType::Grouping { expression } => write!(f, "({expression})"),
            ExpressionType::Literal { value } => write!(f, "{value:?}"),
            ExpressionType::Unary {
                operator, right, ..
            } => write!(f, "({operator:?} {right})"),
            ExpressionType::Assignment {
                identifier,
                expression,
            } => write!(f, "{identifier} = {expression}"),
            ExpressionType::CapabilityCall {
                capability,
                arguments,
                ..
            } => write!(f, "{capability:?}({arguments:?})"),
            ExpressionType::Text {
                operation,
                arguments,
            } => write!(f, "{operation:?}({arguments:?})"),
            ExpressionType::Identifier(name) => write!(f, "{name}"),
        }
    }
}

/// A reference to a pin
/// Like `PortB.2`
#[derive(Debug, Clone)]
pub struct PinRef {
    pub port: String,
    pub pin_number: usize,
}

impl Display for PinRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Port{}{}", self.port, self.pin_number)
    }
}

impl PinRef {
    pub fn new(port: Token, pin_number: Token) -> Result<PinRef, PinRefError> {
        if let TokenType::Port(port_name) = port.token_type {
            if let TokenType::Literal(LiteralType::Integer(pin_int)) = pin_number.token_type {
                let pin_number_convert = usize::try_from(pin_int).map_err(|_| PinRefError {
                    token: pin_number,
                    message: "Invalid number for a pin (maybe it's too large?)",
                });

                Ok(PinRef {
                    port: port_name,
                    pin_number: pin_number_convert?,
                })
            } else {
                Err(PinRefError {
                    token: pin_number,
                    message: "Expected integer as pin number",
                })
            }
        } else {
            Err(PinRefError {
                token: port,
                message: "Incorrect port token",
            })
        }
    }
}

#[derive(Debug, Clone)]
pub struct PinRefError {
    pub token: Token,
    message: &'static str,
}

impl Error for PinRefError {}
impl Display for PinRefError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
