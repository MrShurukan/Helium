pub mod utils;
mod environment;
pub mod dag_builder;
pub mod linear_ir;
mod error_printer;

use crate::frontend::expression::{Expression, ExpressionType};
use crate::frontend::statement::{SingleDefDeclaration, Statement, StatementType};
use crate::frontend::static_type::{FloatType, IntegerType, SignedIntegerType, StaticType, UnsignedIntegerType};
use crate::frontend::token_type::{KeywordType, LiteralType, PunctuationType, TokenType};
use crate::frontend::utils::{LocatableInSource, SourceLocation};
use crate::middle_end::utils::EnvironmentStack;
use colored::Colorize;
use std::error::Error;
use std::fmt::{Display, Formatter};
use crate::middle_end::error_printer::get_error_marked_line_location;

/// Semantic Analyzer is responsible for statically checking the language and figuring out
/// incorrect operations, like type mismatches, incorrect assignments, variable scopes, etc.
pub struct Analyzer<'a> {
    /// Source file, used to print errors
    pub source: &'a str,
    /// Environment stack. The latest environment on the stack represents the
    /// latest code block. You can access outer environments by moving back from
    /// the end
    environment: &'a mut EnvironmentStack,
    /// List of errors that will be accumulated during the call of `analyze`
    errors: Vec<AnalyzerError>
}

impl Analyzer<'_> {
    pub fn new_with_environment<'a>(source: &'a str, environment_stack: &'a mut EnvironmentStack) -> Analyzer<'a> {
        Analyzer { source, environment: environment_stack, errors: Vec::new() }
    }

    pub fn analyze(mut self, statements: &mut [Statement]) -> Result<(), Vec<AnalyzerError>> {
        for statement in statements {
            self.analyze_statement(statement);
        }

        if self.errors.is_empty() {
            Ok(())
        }
        else {
            Err(self.errors)
        }
    }

    fn analyze_statement(&mut self, statement: &mut Statement) {
        match &mut statement.statement_type {
            StatementType::Expression(expr) => {
                self.analyze_expression(expr);
            },
            StatementType::VariableDeclaration { identifier, static_type, initializer } => {
                match (static_type.as_mut(), initializer.as_mut()) {
                    (Some(static_type), Some(initializer)) => {
                        self.environment.define(identifier.clone(), *static_type);
                        if let Some(expression_type) = self.analyze_expression(initializer) {
                            if !Self::can_implicitly_cast(expression_type, *static_type) {
                                self.errors.push(self.produce_error(
                                    AnalyzerErrorType::VarDeclarationTypeMismatch(*static_type, expression_type),
                                    initializer
                                ));
                            }
                        }
                    },
                    (Some(static_type), None) => {
                        self.environment.define(identifier.clone(), *static_type);
                        // If there is no explicit initializer - create a default one
                        let literal = match static_type {
                            StaticType::Integer(_) => Some(LiteralType::Integer(0)),
                            StaticType::Float(_) => Some(LiteralType::Float(0.0)),
                            StaticType::Boolean => Some(LiteralType::Boolean(false)),
                            StaticType::String => Some(LiteralType::String(String::new())),
                            _ => {
                                self.errors.push(self.produce_error_location(
                                    AnalyzerErrorType::NoDefaultInitializer(*static_type),
                                    &statement.source_location
                                ));
                                None
                            }
                        };

                        if let Some(literal) = literal {
                            *initializer = Some(Expression::new_literal_tokenless(literal));
                        }
                    },
                    (None, Some(initializer)) => {
                        if let Some(mut expression_type) = self.analyze_expression(initializer) {
                            // If there is no explicit type - literals are counted as i32 and f32.
                            match expression_type {
                                StaticType::Integer(IntegerType::Literal) =>
                                    expression_type = StaticType::Integer(IntegerType::Signed(SignedIntegerType::Int32)),
                                StaticType::Float(FloatType::Literal) =>
                                    expression_type = StaticType::Float(FloatType::Float32),
                                _ => {}
                            };
                            *static_type = Some(expression_type);
                            self.environment.define(identifier.clone(), expression_type);
                        }
                        else {
                            self.environment.define(identifier.clone(), StaticType::Unknown);
                        }
                    },
                    (None, None) => {
                        self.environment.define(identifier.clone(), StaticType::Unknown);
                        self.errors.push(self.produce_error_all_line(
                            AnalyzerErrorType::VarDeclarationMissingTypeAndInit,
                            statement.source_location.line_start
                        ));
                    },
                }
            },
            StatementType::DefDeclaration(declarations) => {
                for declaration in declarations.iter() {
                    self.analyze_def_declaration(declaration);
                }
            },
            StatementType::SetupDeclaration(statements) |
            StatementType::LoopDeclaration(statements) |
            StatementType::Block(statements) => self.analyze_block(statements),
            StatementType::If { condition, then_body, else_body } => {
                self.check_condition_boolean(condition);

                self.analyze_statement(then_body);
                if let Some(else_body) = else_body {
                    self.analyze_statement(else_body);
                }
            },
            StatementType::While { condition, body } => {
                self.check_condition_boolean(condition);

                self.analyze_statement(body);
            },
            StatementType::For { identifier, from, to, body } => {
                let from_type = self.analyze_expression(from);
                let to_type = self.analyze_expression(to);

                let mut error_happened = false;
                let mut identifier_type = None;

                if let Some(from_type) = from_type {
                    if !matches!(from_type, StaticType::Integer(_)) {
                        self.errors.push(self.produce_error(
                            AnalyzerErrorType::NonIntegerInFor(from_type),
                            from
                        ));

                        error_happened = true;
                    }
                }

                if let Some(to_type) = to_type {
                    if !matches!(to_type, StaticType::Integer(_)) {
                        self.errors.push(self.produce_error(
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

                self.analyze_statement(body);
            },
        }
    }

    fn analyze_block(&mut self, statements: &mut Vec<Statement>) {
        self.environment.push_new();
        for statement in statements {
            self.analyze_statement(statement);
        }
        self.environment.remove_last().expect("last environment popped");
    }

    fn check_condition_boolean(&mut self, condition: &mut Expression) {
        if let Some(condition_type) = self.analyze_expression(condition) {
            if !matches!(condition_type, StaticType::Boolean) {
                self.errors.push(self.produce_error(
                    AnalyzerErrorType::NonBooleanInCondition(condition_type),
                    condition
                ));
            }
        }
    }

    fn analyze_expression(&mut self, expression: &mut Expression) -> Option<StaticType> {
        let static_type = match &mut expression.expression_type {
            ExpressionType::Binary { left, operator, right } => {
                let types = (self.analyze_expression(left)?, self.analyze_expression(right)?);
                match operator {
                    TokenType::Punctuation(PunctuationType::Plus) => {
                        // Concat
                        // TODO: This assumes everything can be converted to string. Which isn't true
                        if matches!(types.0, StaticType::String) || matches!(types.1, StaticType::String) {
                            Some(StaticType::String)
                        }
                        else {
                            let res1 = self.ensure_number_for_arithmetic(left, types.0);
                            let res2 = self.ensure_number_for_arithmetic(right, types.1);

                            if !res1 || !res2 {
                                None
                            }
                            else {
                                Self::larger_numeric(types.0, types.1)
                            }
                        }
                    },
                    TokenType::Punctuation(PunctuationType::Minus | 
                                           PunctuationType::Slash | 
                                           PunctuationType::Star) => {
                        let res1 = self.ensure_number_for_arithmetic(left, types.0);
                        let res2 = self.ensure_number_for_arithmetic(right, types.1);

                        if !res1 || !res2 {
                            None
                        }
                        else {
                            Self::larger_numeric(types.0, types.1)
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
                        self.errors.push(self.produce_error(
                            AnalyzerErrorType::InvalidBinaryOperator(operator.clone()),
                            expression
                        ));
                        None
                    }
                }
            },
            ExpressionType::Ternary { first, second, third } => {
                let first_type = self.analyze_expression(first);
                if let Some(first_type) = first_type {
                    if !matches!(first_type, StaticType::Boolean) {
                        self.errors.push(self.produce_error(
                            AnalyzerErrorType::NonBooleanInCondition(first_type),
                            first.as_ref()
                        ));
                    }
                }

                let second_type = self.analyze_expression(second);
                let third_type = self.analyze_expression(third);

                if let (Some(second_type), Some(third_type)) = (second_type, third_type) {
                    return if second_type == third_type {
                        Some(second_type)
                    } else {
                        self.errors.push(self.produce_error(
                            AnalyzerErrorType::MismatchTernary(second_type, third_type),
                            expression
                        ));
                        None
                    }
                }

                None
            },
            ExpressionType::Grouping { expression } => self.analyze_expression(expression),
            ExpressionType::Identifier(name) => self.try_get_identifier_type(name, &expression.source_location),
            ExpressionType::Literal { value } => {
                match value {
                    LiteralType::String(_) => { Some(StaticType::String) }
                    LiteralType::Integer(_) => { Some(StaticType::Integer(IntegerType::Literal)) }
                    LiteralType::Float(_) => { Some(StaticType::Float(FloatType::Literal)) }
                    LiteralType::Boolean(_) => { Some(StaticType::Boolean) }
                    LiteralType::PinMode(_) => { Some(StaticType::PinMode) }
                    LiteralType::PinState(_) => { Some(StaticType::PinState) }
                }
            },
            ExpressionType::Unary { operator, right } => {
                match operator {
                    PunctuationType::Bang => Some(StaticType::Boolean),
                    PunctuationType::Minus => {
                        let right_type = self.analyze_expression(right)?;
                        match right_type {
                            StaticType::Integer(_) | StaticType::Float(_) => Some(right_type),
                            t => {
                                self.errors.push(self.produce_error(
                                    AnalyzerErrorType::WrongTypeUnaryOperator(operator.clone(), t),
                                    right.as_ref()
                                ));

                                None
                            }
                        }
                    },

                    _ => {
                        self.errors.push(self.produce_error(
                            AnalyzerErrorType::InvalidUnaryOperator(operator.clone()),
                            expression
                        ));

                        None
                    }
                }
            },
            ExpressionType::Assignment { identifier, expression: assign_expr } => {
                let assignee_type = self.try_get_identifier_type(identifier, &expression.source_location);
                let expression_type = self.analyze_expression(assign_expr)?;

                if let Some(assignee_type) = assignee_type {
                    match (assignee_type, expression_type) {
                        (StaticType::Pin, StaticType::PinState | StaticType::PinMode) => Some(expression_type),
                        
                        (t1, t2) if t1 == t2 => Some(t2),
                        
                        (t1, t2) => {
                            // Safe cast check (i.e. float = int)
                            if Self::can_implicitly_cast(t2, t1) {
                                Some(t1)
                            }
                            else {
                                self.errors.push(self.produce_error(
                                    AnalyzerErrorType::AssignmentTypeMismatch {
                                        assignee_type,
                                        assignment_type: expression_type,
                                    },
                                    assign_expr.as_ref()
                                ));

                                None
                            }
                        }
                    }
                }
                else {
                    None
                }
            }
        };
        
        if let Some(static_type) = static_type {
            if let StaticType::Unknown = static_type {
                self.errors.push(self.produce_error(
                    AnalyzerErrorType::UnknownTypeAssignedToExpression,
                    expression
                ));
            }
            expression.static_type = static_type;
        }
        else {
            self.errors.push(self.produce_error(
                AnalyzerErrorType::UnknownTypeAssignedToExpression,
                expression
            ));
        }

        static_type
    }

    fn ensure_number_for_arithmetic(&mut self, expression: &Expression, static_type: StaticType) -> bool {
        match static_type {
            StaticType::Integer(_) | StaticType::Float(_) => { true }
            _ => {
                self.errors.push(self.produce_error(
                    AnalyzerErrorType::NonNumericType(static_type),
                    expression
                ));
                false
            }
        }
    }

    fn analyze_def_declaration(&mut self, declaration: &SingleDefDeclaration) {
        if self.environment.get_type(&declaration.identifier).is_some() {
            self.errors.push(self.produce_error(
                AnalyzerErrorType::PortNameAlreadyDefined(declaration.identifier.clone()),
                declaration
            ));
        }
        else {
            self.environment.define_global(declaration.identifier.clone(), StaticType::Pin);
        }
    }

    fn try_get_identifier_type(&mut self, identifier: &String, source_location: &SourceLocation) -> Option<StaticType> {
        if let Some(t) = self.environment.get_type(identifier) {
            Some(t)
        }
        else {
            self.errors.push(self.produce_error_location(
                AnalyzerErrorType::IdentifierWasNotDefined(identifier.clone()),
                source_location
            ));
            None
        }
    }

    fn produce_error<T: LocatableInSource>(&self, error_type: AnalyzerErrorType, locatable: &T) -> AnalyzerError {
        AnalyzerError::new(error_type, locatable, self.source)
    }

    fn produce_error_location(&self, error_type: AnalyzerErrorType, location: &SourceLocation) -> AnalyzerError {
        AnalyzerError::new_location(error_type, location, self.source)
    }

    fn produce_error_all_line(&self, error_type: AnalyzerErrorType, line: usize) -> AnalyzerError {
        AnalyzerError::new_all_line(error_type, line, self.source)
    }

    fn larger_numeric(numeric1: StaticType, numeric2: StaticType) -> Option<StaticType> {
        if Self::can_implicitly_cast(numeric1, numeric2) {
            Some(numeric2)
        }
        else if Self::can_implicitly_cast(numeric2, numeric1) {
            Some(numeric1)
        }
        else {
            None
        }
    }
    
    fn can_implicitly_cast(cast_from: StaticType, cast_to: StaticType) -> bool {
        // Just in case - if types match then cast is also allowed
        if cast_from == cast_to { return true; }

        // Implicit casts only work for numerical types right now
        if !matches!(cast_from, StaticType::Integer(_) | StaticType::Float(_)) ||
            !matches!(cast_to, StaticType::Integer(_) | StaticType::Float(_)) {
            return false;
        }
        
        match (cast_from, cast_to) {
            // Integer to Float
            (StaticType::Integer(_), StaticType::Float(_)) |
            // Float to Float (only widening allowed)
            (StaticType::Float(FloatType::Float32), StaticType::Float(FloatType::Float64)) |
            (StaticType::Float(FloatType::Literal), StaticType::Float(_)) |
            // Integer Literals
            // TODO: Check for boundaries! Values may overflow/underflow
            (StaticType::Integer(IntegerType::Literal), StaticType::Integer(_)) => true,

            // Integer to Integer
            (StaticType::Integer(IntegerType::Signed(from)), StaticType::Integer(IntegerType::Signed(to))) => {
                Self::signed_integer_widening(from, to)
            }
            (StaticType::Integer(IntegerType::Unsigned(from)), StaticType::Integer(IntegerType::Unsigned(to))) => {
                Self::unsigned_integer_widening(from, to)
            }
            (StaticType::Integer(IntegerType::Unsigned(from)), StaticType::Integer(IntegerType::Signed(to))) => {
                Self::unsigned_to_signed_widening(from, to)
            }

            // Other cases
            _ => false,
        }
    }

    fn signed_integer_widening(from: SignedIntegerType, to: SignedIntegerType) -> bool {
        matches!((from, to),
                | (SignedIntegerType::Int8,  SignedIntegerType::Int16 |
                                             SignedIntegerType::Int32 |
                                             SignedIntegerType::Int64)
            
                | (SignedIntegerType::Int16, SignedIntegerType::Int32 |
                                             SignedIntegerType::Int64)
            
                | (SignedIntegerType::Int32, SignedIntegerType::Int64))
    }

    fn unsigned_integer_widening(from: UnsignedIntegerType, to: UnsignedIntegerType) -> bool {
        matches!((from, to),
                | (UnsignedIntegerType::Uns8,  UnsignedIntegerType::Uns16 |
                                               UnsignedIntegerType::Uns32 |
                                               UnsignedIntegerType::Uns64)
            
                | (UnsignedIntegerType::Uns16, UnsignedIntegerType::Uns32 |
                                               UnsignedIntegerType::Uns64)
            
                | (UnsignedIntegerType::Uns32, UnsignedIntegerType::Uns64))
    }

    fn unsigned_to_signed_widening(from: UnsignedIntegerType, to: SignedIntegerType) -> bool {
        matches!((from, to),
                | (UnsignedIntegerType::Uns8,  SignedIntegerType::Int16 | 
                                               SignedIntegerType::Int32 | 
                                               SignedIntegerType::Int64)
            
                | (UnsignedIntegerType::Uns16, SignedIntegerType::Int32 | 
                                               SignedIntegerType::Int64)
            
                | (UnsignedIntegerType::Uns32, SignedIntegerType::Int64))
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
        Self::new_location(error_type, location, source)
    }

    fn new_location(error_type: AnalyzerErrorType, location: &SourceLocation, source: &str) -> AnalyzerError {
        AnalyzerError {
            location: get_error_marked_line_location(location, source),
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
            location: get_error_marked_line_location(&location, source),
            error_type,
            line_start: location.line_start,
            line_end: location.line_end,
        }
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
    VarDeclarationMissingTypeAndInit,
    UnknownTypeAssignedToExpression,
    NoDefaultInitializer(StaticType)
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
                format!("Type mismatch: cannot assign '{assignment_type}' to '{assignee_type}' (maybe use an explicit cast?)"),
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
                format!("Type mismatch: variable is of type '{t1}', but initializer is '{t2}' (maybe use an explicit cast?)"),
            AnalyzerErrorType::VarDeclarationMissingTypeAndInit =>
                "Variable declaration must either specify a type (: Type) or have an initializer (= expr)".to_string(),
            AnalyzerErrorType::UnknownTypeAssignedToExpression =>
                "Internal error occured. Wasn't able to evaluate this expression. This is an internal compiler bug".to_string(),
            AnalyzerErrorType::NoDefaultInitializer(static_type) =>
                format!("There is no default value for '{static_type}', provide an initializer")
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