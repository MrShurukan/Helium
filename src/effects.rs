//! Context contracts are independent of device names and backend implementations.
use crate::{
    diagnostic::{Diagnostic, DiagnosticKind, LabelKind},
    frontend::utils::SourceLocation,
    middle_end::shared::{Body, Protection},
    semantic::{BodyId, SymbolId, SymbolTable},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrqRequirement {
    Any,
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    TransmitterReady,
    TransmissionComplete,
    InterruptConsumer,
    Custom(&'static str),
}
impl Reason {
    pub fn message(self) -> &'static str {
        match self {
            Self::Custom(message) => message,
            Self::TransmitterReady => "waits for the hardware transmitter to accept a byte",
            Self::TransmissionComplete => "waits for the hardware transmission to finish",
            Self::InterruptConsumer => {
                "progress requires an interrupt handler to consume queued work"
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Contract {
    pub irq: IrqRequirement,
    pub blocking: Option<Reason>,
    pub irq_reason: Option<Reason>,
}

#[derive(Debug, Clone)]
pub struct Witness {
    pub reason: Reason,
    pub calls: Vec<SourceLocation>,
}
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub requires_enabled: Option<Witness>,
    pub blocking: Option<Witness>,
}

/// Least fixed point on the syntactic call graph. Local protected scopes cannot
/// discharge an enabled-IRQ requirement: they produce a local error instead.
pub fn analyze(
    bodies: &BTreeMap<BodyId, Body>,
    symbols: &SymbolTable,
    contracts: &BTreeMap<SymbolId, Contract>,
    runtime_irqs: bool,
) -> (BTreeMap<BodyId, Summary>, Vec<Diagnostic>) {
    let mut summaries: BTreeMap<_, _> = contracts
        .iter()
        .map(|(id, c)| {
            (
                BodyId::Function(*id),
                Summary {
                    requires_enabled: (c.irq == IrqRequirement::Enabled).then(|| Witness {
                        reason: c.irq_reason.expect("enabled IRQ contract needs a reason"),
                        calls: vec![],
                    }),
                    blocking: c.blocking.map(|reason| Witness {
                        reason,
                        calls: vec![],
                    }),
                },
            )
        })
        .collect();
    loop {
        let mut changed = false;
        for (id, body) in bodies {
            let mut result = summaries.get(id).cloned().unwrap_or_default();
            for call in &body.calls {
                let Some(callee) = summaries.get(&BodyId::Function(call.callee)) else {
                    continue;
                };
                for (destination, source, allowed) in [
                    (&mut result.blocking, &callee.blocking, true),
                    (
                        &mut result.requires_enabled,
                        &callee.requires_enabled,
                        call.protection == Protection::None,
                    ),
                ] {
                    if destination.is_none() && allowed {
                        if let Some(witness) = source {
                            let mut witness = witness.clone();
                            witness.calls.insert(0, call.location);
                            *destination = Some(witness);
                            changed = true;
                        }
                    }
                }
            }
            summaries.insert(*id, result);
        }
        if !changed {
            break;
        }
    }
    let mut diagnostics = vec![];
    for (id, body) in bodies {
        for call in &body.calls {
            let Some(callee) = summaries.get(&BodyId::Function(call.callee)) else {
                continue;
            };
            let name = &symbols.get(call.callee).name;
            if let Some(witness) = &callee.requires_enabled {
                if call.protection != Protection::None || (*id == BodyId::MainLoop && !runtime_irqs)
                {
                    diagnostics.push(with_path(
                        DiagnosticKind::IrqRequirement {
                            name: name.clone(),
                            reason: witness.reason,
                        },
                        call.location,
                        witness,
                    ));
                }
            }
            // One warning at the outer call in an ISR, with a witness path through helpers.
            if matches!(id, BodyId::Interrupt(_)) {
                if let Some(witness) = &callee.blocking {
                    diagnostics.push(with_path(
                        DiagnosticKind::IsrBlocking {
                            name: name.clone(),
                            reason: witness.reason,
                        },
                        call.location,
                        witness,
                    ));
                }
            }
        }
    }
    (summaries, diagnostics)
}

fn with_path(kind: DiagnosticKind, location: SourceLocation, witness: &Witness) -> Diagnostic {
    let mut result = Diagnostic::new(kind, Some(location));
    for call in &witness.calls {
        result = result.with_label(*call, LabelKind::EffectCall);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(source: &str, runtime_irqs: bool) -> Vec<Diagnostic> {
        // Attach a contract to a real source function, independently of UART.
        let source = if source.contains("loop{") {
            source.to_string()
        } else {
            format!("{source} loop{{}}")
        };
        let program = crate::analyze(&source).unwrap();
        let id = program
            .symbols()
            .iter()
            .find(|(_, s)| s.name == "wait_for_work")
            .unwrap()
            .0;
        let contracts = BTreeMap::from([(
            id,
            Contract {
                irq: IrqRequirement::Enabled,
                blocking: Some(Reason::Custom("waits for external progress")),
                irq_reason: Some(Reason::InterruptConsumer),
            },
        )]);
        analyze(
            program.shared_analysis().bodies(),
            program.symbols(),
            &contracts,
            runtime_irqs,
        )
        .1
    }
    #[test]
    fn irq_requirement_propagates_through_helpers_and_checks_execution_context() {
        let functions = "fn wait_for_work(){} fn helper(){wait_for_work();}";
        for context in [
            "setup{helper();}",
            "loop{noInterrupt{helper();}}",
            "timer t=Timer1{every:1ms,interrupt{helper();}}",
        ] {
            let ds = check(&format!("{functions} {context}"), true);
            let error = ds
                .iter()
                .find(|d| d.code() == crate::diagnostic::DiagnosticCode::IrqRequirement)
                .unwrap();
            assert_eq!(error.labels.len(), 1);
        }
        assert!(check(&format!("{functions} loop{{helper();}}"), true).is_empty());
        assert_eq!(
            check(&format!("{functions} loop{{helper();}}"), false).len(),
            1
        );
    }
    #[test]
    fn local_protection_is_an_error_even_when_caller_has_enabled_irqs() {
        let ds = check(
            "fn wait_for_work(){} fn helper(){noInterrupt{wait_for_work();}} loop{helper();}",
            true,
        );
        assert_eq!(ds.len(), 1);
        assert!(ds[0].labels.is_empty());
    }
    #[test]
    fn ordinary_function_can_carry_blocking_metadata_with_a_reason() {
        let ds=check("fn wait_for_work(){} fn helper(){wait_for_work();} timer t=Timer1{every:1ms,interrupt{helper();}}",true);
        assert_eq!(ds.len(), 2);
        assert_eq!(
            ds.iter()
                .filter(|d| d.severity() == crate::diagnostic::Severity::Warning)
                .count(),
            1
        );
        assert!(ds.iter().any(|d| d.message().contains("external progress")));
    }
}
