pub mod utils;
mod environment;

use std::error::Error;
use std::fmt::{format, Display, Formatter};
use crate::frontend::expression::{Expression, ExpressionType};
use crate::frontend::statement::{Statement, StatementType};
use crate::frontend::token_type::StaticType;
use crate::frontend::utils::LocatableInSource;
use crate::middle_end::utils::EnvironmentStack;

/// Semantic Analyzer is responsible for statically checking the language and figuring out
/// incorrect operations, like type mismatches, incorrect assignments, variable scopes, etc.
pub struct Analyzer<'a> {
    /// Source file, used to print errors
    pub source: &'a str,
    /// Environment stack. The latest environment on the stack represents the
    /// latest code block. You can access outer environments by moving back from 
    /// the end
    environment: &'a mut EnvironmentStack
}

impl Analyzer<'_> {
    pub fn new_with_environment<'a>(source: &'a str, environment_stack: &'a mut EnvironmentStack) -> Analyzer<'a> {
        Analyzer { source, environment: environment_stack }
    }
    
    pub fn analyze(&mut self, statements: &[Statement]) -> Result<(), Vec<AnalyzerError>> {
        let mut errors: Vec<AnalyzerError> = Vec::new();
        for statement in statements {
            if let Err(e) = self.analyze_statement(statement) {
                errors.push(e);
            }
        }
        
        if errors.is_empty() {
            Ok(())
        }
        else {
            Err(errors)
        }
    }
    
    fn analyze_statement(&mut self, statement: &Statement) -> Result<(), AnalyzerError> {
        match &statement.statement_type {
            StatementType::Expression(expr) => {
                self.analyze_expression(expr)
            },
            StatementType::VariableDeclaration { identifier, initializer } => {
                let expression_type = self.evaluate_type(initializer)?;
                self.environment.define(identifier.clone(), expression_type);
                Ok(())
            },
            StatementType::DefDeclaration(_) => todo!(),
            StatementType::SetupDeclaration(_) => todo!(),
            StatementType::LoopDeclaration(_) => todo!(),
            StatementType::Block(_) => todo!(),
            StatementType::If { .. } => todo!(),
            StatementType::While { .. } => todo!(),
            StatementType::For { .. } => todo!(),
        }
    }
    
    fn analyze_expression(&mut self, expression: &Expression) -> Result<(), AnalyzerError> {
        match &expression.expression_type {
            ExpressionType::Binary { .. } => todo!(),
            ExpressionType::Ternary { .. } => todo!(),
            ExpressionType::Grouping { .. } => todo!(),
            ExpressionType::Literal { .. } => todo!(),
            ExpressionType::Unary { .. } => todo!(),
            ExpressionType::Assignment { identifier, expression } => {
                let assignee_type = self.environment.get_type(identifier)
                    .ok_or_else(|| self.produce_error(
                        AnalyzerErrorType::IdentifierWasNotDefined(identifier.clone()),
                        expression.as_ref()
                    ))?;
                
                let expression_type = self.evaluate_type(expression)?;
                if matches!(assignee_type, expression_type) {
                   Ok(()) 
                }
                else {
                    Err(self.produce_error(
                        AnalyzerErrorType::AssignmentTypeMismatch {
                            assignee_type,
                            assignment_type: expression_type,
                        },
                        expression.as_ref()
                    ))
                }
            }
        }
    }
    
    fn evaluate_type(&mut self, expression: &Expression) -> Result<StaticType, AnalyzerError> {
        todo!()
    }
    
    fn produce_error<T: LocatableInSource>(&self, error_type: AnalyzerErrorType, locatable: &T) -> AnalyzerError {
        AnalyzerError::new(error_type, locatable, self.source)
    }
}


#[derive(Debug)]
pub struct AnalyzerError {
    error_type: AnalyzerErrorType,
    line_start: usize,
    line_end: usize,
    location: String
}

impl AnalyzerError {
    fn new<T: LocatableInSource>(error_type: AnalyzerErrorType, locatable: &T, source: &str) -> AnalyzerError {
        let location = locatable.get_location();
        AnalyzerError {
            location: AnalyzerError::get_error_marked_line(locatable, source),
            error_type,
            line_start: location.line_start,
            line_end: location.line_end,
        }
    }

    // TODO: Refactor that a bit
    /// Gets a helper string for error displaying with a marker on every second line.
    /// ### Example
    /// This function is used to create a string that looks something like this:
    /// ```
    /// 14. var stuff = (1 + 3 +
    ///                  *******
    /// 15.              13 + "hi" +
    ///                  ************
    /// 16.              19);
    ///                  **
    /// ```
    /// Resulting string consists of multiple lines and has a marker denoted by a sequence of
    /// `*` on every other line. The lines are parsed from `expression` by the `expression.line_start` and
    /// the `expression.line_end` numbers. Lines are fetched from `source` string.
    pub fn get_error_marked_line<T: LocatableInSource>(locatable: &T, source: &str) -> String {
        let location = locatable.get_location();
        
        let mut result_string = String::new();
        for line_number in location.line_start..=location.line_end {
            let line = source.split("\n").nth(line_number - 1);
            if line.is_none() {
                return format!("[Internal Analyzer Error] Couldn't read line {} from source file", line_number);
            }

            // Prep the line itself
            // i.e. `14. var stuff = (1 + 3 +`
            let line = line.unwrap();
            let line_number_string = format!("{}. ", line_number);
            result_string += "\n";
            result_string += &line_number_string;
            result_string += line;

            // On the first line we calculate the spaces as an offset to an expression.
            // Marker is calculated until the end of the line unless this line is also the end
            // of the expression
            if line_number == location.line_start {
                let spaces = " ".repeat(line_number_string.len() + location.start_offset - 1);

                if line_number == location.line_end {
                    let marker = "*".repeat(location.end_offset - location.start_offset);
                    result_string += &format!("\n{spaces}{marker}");
                    return result_string;
                }

                let marker = "*".repeat(line.len() - location.start_offset);
                result_string += &format!("\n{spaces}{marker}");
            }
            // On every other line we skip the whitespace at the beginning
            // and then start drawing the marker. If this is the last line
            // then we draw according to the offset. Otherwise we draw until the end of the line
            else {
                let spaces = " ".repeat(line_number_string.len() +
                    line.chars().take_while(|c| c.is_ascii_whitespace()).count());

                if line_number == location.line_end {
                    let marker = "*".repeat(location.end_offset - spaces.len() + 2);
                    result_string += &format!("\n{spaces}{marker}");
                    return result_string;
                }

                let marker = "*".repeat(line.len() - spaces.len());
                result_string += &format!("\n{spaces}{marker}");
            }
        }


        result_string
    }
}

#[derive(Debug)]
pub enum AnalyzerErrorType {
    IdentifierWasNotDefined(String),
    InvalidAssignmentTarget(String),
    AssignmentTypeMismatch { assignee_type: StaticType, assignment_type: StaticType }
}

impl Error for AnalyzerError {}
impl Display for AnalyzerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let error_header =
            if self.line_start == self.line_end
                { format!("[line {}] Analyzer Error", self.line_start) }
            else
                { format!("[lines {}-{}] Analyzer Error", self.line_start, self.line_end) };
        
        let message = match &self.error_type {
            AnalyzerErrorType::IdentifierWasNotDefined(name) =>
                &format!("'{name}' is undefined in this scope"),
            AnalyzerErrorType::InvalidAssignmentTarget(name) =>
                &format!("Cannot assign to '{name}'"),
            AnalyzerErrorType::AssignmentTypeMismatch { assignee_type, assignment_type } =>
                &format!("Type mismatch: cannot assign '{assignment_type}' to '{assignee_type}")
        };

        write!(f, "{error_header}\n{message}")
    }
}