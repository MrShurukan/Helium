//! Forward ownership dataflow over resolved source identities, before IR/optimization.
//! A state records values moved on any incoming path, with a source witness.
//! This pass does not claim to check loans: borrow syntax/storage comes separately.
use crate::{
    diagnostic::{Diagnostic, DiagnosticKind, LabelKind},
    frontend::{
        expression::{Expression, ExpressionType as E},
        statement::{Statement, StatementType as S},
        token_type::{KeywordType as K, TokenType},
        utils::SourceLocation,
    },
    semantic::{SymbolId, SymbolKind, SymbolTable},
    values::{Contracts, Transfer},
};
use std::collections::BTreeMap;

type State = BTreeMap<SymbolId, SourceLocation>;

fn join(mut left: State, right: State) -> State {
    for (id, origin) in right {
        left.entry(id).or_insert(origin);
    }
    left
}

fn merge(left: Option<State>, right: Option<State>) -> Option<State> {
    match (left, right) {
        (Some(a), Some(b)) => Some(join(a, b)),
        (one, None) | (None, one) => one,
    }
}

pub(super) fn check(
    statements: &[Statement],
    symbols: &SymbolTable,
    contracts: &Contracts,
) -> Vec<Diagnostic> {
    let mut checker = Checker {
        symbols,
        contracts,
        errors: vec![],
        reported: vec![],
    };
    let mut initialization = State::new();
    for s in statements {
        match &s.statement_type {
            S::FunctionDeclaration { body, .. }
            | S::SetupDeclaration(body)
            | S::LoopDeclaration(body) => {
                checker.block(body, State::new());
            }
            S::TimerDeclaration(timer) => {
                checker.block(&timer.body, State::new());
            }
            S::VariableDeclaration { .. } => {
                initialization = checker.statement(s, initialization).unwrap();
            }
            _ => {}
        }
    }
    checker.errors
}

struct Checker<'a> {
    symbols: &'a SymbolTable,
    contracts: &'a Contracts,
    errors: Vec<Diagnostic>,
    reported: Vec<(SymbolId, SourceLocation)>,
}

impl Checker<'_> {
    fn use_value(&mut self, id: SymbolId, at: SourceLocation, consume: bool, state: &mut State) {
        let symbol = self.symbols.get(id);
        if !matches!(
            symbol.kind,
            SymbolKind::Local | SymbolKind::Parameter | SymbolKind::Global
        ) {
            return;
        }
        if self.contracts.transfer(id) != Some(Transfer::Move) {
            return;
        }
        let error = if consume && matches!(symbol.kind, SymbolKind::Global) {
            Some(
                Diagnostic::new(
                    DiagnosticKind::GlobalMove {
                        name: symbol.name.clone(),
                    },
                    Some(at),
                )
                .with_label(symbol.declaration, LabelKind::PreviousDeclaration),
            )
        } else {
            state.get(&id).map(|origin| {
                Diagnostic::new(
                    DiagnosticKind::MovedValue {
                        name: symbol.name.clone(),
                    },
                    Some(at),
                )
                .with_label(*origin, LabelKind::ValueMoved)
            })
        };
        if let Some(error) = error {
            // A loop is visited to fixed point; report a given use only once.
            if !self.reported.contains(&(id, at)) {
                self.reported.push((id, at));
                self.errors.push(error);
            }
        }
        if consume {
            state.entry(id).or_insert(at);
        }
    }

    fn expression(&mut self, e: &Expression, consume: bool, state: &mut State) {
        match &e.expression_type {
            E::Borrow { expression, .. } => self.expression(expression, false, state),
            E::Index { array, index } => {
                self.expression(array, false, state);
                self.expression(index, true, state);
            }
            E::IndexAssignment {
                array,
                index,
                value,
            } => {
                self.expression(array, false, state);
                self.expression(index, true, state);
                self.expression(value, true, state);
            }
            E::Identifier(_) => {
                if let Some(id) = e.symbol {
                    self.use_value(
                        id,
                        e.source_location,
                        consume
                            && !matches!(
                                e.static_type,
                                crate::frontend::static_type::StaticType::ArrayRef(..)
                                    | crate::frontend::static_type::StaticType::SliceRef(..)
                            ),
                        state,
                    );
                }
            }
            E::Assignment { expression, .. } => {
                self.expression(expression, true, state);
                if let Some(id) = e.symbol {
                    state.remove(&id);
                    // Assignment in value position transfers the new destination value.
                    if consume {
                        self.use_value(id, e.source_location, true, state);
                    }
                }
            }
            E::Grouping { expression } => self.expression(expression, consume, state),
            E::Ternary {
                first,
                second,
                third,
            } => {
                self.expression(first, true, state);
                let mut other = state.clone();
                self.expression(second, consume, state);
                self.expression(third, consume, &mut other);
                *state = join(std::mem::take(state), other);
            }
            E::Binary {
                left,
                operator: TokenType::Keyword(K::And | K::Or),
                right,
                ..
            } => {
                self.expression(left, true, state);
                let skipped = state.clone();
                self.expression(right, true, state);
                *state = join(skipped, std::mem::take(state));
            }
            E::CapabilityCall { arguments, .. } | E::Text { arguments, .. } => {
                // Receiver is inspected, not consumed; later loan analysis must
                // keep this read alive across evaluation of the other arguments.
                for (i, argument) in arguments.iter().enumerate() {
                    self.expression(argument, i != 0, state);
                }
            }
            _ => e.for_each_child(|child| self.expression(child, true, state)),
        }
    }

    fn block(&mut self, body: &[Statement], state: State) -> Option<State> {
        let mut current = Some(state);
        for s in body {
            current = self.statement(s, current?);
        }
        current
    }

    fn statement(&mut self, s: &Statement, mut state: State) -> Option<State> {
        match &s.statement_type {
            S::VariableDeclaration { initializer, .. } => {
                if let Some(e) = initializer {
                    self.expression(e, true, &mut state);
                }
                if let Some(id) = s.symbol {
                    state.remove(&id);
                }
            }
            S::Expression(e) => self.expression(e, false, &mut state),
            S::Return(e) => {
                if let Some(e) = e {
                    self.expression(e, true, &mut state);
                }
                return None;
            }
            S::Block(body) | S::NoInterrupt(body) => return self.block(body, state),
            S::If {
                condition,
                then_body,
                else_body,
            } => {
                self.expression(condition, true, &mut state);
                let yes = self.statement(then_body, state.clone());
                let no = if let Some(body) = else_body {
                    self.statement(body, state)
                } else {
                    Some(state)
                };
                return merge(yes, no);
            }
            S::While { condition, body } => {
                let entry = state;
                let mut head = entry.clone();
                loop {
                    let mut tested = head.clone();
                    self.expression(condition, true, &mut tested);
                    let back = self.statement(body, tested.clone());
                    let next = back.map_or_else(|| entry.clone(), |back| join(entry.clone(), back));
                    if next == head {
                        return Some(tested);
                    }
                    head = next;
                }
            }
            S::For { from, to, body, .. } => {
                self.expression(from, true, &mut state);
                self.expression(to, true, &mut state);
                let entry = state;
                let mut head = entry.clone();
                loop {
                    let back = self.statement(body, head.clone());
                    let next = back.map_or_else(|| entry.clone(), |back| join(entry.clone(), back));
                    if next == head {
                        return Some(head);
                    }
                    head = next;
                }
            }
            _ => {}
        }
        Some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::{Capability, Implementation, Registry};

    // Resolved owner fixtures: source arrays/records are deliberately not enabled
    // by this stage. Parse and resolve real control flow, then supply move-only
    // contracts for the selected identities, exactly as future type resolution will.
    fn check_owners(body: &str, names: &[&str]) -> Vec<Diagnostic> {
        let source = format!("fn take(value:i16) {{}} fn predicate(value:i16):bool {{ return true; }} fn pair(left:i16,right:i16) {{}} fn run(owner:i16, other:i16, flag:bool) {{ {body} }} setup {{}} loop {{}}");
        check_source(&source, names, false)
    }

    fn check_source(source: &str, names: &[&str], copy: bool) -> Vec<Diagnostic> {
        let program = crate::analyze(source).unwrap_or_else(|e| panic!("{source}\n{e:#?}"));
        let mut registry = Registry::default();
        let owner = registry.record(vec![]).unwrap();
        if copy {
            registry
                .implement(owner, Capability::Clone, Implementation::Structural)
                .unwrap();
            registry
                .implement(owner, Capability::Copy, Implementation::Bitwise)
                .unwrap();
        }
        let symbols = program
            .symbols()
            .iter()
            .map(|(id, s)| {
                (
                    id,
                    if names.contains(&s.name.as_str()) {
                        owner
                    } else {
                        registry.source_type(s.ty)
                    },
                )
            })
            .collect();
        let contracts = Contracts::resolved(registry, program.symbols(), symbols).unwrap();
        check(program.statements(), program.symbols(), &contracts)
    }

    fn moved(body: &str) -> Vec<Diagnostic> {
        check_owners(body, &["owner", "other", "local"])
    }

    #[test]
    fn moving_into_calls_initializers_and_assignments_invalidates_source() {
        for body in [
            "take(owner); take(owner);",
            "var local = owner; take(owner);",
            "other = owner; take(owner);",
            "pair(owner, owner);",
        ] {
            let errors = moved(body);
            assert_eq!(errors.len(), 1, "{body}: {errors:#?}");
            assert_eq!(
                errors[0].code(),
                crate::diagnostic::DiagnosticCode::Ownership
            );
            assert!(matches!(*errors[0].kind, DiagnosticKind::MovedValue { .. }));
            assert_eq!(errors[0].labels[0].kind, LabelKind::ValueMoved);
        }
        assert!(moved("var local = owner; take(local); take(other);").is_empty());
    }

    #[test]
    fn assignment_reinitializes_and_rhs_is_evaluated_before_destination() {
        assert!(moved("take(owner); owner = other; take(owner);").is_empty());
        assert!(moved("owner = owner; take(owner);").is_empty());
        assert_eq!(moved("take(owner); owner = owner;").len(), 1);
        assert_eq!(moved("var local = (owner = other); take(owner);").len(), 1);
        assert!(moved("var local = (owner = other); take(local);").is_empty());
    }

    #[test]
    fn joins_include_possible_moves_but_exclude_returning_paths() {
        assert_eq!(moved("if (flag) { take(owner); } take(owner);").len(), 1);
        assert!(moved("if (flag) { take(owner); return; } take(owner);").is_empty());
        assert!(moved("if (flag) { take(owner); owner = other; } take(owner);").is_empty());
        assert!(moved("if (flag) { take(owner); } else { take(owner); }").is_empty());
        assert_eq!(
            moved("var local = flag ? owner : other; take(owner);").len(),
            1
        );
        assert_eq!(
            moved("if (flag and predicate(owner)) {} take(owner);").len(),
            1
        );
        assert_eq!(
            moved("if (flag or predicate(owner)) {} take(owner);").len(),
            1
        );
        assert!(moved("return; take(owner); take(owner);").is_empty());
    }

    #[test]
    fn loop_backedges_reach_fixed_point_and_local_declarations_reset_each_iteration() {
        assert_eq!(moved("while (flag) { take(owner); }").len(), 1);
        assert_eq!(moved("while (predicate(owner)) {}").len(), 1);
        assert_eq!(moved("while (flag) { take(owner); } take(owner);").len(), 2);
        assert!(moved("while (flag) { take(owner); owner = 1; } take(owner);").is_empty());
        assert!(moved("while (flag) { var local:i16 = 1; take(local); }").is_empty());
        assert!(moved("while (flag) { take(owner); return; } take(owner);").is_empty());
        assert_eq!(
            moved("while (flag) { if (flag) { take(owner); } }").len(),
            1
        );
    }

    #[test]
    fn copy_contract_does_not_move_values_and_identity_is_not_spelling() {
        assert!(check_owners("take(owner); take(owner);", &[]).is_empty());
        assert!(
            moved("{ var local:i16 = 1; take(local); } { var local:i16 = 2; take(local); }")
                .is_empty()
        );
    }

    #[test]
    fn declared_copy_capability_is_used_by_the_same_flow_analysis() {
        let source = "fn take(value:i16) {} fn run(owner:i16) { take(owner); take(owner); } setup {} loop {}";
        assert_eq!(check_source(source, &["owner"], false).len(), 1);
        assert!(check_source(source, &["owner"], true).is_empty());
    }

    #[test]
    fn returns_transfer_ownership_and_globals_cannot_be_moved_out() {
        let valid = "fn run(owner:i16):i16 { return owner; } setup {} loop {}";
        assert!(check_source(valid, &["owner"], false).is_empty());
        let invalid = "fn take(value:i16) {} fn run(owner:i16):i16 { take(owner); return owner; } setup {} loop {}";
        assert_eq!(check_source(invalid, &["owner"], false).len(), 1);
        let global = "var owner:i16 = 1; fn take(value:i16) {} setup { take(owner); } loop {}";
        let errors = check_source(global, &["owner"], false);
        assert_eq!(errors.len(), 1);
        assert!(matches!(*errors[0].kind, DiagnosticKind::GlobalMove { .. }));
    }

    #[test]
    fn for_and_nested_loops_check_repeated_moves_without_duplicate_diagnostics() {
        assert_eq!(moved("for i = 0 to 3 { take(owner); }").len(), 1);
        assert_eq!(
            moved("while (flag) { while (flag) { take(owner); } }").len(),
            1
        );
    }
}
