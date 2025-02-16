use crate::frontend::statement::Statement;
use crate::frontend::{Parser, ParserError};
use crate::middle_end::utils::EnvironmentStack;
use crate::middle_end::Analyzer;
use colored::Colorize;
use std::error::Error;
use std::process::exit;
use std::{env, fs};

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

    let statements = match parse_source(&contents) {
        Ok(statements) => statements,
        Err(errors) => {
            report_errors(&errors, "parser");
        }
    };

    println!("Statements:");
    for (i, statement) in statements.iter().enumerate() {
        println!("{i}) {statement}");
    }

    analyze_statements(&contents, &statements);
}

fn read_source_file(file_path: &str) -> Result<String, std::io::Error> {
    fs::read_to_string(file_path)
}

fn parse_source(contents: &str) -> Result<Vec<Statement>, Vec<ParserError>> {
    let mut parser = Parser::new(contents);
    parser.parse()
}

fn analyze_statements(contents: &str, statements: &[Statement]) {
    let mut empty_environment = EnvironmentStack::new();
    let mut analyzer = Analyzer::new_with_environment(contents, &mut empty_environment);

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