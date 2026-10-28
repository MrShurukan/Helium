//! One resolved call/access graph for conflict diagnostics and ISR visibility.
use crate::{
    diagnostic::{Diagnostic, DiagnosticKind, LabelKind, SharedAccessReason},
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
    Device(SymbolId),
    GlobalMemory(SymbolId),
    GpioPort(PortId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    UseDevice,
    ReadMemory,
    WriteMemory,
    ReadPin,
    /// Backend must update only this pin atomically; whole-port RMW is forbidden.
    WritePin,
    /// Ordered updates of this pin in DDR/PORT, not an atomic whole-mode change.
    ConfigurePin,
}
impl AccessKind {
    fn writes(self) -> bool {
        matches!(
            self,
            Self::WriteMemory | Self::WritePin | Self::ConfigurePin | Self::UseDevice
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

impl Access {
    fn conflict_reason(
        &self,
        irq: &Self,
        main: &Self,
        symbols: &SymbolTable,
    ) -> SharedAccessReason {
        use crate::frontend::static_type::{IntegerType, SignedIntegerType, UnsignedIntegerType};
        match self.resource {
            ResourceId::Device(_) => SharedAccessReason::Device,
            ResourceId::GpioPort(_)
                if irq.kind == AccessKind::ConfigurePin
                    || main.kind == AccessKind::ConfigurePin =>
            {
                SharedAccessReason::PinConfiguration
            }
            ResourceId::GpioPort(_) => SharedAccessReason::SamePin,
            ResourceId::GlobalMemory(_) => match symbols.get(self.symbol).ty {
                StaticType::Integer(t)
                    if !matches!(
                        t,
                        IntegerType::Signed(SignedIntegerType::Int8)
                            | IntegerType::Unsigned(UnsignedIntegerType::Uns8)
                    ) =>
                {
                    SharedAccessReason::MultiByteMemory
                }
                _ => SharedAccessReason::SharedMemory,
            },
        }
    }

    fn conflicts(&self, other: &Self, symbols: &SymbolTable) -> bool {
        if self.resource != other.resource || !(self.kind.writes() || other.kind.writes()) {
            return false;
        }
        match self.resource {
            ResourceId::GlobalMemory(_) | ResourceId::Device(_) => true,
            ResourceId::GpioPort(_) => match (
                &symbols.get(self.symbol).kind,
                &symbols.get(other.symbol).kind,
            ) {
                (SymbolKind::Pin { bit: a, .. }, SymbolKind::Pin { bit: b, .. }) => a == b,
                _ => unreachable!("GPIO resource must refer to pins"),
            },
        }
    }
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
            SymbolKind::Device => (ResourceId::Device(id), AccessKind::UseDevice),
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
            location: if kind == AccessKind::UseDevice {
                location
            } else {
                location.name(&symbol.name)
            },
        });
    }

    fn expression(e: &Expression, protection: Protection, symbols: &SymbolTable, body: &mut Body) {
        match &e.expression_type {
            E::Borrow {
                expression,
                mutable,
            } => {
                if let Some(id) = expression.symbol {
                    Self::access(
                        id,
                        *mutable,
                        false,
                        protection,
                        e.source_location,
                        symbols,
                        body,
                    );
                }
            }
            E::IndexAssignment {
                array,
                index,
                value,
            } => {
                if let Some(id) = array.symbol {
                    Self::access(
                        id,
                        true,
                        false,
                        protection,
                        array.source_location,
                        symbols,
                        body,
                    );
                }
                Self::expression(index, protection, symbols, body);
                Self::expression(value, protection, symbols, body);
            }
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
                if let SymbolKind::Intrinsic { device, .. } = symbols.get(e.symbol.unwrap()).kind {
                    Self::access(
                        device,
                        true,
                        false,
                        protection,
                        e.source_location,
                        symbols,
                        body,
                    );
                }
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
                initializer,
                name_location,
                ..
            } => {
                if let Some(e) = initializer {
                    Self::expression(e, protection, symbols, body);
                }
                Self::access(
                    s.symbol.unwrap(),
                    true,
                    false,
                    protection,
                    *name_location,
                    symbols,
                    body,
                );
            }
            S::DefDeclaration(pins) => {
                for pin in pins {
                    Self::access(
                        pin.symbol.unwrap(),
                        true,
                        true,
                        protection,
                        pin.name_location,
                        symbols,
                        body,
                    );
                }
            }
            S::Expression(e) | S::Return(Some(e)) => Self::expression(e, protection, symbols, body),
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

    /// A deterministic source path for diagnostics. Caller protection is recorded
    /// but never silently promoted into an ordinary helper's contract.
    fn call_path(
        &self,
        roots: impl IntoIterator<Item = BodyId>,
        target: BodyId,
    ) -> Vec<SourceLocation> {
        let mut pending: std::collections::VecDeque<_> =
            roots.into_iter().map(|id| (id, Vec::new())).collect();
        let mut visited = BTreeSet::new();
        while let Some((id, path)) = pending.pop_front() {
            if id == target {
                return path;
            }
            if !visited.insert(id) {
                continue;
            }
            if let Some(body) = self.bodies.get(&id) {
                for call in &body.calls {
                    let mut next = path.clone();
                    next.push(call.location);
                    pending.push_back((BodyId::Function(call.callee), next));
                }
            }
        }
        Vec::new()
    }

    pub(crate) fn check(&self, symbols: &SymbolTable) -> Vec<Diagnostic> {
        let mut irq: BTreeMap<ResourceId, Vec<(BodyId, &Access)>> = BTreeMap::new();
        let mut main: BTreeMap<ResourceId, Vec<(BodyId, &Access)>> = BTreeMap::new();
        for (id, body) in &self.bodies {
            for access in &body.accesses {
                if self.interrupts.contains(id) {
                    irq.entry(access.resource).or_default().push((*id, access));
                }
                // Setup completes before lifecycle IRQ enable: it cannot race with ISR.
                if self.main.contains(id) {
                    main.entry(access.resource).or_default().push((*id, access));
                }
            }
        }
        let irq_roots = || {
            self.bodies
                .keys()
                .copied()
                .filter(|id| matches!(id, BodyId::Interrupt(_)))
        };
        let mut errors = vec![];
        for (id, body) in &self.bodies {
            // Uncalled and setup-only bodies are still type/contract checked by
            // Analyzer, but do not create a concurrent runtime access.
            if !self.main.contains(id) && !self.interrupts.contains(id) {
                continue;
            }
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
                // Select a genuinely conflicting pair involving this access;
                // unrelated writes on the same port must not become witnesses.
                let pair = if self.main.contains(id) {
                    interrupt
                        .iter()
                        .find(|(_, other)| access.conflicts(other, symbols))
                        .map(|&(irq_id, other)| ((irq_id, other), (*id, access)))
                } else {
                    None
                }
                .or_else(|| {
                    if self.interrupts.contains(id) {
                        main.iter()
                            .find(|(_, other)| access.conflicts(other, symbols))
                            .map(|&(main_id, other)| ((*id, access), (main_id, other)))
                    } else {
                        None
                    }
                });
                let Some(((irq_body, irq_access), (main_body, main_access))) = pair else {
                    continue;
                };
                let mut error = Diagnostic::new(
                    DiagnosticKind::SharedAccess {
                        name: symbols.get(access.symbol).name.clone(),
                        reason: access.conflict_reason(irq_access, main_access, symbols),
                    },
                    Some(access.location),
                )
                .with_label(irq_access.location, LabelKind::InterruptAccess);
                if main_access.location != access.location {
                    error = error.with_label(main_access.location, LabelKind::MainAccess);
                }
                let mut paths = vec![
                    (
                        self.call_path([BodyId::MainLoop], main_body),
                        LabelKind::MainCall,
                    ),
                    (
                        self.call_path(irq_roots(), irq_body),
                        LabelKind::InterruptCall,
                    ),
                ];
                if self.main.contains(id) {
                    paths.push((self.call_path([BodyId::MainLoop], *id), LabelKind::MainCall));
                }
                if self.interrupts.contains(id) {
                    paths.push((self.call_path(irq_roots(), *id), LabelKind::InterruptCall));
                }
                for (path, kind) in paths {
                    for location in path {
                        if !error
                            .labels
                            .iter()
                            .any(|label| label.location == location && label.kind == kind)
                        {
                            error = error.with_label(location, kind.clone());
                        }
                    }
                }
                errors.push(error);
            }
        }
        errors
    }
}
