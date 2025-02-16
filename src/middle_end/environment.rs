use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use crate::frontend::token_type::StaticType;

/// Environment is a symbol table for identifiers
#[derive(Clone)]
pub struct Environment {
    pub(crate) values: HashMap<String, StaticType>
}

impl Environment {
    pub fn new() -> Self {
        Self { values: HashMap::new() }
    }

    pub fn define(&mut self, name: String, static_type: StaticType) {
        self.values.insert(name, static_type);
    }
}

#[derive(Debug)]
pub struct EnvironmentError {
    pub environment_error_type: EnvironmentErrorType
}

#[derive(Debug)]
pub enum EnvironmentErrorType {
    UndefinedVariable(String),
    LastEnvironmentRemoved
}

impl Error for EnvironmentError {}
impl Display for EnvironmentError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.environment_error_type {
            EnvironmentErrorType::UndefinedVariable(name) => { write!(f, "Undefined variable '{name}'") },
            EnvironmentErrorType::LastEnvironmentRemoved => { write!(f, "Last environment was popped from the stack") }
        }
    }
}

impl EnvironmentError {
    pub fn new(environment_error_type: EnvironmentErrorType) -> Self {
        Self { environment_error_type }
    }
}