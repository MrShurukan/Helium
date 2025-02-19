use std::collections::{HashMap, HashSet};
use std::fmt::{write, Display, Formatter};
use std::rc::Rc;
use colored::Colorize;
use crate::frontend::expression::{Expression, ExpressionType};
use crate::frontend::statement::{Statement, StatementType};
use crate::frontend::static_type::StaticType;
use crate::frontend::token_type::LiteralType;
use crate::frontend::utils::SourceLocation;
use crate::middle_end::dag_builder::{DagBuilder, DagNode, DagType};

// TODO: Comments
// TODO: Proper errors, not panics
type RegisterNumber = u32;
pub struct IRBuilder {
    ir_vec: Vec<IntermediateRepresentation>,
    last_register_number: RegisterNumber,
    local_variables: HashMap<String, RegisterNumber>,
    processed_dags: HashMap<Rc<DagNode>, RegisterNumber>
}

impl IRBuilder {
    pub fn new() -> Self {
        Self { 
            ir_vec: Vec::new(),
            local_variables: HashMap::new(),
            last_register_number: 0,
            processed_dags: HashMap::new()
        }
    }
    
    pub fn produce_ir(mut self, statements: &[Statement]) -> Vec<IntermediateRepresentation> {
        for statement in statements {
            self.statement(statement);
        }
        
        self.ir_vec
    }
    
    fn statement(&mut self, statement: &Statement) {
        let source_location = Rc::new(statement.source_location);
        
        match &statement.statement_type {
            StatementType::Expression(expression) => _ = self.expression(expression),
            StatementType::VariableDeclaration { identifier, initializer, .. } => {
                if let Some(init_expr) = initializer {
                    let register_number = self.expression(init_expr);
                    self.local_variables.insert(identifier.clone(), register_number);
                }
                else { panic!("Variable declaration without initializer"); }
            },
            _ => panic!("Can't yet produce IR for {}", statement.statement_type)
        }
    }
    
    fn expression(&mut self, expression: &Expression) -> RegisterNumber {
        // We use dag here to produce optimized expressions
        let mut dag_builder = DagBuilder::new();
        let dag = dag_builder.process_expression(expression);
        
        self.dag(&dag, expression)
    }
    
    fn dag(&mut self, dag_node: &Rc<DagNode>, expression: &Expression) -> RegisterNumber {
        let register_number = self.processed_dags.get(dag_node);
        if let Some(register_number) = register_number {
            return *register_number;
        }
        
        let register_number = self.calculate_register_from_dag(&dag_node, expression);
        self.processed_dags.insert(Rc::clone(dag_node), register_number);
        
        register_number
    }

    fn calculate_register_from_dag(&mut self, dag_node: &Rc<DagNode>, expression: &Expression) -> RegisterNumber {
        match &dag_node.dag_type {
            DagType::Identifier(name) => {
                let local_variable = self.local_variables.get(name);
                if let Some(local_variable_register) = local_variable {
                    *local_variable_register
                }
                // Global variable
                else {
                    self.last_register_number += 1;

                    self.ir_vec.push(IntermediateRepresentation {
                        ir_type: IRType::Load(LoadType::Memory(name.clone()), self.last_register_number),
                        source_location: Rc::new(expression.source_location)
                    });

                    self.last_register_number
                }
            }
            DagType::IntegerValue(value) => {
                self.last_register_number += 1;

                self.ir_vec.push(IntermediateRepresentation {
                    ir_type: IRType::Load(LoadType::Immediate(LiteralType::Integer(*value)), self.last_register_number),
                    source_location: Rc::new(expression.source_location)
                });

                self.last_register_number
            },
            DagType::IntegerAddition(left, right) => {
                let left_register = self.dag(left, expression);
                let right_register = self.dag(right, expression);

                self.last_register_number += 1;

                self.ir_vec.push(IntermediateRepresentation {
                    ir_type: IRType::IntegerAdd { a: left_register, b: right_register, output: self.last_register_number },
                    source_location: Rc::new(expression.source_location),
                });

                self.last_register_number
            }
        }
    }
}

pub struct IntermediateRepresentation {
    ir_type: IRType,
    source_location: Rc<SourceLocation>
}

impl Display for IntermediateRepresentation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.ir_type {
            IRType::Load(t, reg) => {
                write!(f, "LOAD ")?;
                match t {
                    LoadType::Immediate(literal_type) => {
                        write!(f, "${literal_type:?}")?;
                    }
                    LoadType::Memory(name) => {
                        write!(f, "{name}")?;
                    }
                }
                write!(f, " -> %r{reg}")
            }
            IRType::Store(reg, name) =>
                write!(f, "STOR {name} -> %r{reg}"),
            IRType::IntegerAdd { a, b, output } =>
                write!(f, "IADD %r{a}, %r{b} -> %r{output}")
        }
    }
}

pub enum IRType {
    Load(LoadType, RegisterNumber),
    Store(RegisterNumber, String),
    IntegerAdd { a: RegisterNumber, b: RegisterNumber, output: RegisterNumber }
}

pub enum LoadType {
    Immediate(LiteralType),
    Memory(String)
}