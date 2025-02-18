use crate::frontend::statement::{Statement, StatementType};
use crate::frontend::{Parser, ParserError};
use crate::middle_end::utils::EnvironmentStack;
use crate::middle_end::Analyzer;
use colored::Colorize;
use std::error::Error;
use std::process::exit;
use std::{env, fs};
use crate::middle_end::dag_builder::DagBuilder;

mod frontend;
mod middle_end;

// TODO: Read up on exit codes. Maybe use something that already makes sense
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: helium [main source file path]");
        exit(1);
    }

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
            report_errors(&errors, "parser");
        }
    };

    analyze_statements(&contents, &mut statements);
    // Just a hack to get setup at the moment
    let setup = statements.first().unwrap();
    let mut dag_builder = DagBuilder::new();
    
    // TODO: Remake statements that require block in them to just hold the Vec itself, not the
    // TODO: redundant Box<Statement> variable
    if let Statement { statement_type: StatementType::SetupDeclaration(block), .. } = setup {
        if let Statement { statement_type: StatementType::Block(setup_statements), .. } = block.as_ref() {
            for statement in setup_statements {
                if let StatementType::VariableDeclaration { initializer, .. } = &statement.statement_type {
                    if let Some(init) = initializer {
                        dag_builder.process_expression(init);
                    }
                }
            }
        }
        else { panic!() }
    }
    else { panic!("Didn't find setup for DAG") }
    
    println!("Dag table:");
    dag_builder.print_table();
    
    println!("Statements:");
    for (i, statement) in statements.iter().enumerate() {
        println!("{i}) {statement}");
    }
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
        report_errors(errors.as_slice(), "analyzer")
    }
}

fn report_errors<E: Error>(errors: &[E], error_source: &'static str) -> ! {
    eprintln!("{} (originated from {})", "Wasn't able to compile the file!".yellow(), error_source);
    let error_errors = if errors.len() == 1 { "error" } else { "errors" };
    eprintln!("{} {error_errors} found.\n", errors.len());

    for (i, error) in errors.iter().enumerate() {
        eprintln!("{} {error}", format!("{})", i+1).red());
    }
    exit(3);
}