use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use crate::frontend::expression::{Expression, ExpressionType};
use crate::frontend::static_type::StaticType;
use crate::frontend::token_type::{LiteralType, PunctuationType, TokenType};

pub struct DagBuilder {
    table: HashSet<Rc<DagNode>>,
}

impl DagBuilder {
    pub fn new() -> Self {
        Self { table: HashSet::new() }
    }
    
    pub fn process_expression(&mut self, expression: &Expression) -> Rc<DagNode> {
        match &expression.expression_type {
            ExpressionType::Literal { value } => {
                match value {
                    LiteralType::Integer(int) => {
                        self.get_or_insert(DagNode { dag_type: DagType::IntegerValue(*int) })
                    },
                    _ => panic!("Dag is not ready to accept literal type of {}", expression.static_type)
                }
            },
            ExpressionType::Binary { left, operator, right } => {
                match operator {
                    TokenType::Punctuation(PunctuationType::Plus) => {
                        match expression.static_type {
                            StaticType::Integer(int_type) => {
                                let left = self.process_expression(left);
                                let right = self.process_expression(right);
                                
                                self.get_or_insert(DagNode { dag_type: 
                                    DagType::IntegerAddition(left, right) })
                            },
                            _ => panic!("Dag is not ready to accept addition of type {}", expression.static_type)
                        }
                    },
                    _ => panic!("Dag is not ready to accept binary operator {:?}", operator)
                }
            },
            ExpressionType::Grouping { expression } => {
                self.process_expression(expression)
            },
            ExpressionType::Identifier(name) => {
                self.get_or_insert(DagNode { dag_type: DagType::Identifier(name.clone()) })
            }
            _ => panic!("Dag is not ready to accept {:?}", expression.expression_type)
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
            println!("{i}. {:?}", value);
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
    IntegerAddition(Rc<DagNode>, Rc<DagNode>)
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