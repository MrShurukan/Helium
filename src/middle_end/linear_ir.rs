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

                self.ir_vec.push(IntermediateRepresentation {
                    ir_type: IRType::Store(register, identifier)
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
                    IRType::ClearBitDDR(single_def_declaration.pin.port.clone(), single_def_declaration.pin.pin_number as u8)
                );
            },

            PinModeConstantType::Output => {
                self.push_ir(
                    IRType::SetBitDDR(single_def_declaration.pin.port.clone(), single_def_declaration.pin.pin_number as u8)
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
                    self.local_variables.insert(identifier.clone(), register_number);
                    Ok(())
                }
                else { Err(IRError::new(IRErrorType::VariableDeclarationWithoutInit, &source_location, self.source)) }
            },
            StatementType::If { condition, then_body, else_body } => {
                let condition_register_number = self.expression(condition)?;

                let else_label = self.register_random_label("else");
                self.push_ir(
                    IRType::TestForZero(condition_register_number)
                );
                self.push_ir(
                    IRType::BranchIfEqual(else_label.clone())
                );
                self.statement(then_body)?;
                self.push_ir(
                    IRType::Label(else_label)
                );
                if let Some(else_body) = else_body {
                    let skip_else_label = self.register_random_label("skip_else");
                    self.push_ir(
                        IRType::Jump(skip_else_label.clone())
                    );
                    self.statement(else_body)?;
                    self.push_ir(
                        IRType::Label(skip_else_label)
                    );
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
                        IRType::Load(LoadType::Memory(name.clone()), self.last_register_number)
                    );

                    self.last_register_number
                }
            }
            DagType::IntegerValue(value) => {
                self.last_register_number += 1;

                self.push_ir(
                    IRType::Load(LoadType::Immediate(LiteralType::Integer(*value)), self.last_register_number)
                );

                self.last_register_number
            },
            DagType::PinModeValue(value) => {
                self.last_register_number += 1;

                // TODO: PullUp and PullDown
                match value {
                    PinModeConstantType::Input => {
                        self.push_ir(
                            IRType::Load(LoadType::Immediate(LiteralType::Integer(0)), self.last_register_number)
                        );
                    },
                    PinModeConstantType::Output => {
                        self.push_ir(
                            IRType::Load(LoadType::Immediate(LiteralType::Integer(1)), self.last_register_number)
                        );
                    },
                    _ => panic!("Can't work with PULLUP and PULLDOWN yet")
                }

                self.last_register_number
            },
            DagType::PinStateValue(value) => {
                self.last_register_number += 1;

                let integer_type = match value {
                    PinStateConstantType::Low => LiteralType::Integer(0),
                    PinStateConstantType::High => LiteralType::Integer(1),
                };

                self.push_ir(
                    IRType::Load(LoadType::Immediate(integer_type), self.last_register_number)
                );

                self.last_register_number
            },
            DagType::IntegerAddition(left, right) => {
                let left_register = self.dag(left, expression);
                let right_register = self.dag(right, expression);

                self.last_register_number += 1;

                self.push_ir(
                    IRType::IntegerAdd { a: left_register, b: right_register, output: self.last_register_number }
                );

                self.last_register_number
            },
            DagType::Assignment(identifier, value) => {
                let value_register = self.dag(value, expression);
                self.assignment(identifier, value_register, expression)
            },
            DagType::AssignmentPinMode(identifier, value) => {
                let value_register = self.dag(value, expression);

                // This could be assignment to a pin. In this case this is a different operation
                if let Some(pin) = self.registered_pins.get(identifier) {
                    self.push_ir(
                        IRType::OutDDR(pin.port.clone(), value_register)
                    );

                    value_register
                }
                // This wasn't a pin, so it's an assignment to a local/global variable
                else {
                    self.assignment(identifier, value_register, expression)
                }
            },
            DagType::AssignmentPinState(identifier, value) => {
                let value_register = self.dag(value, expression);

                // This could be assignment to a pin. In this case this is a different operation
                if let Some(pin) = self.registered_pins.get(identifier) {
                    let pin = pin.clone();
                    let source_location = Rc::new(expression.source_location);

                    // If value == 0 then we clear the bit
                    // Otherwise we set it

                    // TODO: This is overkill if the value is a literal/constant.
                    // TODO: In case of that this should just output Set or Clear directly
                    let if_zero_label = self.register_random_label("if_zero");
                    let if_not_zero_label = self.register_random_label("if_not_zero");

                    self.push_ir(IRType::TestForZero(value_register));
                    self.push_ir(IRType::BranchIfEqual(if_zero_label.clone()));
                    // if not zero code
                    self.push_ir(IRType::SetBitPort(pin.port.clone(), pin.pin_number as u8));
                    self.push_ir(IRType::Jump(if_not_zero_label.clone()));

                    // if zero code
                    self.push_ir(IRType::Label(if_zero_label.clone()));
                    self.push_ir(IRType::ClearBitPort(pin.port.clone(), pin.pin_number as u8));

                    // if not zero jump
                    self.push_ir(IRType::Label(if_not_zero_label.clone()));

                    value_register
                }
                // This wasn't a pin, so it's an assignment to a local/global variable
                else {
                    self.assignment(identifier, value_register, expression)
                }
            },
            DagType::PinStateComparison(identifier, right) => {
                if let Some(pin) = self.registered_pins.get(identifier) {
                    let pin = pin.clone();
                    let right_register = self.dag(right, expression);

                    // Check if register contains a zero
                    self.push_ir(IRType::TestForZero(right_register));
                    let if_not_zero_label = self.register_random_label("if_not_zero");
                    let if_zero_label = self.register_random_label("if_zero");
                    let true_label = self.register_random_label("true");
                    let false_label = self.register_random_label("false");
                    let end_label = self.register_random_label("end");
                    self.push_ir(IRType::BranchIfEqual(if_zero_label.clone()));

                    let full_port_name = format!("Port{}", pin.port.clone());
                    
                    // if_not_zero:
                    self.push_ir(IRType::Label(if_not_zero_label));
                    self.push_ir(IRType::SkipIfBitIOSet(full_port_name.clone(), pin.pin_number as u8));
                    self.push_ir(IRType::Jump(false_label.clone()));
                    self.push_ir(IRType::Jump(true_label.clone()));
                    
                    // if_zero:
                    self.push_ir(IRType::Label(if_zero_label));
                    self.push_ir(IRType::SkipIfBitIOClear(full_port_name, pin.pin_number as u8));
                    self.push_ir(IRType::Jump(false_label.clone()));
                    self.push_ir(IRType::Jump(true_label.clone()));
                    
                    // true:
                    self.last_register_number += 1;
                    self.push_ir(IRType::Label(true_label));
                    self.push_ir(IRType::Load(LoadType::Immediate(LiteralType::Integer(1)), self.last_register_number));
                    self.push_ir(IRType::Jump(end_label.clone()));

                    // false:
                    self.push_ir(IRType::Label(false_label));
                    self.push_ir(IRType::Load(LoadType::Immediate(LiteralType::Integer(0)), self.last_register_number));
                    // end:
                    self.push_ir(IRType::Label(end_label));

                    self.last_register_number
                }
                else {
                    unreachable!("Couldn't find pin while producing IR");
                }
            }
        }
    }

    fn get_binary_pin_number(regular: usize) -> i32 {
        0b1 << (regular - 1)
    }

    fn assignment(&mut self, identifier: &String, value_register: RegisterNumber, expression: &Expression) -> RegisterNumber {
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
                write!(f, "STOR %r{reg} -> {name}"),
            IRType::Move { reg_from, reg_to } =>
                write!(f, "MOV %r{reg_from} -> %r{reg_to}"),
            IRType::IntegerAdd { a, b, output } =>
                write!(f, "IADD %r{a}, %r{b} -> %r{output}"),
            IRType::Label(label) =>
                write!(f, "{}:", label.black().on_yellow()),
            IRType::Jump(label) =>
                write!(f, "JMP {}", label.yellow()),
            IRType::OutDDR(port, reg) =>
                write!(f, "OUT %r{reg} -> DDR{port}"),
            IRType::OutPort(port, reg) =>
                write!(f, "OUT %r{reg} -> Port{port}"),
            IRType::AndImmediate(reg, num) =>
                write!(f, "ANDI %r{reg}, {num:#010b} -> %r{reg}"),
            IRType::OrImmediate(reg, num) =>
                write!(f, "ORI %r{reg}, {num:#010b} -> %r{reg}"),
            IRType::LogicalShiftLeft(reg) =>
                write!(f, "LSL %r{reg} -> %r{reg}"),
            IRType::LogicalShiftRight(reg) =>
                write!(f, "LSR %r{reg} -> %r{reg}"),
            IRType::SetBitDDR(port, bit_number) =>
                write!(f, "SBI DDR{port}, {bit_number}"),
            IRType::ClearBitDDR(port, bit_number) =>
                write!(f, "CBI DDR{port}, {bit_number}"),
            IRType::SetBitPort(port, bit_number) =>
                write!(f, "SBI Port{port}, {bit_number}"),
            IRType::ClearBitPort(port, bit_number) =>
                write!(f, "CBI Port{port}, {bit_number}"),
            IRType::TestForZero(reg) =>
                write!(f, "TST %r{reg} -> Z"),
            IRType::BranchIfEqual(label) =>
                write!(f, "BREQ {}", label.yellow()),
            IRType::SkipIfBitIOClear(io, bit_number) =>
                write!(f, "SBIS {io}, {bit_number}"),
            IRType::SkipIfBitIOSet(io, bit_number) =>
                write!(f, "SBIC {io}, {bit_number}"),
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
    OutDDR(String, RegisterNumber),
    OutPort(String, RegisterNumber),
    SetBitDDR(String, u8),
    ClearBitDDR(String, u8),
    SetBitPort(String, u8),
    ClearBitPort(String, u8),
    AndImmediate(RegisterNumber, i8),
    OrImmediate(RegisterNumber, i8),
    TestForZero(RegisterNumber),
    BranchIfEqual(String),
    SkipIfBitIOClear(String, u8),
    SkipIfBitIOSet(String, u8)
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