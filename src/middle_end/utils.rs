use crate::frontend::token_type::StaticType;
use crate::middle_end::environment::{Environment, EnvironmentError, EnvironmentErrorType};

pub struct EnvironmentStack {
    environments: Vec<Environment>
}

impl EnvironmentStack {
    pub fn new() -> Self {
        Self { environments: vec![Environment::new()] }
    }

    fn last(&mut self) -> &mut Environment {
        self.environments
            .iter_mut()
            .nth_back(0)
            .expect("Environment stack must always have at least one environment")
    }

    pub fn define(&mut self, name: String, static_type: StaticType) {
        self.last().define(name, static_type);
    }

    pub fn get_type(&self, name: &String) -> Option<StaticType> {
        for environment in self.environments.iter().rev() {
            match environment.values.get(name) {
                // Try to get the value from the current environment
                Some(t) => return Some(*t),
                // Otherwise continue and try our luck in the next environment in the stack
                None => continue
            }
        }

        None
    }

    // pub fn assign(&mut self, name: String, static_type: StaticType) -> Result<(), EnvironmentError> {
    //     for environment in self.environments.iter_mut().rev() {
    //         // Try to set the value in the current environment
    //         if environment.values.contains_key(&name) {
    //             environment.values.insert(name, static_type);
    //             return Ok(());
    //         }
    //         else {
    //             // If not found, try moving up by one environment
    //             continue;
    //         }
    //     }
    // 
    //     Err(EnvironmentError::new(EnvironmentErrorType::UndefinedVariable(name)))
    // }

    pub fn push_new(&mut self) {
        self.environments.push(Environment::new());
    }

    pub fn remove_last(&mut self) -> Result<(), EnvironmentError> {
        if self.environments.len() == 1 {
            Err(EnvironmentError::new(EnvironmentErrorType::LastEnvironmentRemoved))
        }
        else {
            self.environments.pop();
            Ok(())
        }
    }
}