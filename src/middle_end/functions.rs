use super::*;

#[derive(Clone)]
pub(super) struct Signature {
    pub(super) id: SymbolId,
    no_interrupt: Option<SourceLocation>,
    parameters: Vec<T>,
    result: Option<T>,
    pub(super) name_location: SourceLocation,
    parameter_locations: Vec<SourceLocation>,
    pub(super) return_location: Option<SourceLocation>,
}

impl Analyzer {
    pub(super) fn collect_signatures(&mut self, statements: &mut [Statement]) {
        for statement in statements {
            let S::FunctionDeclaration {
                no_interrupt,
                identifier,
                parameters,
                return_type,
                name_location,
                return_location,
                ..
            } = &statement.statement_type
            else {
                continue;
            };

            for (ty, span) in parameters
                .iter()
                .map(|p| (p.static_type, p.type_location))
                .chain(return_type.map(|t| (t, return_location.unwrap())))
            {
                if !matches!(ty, T::Integer(_) | T::Boolean | T::PinState) {
                    self.error(DiagnosticKind::FunctionSignatureType { ty }, span);
                }
            }

            if let Some(previous) = self.functions.get(identifier) {
                self.errors.push(
                    Diagnostic::new(
                        DiagnosticKind::DuplicateName {
                            name: identifier.clone(),
                        },
                        Some(*name_location),
                    )
                    .with_label(previous.name_location, LabelKind::PreviousDeclaration),
                );
            } else {
                let id = self.symbols.add(Symbol {
                    name: identifier.clone(),
                    kind: SymbolKind::Function {
                        parameters: parameters.iter().map(|p| p.static_type).collect(),
                        result: *return_type,
                        requires_no_interrupt: no_interrupt.is_some(),
                    },
                    ty: return_type.unwrap_or(T::NoValue),
                    declaration: *name_location,
                    owner: None,
                });
                statement.symbol = Some(id);
                self.functions.insert(
                    identifier.clone(),
                    Signature {
                        id,
                        no_interrupt: *no_interrupt,
                        parameters: parameters.iter().map(|p| p.static_type).collect(),
                        result: *return_type,
                        name_location: *name_location,
                        parameter_locations: parameters.iter().map(|p| p.type_location).collect(),
                        return_location: *return_location,
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
                self.coerce_with_origin(
                    argument,
                    *target,
                    signature.as_ref().unwrap().parameter_locations[index],
                );
            }
        }

        let Some(signature) = signature else {
            let kind = if self.lookup(name).is_some() {
                DiagnosticKind::NotCallable { name: name.into() }
            } else {
                DiagnosticKind::UndefinedName { name: name.into() }
            };
            let mut error = Diagnostic::new(kind, Some(span.name(name)));
            if let Some(location) = self
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name).map(|id| self.symbols.get(*id).declaration))
            {
                error = error.with_label(location, LabelKind::PreviousDeclaration);
            }
            self.errors.push(error);
            return Some(T::Unknown);
        };
        if let Some(requirement) = signature.no_interrupt {
            if !self.interrupts_disabled {
                self.errors.push(
                    Diagnostic::new(
                        DiagnosticKind::InterruptContext { name: name.into() },
                        Some(span),
                    )
                    .with_label(requirement, LabelKind::InterruptRequirement),
                );
            }
        }

        if arguments.len() != signature.parameters.len() {
            self.errors.push(
                Diagnostic::new(
                    DiagnosticKind::ArgumentCount {
                        name: name.into(),
                        expected: signature.parameters.len(),
                        actual: arguments.len(),
                    },
                    Some(span),
                )
                .with_label(signature.name_location, LabelKind::FunctionDeclaration),
            );
        }
        if let Some(caller) = &self.current_function {
            self.calls
                .entry(self.functions[caller].id)
                .or_default()
                .push((signature.id, span));
        }
        signature.result
    }

    pub(super) fn check_recursion(&mut self) {
        fn visit(
            id: SymbolId,
            edges: &BTreeMap<SymbolId, Vec<(SymbolId, SourceLocation)>>,
            symbols: &SymbolTable,
            state: &mut HashMap<SymbolId, u8>,
            stack: &mut Vec<SymbolId>,
            errors: &mut Vec<Diagnostic>,
        ) {
            state.insert(id, 1);
            stack.push(id);
            for (callee, span) in edges.get(&id).into_iter().flatten() {
                match state.get(callee).copied().unwrap_or(0) {
                    0 => visit(*callee, edges, symbols, state, stack, errors),
                    1 => {
                        let start = stack.iter().position(|n| n == callee).unwrap();
                        let mut chain = stack[start..].to_vec();
                        chain.push(*callee);
                        let mut error = Diagnostic::new(
                            DiagnosticKind::RecursiveCalls {
                                chain: chain
                                    .iter()
                                    .map(|id| symbols.get(*id).name.clone())
                                    .collect(),
                            },
                            Some(*span),
                        );
                        for pair in chain.windows(2) {
                            if let Some((_, location)) =
                                edges[&pair[0]].iter().find(|(id, _)| *id == pair[1])
                            {
                                if location != span {
                                    error = error.with_label(*location, LabelKind::RecursiveCall);
                                }
                            }
                        }
                        errors.push(error);
                    }
                    _ => {}
                }
            }
            stack.pop();
            state.insert(id, 2);
        }
        let mut state = HashMap::new();
        for signature in self.functions.values() {
            if !state.contains_key(&signature.id) {
                visit(
                    signature.id,
                    &self.calls,
                    &self.symbols,
                    &mut state,
                    &mut Vec::new(),
                    &mut self.errors,
                );
            }
        }
    }
}

/// Keep only paths that can fall through the entire sequence. A later return
/// discharges earlier branch witnesses; a return inside a loop does not.
pub(super) fn fallthrough(
    body: &[Statement],
    end: SourceLocation,
) -> Vec<(SourceLocation, LabelKind)> {
    let mut exits = vec![(end, LabelKind::Fallthrough)];

    for statement in body {
        if exits.is_empty() {
            break;
        }

        match &statement.statement_type {
            S::Return(_) => exits.clear(),
            S::Block(body) | S::NoInterrupt(body) => {
                let inner = fallthrough(body, statement.source_location.closing());
                if inner.is_empty() {
                    exits.clear();
                } else if inner
                    .iter()
                    .any(|(_, kind)| *kind != LabelKind::Fallthrough)
                {
                    exits = inner;
                }
            }
            S::If {
                condition,
                then_body,
                else_body,
            } => {
                exits = fallthrough(
                    std::slice::from_ref(then_body.as_ref()),
                    then_body.source_location.closing(),
                );
                if let Some(other) = else_body {
                    exits.extend(fallthrough(
                        std::slice::from_ref(other.as_ref()),
                        other.source_location.closing(),
                    ));
                } else {
                    exits.push((condition.source_location, LabelKind::FalseBranch));
                }
            }
            S::While { condition, .. } => {
                exits.push((condition.source_location, LabelKind::LoopMayNotRun));
            }
            S::For { from, to, .. } => exits.push((
                SourceLocation::new_from_start_end(from.source_location, to.source_location),
                LabelKind::LoopMayNotRun,
            )),
            _ => {}
        }
    }

    exits
}
