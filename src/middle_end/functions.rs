use super::*;

#[derive(Clone)]
pub(super) struct Signature {
    parameters: Vec<T>,
    result: Option<T>,
}

impl Analyzer {
    pub(super) fn collect_signatures(&mut self, statements: &[Statement]) {
        for statement in statements {
            let S::FunctionDeclaration {
                identifier,
                parameters,
                return_type,
                ..
            } = &statement.statement_type
            else {
                continue;
            };

            for (ty, span) in parameters
                .iter()
                .map(|p| (p.static_type, p.source_location))
                .chain(return_type.map(|t| (t, statement.source_location)))
            {
                if !matches!(ty, T::Integer(_) | T::Boolean | T::PinState) {
                    self.error(DiagnosticKind::FunctionSignatureType { ty }, span);
                }
            }

            if self.functions.contains_key(identifier) {
                self.error(
                    DiagnosticKind::DuplicateName {
                        name: identifier.clone(),
                    },
                    statement.source_location,
                );
            } else {
                self.functions.insert(
                    identifier.clone(),
                    Signature {
                        parameters: parameters.iter().map(|p| p.static_type).collect(),
                        result: *return_type,
                    },
                );
            }
        }
    }

    pub(super) fn call(
        &mut self,
        name: &str,
        arguments: &mut [Expression],
        span: SourceLocation,
    ) -> Option<T> {
        if self.global_initializer {
            self.error(DiagnosticKind::GlobalInitializerCall, span);
        }

        let signature = self.functions.get(name).cloned();
        for (index, argument) in arguments.iter_mut().enumerate() {
            self.expression(argument);
            if let Some(target) = signature.as_ref().and_then(|s| s.parameters.get(index)) {
                self.coerce(argument, *target);
            }
        }

        let Some(signature) = signature else {
            let kind = if self.lookup(name).is_some() {
                DiagnosticKind::NotCallable { name: name.into() }
            } else {
                DiagnosticKind::UndefinedName { name: name.into() }
            };
            self.error(kind, span);
            return Some(T::Unknown);
        };
        if arguments.len() != signature.parameters.len() {
            self.error(
                DiagnosticKind::ArgumentCount {
                    name: name.into(),
                    expected: signature.parameters.len(),
                    actual: arguments.len(),
                },
                span,
            );
        }
        if let Some(caller) = &self.current_function {
            self.calls
                .entry(caller.clone())
                .or_default()
                .push((name.into(), span));
        }
        signature.result
    }

    pub(super) fn check_recursion(&mut self) {
        // Visit in deterministic name/source order, including unused functions.
        fn visit(
            name: &str,
            edges: &BTreeMap<String, Vec<(String, SourceLocation)>>,
            state: &mut HashMap<String, u8>,
            stack: &mut Vec<String>,
            errors: &mut Vec<Diagnostic>,
        ) {
            state.insert(name.into(), 1);
            stack.push(name.into());
            for (callee, span) in edges.get(name).into_iter().flatten() {
                match state.get(callee).copied().unwrap_or(0) {
                    0 => visit(callee, edges, state, stack, errors),
                    1 => {
                        let start = stack.iter().position(|n| n == callee).unwrap();
                        let mut chain = stack[start..].to_vec();
                        chain.push(callee.clone());
                        errors.push(Diagnostic::new(
                            DiagnosticKind::RecursiveCalls { chain },
                            Some(*span),
                        ));
                    }
                    _ => {}
                }
            }
            stack.pop();
            state.insert(name.into(), 2);
        }

        let mut state = HashMap::new();
        for name in self.functions.keys() {
            if !state.contains_key(name) {
                visit(
                    name,
                    &self.calls,
                    &mut state,
                    &mut Vec::new(),
                    &mut self.errors,
                );
            }
        }
    }
}

pub(super) fn body_returns(body: &[Statement]) -> bool {
    body.iter().any(statement_returns)
}

fn statement_returns(statement: &Statement) -> bool {
    match &statement.statement_type {
        S::Return(_) => true,
        S::Block(body) => body_returns(body),
        S::If {
            then_body,
            else_body: Some(other),
            ..
        } => statement_returns(then_body) && statement_returns(other),
        // Conservative: loops may execute zero times, even with a constant test.
        _ => false,
    }
}
