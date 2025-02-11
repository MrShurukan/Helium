use std::{env, fs};
use std::process::exit;
use colored::Colorize;
use crate::parser::{Parser, ParserError};

mod parser;

// TODO: Read up on exit codes. Maybe use something that already makes sense
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: helium [main source file path]");
        exit(1);
    }

    let file_path = args.get(1).unwrap();
    match fs::read_to_string(file_path) {
        Ok(contents) => {
            // Right now the program will only output a list of scanned tokens and then exit
            let mut parser = Parser::new(&contents);
            match parser.parse() {
                Ok(statements) => { 
                    for statement in statements {
                        println!("{statement:?}");
                    }
                },
                Err(errors) => report_errors(errors.as_slice())
            }
        },
        Err(err) => {
            eprintln!("{}", "Encountered a problem while reading the source file".red());
            eprintln!("{err}");
            exit(2);
        }
    }
}

fn report_errors(errors: &[ParserError]) {
    eprintln!("{}", "Wasn't able to parse the file".red());
    let error_errors = if errors.len() == 1 { "error" } else { "errors" };
    eprintln!("{} {error_errors} found.", errors.len());

    for (i, error) in errors.iter().enumerate() {
        eprintln!("{}. {error}", i+1);
    }
    exit(3);
}