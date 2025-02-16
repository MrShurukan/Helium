pub mod utils;
mod environment;

use crate::frontend::expression::{Expression, ExpressionType};
use crate::frontend::statement::{SingleDefDeclaration, Statement, StatementType};
use crate::frontend::token_type::{LiteralType, PunctuationType, StaticType};
use crate::frontend::utils::LocatableInSource;
use crate::middle_end::utils::EnvironmentStack;
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};

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

    // TODO: Create new environments when processing blocks. Remove them after processing
    fn analyze_statement(&mut self, statement: &Statement) -> Result<(), AnalyzerError> {
        match &statement.statement_type {
            StatementType::Expression(expr) => {
                self.analyze_expression(expr)?;
                Ok(())
            },
            StatementType::VariableDeclaration { identifier, initializer } => {
                let expression_type = self.analyze_expression(initializer)?;
                self.environment.define(identifier.clone(), expression_type);
                Ok(())
            },
            StatementType::DefDeclaration(declarations) => {
                // TODO: This will only yield one error at a time in a block
                for declaration in declarations.iter() {
                    self.analyze_def_declaration(declaration)?;
                }

                Ok(())
            },
            StatementType::SetupDeclaration(statement) |
            StatementType::LoopDeclaration(statement) => self.analyze_statement(statement),
            StatementType::Block(statements) => {
                // TODO: This will only yield one error at a time in a block
                for statement in statements.iter() {
                    self.analyze_statement(statement)?;
                }

                Ok(())
            },
            StatementType::If { condition, then_body, else_body } => {
                let condition_type = self.analyze_expression(condition)?;
                if !matches!(condition_type, StaticType::Boolean) {
                    return Err(self.produce_error(
                        AnalyzerErrorType::NonBooleanInCondition(condition_type),
                        condition
                    ))
                }
                
                self.analyze_statement(then_body)?;
                if let Some(else_body) = else_body {
                    self.analyze_statement(else_body)?;
                }
                
                Ok(())
            },
            StatementType::While { condition, body } => {
                let condition_type = self.analyze_expression(condition)?;
                if !matches!(condition_type, StaticType::Boolean) {
                    return Err(self.produce_error(
                        AnalyzerErrorType::NonBooleanInCondition(condition_type),
                        condition
                    ))
                }

                self.analyze_statement(body)
            },
            StatementType::For { identifier, from, to, body } => {
                let from_type = self.analyze_expression(from)?;
                let to_type = self.analyze_expression(to)?;
                
                // TODO: Ints should be separated into their own group for ease of checking
                // TODO: This should check for all ints
                if from_type != StaticType::Int64 {
                    return Err(self.produce_error(
                        AnalyzerErrorType::NonIntegerInFor(from_type),
                        from
                    ));
                }

                // TODO: Same goes here
                if to_type != StaticType::Int64 {
                    return Err(self.produce_error(
                        AnalyzerErrorType::NonIntegerInFor(to_type),
                        to
                    ));
                }
                
                self.environment.define(identifier.clone(), from_type);
                self.analyze_statement(body)
            },
        }
    }

    fn analyze_expression(&mut self, expression: &Expression) -> Result<StaticType, AnalyzerError> {
        match &expression.expression_type {
            ExpressionType::Binary { left, operator, right } => {
                let types = (self.analyze_expression(left)?, self.analyze_expression(right)?);
                todo!()
            },
            ExpressionType::Ternary { first, second, third } => {
                let first_type = self.analyze_expression(first)?;
                if !matches!(first_type, StaticType::Boolean) {
                    return Err(self.produce_error(
                        AnalyzerErrorType::NonBooleanInCondition(first_type),
                        first.as_ref()
                    ));
                }

                let second_type = self.analyze_expression(second)?;
                let third_type = self.analyze_expression(third)?;

                if second_type == third_type {
                    Ok(second_type)
                }
                else {
                    Err(self.produce_error(
                        AnalyzerErrorType::MismatchTernary(second_type, third_type),
                        expression
                    ))
                }
            },
            ExpressionType::Grouping { expression } => self.analyze_expression(expression),
            ExpressionType::Literal { value } => {
                match value {
                    LiteralType::Identifier(name) => { self.try_get_identifier_type(name, expression) }
                    LiteralType::String(_) => { Ok(StaticType::String) }
                    LiteralType::Integer(_) => { Ok(StaticType::Int64) }
                    LiteralType::Float(_) => { Ok(StaticType::Float64) }
                    LiteralType::Boolean(_) => { Ok(StaticType::Boolean) }
                    LiteralType::PinMode(_) => { Ok(StaticType::PinMode)}
                    LiteralType::PinState(_) => { Ok(StaticType::PinState)}
                }
            },
            ExpressionType::Unary { operator, right } => {
                match operator {
                    PunctuationType::Bang => Ok(StaticType::Boolean),
                    PunctuationType::Minus => {
                        let right_type = self.analyze_expression(right)?;
                        match right_type {
                            t @ (StaticType::Int8 |
                                StaticType::Int16 |
                                StaticType::Int32 |
                                StaticType::Int64 |
                                StaticType::Uns8 |
                                StaticType::Uns16 |
                                StaticType::Uns32 |
                                StaticType::Uns64 |
                                StaticType::Float32 |
                                StaticType::Float64) => Ok(t),
                            t => Err(self.produce_error(
                                AnalyzerErrorType::WrongTypeUnaryOperator(operator.clone(), t),
                                right.as_ref()
                            ))
                        }
                    },

                    _ => Err(self.produce_error(
                        AnalyzerErrorType::InvalidUnaryOperator(operator.clone()),
                        expression
                    ))
                }
            },
            ExpressionType::Assignment { identifier, expression } => {
                let assignee_type = self.try_get_identifier_type(identifier, expression)?;

                let expression_type = self.analyze_expression(expression)?;
                if assignee_type == expression_type {
                   Ok(expression_type)
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

    fn analyze_def_declaration(&mut self, declaration: &SingleDefDeclaration) -> Result<StaticType, AnalyzerError> {
        todo!("Check ports");
    }

    fn try_get_identifier_type(&mut self, identifier: &String, expression: &Expression) -> Result<StaticType, AnalyzerError> {
        self.environment.get_type(identifier)
            .ok_or_else(|| self.produce_error(
                AnalyzerErrorType::IdentifierWasNotDefined(identifier.clone()),
                expression
            ))
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
            let line = source.split('\n').nth(line_number - 1);
            if line.is_none() {
                return format!("[Internal Analyzer Error] Couldn't read line {line_number} from source file");
            }

            // Prep the line itself
            // i.e. `14. var stuff = (1 + 3 +`
            let line = line.unwrap();
            let line_number_string = format!("{line_number}. ");
            result_string += "\n";
            result_string += &line_number_string;
            result_string += line;

            // On the first line we calculate the spaces as an offset to an expression.
            // Marker is calculated until the end of the line unless this line is also the end
            // of the expression
            if line_number == location.line_start {
                let spaces = " ".repeat(line_number_string.len() + location.start_offset);

                if line_number == location.line_end {
                    let marker = "*".repeat(location.end_offset - location.start_offset).red();
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
                    line.chars().take_while(char::is_ascii_whitespace).count());

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
    AssignmentTypeMismatch { assignee_type: StaticType, assignment_type: StaticType },
    InvalidUnaryOperator(PunctuationType),
    WrongTypeUnaryOperator(PunctuationType, StaticType),
    NonBooleanInCondition(StaticType),
    MismatchTernary(StaticType, StaticType),
    NonIntegerInFor(StaticType)
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
                format!("'{name}' is undefined in this scope"),
            AnalyzerErrorType::InvalidAssignmentTarget(name) =>
                format!("Cannot assign to '{name}'"),
            AnalyzerErrorType::AssignmentTypeMismatch { assignee_type, assignment_type } =>
                format!("Type mismatch: cannot assign '{assignment_type}' to '{assignee_type}"),
            AnalyzerErrorType::InvalidUnaryOperator(punctuation) =>
                format!("Invalid unary operator: {punctuation}"),
            AnalyzerErrorType::WrongTypeUnaryOperator(p, t) =>
                format!("Unary operator '{p}' cannot work with type '{t}'"),
            AnalyzerErrorType::NonBooleanInCondition(t) =>
                format!("Expected boolean in a condition, got '{t}'"),
            AnalyzerErrorType::MismatchTernary(t1, t2) =>
                format!("Two branches of ternary operator have different types ('{t1}' and '{t2}'). They must be the same"),
            AnalyzerErrorType::NonIntegerInFor(t) =>
                format!("For loops only support integers, instead found '{t}'")
        }.red();

        // Add tabs to each line of the location so it shifts to the right
        let location: String = self.location.split('\n')
            .fold(String::new(), |mut output, line| {
                output += "\t"; output += line; output += "\n";
                output
            });
        
        write!(f, "{error_header}\n{message}")?;
        write!(f, "\n\t{}\n{}\n", "Happened here:".green(), location.trim_end())
    }
}