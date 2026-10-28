//! Call-scoped, whole-array loans. No stored/returned references in this subset.
use super::shared::{AccessKind, Program, ResourceId};
use crate::{
    diagnostic::{Diagnostic, DiagnosticKind, LabelKind},
    frontend::{
        expression::{Expression, ExpressionType as E},
        statement::Statement,
        static_type::StaticType as T,
        utils::SourceLocation,
    },
    semantic::{BodyId, SymbolId, SymbolTable},
};
use std::collections::BTreeMap;

pub(super) fn check(
    statements: &[Statement],
    symbols: &SymbolTable,
    shared: &Program,
) -> Vec<Diagnostic> {
    let mut summary: BTreeMap<BodyId, BTreeMap<SymbolId, bool>> = shared
        .bodies()
        .iter()
        .map(|(id, b)| {
            let mut accesses = BTreeMap::new();
            for a in &b.accesses {
                if let ResourceId::GlobalMemory(g) = a.resource {
                    if symbols.get(g).ty.sequence().is_some() {
                        *accesses.entry(g).or_insert(false) |= a.kind == AccessKind::WriteMemory;
                    }
                }
            }
            (*id, accesses)
        })
        .collect();
    loop {
        let mut changed = false;
        for (id, b) in shared.bodies() {
            for call in &b.calls {
                let accesses = summary
                    .get(&BodyId::Function(call.callee))
                    .cloned()
                    .unwrap_or_default();
                for (g, write) in accesses {
                    let target = summary.get_mut(id).unwrap();
                    match target.get(&g) {
                        Some(old) if *old || !write => {}
                        _ => {
                            target.insert(g, write);
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut checker = Checker {
        symbols,
        summary,
        active: vec![],
        errors: vec![],
    };
    for statement in statements {
        statement.for_each_expression(&mut |e| checker.expression(e, true));
    }
    checker.errors
}

struct Checker<'a> {
    symbols: &'a SymbolTable,
    summary: BTreeMap<BodyId, BTreeMap<SymbolId, bool>>,
    active: Vec<(SymbolId, bool, SourceLocation)>,
    errors: Vec<Diagnostic>,
}
impl Checker<'_> {
    fn access(&mut self, id: SymbolId, write: bool, at: SourceLocation) {
        if let Some((_, _, origin)) = self
            .active
            .iter()
            .find(|(other, mutable, _)| *other == id && (write || *mutable))
        {
            self.errors.push(
                Diagnostic::new(
                    DiagnosticKind::Array {
                        detail: format!(
                            "Conflicting access to '{}' while it is borrowed",
                            self.symbols.get(id).name
                        ),
                    },
                    Some(at),
                )
                .with_label(*origin, LabelKind::ArrayBorrow),
            );
        }
    }
    fn expression(&mut self, e: &Expression, consume: bool) {
        match &e.expression_type {
            E::Identifier(_) if e.static_type.sequence().is_some() => {
                if let Some(id) = e.symbol {
                    self.access(
                        id,
                        consume && matches!(e.static_type, T::FixedArray(..)),
                        e.source_location,
                    );
                }
            }
            E::Borrow {
                expression,
                mutable,
            } => {
                if let Some(id) = expression.symbol {
                    self.access(id, *mutable, e.source_location);
                }
            }
            E::Call { arguments, .. } => {
                let start = self.active.len();
                for argument in arguments {
                    if let T::ArrayRef(_, _, mutable) | T::SliceRef(_, mutable) =
                        argument.static_type
                    {
                        let place = match &argument.expression_type {
                            E::Borrow { expression, .. } => Some(expression.as_ref()),
                            E::Identifier(_) => Some(argument),
                            _ => None,
                        };
                        if let Some(id) = place.and_then(|p| p.symbol) {
                            self.access(id, mutable, argument.source_location);
                            self.active.push((id, mutable, argument.source_location));
                        } else {
                            self.errors.push(Diagnostic::new(DiagnosticKind::Array {detail:"Reference arguments must directly borrow a named array or reborrow a parameter".into()},Some(argument.source_location)));
                        }
                    } else {
                        self.expression(argument, true);
                    }
                }
                if let Some(id) = e.symbol {
                    for (global, write) in self
                        .summary
                        .get(&BodyId::Function(id))
                        .cloned()
                        .unwrap_or_default()
                    {
                        self.access(global, write, e.source_location);
                    }
                }
                self.active.truncate(start);
            }
            E::Index { array, index } => {
                let start = self.active.len();
                if let Some(id) = array.symbol {
                    self.access(id, false, array.source_location);
                    self.active.push((id, false, array.source_location));
                }
                self.expression(index, true);
                self.active.truncate(start);
            }
            E::IndexAssignment {
                array,
                index,
                value,
            } => {
                let start = self.active.len();
                if let Some(id) = array.symbol {
                    self.access(id, true, array.source_location);
                    // Reserve the destination while operands run; RHS may read it,
                    // but cannot move/rebind it or call a mutable borrower.
                    self.active.push((id, false, array.source_location));
                }
                self.expression(index, true);
                self.expression(value, true);
                self.active.truncate(start);
            }
            E::Assignment { expression, .. } => {
                self.expression(expression, true);
                if let Some(id) = e.symbol {
                    if e.static_type.sequence().is_some() {
                        self.access(id, true, e.source_location);
                    }
                }
            }
            E::CapabilityCall { arguments, .. } | E::Text { arguments, .. } => {
                for a in arguments {
                    self.expression(a, false);
                }
            }
            E::Grouping { expression } => self.expression(expression, consume),
            _ => e.for_each_child(|child| self.expression(child, true)),
        }
    }
}
