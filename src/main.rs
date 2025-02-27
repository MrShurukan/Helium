use crate::frontend::statement::{SingleDefDeclaration, Statement, StatementType};
use crate::frontend::{Parser, ParserError};
use crate::middle_end::linear_ir::{IRBuilder, IRError, IntermediateRepresentation};
use crate::middle_end::utils::EnvironmentStack;
use crate::middle_end::Analyzer;
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::process::exit;
use std::{env, fs};
use crate::backend::{AssemblyError, AssemblyProducer};
use crate::backend::assembler::assemble_and_load;
use crate::backend::microcontroller::{Microcontroller, MICROCONTROLLERS};

mod frontend;
mod middle_end;
mod backend;

// TODO: Read up on exit codes. Maybe use something that already makes sense
// TODO: Remove all the explicit panic! from the code, handle errors gracefully
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("{}", "Usage: helium [main source file path] [microcontroller name]".red());
        exit(1);
    }
    
    let microcontroller_name = args[2].to_lowercase();
    let microcontroller = MICROCONTROLLERS.get(microcontroller_name.as_str());
    if microcontroller.is_none() {
        let output = format!("'{microcontroller_name}' is not a currently supported \
                                    microcontroller. List of supported:").red();
        eprintln!("{}", output);
        for key in MICROCONTROLLERS.keys() {
            eprintln!("{}", key.red());
        }
        
        exit(2);
    }
    let microcontroller = microcontroller.unwrap();

    let file_path = &args[1];
    let contents = match read_source_file(file_path) {
        Ok(contents) => contents,
        Err(err) => {
            eprintln!("{}", "Encountered a problem while reading the source file".red());
            eprintln!("{err}");
            exit(2);
        }
    };

    let mut statements = match parse_source(&contents) {
        Ok(statements) => statements,
        Err(errors) => {
            report_errors_and_exit(&errors, "parser");
        }
    };

    analyze_statements(&contents, &mut statements);
    
    println!("Statements:");
    for (i, statement) in statements.iter().enumerate() {
        println!("{i}) {statement}");
    }

    let program = structure_program(statements);
    if let Err(structure_error) = program {
        report_errors_and_exit(&[structure_error], "structure organizer");
    }
    let program = program.unwrap();
    
    let ir_vec = convert_to_ir(program, &contents);
    if let Err(ir_error) = ir_vec {
        report_errors_and_exit(&[ir_error], "IR builder");
    }
    let ir_vec = ir_vec.unwrap();

    println!("IR:");
    for ir in &ir_vec {
        println!("{ir}");
    }
    
    let assembly_language = convert_to_assembly(ir_vec, microcontroller, &contents);
    if let Err(assembly_producer_error) = assembly_language {
        report_errors_and_exit(&[assembly_producer_error], "assembly producer");
    }
    let assembly_language = assembly_language.unwrap().join("\n");
    
    println!("Assembly:");
    println!("{assembly_language}");
    
    assemble_and_load(assembly_language, microcontroller);
}

fn read_source_file(file_path: &str) -> Result<String, std::io::Error> {
    fs::read_to_string(file_path)
}

fn parse_source(contents: &str) -> Result<Vec<Statement>, Vec<ParserError>> {
    let parser = Parser::new(contents);
    parser.parse()
}

/// This modifies the statements and fills them with types + performs a static analysis
fn analyze_statements(contents: &str, statements: &mut [Statement]) {
    let mut empty_environment = EnvironmentStack::new();
    let analyzer = Analyzer::new_with_environment(contents, &mut empty_environment);

    if let Err(errors) = analyzer.analyze(statements) {
        report_errors_and_exit(errors.as_slice(), "analyzer")
    }
}

fn structure_program(statements: Vec<Statement>) -> Result<Program, StructureError> {
    let mut def_declarations: Vec<SingleDefDeclaration> = Vec::new();
    let mut global_var_declarations: Vec<Statement> = Vec::new();
    let mut setup_block: Vec<Statement> = Vec::new();
    let mut loop_block: Vec<Statement> = Vec::new();
    
    let mut loop_found = false;
    
    for statement in statements {
        match statement.statement_type {
            StatementType::DefDeclaration(declarations) => def_declarations = declarations,
            StatementType::VariableDeclaration { .. } => global_var_declarations.push(statement),
            StatementType::SetupDeclaration(statements) => setup_block = statements,
            StatementType::LoopDeclaration(statements) => {
                loop_block = statements;
                loop_found = true;
            },
            _ => {}
        }
    }
    
    if loop_found {
        Ok(Program { def_declarations, global_var_declarations, setup_block, loop_block })
    }
    else {
        Err(StructureError::NoLoopPresent)
    }
}

fn convert_to_ir(program: Program, source: &str) -> Result<Vec<IntermediateRepresentation>, IRError> {
    let ir_builder = IRBuilder::new(source);
    ir_builder.produce_ir(program)
}

fn convert_to_assembly(ir_vec: Vec<IntermediateRepresentation>, microcontroller: &Microcontroller, source: &str) -> Result<Vec<String>, AssemblyError> {
    let mut assembly_producer = AssemblyProducer::new(source);
    assembly_producer.produce(ir_vec, microcontroller)
}

struct Program {
    def_declarations: Vec<SingleDefDeclaration>,
    global_var_declarations: Vec<Statement>,
    /// Will be empty if setup is absent
    setup_block: Vec<Statement>,
    loop_block: Vec<Statement>
}

#[derive(Debug)]
enum StructureError {
    NoLoopPresent
}

impl Error for StructureError {}
impl Display for StructureError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            StructureError::NoLoopPresent => write!(f, "There was no loop block present in the source file")
        }
    }
}

fn report_errors_and_exit<E: Error>(errors: &[E], error_source: &'static str) -> ! {
    eprintln!("{} (originated from {})", "Wasn't able to compile the file!".yellow(), error_source);
    let error_errors = if errors.len() == 1 { "error" } else { "errors" };
    eprintln!("{} {error_errors} found.\n", errors.len());

    for (i, error) in errors.iter().enumerate() {
        eprintln!("{} {error}", format!("{})", i+1).red());
    }
    exit(3);
}