//! Resolved source identities. IDs belong to one analyzed program, not to IR.
use crate::frontend::{statement::Statement, static_type::StaticType, utils::SourceLocation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(pub(crate) usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(pub(crate) usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BodyId {
    Initialization,
    Setup,
    MainLoop,
    Function(SymbolId),
    Interrupt(SymbolId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    Device,
    Intrinsic {
        device: SymbolId,
        operation: crate::intrinsics::Intrinsic,
    },
    Global,
    Local,
    Parameter,
    Function {
        parameters: Vec<StaticType>,
        result: Option<StaticType>,
        requires_no_interrupt: bool,
    },
    Pin {
        port: PortId,
        bit: usize,
    },
    Timer,
}

#[derive(Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub ty: StaticType,
    pub declaration: SourceLocation,
    pub owner: Option<BodyId>,
}

#[derive(Debug, Default)]
pub struct SymbolTable {
    symbols: Vec<Symbol>,
    ports: Vec<String>,
}

impl SymbolTable {
    pub fn get(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0]
    }

    pub fn iter(&self) -> impl Iterator<Item = (SymbolId, &Symbol)> {
        self.symbols
            .iter()
            .enumerate()
            .map(|(i, s)| (SymbolId(i), s))
    }

    pub fn port_name(&self, id: PortId) -> &str {
        &self.ports[id.0]
    }

    pub(crate) fn add(&mut self, symbol: Symbol) -> SymbolId {
        let id = SymbolId(self.symbols.len());
        self.symbols.push(symbol);
        id
    }

    pub(crate) fn port(&mut self, name: &str) -> PortId {
        if let Some(i) = self.ports.iter().position(|p| p == name) {
            return PortId(i);
        }
        let id = PortId(self.ports.len());
        self.ports.push(name.into());
        id
    }
}

/// Produced only after successful semantic analysis. Keep the tree and its
/// identities together; consumers cannot mutate one independently of the other.
#[derive(Debug)]
pub struct AnalyzedProgram {
    pub(crate) values: crate::values::Contracts,
    pub sources: std::sync::Arc<crate::diagnostic::source::SourceSet>,
    pub(crate) template_checks: Vec<Statement>,
    pub(crate) warnings: Vec<crate::diagnostic::Diagnostic>,
    pub(crate) effects: std::collections::BTreeMap<BodyId, crate::effects::Summary>,
    pub(crate) statements: Vec<Statement>,
    pub(crate) symbols: SymbolTable,
    pub(crate) shared: crate::middle_end::shared::Program,
}

impl AnalyzedProgram {
    pub fn value_contracts(&self) -> &crate::values::Contracts {
        &self.values
    }
    pub fn warnings(&self) -> &[crate::diagnostic::Diagnostic] {
        &self.warnings
    }
    pub fn effects(&self) -> &std::collections::BTreeMap<BodyId, crate::effects::Summary> {
        &self.effects
    }
    pub fn statements(&self) -> &[Statement] {
        &self.statements
    }
    pub fn symbols(&self) -> &SymbolTable {
        &self.symbols
    }
    pub fn shared_analysis(&self) -> &crate::middle_end::shared::Program {
        &self.shared
    }
}
