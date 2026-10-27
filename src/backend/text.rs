//! Universal text ABI for the ATmega328P adapter: address, byte length, space.
//! Flash is 1, SRAM is 0. This is internal, not an external C ABI.
use crate::ir::{Module, Op, TextSpace};
use std::fmt::Write;

pub const LLVM_TYPE: &str = "{ i16, i16, i8 }";

pub fn literals(module: &Module) -> Vec<(String, TextSpace)> {
    let mut result = Vec::new();
    for op in module
        .functions
        .iter()
        .flat_map(|f| &f.blocks)
        .flat_map(|b| &b.instructions)
        .map(|i| &i.op)
    {
        if let Op::TextLiteral { text, space } = op {
            let key = (text.clone(), *space);
            if !result.contains(&key) {
                result.push(key);
            }
        }
    }
    result
}

pub fn emit(module: &Module, out: &mut String) {
    let literals = literals(module);
    let used = module
        .functions
        .iter()
        .flat_map(|f| &f.blocks)
        .flat_map(|b| &b.instructions)
        .any(|i| matches!(i.op, Op::TextByteOr { .. }));
    for (id, (text, space)) in literals.iter().enumerate() {
        let address_space = if *space == TextSpace::Flash {
            "addrspace(1) "
        } else {
            ""
        };
        let bytes = text
            .as_bytes()
            .iter()
            .map(|b| format!("\\{b:02X}"))
            .collect::<String>();
        writeln!(
            out,
            "@helium_text{id} = private {address_space}constant [{} x i8] c\"{bytes}\", align 1",
            text.len()
        )
        .unwrap();
    }
    if used {
        out.push_str(r#"
; Bounds are checked before either memory space is accessed. No terminator scan.
define internal i8 @helium_text_byte({ i16, i16, i8 } %text, i16 %index, i8 %fallback) addrspace(1) alwaysinline {
entry:
  %len = extractvalue { i16, i16, i8 } %text, 1
  %valid = icmp ult i16 %index, %len
  br i1 %valid, label %read, label %missing
read:
  %base = extractvalue { i16, i16, i8 } %text, 0
  %address = add i16 %base, %index
  %space = extractvalue { i16, i16, i8 } %text, 2
  %flash = icmp eq i8 %space, 1
  br i1 %flash, label %program, label %data
program:
  %fp = inttoptr i16 %address to ptr addrspace(1)
  %fb = load i8, ptr addrspace(1) %fp, align 1
  ret i8 %fb
data:
  %dp = inttoptr i16 %address to ptr
  %db = load i8, ptr %dp, align 1
  ret i8 %db
missing:
  ret i8 %fallback
}
"#);
    }
}
