//! A source inventory, deliberately not a timing bound or a reachability proof.
use crate::ir::{FunctionId, Module, Op, Origin};
use std::collections::BTreeSet;

pub fn inventory(module: &Module) -> Vec<String> {
    let mut lines = vec!["Timing inventory: before LLVM optimization; includes unused/unreachable code; durations unknown".into()];
    for f in &module.functions {
        let mut sites = Vec::new();
        if module.interrupt_handler == Some(f.id) {
            sites.push("ISR".to_string());
        }
        if f.requires_no_interrupt {
            sites.push("requires noInterrupt".to_string());
        }
        for instruction in f.blocks.iter().flat_map(|b| &b.instructions) {
            if matches!(instruction.op, Op::InterruptSave | Op::InterruptDisable) {
                sites.push(match &instruction.origin {
                    Origin::Source(s) => {
                        format!("IRQ mask at line {}:{}", s.line_start, s.start_offset + 1)
                    }
                    Origin::Generated(reason) => format!("IRQ mask generated: {reason}"),
                });
            }
        }
        if sites.is_empty() {
            continue;
        }
        sites.sort();
        sites.dedup();
        let mut calls = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut pending = vec![f.id];
        while let Some(FunctionId(id)) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            for i in module.functions[id]
                .blocks
                .iter()
                .flat_map(|b| &b.instructions)
            {
                if let Op::Call(callee, _) = &i.op {
                    calls.insert(module.functions[callee.0].name.clone());
                    pending.push(*callee);
                }
            }
        }
        lines.push(format!(
            "{}: {}; possible callees of whole function (not only masked regions): {}",
            f.name,
            sites.join(", "),
            if calls.is_empty() {
                "none".into()
            } else {
                calls.into_iter().collect::<Vec<_>>().join(", ")
            }
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_helper_and_source_without_promising_duration() {
        let source = "noInterrupt fn helper() {}\nloop { noInterrupt { helper(); } }";
        let m = crate::compile_typed_ir(source, "atmega328p", Default::default()).unwrap();
        let report = super::inventory(&m).join("\n");
        assert!(report.contains("helper: requires noInterrupt"));
        assert!(report.contains("IRQ mask at line 2:"));
        assert!(report.contains("masked regions): helper"));
        assert!(report.contains("durations unknown"));
    }
}
