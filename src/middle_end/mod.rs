pub mod utils;
mod environment;

use crate::frontend::expression::{Expression, ExpressionType};
use crate::frontend::statement::{SingleDefDeclaration, Statement, StatementType};
use crate::frontend::token_type::{KeywordType, LiteralType, PunctuationType, TokenType};
use crate::frontend::utils::{LocatableInSource, SourceLocation};
use crate::middle_end::utils::EnvironmentStack;
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};
use crate::frontend::static_type::{FloatType, IntegerType, StaticType};

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
            self.analyze_statement(statement, &mut errors);
        }

        if errors.is_empty() {
            Ok(())
        }
        else {
            Err(errors)
        }
    }

    fn analyze_statement(&mut self, statement: &Statement, errors: &mut Vec<AnalyzerError>) {
        match &statement.statement_type {
            StatementType::Expression(expr) => {
                self.analyze_expression(expr, errors);
            },
            StatementType::VariableDeclaration { identifier, static_type, initializer } => {
                match (static_type, initializer) {
                    (Some(static_type), Some(initializer)) => {
                        self.environment.define(identifier.clone(), *static_type);
                        if let Some(expression_type) = self.analyze_expression(initializer, errors) {
                            if expression_type != *static_type {
                                errors.push(self.produce_error(
                                    AnalyzerErrorType::VarDeclarationTypeMismatch(*static_type, expression_type),
                                    initializer
                                ));
                            }
                        }
                    },
                    (Some(static_type), None) => {
                        self.environment.define(identifier.clone(), *static_type);
                    },
                    (None, Some(initializer)) => {
                        if let Some(expression_type) = self.analyze_expression(initializer, errors) {
                            self.environment.define(identifier.clone(), expression_type);
                        }
                        else {
                            self.environment.define(identifier.clone(), StaticType::Unknown);
                        }
                    },
                    (None, None) => {
                        self.environment.define(identifier.clone(), StaticType::Unknown);
                        errors.push(self.produce_error_all_line(
                            AnalyzerErrorType::VarDeclarationMissingTypeAndInit,
                            statement.source_location.line_start
                        ));
                    },
                }
            },
            StatementType::DefDeclaration(declarations) => {
                for declaration in declarations.iter() {
                    self.analyze_def_declaration(declaration, errors);
                }
            },
            StatementType::SetupDeclaration(statement) |
            StatementType::LoopDeclaration(statement) => self.analyze_statement(statement, errors),
            StatementType::Block(statements) => {
                self.environment.push_new();
                for statement in statements.iter() {
                    self.analyze_statement(statement, errors);
                }
                self.environment.remove_last().expect("last environment popped");
            },
            StatementType::If { condition, then_body, else_body } => {
                self.check_condition_boolean(errors, condition);

                self.analyze_statement(then_body, errors);
                if let Some(else_body) = else_body {
                    self.analyze_statement(else_body, errors);
                }
            },
            StatementType::While { condition, body } => {
                self.check_condition_boolean(errors, condition);

                self.analyze_statement(body, errors);
            },
            StatementType::For { identifier, from, to, body } => {
                let from_type = self.analyze_expression(from, errors);
                let to_type = self.analyze_expression(to, errors);

                let mut error_happened = false;
                let mut identifier_type = None;

                if let Some(from_type) = from_type {
                    if !matches!(from_type, StaticType::Integer(_)) {
                        errors.push(self.produce_error(
                            AnalyzerErrorType::NonIntegerInFor(from_type),
                            from
                        ));

                        error_happened = true;
                    }
                }

                if let Some(to_type) = to_type {
                    if !matches!(to_type, StaticType::Integer(_)) {
                        errors.push(self.produce_error(
                            AnalyzerErrorType::NonIntegerInFor(to_type),
                            from
                        ));

                        error_happened = true;
                    }
                    else if !error_happened {
                        identifier_type = Some(to_type);
                    }
                }

                if let Some(identifier_type) = identifier_type {
                    self.environment.define(identifier.clone(), identifier_type);
                }

                self.analyze_statement(body, errors);
            },
        }
    }

    fn check_condition_boolean(&mut self, errors: &mut Vec<AnalyzerError>, condition: &Expression) {
        if let Some(condition_type) = self.analyze_expression(condition, errors) {
            if !matches!(condition_type, StaticType::Boolean) {
                errors.push(self.produce_error(
                    AnalyzerErrorType::NonBooleanInCondition(condition_type),
                    condition
                ));
            }
        }
    }

    fn analyze_expression(&mut self, expression: &Expression, errors: &mut Vec<AnalyzerError>) -> Option<StaticType> {
        match &expression.expression_type {
            ExpressionType::Binary { left, operator, right } => {
                let types = (self.analyze_expression(left, errors)?, self.analyze_expression(right, errors)?);
                match operator {
                    TokenType::Punctuation(PunctuationType::Plus) => {
                        // Concat
                        // TODO: This assumes everything can be converted to string. Which isn't true
                        if matches!(types.0, StaticType::String) || matches!(types.1, StaticType::String) {
                            Some(StaticType::String)
                        }
                        else {
                            let res1 = self.ensure_number_for_arithmetic(left, types.0, errors);
                            let res2 = self.ensure_number_for_arithmetic(right, types.1, errors);

                            if res1.is_err() || res2.is_err() {
                                None
                            }
                            else {
                                Some(Self::larger_numeric(types.0, types.1))
                            }
                        }
                    },
                    TokenType::Punctuation(PunctuationType::Minus | 
                                           PunctuationType::Slash | 
                                           PunctuationType::Star) => {
                        let res1 = self.ensure_number_for_arithmetic(left, types.0, errors);
                        let res2 = self.ensure_number_for_arithmetic(right, types.1, errors);

                        if res1.is_err() || res2.is_err() {
                            None
                        }
                        else {
                            Some(Self::larger_numeric(types.0, types.1))
                        }
                    },
                    // TODO: This isn't right, but this will have to do for now
                    TokenType::Keyword(KeywordType::Or | KeywordType::And) |
                    TokenType::Punctuation(PunctuationType::EqualEqual | 
                                           PunctuationType::BangEqual  |
                                           PunctuationType::Greater |
                                           PunctuationType::GreaterEqual |
                                           PunctuationType::Less |
                                           PunctuationType::LessEqual) => Some(StaticType::Boolean),
                    _ => {
                        errors.push(self.produce_error(
                            AnalyzerErrorType::InvalidBinaryOperator(operator.clone()),
                            expression
                        ));
                        None
                    }
                }
            },
            ExpressionType::Ternary { first, second, third } => {
                let first_type = self.analyze_expression(first, errors);
                if let Some(first_type) = first_type {
                    if !matches!(first_type, StaticType::Boolean) {
                        errors.push(self.produce_error(
                            AnalyzerErrorType::NonBooleanInCondition(first_type),
                            first.as_ref()
                        ));
                    }
                }

                let second_type = self.analyze_expression(second, errors);
                let third_type = self.analyze_expression(third, errors);

                if let (Some(second_type), Some(third_type)) = (second_type, third_type) {
                    return if second_type == third_type {
                        Some(second_type)
                    } else {
                        errors.push(self.produce_error(
                            AnalyzerErrorType::MismatchTernary(second_type, third_type),
                            expression
                        ));
                        None
                    }
                }

                None
            },
            ExpressionType::Grouping { expression } => self.analyze_expression(expression, errors),
            ExpressionType::Literal { value } => {
                match value {
                    LiteralType::Identifier(name) => { self.try_get_identifier_type(name, expression, errors) }
                    LiteralType::String(_) => { Some(StaticType::String) }
                    LiteralType::Integer(_) => { Some(StaticType::Integer(IntegerType::Int64)) }
                    LiteralType::Float(_) => { Some(StaticType::Float(FloatType::Float64)) }
                    LiteralType::Boolean(_) => { Some(StaticType::Boolean) }
                    LiteralType::PinMode(_) => { Some(StaticType::PinMode) }
                    LiteralType::PinState(_) => { Some(StaticType::PinState) }
                }
            },
            ExpressionType::Unary { operator, right } => {
                match operator {
                    PunctuationType::Bang => Some(StaticType::Boolean),
                    PunctuationType::Minus => {
                        let right_type = self.analyze_expression(right, errors)?;
                        match right_type {
                            StaticType::Integer(_) | StaticType::Float(_) => Some(right_type),
                            t => {
                                errors.push(self.produce_error(
                                    AnalyzerErrorType::WrongTypeUnaryOperator(operator.clone(), t),
                                    right.as_ref()
                                ));

                                None
                            }
                        }
                    },

                    _ => {
                        errors.push(self.produce_error(
                            AnalyzerErrorType::InvalidUnaryOperator(operator.clone()),
                            expression
                        ));

                        None
                    }
                }
            },
            ExpressionType::Assignment { identifier, expression } => {
                let assignee_type = self.try_get_identifier_type(identifier, expression, errors);
                let expression_type = self.analyze_expression(expression, errors)?;

                if let Some(assignee_type) = assignee_type {
                    return match (assignee_type, expression_type) {
                        (StaticType::Pin, StaticType::PinState | StaticType::PinMode) => Some(expression_type),
                        
                        (t1, t2) if t1 == t2 => Some(t2),
                        
                        (t1, t2) => {
                            // TODO: Cast check (i.e. float = int)
                            
                            errors.push(self.produce_error(
                                AnalyzerErrorType::AssignmentTypeMismatch {
                                    assignee_type,
                                    assignment_type: expression_type,
                                },
                                expression.as_ref()
                            ));

                            None
                        }
                    }
                }

                None
            }
        }
    }

    fn ensure_number_for_arithmetic(&self, expression: &Expression, static_type: StaticType, errors: &mut Vec<AnalyzerError>) -> Result<(), ()> {
        match static_type {
            StaticType::Integer(_) | StaticType::Float(_) => { Ok(())}
            _ => {
                errors.push(self.produce_error(
                    AnalyzerErrorType::NonNumericType(static_type),
                    expression
                ));
                Err(())
            }
        }
    }

    fn analyze_def_declaration(&mut self, declaration: &SingleDefDeclaration, errors: &mut Vec<AnalyzerError>) {
        if self.environment.get_type(&declaration.identifier).is_some() {
            errors.push(self.produce_error(
                AnalyzerErrorType::PortNameAlreadyDefined(declaration.identifier.clone()),
                declaration
            ));
        }
        else {
            self.environment.define_global(declaration.identifier.clone(), StaticType::Pin);
        }
    }

    fn try_get_identifier_type(&mut self, identifier: &String, expression: &Expression, errors: &mut Vec<AnalyzerError>) -> Option<StaticType> {
        if let Some(t) = self.environment.get_type(identifier) {
            Some(t)
        }
        else {
            errors.push(self.produce_error(
                AnalyzerErrorType::IdentifierWasNotDefined(identifier.clone()),
                expression
            ));
            None
        }
    }

    fn produce_error<T: LocatableInSource>(&self, error_type: AnalyzerErrorType, locatable: &T) -> AnalyzerError {
        AnalyzerError::new(error_type, locatable, self.source)
    }

    fn produce_error_all_line(&self, error_type: AnalyzerErrorType, line: usize) -> AnalyzerError {
        AnalyzerError::new_all_line(error_type, line, self.source)
    }

    fn larger_numeric(numeric1: StaticType, numeric2: StaticType) -> StaticType {
        let rank1 = Self::numeric_rank(numeric1);
        let rank2 = Self::numeric_rank(numeric2);

        // Return the larger ranked type
        if rank1 >= rank2 {
            numeric1
        } else {
            numeric2
        }
    }

    fn numeric_rank(typ: StaticType) -> u8 {
        match typ {
            StaticType::Float(FloatType::Float64) => 9,
            StaticType::Float(FloatType::Float32) => 8,
            StaticType::Integer(IntegerType::Int64) => 7,
            StaticType::Integer(IntegerType::Int32) => 6,
            StaticType::Integer(IntegerType::Int16) => 5,
            StaticType::Integer(IntegerType::Int8) => 4,
            StaticType::Integer(IntegerType::Uns64) => 3,
            StaticType::Integer(IntegerType::Uns32) => 2,
            StaticType::Integer(IntegerType::Uns16) => 1,
            StaticType::Integer(IntegerType::Uns8) => 0,
            _ => panic!("Non-numeric type passed to numeric_rank function (larger_numeric)")
        }
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

    fn new_all_line(error_type: AnalyzerErrorType, line: usize, source: &str) -> AnalyzerError {
        let line_len = source
            .split('\n')
            .nth(line-1)
            .expect("Couldn't get line from source (new_all_line)")
            .len();
        
        let location = SourceLocation {
            line_start: line,
            line_end: line,
            start_offset: 0,
            end_offset: line_len,
        };
        
        AnalyzerError {
            location: AnalyzerError::get_error_marked_line_location(&location, source),
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
        Self::get_error_marked_line_location(location, source)
    }
    
    pub fn get_error_marked_line_location(location: &SourceLocation, source: &str) -> String {
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

                let marker = "*".repeat(line.len() - location.start_offset).red();
                result_string += &format!("\n{spaces}{marker}");
            }
            // On every other line we skip the whitespace at the beginning
            // and then start drawing the marker. If this is the last line
            // then we draw according to the offset. Otherwise we draw until the end of the line
            else {
                let spaces = " ".repeat(line_number_string.len() +
                    line.chars().take_while(char::is_ascii_whitespace).count());

                if line_number == location.line_end {
                    let marker = "*".repeat(location.end_offset - spaces.len() + 2).red();
                    result_string += &format!("\n{spaces}{marker}");
                    return result_string;
                }

                let marker = "*".repeat(line.len() - spaces.len()).red();
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
    InvalidBinaryOperator(TokenType),
    WrongTypeUnaryOperator(PunctuationType, StaticType),
    NonBooleanInCondition(StaticType),
    MismatchTernary(StaticType, StaticType),
    NonIntegerInFor(StaticType),
    NonNumericType(StaticType),
    PortNameAlreadyDefined(String),
    VarDeclarationTypeMismatch(StaticType, StaticType),
    VarDeclarationMissingTypeAndInit
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
                format!("Type mismatch: cannot assign '{assignment_type}' to '{assignee_type}'"),
            AnalyzerErrorType::InvalidUnaryOperator(punctuation) =>
                format!("Invalid unary operator: {punctuation}"),
            AnalyzerErrorType::WrongTypeUnaryOperator(p, t) =>
                format!("Unary operator '{p}' cannot work with type '{t}'"),
            AnalyzerErrorType::NonBooleanInCondition(t) =>
                format!("Expected boolean in a condition, got '{t}'"),
            AnalyzerErrorType::MismatchTernary(t1, t2) =>
                format!("Two branches of ternary operator have different types ('{t1}' and '{t2}'). They must be the same"),
            AnalyzerErrorType::NonIntegerInFor(t) =>
                format!("For loops only support integers, instead found '{t}'"),
            AnalyzerErrorType::InvalidBinaryOperator(token_type) =>
                format!("'{token_type:?}' can't be used as a binary operator"),
            AnalyzerErrorType::NonNumericType(t) =>
                format!("Expected a numeric type (integer or a float), but got '{t}'"),
            AnalyzerErrorType::PortNameAlreadyDefined(identifier) =>
                format!("Name '{identifier}' is already in use by another port or a variable"),
            AnalyzerErrorType::VarDeclarationTypeMismatch(t1, t2) =>
                format!("Type mismatch: variable is of type '{t1}', but initializer is '{t2}'"),
            AnalyzerErrorType::VarDeclarationMissingTypeAndInit =>
                "Variable declaration must either specify a type (: Type) or have an initializer (= expr)".to_string(),
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