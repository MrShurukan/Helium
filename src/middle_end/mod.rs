pub mod utils;
mod environment;

use std::error::Error;
use std::fmt::{Display, Formatter};
use crate::frontend::statement::Statement;
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
        
        
        Ok(())
    }
}

#[derive(Debug)]
pub struct AnalyzerError {
    error_type: AnalyzerErrorType
}

#[derive(Debug)]
pub enum AnalyzerErrorType {
    
}

impl Error for AnalyzerError {}
impl Display for AnalyzerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.error_type { 
            
        }
    }
}