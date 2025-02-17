use crate::frontend::static_type::StaticType;
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

    fn first(&mut self) -> &mut Environment {
        self.environments
            .get_mut(0)
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

    pub fn define_global(&mut self, name: String, static_type: StaticType) {
        self.first().define(name, static_type);
    }

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