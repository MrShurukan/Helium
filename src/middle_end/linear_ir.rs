use crate::frontend::expression::{Expression, ExpressionType, PinRef};
use crate::frontend::statement::{SingleDefDeclaration, Statement, StatementType};
use crate::frontend::token_type::{LiteralType, PinModeConstantType, PinStateConstantType};
use crate::frontend::utils::{LocatableInSource, SourceLocation};
use crate::middle_end::dag_builder::{DagBuilder, DagError, DagNode, DagType};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{write, Display, Formatter};
use std::pin::Pin;
use std::process::id;
use std::rc::Rc;
use colored::Colorize;
use rand::Rng;
use crate::middle_end::AnalyzerErrorType;
use crate::middle_end::error_printer::{get_error_marked_line, get_error_marked_line_location};
use crate::Program;
use rand::rngs::ThreadRng;

// TODO: Comments
// TODO: Proper errors, not panics
type RegisterNumber = u32;
pub struct IRBuilder<'a> {
    ir_vec: Vec<IntermediateRepresentation>,
    last_register_number: RegisterNumber,
    local_variables: HashMap<String, RegisterNumber>,
    registered_pins: HashMap<String, PinRef>,
    registered_labels: HashSet<String>,
    processed_dags: HashMap<Rc<DagNode>, RegisterNumber>,
    source: &'a str,

    /// Used for generating labels
    rng: ThreadRng
}

impl IRBuilder<'_> {
    pub fn new(source: &str) -> IRBuilder {
        // For simplicity, setup and loop labels are declared here already
        // As it's assumed labels are always going to be there
        let mut labels = HashSet::new();
        labels.insert("setup".to_string());
        labels.insert("loop".to_string());

        IRBuilder {
            ir_vec: Vec::new(),
            local_variables: HashMap::new(),
            last_register_number: 0,
            processed_dags: HashMap::new(),
            registered_pins: HashMap::new(),
            registered_labels: labels,
            source,
            rng: rand::rng()
        }
    }
    
    pub fn produce_ir(mut self, program: Program) -> Result<Vec<IntermediateRepresentation>, IRError> {
        self.ir_vec.push(IntermediateRepresentation {
            ir_type: IRType::Label("setup".to_string())
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
            self.statement(&statement)?;
        }

        // self.reset();

        self.ir_vec.push(IntermediateRepresentation {
            ir_type: IRType::Label("loop".to_string())
        });

        for statement in program.loop_block {
            self.statement(&statement)?;
        }

        self.ir_vec.push(IntermediateRepresentation {
            ir_type: IRType::Jump("loop".to_string())
        });
        
        Ok(self.ir_vec)
    }

    fn global_variable(&mut self, statement: Statement) -> Result<(), IRError> {
        if let StatementType::VariableDeclaration {
            identifier, initializer, ..
        } = statement.statement_type {
            if let Some(init) = initializer {
                let register = self.expression(&init)?;
                if register.is_none() {
                    panic!("Can't assign value of this expression to a constant");
                }

                self.ir_vec.push(IntermediateRepresentation {
                    ir_type: IRType::Store(register.unwrap(), identifier)
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
        // Inputs are represented as '0'
        // Outputs are represented as '1'
        match single_def_declaration.pin_mode {
            PinModeConstantType::Input |
            PinModeConstantType::InputPullDown |
            PinModeConstantType::InputPullUp => {
                self.push_ir(
                    IRType::SetInputPortMode(single_def_declaration.pin.clone())
                );
            },

            PinModeConstantType::Output => {
                self.push_ir(
                    IRType::SetOutputPortMode(single_def_declaration.pin.clone())
                );
            }
        }

        // TODO: PullUp and PullDown processing
        match single_def_declaration.pin_mode {
            PinModeConstantType::InputPullDown |
            PinModeConstantType::InputPullUp => panic!("Pulldown and pullup are not implemented"),
            _ => {}
        }

        self.registered_pins.insert(single_def_declaration.identifier, single_def_declaration.pin);

        Ok(())
    }

    fn statement(&mut self, statement: &Statement) -> Result<(), IRError> {
        let source_location = statement.source_location;

        match &statement.statement_type {
            StatementType::Expression(expression) => {
                self.expression(expression)?;
                Ok(())
            },
            StatementType::VariableDeclaration { identifier, initializer, .. } => {
                if let Some(init_expr) = initializer {
                    let register_number = self.expression(init_expr)?;
                    if register_number.is_none() {
                        panic!("Can't assign value of this expression to a variable");
                    }
                    
                    self.local_variables.insert(identifier.clone(), register_number.unwrap());
                    Ok(())
                }
                else { Err(IRError::new(IRErrorType::VariableDeclarationWithoutInit, &source_location, self.source)) }
            },
            StatementType::If { condition, then_body, else_body } => {
                let condition_register_number = self.expression(condition)?;
                if condition_register_number.is_none() {
                    panic!("Can't use this expression as a condition - it returned no value")
                }

                let else_label = self.register_random_label("else");
                self.push_ir(
                    IRType::JumpIfZero(condition_register_number.unwrap(), else_label.clone())
                );
                self.statement(then_body)?;
                
                if let Some(else_body) = else_body {
                    let skip_else_label = self.register_random_label("skip_else");
                    self.push_ir(
                        IRType::Jump(skip_else_label.clone())
                    );
                    
                    self.push_ir(IRType::Label(else_label));
                    self.statement(else_body)?;
                    self.push_ir(IRType::Label(skip_else_label));
                }
                else {
                    self.push_ir(IRType::Label(else_label));
                }

                Ok(())
            },
            StatementType::Block(statements) => {
                for statement in statements {
                    self.statement(statement)?;
                }
                
                Ok(())
            },
            _ => Err(IRError::new(IRErrorType::CannotProduceIRForStmt {
                statement_type: statement.statement_type.to_string()
            }, &source_location, self.source))
        }
    }
    
    fn expression(&mut self, expression: &Expression) -> Result<Option<RegisterNumber>, IRError> {
        // We use dag here to produce optimized expressions
        let mut dag_builder = DagBuilder::new();
        let dag = dag_builder.process_expression(expression)
            .map_err(|err| IRError::new(IRErrorType::DagError(err), &expression.source_location, self.source))?;
        
        Ok(self.dag(&dag))
    }
    
    fn dag(&mut self, dag_node: &Rc<DagNode>) -> Option<RegisterNumber> {
        let register_number = self.processed_dags.get(dag_node);
        if let Some(register_number) = register_number {
            return Some(*register_number);
        }
        
        let register_number = self.calculate_register_from_dag(dag_node);
        if let Some(register_number) = &register_number {
            self.processed_dags.insert(Rc::clone(dag_node), *register_number);
        }
        
        register_number
    }

    fn calculate_register_from_dag(&mut self, dag_node: &Rc<DagNode>) -> Option<RegisterNumber> {
        match &dag_node.dag_type {
            DagType::Identifier(name) => {
                let local_variable = self.local_variables.get(name);
                if let Some(local_variable_register) = local_variable {
                    Some(*local_variable_register)
                }
                // Global variable
                else {
                    self.last_register_number += 1;

                    self.push_ir(
                        IRType::Load(LoadType::Memory(name.clone()), self.last_register_number)
                    );

                    Some(self.last_register_number)
                }
            }
            DagType::IntegerValue(_) => {
                None
            },
            DagType::PinModeValue(_) => {
                None
            },
            DagType::PinStateValue(_) => {
                None
            },
            DagType::IntegerAddition(left, right) => {
                let left_register = self.process_integer_dag(left);
                let right_register = self.process_integer_dag(right);

                self.last_register_number += 1;

                self.push_ir(
                    IRType::IntegerAdd { a: left_register, b: right_register, output: self.last_register_number }
                );

                Some(self.last_register_number)
            },
            DagType::AssignmentInteger(identifier, value) => {
                let value_register = self.process_integer_dag(value);
                Some(self.assignment_integer(identifier, value_register))
            },
            DagType::AssignmentPinMode(identifier, value) => {
                // This could be assignment to a pin. In this case this is a different operation
                if let Some(pin) = self.registered_pins.get(identifier) {
                    let pin = pin.clone();
                    // Step 1. Check if it's a compiler known value
                    let known_value = Self::try_get_end_value(value);
                    match known_value {
                        // Step 2. We got the constant value
                        Some(DagType::PinModeValue(pin_mode_value)) => {
                            if matches!(pin_mode_value, PinModeConstantType::Output) {
                                self.push_ir(
                                    IRType::SetOutputPortMode(pin.clone())
                                );
                            }
                            else {
                                // TODO: INPUT_PULLUP and INPUT_PULLDOWN
                                if matches!(pin_mode_value, PinModeConstantType::InputPullDown | PinModeConstantType::InputPullUp) {
                                    panic!("Can't work with input_pullup and pulldown yet");
                                }

                                self.push_ir(
                                    IRType::SetInputPortMode(pin.clone())
                                );
                            }

                            None
                        },
                        Some(other) => panic!("Incorrect type for pin mode assignment {other:?}"),
                        // Step 2. It's not a constant, need to runtime check it
                        _ => {
                            // TODO: INPUT_PULLUP and INPUT_PULLDOWN
                            let value_register = self.process_pin_mode_dag(value);
                            let input_label = self.register_random_label("input");
                            let end_label = self.register_random_label("end");

                            self.push_ir(IRType::JumpIfZero(value_register, input_label.clone()));
                            self.push_ir(IRType::SetOutputPortMode(pin.clone()));
                            self.push_ir(IRType::Jump(end_label.clone()));

                            self.push_ir(IRType::Label(input_label));
                            self.push_ir(IRType::SetInputPortMode(pin.clone()));

                            self.push_ir(IRType::Label(end_label));

                            Some(value_register)
                        }
                    }
                }
                // This wasn't a pin, so it's an assignment to a local/global variable
                else {
                    let value_register = self.process_pin_mode_dag(value);
                    Some(self.assignment_integer(identifier, value_register))
                }
            },
            DagType::AssignmentPinState(identifier, value) => {
                // This could be assignment to a pin. In this case this is a different operation
                if let Some(pin) = self.registered_pins.get(identifier) {
                    let pin = pin.clone();
                    // Step 1. Check if it's a compiler known value
                    let known_value = Self::try_get_end_value(value);
                    match known_value {
                        // Step 2. We got the constant value
                        Some(DagType::PinStateValue(pin_state_value)) => {
                            if matches!(pin_state_value, PinStateConstantType::High) {
                                self.push_ir(
                                    IRType::SetPortHigh(pin.clone())
                                );
                            }
                            else {
                                self.push_ir(
                                    IRType::SetPortLow(pin.clone())
                                );
                            }

                            None
                        },
                        Some(other) => panic!("Incorrect type for pin state assignment {other:?}"),
                        // Step 2. It's not a constant, need to runtime check it
                        _ => {
                            let value_register = self.process_pin_state_dag(value);
                            let low_label = self.register_random_label("low");
                            let high_label = self.register_random_label("high");

                            self.push_ir(IRType::JumpIfZero(value_register, low_label.clone()));
                            self.push_ir(IRType::SetPortHigh(pin.clone()));
                            self.push_ir(IRType::Jump(high_label.clone()));

                            self.push_ir(IRType::Label(low_label));
                            self.push_ir(IRType::SetPortLow(pin.clone()));

                            self.push_ir(IRType::Label(high_label));

                            Some(value_register)
                        }
                    }
                }
                // This wasn't a pin, so it's an assignment to a local/global variable
                else {
                    let value_register = self.process_pin_state_dag(value);
                    Some(self.assignment_integer(identifier, value_register))
                }
            },
            DagType::PinStateComparison(identifier, right) => {
                if let Some(pin) = self.registered_pins.get(identifier) {
                    let pin = pin.clone();
                    // Step 1. Check if it's a compiler known value
                    let known_value = Self::try_get_end_value(right);
                    match known_value {
                        // Step 2. We got the constant value
                        Some(DagType::PinStateValue(pin_state_value)) => {
                            self.last_register_number += 1;
                            let one_label = self.register_random_label("true");
                            let end_label = self.register_random_label("end");
                            if matches!(pin_state_value, PinStateConstantType::Low) {
                                self.push_ir(IRType::SkipIfPortHigh(pin.clone()));
                            }
                            else {
                                self.push_ir(IRType::SkipIfPortLow(pin.clone()));
                            }

                            self.push_ir(IRType::Jump(one_label.clone()));
                            self.push_ir(
                                IRType::Load(LoadType::ImmediateInt(0), self.last_register_number)
                            );
                            self.push_ir(IRType::Jump(end_label.clone()));

                            self.push_ir(IRType::Label(one_label));
                            self.push_ir(
                                IRType::Load(LoadType::ImmediateInt(1), self.last_register_number)
                            );

                            self.push_ir(IRType::Label(end_label));

                            Some(self.last_register_number)
                        },
                        // Step 2. It's not a constant, need to runtime check it
                        _ => {
                            panic!("Can't do runtime pin state check yet");
                        }
                    }
                }
                else {
                    unreachable!("Couldn't find pin while producing IR");
                }
            }
        }
    }

    fn process_integer_dag(&mut self, dag_node: &Rc<DagNode>) -> RegisterNumber {
        match dag_node.dag_type {
            DagType::IntegerValue(value) => {
                self.last_register_number += 1;
                self.push_ir(
                    IRType::Load(LoadType::ImmediateInt(value), self.last_register_number)
                );

                self.processed_dags.insert(Rc::clone(dag_node), self.last_register_number);
                self.last_register_number
            },
            _ => {
                let processed = self.dag(dag_node);
                if let Some(reg) = processed {
                    reg
                }
                else {
                    panic!("Can't process a dag as integer")
                }
            }
        }
    }

    fn process_pin_mode_dag(&mut self, dag_node: &Rc<DagNode>) -> RegisterNumber {
        match dag_node.dag_type {
            DagType::PinModeValue(value) => {
                // TODO: PULLUP PULLDOWN
                self.last_register_number += 1;
                let value = match value {
                    PinModeConstantType::Input |
                    PinModeConstantType::InputPullDown |
                    PinModeConstantType::InputPullUp => 0,
                    PinModeConstantType::Output => 1
                };
                self.push_ir(
                    IRType::Load(LoadType::ImmediateInt(value), self.last_register_number)
                );

                self.processed_dags.insert(Rc::clone(dag_node), self.last_register_number);
                self.last_register_number
            },
            _ => {
                let processed = self.dag(dag_node);
                if let Some(reg) = processed {
                    reg
                }
                else {
                    panic!("Can't process a dag as pin_mode")
                }
            }
        }
    }

    fn process_pin_state_dag(&mut self, dag_node: &Rc<DagNode>) -> RegisterNumber {
        match dag_node.dag_type {
            DagType::PinStateValue(value) => {
                self.last_register_number += 1;
                let value = match value {
                    PinStateConstantType::Low => 0,
                    PinStateConstantType::High => 1
                };
                self.push_ir(
                    IRType::Load(LoadType::ImmediateInt(value), self.last_register_number)
                );

                self.processed_dags.insert(Rc::clone(dag_node), self.last_register_number);
                self.last_register_number
            },
            _ => {
                let processed = self.dag(dag_node);
                if let Some(reg) = processed {
                    reg
                }
                else {
                    panic!("Can't process a dag as pin_state")
                }
            }
        }
    }

    fn try_get_end_value(dag_node: &Rc<DagNode>) -> Option<DagType> {
        match &dag_node.dag_type {
            DagType::IntegerValue(_) |
            DagType::PinModeValue(_) |
            DagType::PinStateValue(_) => Some(dag_node.dag_type.clone()),

            DagType::AssignmentInteger(_, right) |
            DagType::AssignmentPinMode(_, right) |
            DagType::AssignmentPinState(_, right) |
            DagType::PinStateComparison(_, right) => Self::try_get_end_value(right),
            _ => None
        }
    }

    fn get_binary_pin_number(regular: usize) -> i32 {
        0b1 << (regular - 1)
    }

    fn assignment_integer(&mut self, identifier: &String, value_register: RegisterNumber) -> RegisterNumber {
        // Local variable
        if let Some(local_var) = self.local_variables.get(identifier) {
            self.push_ir(
                IRType::Move { reg_from: *local_var, reg_to: value_register }
            );
        }
        // Just some global value
        else {
            self.push_ir(
                IRType::Store(value_register, identifier.clone())
            );
        }

        value_register
    }

    fn push_ir(&mut self, ir_type: IRType) {
        self.ir_vec.push(IntermediateRepresentation {
            ir_type
        })
    }

    fn register_random_label(&mut self, prefix: &str) -> String {
        let max_iters = 10000;
        let mut label: String = "".to_string();

        for iter in 1..=max_iters {
            if iter == max_iters {
                panic!("Stuck in an infinite loop while generating labels");
            }

            label = format!("{prefix}_{}", self.rng.random::<u32>());
            if !self.registered_labels.contains(&label) { break; }
        }

        self.registered_labels.insert(label.clone());

        label
    }
}

pub struct IntermediateRepresentation {
    ir_type: IRType
}

impl Display for IntermediateRepresentation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.ir_type {
            IRType::Load(t, reg) => {
                match t {
                    LoadType::ImmediateInt(int) =>
                        write!(f, "LOAD ${int}")?,
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
                write!(f, "STOR %r{reg} -> {name}"),
            IRType::Move { reg_from, reg_to } =>
                write!(f, "MOV %r{reg_from} -> %r{reg_to}"),
            IRType::IntegerAdd { a, b, output } =>
                write!(f, "IADD %r{a}, %r{b} -> %r{output}"),
            IRType::Label(label) =>
                write!(f, "{}:", label.black().on_yellow()),
            IRType::Jump(label) =>
                write!(f, "JMP {}", label.yellow()),
            IRType::AndImmediate(reg, num) =>
                write!(f, "ANDI %r{reg}, {num:#010b} -> %r{reg}"),
            IRType::OrImmediate(reg, num) =>
                write!(f, "ORI %r{reg}, {num:#010b} -> %r{reg}"),
            IRType::LogicalShiftLeft(reg) =>
                write!(f, "LSL %r{reg} -> %r{reg}"),
            IRType::LogicalShiftRight(reg) =>
                write!(f, "LSR %r{reg} -> %r{reg}"),
            IRType::JumpIfZero(reg, label) =>
                write!(f, "JMPZero %r{reg}, {}", label.yellow()),
            IRType::BranchIfEqual(label) =>
                write!(f, "BREQ {}", label.yellow()),
            IRType::SkipIfPortLow(pin_ref) =>
                write!(f, "SkipIfPortBitClear {pin_ref}"),
            IRType::SkipIfPortHigh(pin_ref) =>
                write!(f, "SkipIfPortBitSet {pin_ref}"),
            IRType::SetPortLow(pin_ref) =>
                write!(f, "SetPortLow {pin_ref}"),
            IRType::SetPortHigh(pin_ref) =>
                write!(f, "SetPortHigh {pin_ref}"),
            IRType::SetInputPortMode(pin_ref) =>
                write!(f, "SetInputPortMode {pin_ref}"),
            IRType::SetOutputPortMode(pin_ref) =>
                write!(f, "SetOutputPortMode {pin_ref}"),
        }
    }
}

pub enum IRType {
    Label(String),
    Jump(String),
    Load(LoadType, RegisterNumber),
    Store(RegisterNumber, String),
    Move { reg_from: RegisterNumber, reg_to: RegisterNumber },
    IntegerAdd { a: RegisterNumber, b: RegisterNumber, output: RegisterNumber },
    LogicalShiftLeft(RegisterNumber),
    LogicalShiftRight(RegisterNumber),
    SetPortLow(PinRef),
    SetPortHigh(PinRef),
    SetInputPortMode(PinRef),
    SetOutputPortMode(PinRef),
    AndImmediate(RegisterNumber, i8),
    OrImmediate(RegisterNumber, i8),
    JumpIfZero(RegisterNumber, String),
    BranchIfEqual(String),
    SkipIfPortLow(PinRef),
    SkipIfPortHigh(PinRef)
}

pub enum LoadType {
    ImmediateInt(u64),
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
    CannotProduceIRForStmt { statement_type: String },
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
            IRErrorType::CannotProduceIRForStmt { statement_type} =>
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