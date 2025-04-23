use crate::frontend::expression::{Expression, ExpressionType};
use crate::frontend::static_type::StaticType;
use crate::frontend::token_type::{LiteralType, PinModeConstantType, PinStateConstantType, PunctuationType, TokenType};
use std::collections::HashSet;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

pub struct DagBuilder {
    table: HashSet<Rc<DagNode>>,
}

impl DagBuilder {
    pub fn new() -> Self {
        Self { table: HashSet::new() }
    }

    pub fn process_expression(&mut self, expression: &Expression) -> Result<Rc<DagNode>, DagError> {
        match &expression.expression_type {
            ExpressionType::Literal { value } => {
                match value {
                    LiteralType::Integer(int) => {
                        Ok(self.get_or_insert(DagNode { dag_type: DagType::IntegerValue(*int) }))
                    },
                    LiteralType::PinState(pin_state) => {
                        Ok(self.get_or_insert(DagNode { dag_type: DagType::PinStateValue(pin_state.clone()) }))
                    },
                    LiteralType::PinMode(pin_mode) => {
                        Ok(self.get_or_insert(DagNode { dag_type: DagType::PinModeValue(pin_mode.clone()) }))
                    },
                    LiteralType::Boolean(bool) => {
                        Ok(self.get_or_insert(DagNode { dag_type: DagType::BooleanValue(*bool) }))
                    }
                    _ => Err(DagError::CannotProcessLiteral(expression.static_type))
                }
            },
            ExpressionType::Binary { left, operator, right } => {
                match operator {
                    TokenType::Punctuation(PunctuationType::Plus) => {
                        match expression.static_type {
                            StaticType::Integer(int_type) => {
                                // TODO: Casts if the types don't match the outer expression type
                                let left = self.process_expression(left)?;
                                let right = self.process_expression(right)?;

                                Ok(self.get_or_insert(DagNode { dag_type:
                                    DagType::IntegerAddition(left, right) }))
                            },
                            _ => Err(DagError::UnknownAdditionType(expression.static_type))
                        }
                    },
                    TokenType::Punctuation(PunctuationType::EqualEqual) => {
                        // TODO: This is a hack just to check comparison working right now
                        // I say this block should be rewritten from scratch
                        match (left.static_type, right.static_type) {
                            (StaticType::Pin, StaticType::PinState) => {
                                let right = self.process_expression(right)?;
                                Ok(self.get_or_insert(DagNode { dag_type:
                                    DagType::PinStateComparison("Button".to_string(), right) }))
                            },
                            _ => Err(DagError::CannotProcessComparisonOfTypes(left.static_type, right.static_type))
                        }
                    },
                    _ => Err(DagError::UnknownBinaryOperator(operator.clone()))
                }
            },
            ExpressionType::Grouping { expression } => {
                self.process_expression(expression)
            },
            ExpressionType::Identifier(name) => {
                Ok(self.get_or_insert(DagNode { dag_type: DagType::Identifier(name.clone()) }))
            },
            ExpressionType::Assignment { identifier, expression } => {
                let expr_dag = self.process_expression(expression)?;
                // PinState and PinMode assignments could be accomplished both
                // to variables and to Pins. But they are still marked as a separate node
                // to simplify IR generation
                let dag_type = match expression.static_type {
                    StaticType::PinState => DagType::AssignmentPinState(identifier.clone(), expr_dag),
                    StaticType::PinMode => DagType::AssignmentPinMode(identifier.clone(), expr_dag),
                    StaticType::Integer(_) => DagType::AssignmentInteger(identifier.clone(), expr_dag),
                    _ => return Err(DagError::UnknownAssignmentType(expression.static_type)),
                };

                Ok(self.get_or_insert(DagNode { dag_type }))
            },
            _ => Err(DagError::UnknownExpressionType(expression.expression_type.clone()))
        }
    }

    pub fn get_or_insert(&mut self, node: DagNode) -> Rc<DagNode> {
        let node_rc = Rc::new(node);
        if let Some(existing) = self.table.get(&node_rc) {
            // Return existing node if found
            existing.clone()
        } else {
            self.table.insert(node_rc.clone());
            node_rc
        }
    }

    pub fn print_table(&self) {
        for (i, value) in self.table.iter().enumerate() {
            println!("{i}. {value:?}");
        }
    }
}

/// Directed Acyclic Graph - used for optimizing the AST (specifically the expressions)
/// DAG is not great at working out code flow, so it's used to just optimized expressions.
/// This bundled with AST will be converted to Linear IR later
#[derive(Debug, Clone, Eq)]
pub struct DagNode {
    pub dag_type: DagType,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum DagType {
    Identifier(String),
    IntegerValue(u64),
    BooleanValue(bool),
    PinModeValue(PinModeConstantType),
    PinStateValue(PinStateConstantType),
    IntegerAddition(Rc<DagNode>, Rc<DagNode>),
    AssignmentInteger(String, Rc<DagNode>),
    AssignmentPinMode(String, Rc<DagNode>),
    AssignmentPinState(String, Rc<DagNode>),
    PinStateComparison(String, Rc<DagNode>)
}

impl PartialEq for DagNode {
    fn eq(&self, other: &DagNode) -> bool {
        self.dag_type == other.dag_type
    }
}

impl Hash for DagNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dag_type.hash(state);
    }
}

#[derive(Debug)]
pub enum DagError {
    CannotProcessLiteral(StaticType),
    UnknownAdditionType(StaticType),
    UnknownBinaryOperator(TokenType),
    UnknownExpressionType(ExpressionType),
    CannotProcessComparisonOfTypes(StaticType, StaticType),
    UnknownAssignmentType(StaticType)
}

impl Error for DagError {}
impl Display for DagError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            DagError::CannotProcessLiteral(static_type) =>
                write!(f, "Dag is not ready to accept literal type of {static_type}"),
            DagError::UnknownAdditionType(static_type) =>
                write!(f, "Dag is not ready to accept addition of type {static_type}"),
            DagError::UnknownBinaryOperator(token_type) =>
                write!(f, "Dag is not ready to accept binary operator {token_type:?}"),
            DagError::UnknownExpressionType(expression_type) =>
                write!(f, "Dag is not ready to accept {expression_type:?}"),
            DagError::CannotProcessComparisonOfTypes(left, right) =>
                write!(f, "Dag is not ready to process '{left}' == '{right}'"),
            DagError::UnknownAssignmentType(static_type) =>
                write!(f, "Dag is not ready to accept assigment type of {static_type}"),
        }
    }
}