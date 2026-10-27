use helium::backend::llvm_toolchain::{build, Optimization};
use std::{fs, path::Path};

#[test]
#[ignore = "requires LLVM AVR and AVR GNU toolchain"]
fn final_elf_accounts_for_initialized_uninitialized_and_noinit_storage() {
    let ir = r#"target triple = "avr-unknown-unknown"
@initial = global i16 7, align 1
@zero = global i16 0, align 1
module asm ".section .noinit,\22aw\22,@nobits"
module asm ".global retained"
module asm "retained: .skip 2"
@retained = external global i16
define i16 @main() addrspace(1) {
  %a = load volatile i16, ptr @initial
  %b = load volatile i16, ptr @zero
  %c = load volatile i16, ptr @retained
  %d = add i16 %a, %b
  %e = add i16 %c, %d
  ret i16 %e
}
"#;
    for mode in [Optimization::None, Optimization::Speed, Optimization::Size] {
        let output = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("build/b3-tests/storage-{mode:?}.elf"));
        let result = build(ir, &output, "elf", mode).unwrap();
        let memory = result.memory.unwrap();
        assert_eq!(memory.static_sram, 6);
        let sizes = result
            .sections
            .lines()
            .filter_map(|l| {
                let mut fields = l.split_whitespace();
                Some((fields.next()?, fields.next()?.parse::<u32>().ok()?))
            })
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(sizes[".data"], 2);
        assert_eq!(sizes[".bss"], 2);
        assert_eq!(sizes[".noinit"], 2);
        assert_eq!(memory.flash_extent, sizes[".text"] + 2);
        assert_eq!(
            fs::read(output).unwrap(),
            fs::read(result.directory.join("program.elf")).unwrap()
        );
    }
}
