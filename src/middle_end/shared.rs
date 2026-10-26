//! One resolved call/access graph for conflict diagnostics and ISR visibility.
use crate::{
    diagnostic::{Diagnostic, DiagnosticKind, LabelKind},
    frontend::{
        expression::{Expression, ExpressionType as E},
        statement::{Statement, StatementType as S},
        static_type::StaticType,
        utils::SourceLocation,
    },
    semantic::{BodyId, PortId, SymbolId, SymbolKind, SymbolTable},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceId {
    GlobalMemory(SymbolId),
    GpioPort(PortId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    ReadMemory,
    WriteMemory,
    ReadPin,
    WritePin,
    ConfigurePin,
}
impl AccessKind {
    fn writes(self) -> bool {
        matches!(
            self,
            Self::WriteMemory | Self::WritePin | Self::ConfigurePin
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    None,
    Block,
    FunctionContract,
    Setup,
    Initialization,
    Interrupt,
}

#[derive(Debug)]
pub struct Access {
    pub symbol: SymbolId,
    pub resource: ResourceId,
    pub kind: AccessKind,
    pub protection: Protection,
    pub location: SourceLocation,
}

#[derive(Debug)]
pub struct Call {
    pub callee: SymbolId,
    pub location: SourceLocation,
    pub protection: Protection,
}

#[derive(Debug, Default)]
pub struct Body {
    pub accesses: Vec<Access>,
    pub calls: Vec<Call>,
}

#[derive(Debug, Default)]
pub struct Program {
    bodies: BTreeMap<BodyId, Body>,
    setup: BTreeSet<BodyId>,
    main: BTreeSet<BodyId>,
    interrupts: BTreeSet<BodyId>,
    interrupt_globals: BTreeSet<SymbolId>,
}

impl Program {
    pub fn bodies(&self) -> &BTreeMap<BodyId, Body> {
        &self.bodies
    }
    pub fn setup_bodies(&self) -> &BTreeSet<BodyId> {
        &self.setup
    }
    pub fn main_bodies(&self) -> &BTreeSet<BodyId> {
        &self.main
    }
    pub fn interrupt_bodies(&self) -> &BTreeSet<BodyId> {
        &self.interrupts
    }
    pub fn interrupt_globals(&self) -> &BTreeSet<SymbolId> {
        &self.interrupt_globals
    }

    pub(crate) fn collect(statements: &[Statement], symbols: &SymbolTable) -> Self {
        let mut result = Self::default();
        let mut interrupt_roots = vec![];
        let mut initialization = Body::default();
        for statement in statements {
            let (id, body, protection) = match &statement.statement_type {
                S::FunctionDeclaration {
                    body, no_interrupt, ..
                } => (
                    BodyId::Function(statement.symbol.unwrap()),
                    body,
                    if no_interrupt.is_some() {
                        Protection::FunctionContract
                    } else {
                        Protection::None
                    },
                ),
                S::SetupDeclaration(body) => (BodyId::Setup, body, Protection::Setup),
                S::LoopDeclaration(body) => (BodyId::MainLoop, body, Protection::None),
                S::TimerDeclaration(timer) => {
                    let id = BodyId::Interrupt(statement.symbol.unwrap());
                    interrupt_roots.push(id);
                    (id, &timer.body, Protection::Interrupt)
                }
                _ => {
                    Self::statement(
                        statement,
                        Protection::Initialization,
                        symbols,
                        &mut initialization,
                    );
                    continue;
                }
            };
            let mut summary = Body::default();
            for statement in body {
                Self::statement(statement, protection, symbols, &mut summary);
            }
            result.bodies.insert(id, summary);
        }
        result.bodies.insert(BodyId::Initialization, initialization);
        result.setup = result.reachable([BodyId::Initialization, BodyId::Setup]);
        result.main = result.reachable([BodyId::MainLoop]);
        result.interrupts = result.reachable(interrupt_roots);
        result.interrupt_globals = result
            .interrupts
            .iter()
            .filter_map(|id| result.bodies.get(id))
            .flat_map(|body| &body.accesses)
            .filter_map(|access| match access.resource {
                ResourceId::GlobalMemory(id) => Some(id),
                _ => None,
            })
            .collect();
        result
    }

    fn access(
        id: SymbolId,
        write: bool,
        configure: bool,
        protection: Protection,
        location: SourceLocation,
        symbols: &SymbolTable,
        body: &mut Body,
    ) {
        let symbol = symbols.get(id);
        let (resource, kind) = match symbol.kind {
            SymbolKind::Global => (
                ResourceId::GlobalMemory(id),
                if write {
                    AccessKind::WriteMemory
                } else {
                    AccessKind::ReadMemory
                },
            ),
            SymbolKind::Pin { port, .. } => (
                ResourceId::GpioPort(port),
                if configure {
                    AccessKind::ConfigurePin
                } else if write {
                    AccessKind::WritePin
                } else {
                    AccessKind::ReadPin
                },
            ),
            _ => return,
        };
        body.accesses.push(Access {
            symbol: id,
            resource,
            kind,
            protection,
            location: location.name(&symbol.name),
        });
    }

    fn expression(e: &Expression, protection: Protection, symbols: &SymbolTable, body: &mut Body) {
        match &e.expression_type {
            E::Identifier(_) => Self::access(
                e.symbol.unwrap(),
                false,
                false,
                protection,
                e.source_location,
                symbols,
                body,
            ),
            E::Assignment { expression, .. } => {
                Self::expression(expression, protection, symbols, body);
                Self::access(
                    e.symbol.unwrap(),
                    true,
                    expression.static_type == StaticType::PinMode,
                    protection,
                    e.source_location,
                    symbols,
                    body,
                );
            }
            E::Call { arguments, .. } => {
                body.calls.push(Call {
                    callee: e.symbol.unwrap(),
                    location: e.source_location,
                    protection,
                });
                for argument in arguments {
                    Self::expression(argument, protection, symbols, body);
                }
            }
            _ => e.for_each_child(|child| Self::expression(child, protection, symbols, body)),
        }
    }

    fn statement(s: &Statement, protection: Protection, symbols: &SymbolTable, body: &mut Body) {
        match &s.statement_type {
            S::NoInterrupt(statements) => {
                for s in statements {
                    Self::statement(s, Protection::Block, symbols, body);
                }
            }
            S::Block(statements) => {
                for s in statements {
                    Self::statement(s, protection, symbols, body);
                }
            }
            S::VariableDeclaration {
                initializer: Some(e),
                ..
            }
            | S::Expression(e)
            | S::Return(Some(e)) => Self::expression(e, protection, symbols, body),
            S::If {
                condition,
                then_body,
                else_body,
            } => {
                Self::expression(condition, protection, symbols, body);
                Self::statement(then_body, protection, symbols, body);
                if let Some(s) = else_body {
                    Self::statement(s, protection, symbols, body);
                }
            }
            S::While {
                condition,
                body: inner,
            } => {
                Self::expression(condition, protection, symbols, body);
                Self::statement(inner, protection, symbols, body);
            }
            S::For {
                from,
                to,
                body: inner,
                ..
            } => {
                Self::expression(from, protection, symbols, body);
                Self::expression(to, protection, symbols, body);
                Self::statement(inner, protection, symbols, body);
            }
            _ => {}
        }
    }

    fn reachable(&self, roots: impl IntoIterator<Item = BodyId>) -> BTreeSet<BodyId> {
        let mut pending: Vec<_> = roots.into_iter().collect();
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if visited.insert(id) {
                if let Some(body) = self.bodies.get(&id) {
                    pending.extend(body.calls.iter().map(|c| BodyId::Function(c.callee)));
                }
            }
        }
        visited
    }

    pub(crate) fn check(&self, symbols: &SymbolTable) -> Vec<Diagnostic> {
        let mut irq: BTreeMap<ResourceId, Vec<&Access>> = BTreeMap::new();
        let mut main: BTreeMap<ResourceId, Vec<&Access>> = BTreeMap::new();
        for (id, body) in &self.bodies {
            for access in &body.accesses {
                if self.interrupts.contains(id) {
                    irq.entry(access.resource).or_default().push(access);
                }
                // Preserve existing policy during the identity migration.
                if self.main.contains(id)
                    || (self.setup.contains(id) && *id != BodyId::Initialization)
                {
                    main.entry(access.resource).or_default().push(access);
                }
            }
        }
        let mut errors = vec![];
        for body in self.bodies.values() {
            for access in &body.accesses {
                if access.protection != Protection::None {
                    continue;
                }
                let Some(interrupt) = irq.get(&access.resource) else {
                    continue;
                };
                let Some(main) = main.get(&access.resource) else {
                    continue;
                };
                if !interrupt.iter().chain(main).any(|a| a.kind.writes()) {
                    continue;
                }
                errors.push(
                    Diagnostic::new(
                        DiagnosticKind::SharedAccess {
                            name: symbols.get(access.symbol).name.clone(),
                        },
                        Some(access.location),
                    )
                    .with_label(interrupt[0].location, LabelKind::InterruptAccess),
                );
            }
        }
        errors
    }
}
