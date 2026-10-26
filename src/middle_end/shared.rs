//! Whole-program interrupt reachability and protected-access checks.
//! GPIO is conservatively grouped by port because writes can be read/modify/write.
use crate::{
    diagnostic::{Diagnostic, DiagnosticKind, LabelKind},
    frontend::{
        expression::{Expression, ExpressionType as E},
        statement::{Statement, StatementType as S},
        utils::SourceLocation,
    },
};
use std::collections::{BTreeMap, BTreeSet};

struct Access {
    name: String,
    resource: String,
    write: bool,
    protected: bool,
    location: SourceLocation,
}

#[derive(Default)]
struct Body {
    accesses: Vec<Access>,
    calls: Vec<String>,
}

struct Program {
    resources: BTreeMap<String, String>,
    globals: BTreeSet<String>,
    bodies: BTreeMap<String, Body>,
    interrupts: Vec<String>,
}

impl Program {
    fn collect(statements: &[Statement]) -> Self {
        let mut result = Self {
            resources: BTreeMap::new(),
            globals: BTreeSet::new(),
            bodies: BTreeMap::new(),
            interrupts: Vec::new(),
        };
        for statement in statements {
            match &statement.statement_type {
                S::VariableDeclaration { identifier, .. } => {
                    result
                        .resources
                        .insert(identifier.clone(), identifier.clone());
                    result.globals.insert(identifier.clone());
                }
                S::DefDeclaration(pins) => {
                    for pin in pins {
                        result
                            .resources
                            .insert(pin.identifier.clone(), format!("$port{}", pin.pin.port));
                    }
                }
                _ => {}
            }
        }
        for statement in statements {
            let (name, body, protected) = match &statement.statement_type {
                S::FunctionDeclaration {
                    identifier,
                    body,
                    no_interrupt,
                    ..
                } => (identifier.clone(), body, no_interrupt.is_some()),
                S::SetupDeclaration(body) => ("$setup".into(), body, true),
                S::LoopDeclaration(body) => ("$loop".into(), body, false),
                S::TimerDeclaration(timer) => {
                    let name = format!("$isr:{}", timer.identifier);
                    result.interrupts.push(name.clone());
                    (name, &timer.body, true)
                }
                _ => continue,
            };
            let mut summary = Body::default();
            for statement in body {
                result.statement(statement, protected, &mut summary);
            }
            result.bodies.insert(name, summary);
        }
        result
    }

    fn access(
        &self,
        name: &str,
        write: bool,
        protected: bool,
        location: SourceLocation,
        body: &mut Body,
    ) {
        if let Some(resource) = self.resources.get(name) {
            body.accesses.push(Access {
                name: name.into(),
                resource: resource.clone(),
                write,
                protected,
                location: location.name(name),
            });
        }
    }

    fn expression(&self, expression: &Expression, protected: bool, body: &mut Body) {
        match &expression.expression_type {
            E::Identifier(name) => {
                self.access(name, false, protected, expression.source_location, body)
            }
            E::Assignment {
                identifier,
                expression: value,
            } => {
                self.expression(value, protected, body);
                self.access(
                    identifier,
                    true,
                    protected,
                    expression.source_location,
                    body,
                );
            }
            E::Call {
                identifier,
                arguments,
            } => {
                body.calls.push(identifier.clone());
                for argument in arguments {
                    self.expression(argument, protected, body);
                }
            }
            E::Binary { left, right, .. } => {
                self.expression(left, protected, body);
                self.expression(right, protected, body);
            }
            E::Unary { right, .. } | E::Grouping { expression: right } => {
                self.expression(right, protected, body)
            }
            E::Ternary {
                first,
                second,
                third,
            } => {
                for value in [first, second, third] {
                    self.expression(value, protected, body);
                }
            }
            E::Literal { .. } => {}
        }
    }

    fn statement(&self, statement: &Statement, protected: bool, body: &mut Body) {
        match &statement.statement_type {
            S::NoInterrupt(statements) => {
                for s in statements {
                    self.statement(s, true, body);
                }
            }
            S::Block(statements) => {
                for s in statements {
                    self.statement(s, protected, body);
                }
            }
            S::VariableDeclaration {
                initializer: Some(value),
                ..
            }
            | S::Expression(value)
            | S::Return(Some(value)) => self.expression(value, protected, body),
            S::If {
                condition,
                then_body,
                else_body,
            } => {
                self.expression(condition, protected, body);
                self.statement(then_body, protected, body);
                if let Some(other) = else_body {
                    self.statement(other, protected, body);
                }
            }
            S::While {
                condition,
                body: inner,
            } => {
                self.expression(condition, protected, body);
                self.statement(inner, protected, body);
            }
            S::For {
                from,
                to,
                body: inner,
                ..
            } => {
                self.expression(from, protected, body);
                self.expression(to, protected, body);
                self.statement(inner, protected, body);
            }
            _ => {}
        }
    }

    fn reachable(&self, roots: impl IntoIterator<Item = String>) -> BTreeSet<String> {
        let mut pending: Vec<_> = roots.into_iter().collect();
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if visited.insert(name.clone()) {
                if let Some(body) = self.bodies.get(&name) {
                    pending.extend(body.calls.iter().cloned());
                }
            }
        }
        visited
    }
}

pub(crate) fn interrupt_globals(statements: &[Statement]) -> BTreeSet<String> {
    let program = Program::collect(statements);
    program
        .reachable(program.interrupts.clone())
        .iter()
        .filter_map(|name| program.bodies.get(name))
        .flat_map(|body| &body.accesses)
        .filter(|access| program.globals.contains(&access.name))
        .map(|access| access.name.clone())
        .collect()
}

pub(super) fn check(statements: &[Statement]) -> Vec<Diagnostic> {
    let program = Program::collect(statements);
    let interrupt_bodies = program.reachable(program.interrupts.clone());
    let main_bodies = program.reachable(["$setup".into(), "$loop".into()]);
    let mut interrupt_accesses: BTreeMap<&str, Vec<&Access>> = BTreeMap::new();
    let mut main_accesses: BTreeMap<&str, Vec<&Access>> = BTreeMap::new();
    for (name, body) in &program.bodies {
        for access in &body.accesses {
            if interrupt_bodies.contains(name) {
                interrupt_accesses
                    .entry(&access.resource)
                    .or_default()
                    .push(access);
            }
            if main_bodies.contains(name) {
                main_accesses
                    .entry(&access.resource)
                    .or_default()
                    .push(access);
            }
        }
    }
    let mut errors = Vec::new();
    for body in program.bodies.values() {
        for access in &body.accesses {
            if access.protected {
                continue;
            }
            let Some(interrupt) = interrupt_accesses.get(access.resource.as_str()) else {
                continue;
            };
            let Some(main) = main_accesses.get(access.resource.as_str()) else {
                continue;
            };
            if !interrupt.iter().chain(main).any(|other| other.write) {
                continue;
            }
            errors.push(
                Diagnostic::new(
                    DiagnosticKind::SharedAccess {
                        name: access.name.clone(),
                    },
                    Some(access.location),
                )
                .with_label(interrupt[0].location, LabelKind::InterruptAccess),
            );
        }
    }
    errors
}
