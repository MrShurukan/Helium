//! Mandatory machine preparation, independent of LLVM optimization policy.
//! Universal entry points dispatch once; concrete text parameters have no tag.
use super::*;
use std::collections::HashMap;

pub(crate) const MAX_TEXT_PARAMETERS: usize = 6;
pub(crate) const MAX_VARIANTS: usize = 4096;

fn space(mask: u8) -> Option<TextSpace> {
    match mask {
        1 => Some(TextSpace::Flash),
        2 => Some(TextSpace::Sram),
        _ => None,
    }
}
fn mask(space: TextSpace) -> u8 {
    if space == TextSpace::Flash {
        1
    } else {
        2
    }
}
fn type_mask(t: Type) -> u8 {
    match t {
        Type::TextIn(s) => mask(s),
        Type::Text => 3,
        _ => 0,
    }
}

/// Input remains unchanged, including on failure. A specialized module is idempotent.
pub fn specialize(input: &Module) -> Result<Module, String> {
    verify::verify(input).map_err(|e| e.to_string())?;
    if input.functions.iter().any(|f| {
        f.parameters
            .iter()
            .any(|(_, t)| matches!(t, Type::TextIn(_)))
    }) {
        if input.functions.iter().any(|f| {
            f.parameters.iter().any(|(_, t)| *t == Type::Text)
                && (!f.locals.is_empty()
                    || f.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
                        !matches!(i.op, Op::TextIsFlash(_) | Op::TextNarrow(..) | Op::Call(..))
                    }))
        }) {
            return Err("Partially specialized text module is not supported".into());
        }
        return Ok(input.clone());
    }
    let mut module = input.clone();
    let mut variants = HashMap::new();
    let mut added = 0;
    for f in &input.functions {
        let positions: Vec<_> = f
            .parameters
            .iter()
            .enumerate()
            .filter_map(|(i, (_, t))| (*t == Type::Text).then_some(i))
            .collect();
        if positions.is_empty() {
            continue;
        }
        if positions.len() > MAX_TEXT_PARAMETERS {
            return Err(format!(
                "Text specialization of '{}' exceeds {MAX_TEXT_PARAMETERS} text parameters",
                f.name
            ));
        }
        let count = 1usize << positions.len();
        added += count;
        if added > MAX_VARIANTS {
            return Err(format!(
                "Text specialization exceeds {MAX_VARIANTS} variants"
            ));
        }
        let mut ids = Vec::new();
        for bits in 0..count {
            let id = FunctionId(module.functions.len());
            let mut clone = f.clone();
            remap(&mut clone, id);
            let mut names = Vec::new();
            for (bit, &position) in positions.iter().enumerate() {
                let s = if bits & (1 << bit) == 0 {
                    TextSpace::Flash
                } else {
                    TextSpace::Sram
                };
                clone.parameters[position].1 = Type::TextIn(s);
                names.push(format!("{s:?}"));
            }
            clone.name = format!("{}[{}]", f.name, names.join(","));
            ids.push(id);
            module.functions.push(clone);
        }
        variants.insert(f.id, (positions, ids));
    }
    for f in &mut module.functions {
        if !variants.contains_key(&f.id) {
            specialize_body(f, input, &variants);
        }
    }
    for (&id, (positions, ids)) in &variants {
        module.functions[id.0] = dispatcher(&input.functions[id.0], positions, ids);
    }
    verify::verify(&module).map_err(|e| format!("text specialization: {e}"))?;
    Ok(module)
}

fn values(op: &mut Op, mut apply: impl FnMut(&mut ValueId)) {
    match op {
        Op::ArrayLiteral(v) => v.iter_mut().for_each(apply),
        Op::ArrayRepeat(v, _) | Op::Reborrow(v, _) | Op::Slice(v, _) | Op::SliceLen(v) => apply(v),
        Op::ArrayRead(a, b) => {
            apply(a);
            apply(b);
        }
        Op::ArrayWrite(a, b, c) => {
            apply(a);
            apply(b);
            apply(c);
        }
        Op::TextLen(v)
        | Op::TextIsFlash(v)
        | Op::TextNarrow(v, _)
        | Op::TextErase(v)
        | Op::InterruptRestore(v)
        | Op::Unary(_, v)
        | Op::Widen(v, _)
        | Op::StoreLocal(_, v)
        | Op::StoreGlobal(_, v)
        | Op::WritePin(_, v) => apply(v),
        Op::Binary(_, a, b) => {
            apply(a);
            apply(b);
        }
        Op::TextByteOr {
            text,
            index,
            fallback,
        } => {
            apply(text);
            apply(index);
            apply(fallback);
        }
        Op::Call(_, args)
        | Op::Intrinsic {
            arguments: args, ..
        } => args.iter_mut().for_each(apply),
        _ => {}
    }
}
fn remap(f: &mut Function, id: FunctionId) {
    f.id = id;
    f.entry.function = id;
    for (v, _) in &mut f.parameters {
        v.function = id;
    }
    for b in &mut f.blocks {
        b.id.function = id;
        for i in &mut b.instructions {
            if let Some((v, _)) = &mut i.result {
                v.function = id;
            }
            values(&mut i.op, |v| v.function = id);
            if let Op::LoadLocal(l) | Op::StoreLocal(l, _) | Op::AddressLocal(l, _) = &mut i.op {
                l.function = id;
            }
        }
        match &mut b.terminator {
            Terminator::Jump(b) => b.function = id,
            Terminator::Branch { condition, yes, no } => {
                condition.function = id;
                yes.function = id;
                no.function = id;
            }
            Terminator::Return(Some(v)) => v.function = id,
            _ => {}
        }
    }
}

type Variants = HashMap<FunctionId, (Vec<usize>, Vec<FunctionId>)>;

fn specialize_body(f: &mut Function, input: &Module, variants: &Variants) {
    // Monotone union of *all* writes: reassignment and loop-carried mixed
    // placement cannot accidentally inherit a parameter's initial placement.
    let mut known: HashMap<ValueId, u8> = f
        .parameters
        .iter()
        .map(|(v, t)| (*v, type_mask(*t)))
        .collect();
    let mut locals = vec![0u8; f.locals.len()];
    loop {
        let mut changed = false;
        for i in f.blocks.iter().flat_map(|b| &b.instructions) {
            if let Op::StoreLocal(l, v) = i.op {
                let new = locals[l.index] | known.get(&v).copied().unwrap_or(0);
                changed |= new != locals[l.index];
                locals[l.index] = new;
            }
            if let Some((v, t)) = i.result {
                let m = match i.op {
                    Op::TextLiteral { space, .. } => mask(space),
                    Op::LoadLocal(l) => locals[l.index],
                    _ => type_mask(t),
                };
                let old = known.get(&v).copied().unwrap_or(0);
                changed |= (old | m) != old;
                known.insert(v, old | m);
            }
        }
        if !changed {
            break;
        }
    }
    for (l, m) in f.locals.iter_mut().zip(locals) {
        if l.ty == Type::Text {
            if let Some(s) = space(m) {
                l.ty = Type::TextIn(s);
            }
        }
    }
    let mut types: HashMap<_, _> = f.parameters.iter().copied().collect();
    for i in f.blocks.iter_mut().flat_map(|b| &mut b.instructions) {
        if let Some((v, t)) = &mut i.result {
            if *t == Type::Text && matches!(i.op, Op::TextLiteral { .. } | Op::LoadLocal(_)) {
                if let Some(s) = space(known[v]) {
                    *t = Type::TextIn(s);
                }
            }
            types.insert(*v, *t);
        }
    }
    let mut next = types.keys().map(|v| v.index + 1).max().unwrap_or(0);
    for b in &mut f.blocks {
        let mut out = Vec::new();
        for mut i in std::mem::take(&mut b.instructions) {
            let mut widen = |v: &mut ValueId, target: Type| {
                if target == Type::Text && matches!(types[v], Type::TextIn(_)) {
                    let id = ValueId {
                        function: f.id,
                        index: next,
                    };
                    next += 1;
                    out.push(Instruction {
                        result: Some((id, Type::Text)),
                        op: Op::TextErase(*v),
                        origin: i.origin.clone(),
                    });
                    *v = id;
                }
            };
            match &mut i.op {
                Op::StoreLocal(l, v) => widen(v, f.locals[l.index].ty),
                Op::StoreGlobal(g, v) => widen(v, input.globals[g.0].ty),
                Op::Call(id, args) => {
                    if let Some((positions, ids)) = variants.get(id) {
                        let mut bits = 0;
                        let all_known = positions.iter().enumerate().all(|(bit, &pos)| {
                            if let Type::TextIn(s) = types[&args[pos]] {
                                if s == TextSpace::Sram {
                                    bits |= 1 << bit;
                                }
                                true
                            } else {
                                false
                            }
                        });
                        if all_known {
                            *id = ids[bits];
                        }
                    }
                    if id.0 < input.functions.len() {
                        for (v, (_, t)) in args.iter_mut().zip(&input.functions[id.0].parameters) {
                            widen(v, *t);
                        }
                    }
                }
                _ => {}
            }
            out.push(i);
        }
        if let Terminator::Return(Some(v)) = &mut b.terminator {
            if f.return_type == Some(Type::Text) && matches!(types[v], Type::TextIn(_)) {
                let id = ValueId {
                    function: f.id,
                    index: next,
                };
                next += 1;
                out.push(Instruction {
                    result: Some((id, Type::Text)),
                    op: Op::TextErase(*v),
                    origin: b.origin.clone(),
                });
                *v = id;
            }
        }
        b.instructions = out;
    }
}

fn dispatcher(original: &Function, positions: &[usize], variants: &[FunctionId]) -> Function {
    struct Builder {
        f: Function,
        next: usize,
    }
    impl Builder {
        fn emit(&mut self, b: usize, op: Op, t: Type) -> ValueId {
            let v = ValueId {
                function: self.f.id,
                index: self.next,
            };
            self.next += 1;
            self.f.blocks[b].instructions.push(Instruction {
                result: Some((v, t)),
                op,
                origin: Origin::Generated("text dispatch"),
            });
            v
        }
        fn branch(
            &mut self,
            positions: &[usize],
            variants: &[FunctionId],
            depth: usize,
            bits: usize,
        ) -> BlockId {
            let b = self.f.blocks.len();
            let id = BlockId {
                function: self.f.id,
                index: b,
            };
            self.f.blocks.push(Block {
                id,
                instructions: vec![],
                terminator: Terminator::Return(None),
                origin: Origin::Generated("text dispatch"),
            });
            if depth == positions.len() {
                let mut args: Vec<_> = self.f.parameters.iter().map(|(v, _)| *v).collect();
                for (bit, &position) in positions.iter().enumerate() {
                    let s = if bits & (1 << bit) == 0 {
                        TextSpace::Flash
                    } else {
                        TextSpace::Sram
                    };
                    args[position] =
                        self.emit(b, Op::TextNarrow(args[position], s), Type::TextIn(s));
                }
                let op = Op::Call(variants[bits], args);
                let ret = if let Some(t) = self.f.return_type {
                    Some(self.emit(b, op, t))
                } else {
                    self.f.blocks[b].instructions.push(Instruction {
                        result: None,
                        op,
                        origin: Origin::Generated("text dispatch"),
                    });
                    None
                };
                self.f.blocks[b].terminator = Terminator::Return(ret);
            } else {
                let v = self.f.parameters[positions[depth]].0;
                let condition = self.emit(b, Op::TextIsFlash(v), Type::Bool);
                let yes = self.branch(positions, variants, depth + 1, bits);
                let no = self.branch(positions, variants, depth + 1, bits | (1 << depth));
                self.f.blocks[b].terminator = Terminator::Branch { condition, yes, no };
            }
            id
        }
    }
    let mut b = Builder {
        f: original.clone(),
        next: original
            .parameters
            .iter()
            .map(|(v, _)| v.index + 1)
            .max()
            .unwrap_or(0),
    };
    b.f.locals.clear();
    b.f.blocks.clear();
    b.f.entry = b.branch(positions, variants, 0, 0);
    b.f
}
