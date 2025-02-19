use crate::frontend::expression::{Expression, ExpressionType, PinRef};
use crate::frontend::statement::{SingleDefDeclaration, Statement, StatementType};
use crate::frontend::token_type::{LiteralType, PinModeConstantType};
use crate::frontend::utils::{LocatableInSource, SourceLocation};
use crate::middle_end::dag_builder::{DagBuilder, DagError, DagNode, DagType};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::pin::Pin;
use std::rc::Rc;
use colored::Colorize;
use crate::middle_end::AnalyzerErrorType;
use crate::middle_end::error_printer::{get_error_marked_line, get_error_marked_line_location};
use crate::Program;

// TODO: Comments
// TODO: Proper errors, not panics
type RegisterNumber = u32;
pub struct IRBuilder<'a> {
    ir_vec: Vec<IntermediateRepresentation>,
    last_register_number: RegisterNumber,
    local_variables: HashMap<String, RegisterNumber>,
    processed_dags: HashMap<Rc<DagNode>, RegisterNumber>,
    labels: HashSet<String>,
    source: &'a str,
}

impl IRBuilder<'_> {
    pub fn new(source: &str) -> IRBuilder {
        IRBuilder {
            ir_vec: Vec::new(),
            local_variables: HashMap::new(),
            last_register_number: 0,
            processed_dags: HashMap::new(),
            labels: HashSet::new(),
            source
        }
    }

    fn reset(&mut self) {
        self.local_variables.clear();
        self.last_register_number = 0;
        self.processed_dags.clear();
        self.labels.clear();
    }
    
    pub fn produce_ir(mut self, program: Program) -> Result<Vec<IntermediateRepresentation>, IRError> {
        self.ir_vec.push(IntermediateRepresentation {
            ir_type: IRType::Label("setup".to_string()),
            source_location: None
        });

        if !program.global_var_declarations.is_empty() || !program.def_declarations.is_empty() {
            for var_decl in program.global_var_declarations {
                self.global_variable(var_decl)?;
            }

            for def_decl in program.def_declarations {
                self.pin_setup(def_decl)?;
            }
        }

        for statement in program.setup_block {
            self.statement(statement)?;
        }

        self.reset();

        self.ir_vec.push(IntermediateRepresentation {
            ir_type: IRType::Label("loop".to_string()),
            source_location: None
        });

        for statement in program.loop_block {
            self.statement(statement)?;
        }

        self.ir_vec.push(IntermediateRepresentation {
            ir_type: IRType::Goto("loop".to_string()),
            source_location: None
        });
        
        Ok(self.ir_vec)
    }

    fn global_variable(&mut self, statement: Statement) -> Result<(), IRError> {
        if let StatementType::VariableDeclaration {
            identifier, initializer, ..
        } = statement.statement_type {
            if let Some(init) = initializer {
                let register = self.expression(&init)?;

                self.ir_vec.push(IntermediateRepresentation {
                    ir_type: IRType::Store(register, identifier),
                    source_location: Some(Rc::new(statement.source_location))
                });

                Ok(())
            }
            else {
                Err(IRError::new(IRErrorType::VariableDeclarationWithoutInit, &statement.source_location, self.source))
            }
        }
        else {
            Err(IRError::new(IRErrorType::GlobalVarDeclarationIsNotVarStatement, &statement.source_location, self.source))
        }
    }

    fn pin_setup(&mut self, single_def_declaration: SingleDefDeclaration) -> Result<(), IRError> {
        let source_location = Rc::new(single_def_declaration.source_location);

        self.last_register_number += 1;
        // TODO: Optimize this by combining multiple ddrs together instead of constantly loading current state
        self.push_ir(
            IRType::Load(LoadType::DDR(single_def_declaration.pin.port.clone()), self.last_register_number),
            &source_location
        );

        let current_ddr_value = self.last_register_number;
        let desired_pin_binary = Self::get_binary_pin_number(single_def_declaration.pin.pin_number);
        
        // Determine if we need to make it input or off
        match single_def_declaration.pin_mode {
            PinModeConstantType::Input |
            PinModeConstantType::InputPullDown |
            PinModeConstantType::InputPullUp => { 
                // Inputs are represented by `0` and are done via &
                // ! - binary not
                self.push_ir(
                    IRType::AndImmediate(current_ddr_value, !desired_pin_binary as i8),
                    &source_location
                );
            },
            PinModeConstantType::Output => {
                // Outputs are represented by '1' and are done via |
                self.push_ir(
                    IRType::OrImmediate(current_ddr_value, desired_pin_binary as i8),
                    &source_location
                );
            }
        }

        self.push_ir(
            IRType::OutDDR(single_def_declaration.pin.port, current_ddr_value),
            &source_location
        );
        
        // TODO: PullUp and PullDown processing
        match single_def_declaration.pin_mode {
            PinModeConstantType::InputPullDown |
            PinModeConstantType::InputPullUp => panic!("Pulldown and pullup are not implemented"),
            _ => {}
        }
        Ok(())
    }
    
    fn get_binary_pin_number(regular: usize) -> i32 {
        0b1 << (regular - 1)
    }

    fn statement(&mut self, statement: Statement) -> Result<(), IRError> {
        let source_location = statement.source_location;

        match &statement.statement_type {
            StatementType::Expression(expression) => {
                _ = self.expression(expression);
                Ok(())
            },
            StatementType::VariableDeclaration { identifier, initializer, .. } => {
                if let Some(init_expr) = initializer {
                    let register_number = self.expression(init_expr)?;
                    self.local_variables.insert(identifier.clone(), register_number);
                    Ok(())
                }
                else { Err(IRError::new(IRErrorType::VariableDeclarationWithoutInit, &source_location, self.source)) }
            },
            _ => Err(IRError::new(IRErrorType::CannotProduceIRForStmt(statement.statement_type), &source_location, self.source))
        }
    }
    
    fn expression(&mut self, expression: &Expression) -> Result<RegisterNumber, IRError> {
        // We use dag here to produce optimized expressions
        let mut dag_builder = DagBuilder::new();
        let dag = dag_builder.process_expression(expression)
            .map_err(|err| IRError::new(IRErrorType::DagError(err), &expression.source_location, self.source))?;
        
        Ok(self.dag(&dag, expression))
    }
    
    fn dag(&mut self, dag_node: &Rc<DagNode>, expression: &Expression) -> RegisterNumber {
        let register_number = self.processed_dags.get(dag_node);
        if let Some(register_number) = register_number {
            return *register_number;
        }
        
        let register_number = self.calculate_register_from_dag(dag_node, expression);
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

                    self.push_ir(
                        IRType::Load(LoadType::Memory(name.clone()), self.last_register_number),
                        &Rc::new(expression.source_location)
                    );

                    self.last_register_number
                }
            }
            DagType::IntegerValue(value) => {
                self.last_register_number += 1;

                self.push_ir(
                    IRType::Load(LoadType::Immediate(LiteralType::Integer(*value)), self.last_register_number),
                    &Rc::new(expression.source_location)
                );

                self.last_register_number
            },
            DagType::IntegerAddition(left, right) => {
                let left_register = self.dag(left, expression);
                let right_register = self.dag(right, expression);

                self.last_register_number += 1;

                self.push_ir(
                    IRType::IntegerAdd { a: left_register, b: right_register, output: self.last_register_number },
                    &Rc::new(expression.source_location)
                );

                self.last_register_number
            }
        }
    }

    fn push_ir(&mut self, ir_type: IRType, source_location: &Rc<SourceLocation>) {
        self.ir_vec.push(IntermediateRepresentation {
            ir_type,
            source_location: Some(Rc::clone(source_location))
        })
    }

    fn push_ir_no_location(&mut self, ir_type: IRType) {
        self.ir_vec.push(IntermediateRepresentation {
            ir_type,
            source_location: None
        })
    }
}

pub struct IntermediateRepresentation {
    ir_type: IRType,
    source_location: Option<Rc<SourceLocation>>
}

impl Display for IntermediateRepresentation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.ir_type {
            IRType::Load(t, reg) => {
                match t {
                    LoadType::Immediate(literal_type) =>
                        write!(f, "LOAD ${literal_type:?}")?,
                    LoadType::Memory(name) =>
                        write!(f, "LOAD {name}")?,
                    LoadType::DDR(port) =>
                        write!(f, "IN DDR{port}")?,
                    LoadType::Port(port) =>
                        write!(f, "IN Port{port}")?,
                }
                write!(f, " -> %r{reg}")
            }
            IRType::Store(reg, name) =>
                write!(f, "STOR {name} -> %r{reg}"),
            IRType::IntegerAdd { a, b, output } =>
                write!(f, "IADD %r{a}, %r{b} -> %r{output}"),
            IRType::Label(label) =>
                write!(f, "{label}:"),
            IRType::Goto(label) =>
                write!(f, "GOTO {label}"),
            IRType::OutDDR(port, reg) =>
                write!(f, "OUT DDR{port} -> %r{reg}"),
            IRType::OutPort(port, reg) =>
                write!(f, "OUT Port{port} -> %r{reg}"),
            IRType::AndImmediate(reg, num) =>
                write!(f, "ANDI %r{reg}, {num:#010b} -> %r{reg}"),
            IRType::OrImmediate(reg, num) =>
                write!(f, "ORI %r{reg}, {num:#010b} -> %r{reg}"),
        }
    }
}

pub enum IRType {
    Label(String),
    Goto(String),
    Load(LoadType, RegisterNumber),
    Store(RegisterNumber, String),
    IntegerAdd { a: RegisterNumber, b: RegisterNumber, output: RegisterNumber },
    OutDDR(String, RegisterNumber),
    OutPort(String, RegisterNumber),
    AndImmediate(RegisterNumber, i8),
    OrImmediate(RegisterNumber, i8),
}

pub enum LoadType {
    Immediate(LiteralType),
    Memory(String),
    DDR(String),
    Port(String)
}

#[derive(Debug)]
pub struct IRError {
    error_type: IRErrorType,
    location: Option<String>
}

impl IRError {
    pub fn new(error_type: IRErrorType, source_location: &SourceLocation, source: &str) -> Self {
        let location = get_error_marked_line_location(source_location, source);
        Self { error_type, location: Some(location) }
    }

    pub fn new_no_location(error_type: IRErrorType) -> Self {
        Self { error_type, location: None }
    }
}

#[derive(Debug)]
pub enum IRErrorType {
    VariableDeclarationWithoutInit,
    CannotProduceIRForStmt(StatementType),
    DagError(DagError),
    GlobalVarDeclarationIsNotVarStatement,
}

impl Error for IRError {}
impl Display for IRError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let error_header = "[Internal compiler error] IR Error";

        let message = match &self.error_type {
            IRErrorType::VariableDeclarationWithoutInit =>
                "Variable declaration without initializer".to_string(),
            IRErrorType::CannotProduceIRForStmt(statement_type) =>
                format!("Can't produce IR for statement '{statement_type}', not yet implemented"),
            IRErrorType::DagError(dag_error) => 
                dag_error.to_string(),
            IRErrorType::GlobalVarDeclarationIsNotVarStatement =>
                "Variable declaration of incorrect statement type".to_string(),
        }.red();

        write!(f, "{error_header}\n{message}")?;

        // Add tabs to each line of the location so it shifts to the right
        if let Some(location) = &self.location {
            let location: String = location.split('\n')
                .fold(String::new(), |mut output, line| {
                    output += "\t";
                    output += line;
                    output += "\n";
                    output
                });
            write!(f, "\n\t{}\n{}\n", "Happened here:".green(), location.trim_end())?;
        }

        Ok(())
    }
}