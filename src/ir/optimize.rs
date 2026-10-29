//! Small, conservative passes. No store elimination, load forwarding or hardware CSE.
use super::*;
use std::collections::{HashMap, HashSet};

pub fn optimize(module: &mut Module, verify_each: bool) -> Result<(), verify::Error> {
    if verify_each {
        verify::verify(module)?;
    }

    fold_constants(module);
    if verify_each {
        verify::verify(module)?;
    }

    simplify_control_flow(module);
    if verify_each {
        verify::verify(module)?;
    }

    eliminate_unused_values(module);
    if verify_each {
        verify::verify(module)?;
    }
    Ok(())
}

fn fold_constants(module: &mut Module) {
    for function in &mut module.functions {
        let mut constants = HashMap::new();
        // Repeat for definitions in a different layout order; IDs are immutable values.
        loop {
            let mut changed = false;
            for block in &mut function.blocks {
                for instruction in &mut block.instructions {
                    let Some((id, _)) = instruction.result else {
                        continue;
                    };
                    if constants.contains_key(&id) {
                        continue;
                    }

                    let folded = match instruction.op {
                        Op::Const(value) => Some(value),
                        Op::Unary(op, value) => {
                            constants.get(&value).map(|v| interpret::eval_unary(op, *v))
                        }
                        Op::Binary(op, left, right) => constants
                            .get(&left)
                            .zip(constants.get(&right))
                            .and_then(|(a, b)| interpret::eval_binary(op, *a, *b).ok()),
                        Op::NumericCast(value, target) | Op::Widen(value, target) => constants
                            .get(&value)
                            .map(|v| Constant::new(target, v.ty.numeric_value(v.bits) as u64)),
                        _ => None,
                    };
                    if let Some(value) = folded {
                        instruction.op = Op::Const(value);
                        constants.insert(id, value);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }
}

fn simplify_control_flow(module: &mut Module) {
    for function in &mut module.functions {
        let constants: HashMap<_, _> = function
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|i| match (i.result, &i.op) {
                (Some((id, _)), Op::Const(value)) => Some((id, *value)),
                _ => None,
            })
            .collect();

        for block in &mut function.blocks {
            if let Terminator::Branch { condition, yes, no } = block.terminator {
                if let Some(value) = constants.get(&condition) {
                    block.terminator = Terminator::Jump(if value.bits != 0 { yes } else { no });
                }
            }
        }

        let reachable = verify::reachable(function);
        function
            .blocks
            .retain(|block| reachable.contains(&block.id));
    }
}

fn eliminate_unused_values(module: &mut Module) {
    for function in &mut module.functions {
        loop {
            let mut used = HashSet::new();
            for block in &function.blocks {
                used.extend(block.terminator.operands());
                for instruction in &block.instructions {
                    used.extend(instruction.op.operands());
                }
            }

            let mut removed = false;
            for block in &mut function.blocks {
                block.instructions.retain(|instruction| {
                    let dead = instruction.op.is_pure()
                        && instruction
                            .result
                            .is_some_and(|(id, _)| !used.contains(&id));
                    removed |= dead;
                    !dead
                });
            }
            if !removed {
                break;
            }
        }
    }
}
