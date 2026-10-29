//! Guarded integer operations. Invalid inputs never reach LLVM poison operations.
use crate::ir::{Binary, Module, Op, Type};
use std::collections::BTreeSet;
use std::fmt::Write;

pub(super) fn helper_name(op: Binary, ty: Type, count: Type) -> String {
    format!("helium_{op:?}_{ty}_{count}")
}

pub(super) fn emit_helpers(module: &Module, out: &mut String) {
    let needs_panic = module
        .functions
        .iter()
        .flat_map(|f| &f.blocks)
        .flat_map(|b| &b.instructions)
        .any(|i| {
            matches!(
                i.op,
                Op::ArrayRead(..)
                    | Op::ArrayWrite(..)
                    | Op::ElementAddress(..)
                    | Op::Binary(
                        Binary::Divide | Binary::Remainder | Binary::ShiftLeft | Binary::ShiftRight,
                        ..
                    )
            )
        });
    if needs_panic {
        out.push_str("define internal void @helium_panic(i8 %reason) addrspace(1) noreturn cold {\nentry:\n  call void asm sideeffect \"cli\", \"~{memory}\"()\n  br label %halt\nhalt:\n  br label %halt\n}\n");
    }
    let mut emitted = BTreeSet::new();
    for function in &module.functions {
        let types: std::collections::HashMap<_, _> = function
            .parameters
            .iter()
            .copied()
            .chain(
                function
                    .blocks
                    .iter()
                    .flat_map(|b| &b.instructions)
                    .filter_map(|i| i.result),
            )
            .collect();
        for instruction in function.blocks.iter().flat_map(|b| &b.instructions) {
            if let Op::NumericCast(value, target) | Op::Widen(value, target) = instruction.op {
                if let Some((bits, true)) = types[&value].integer() {
                    if bits < 64
                        && target.integer().is_some_and(|(w, _)| w == 64)
                        && emitted.insert(format!("sext{bits}"))
                    {
                        emit_signed_extension(bits, out);
                    }
                }
            }
            let Op::Binary(
                op @ (Binary::Divide | Binary::Remainder | Binary::ShiftLeft | Binary::ShiftRight),
                _,
                rhs,
            ) = instruction.op
            else {
                continue;
            };
            let Some((_, ty)) = instruction.result else {
                continue;
            };
            let count = types[&rhs];
            let name = helper_name(op, ty, count);
            if !emitted.insert(name.clone()) {
                continue;
            }
            let (bits, signed) = ty.integer().expect("verified integer operation");
            let t = format!("i{bits}");
            let count_bits = count.integer().unwrap().0;
            let c = format!("i{count_bits}");
            writeln!(
                out,
                "define internal {t} @{name}({t} %a, {c} %b) addrspace(1) alwaysinline {{\nentry:"
            )
            .unwrap();
            if matches!(op, Binary::Divide | Binary::Remainder) {
                writeln!(out, "  %zero = icmp eq {t} %b, 0\n  br i1 %zero, label %zero_fail, label %nonzero\nzero_fail:\n  call addrspace(1) void @helium_panic(i8 2)\n  unreachable\nnonzero:").unwrap();
                if signed {
                    let min = -(1i128 << (bits - 1));
                    writeln!(out, "  %min = icmp eq {t} %a, {min}\n  %negone = icmp eq {t} %b, -1\n  %overflow = and i1 %min, %negone\n  br i1 %overflow, label %special, label %calculate\nspecial:").unwrap();
                    if op == Binary::Remainder {
                        writeln!(out, "  ret {t} 0").unwrap();
                    } else {
                        out.push_str(
                            "  call addrspace(1) void @helium_panic(i8 3)\n  unreachable\n",
                        );
                    }
                    out.push_str("calculate:\n");
                }
            } else {
                // Unsigned comparison also rejects a negative signed count.
                writeln!(out, "  %valid = icmp ult {c} %b, {bits}\n  br i1 %valid, label %calculate, label %shift_fail\nshift_fail:\n  call addrspace(1) void @helium_panic(i8 4)\n  unreachable\ncalculate:").unwrap();
            }
            let rhs = if count_bits != bits {
                writeln!(
                    out,
                    "  %count = {} {c} %b to {t}",
                    if count_bits > bits { "trunc" } else { "zext" }
                )
                .unwrap();
                "%count"
            } else {
                "%b"
            };
            let opcode = match (op, signed) {
                (Binary::Divide, true) => "sdiv",
                (Binary::Divide, false) => "udiv",
                (Binary::Remainder, true) => "srem",
                (Binary::Remainder, false) => "urem",
                (Binary::ShiftLeft, _) => "shl",
                (Binary::ShiftRight, true) => "ashr",
                (Binary::ShiftRight, false) => "lshr",
                _ => unreachable!(),
            };
            writeln!(
                out,
                "  %result = {opcode} {t} %a, {rhs}\n  ret {t} %result\n}}"
            )
            .unwrap();
        }
    }
}

// Work around the bundled LLVM AVR sign-extension legalization defect.
fn emit_signed_extension(bits: u32, out: &mut String) {
    writeln!(out, "define internal i64 @helium_sext{bits}_64(i{bits} %a) addrspace(1) noinline {{\nentry:\n  %cell = alloca i64, align 1\n  %negative = icmp slt i{bits} %a, 0\n  br i1 %negative, label %neg, label %pos\nneg:\n  store volatile i64 -1, ptr %cell, align 1\n  br label %finish\npos:\n  store volatile i64 0, ptr %cell, align 1\n  br label %finish\nfinish:\n  store volatile i{bits} %a, ptr %cell, align 1\n  %result = load volatile i64, ptr %cell, align 1\n  ret i64 %result\n}}").unwrap();
}
