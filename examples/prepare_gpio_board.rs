//! Prepare test-only LLVM for the physical Nano fixture. Does not flash hardware.
use helium::{backend::llvm, compile_typed_ir, ir::Options};
use std::{fs, path::Path};
fn main() {
    let optimize = std::env::args().any(|a| a == "--optimize-ir");
    let module = compile_typed_ir(
        include_str!("../tools/board-gpio/program.he"),
        "atmega328p",
        Options {
            optimize,
            cpu_hz: Some(16_000_000),
            ..Options::default()
        },
    )
    .unwrap();
    let loop_id = module.lifecycle.as_ref().unwrap().loop_body.0;
    let text = llvm::emit(&module, true).unwrap();
    let call = format!("  call addrspace(1) void @helium_f{loop_id}()");
    assert_eq!(text.matches(&call).count(), 1);
    let text = text.replace(
        &call,
        &format!("{call}\n  call addrspace(1) void @b2a_poll()"),
    );
    let text = format!("{text}\ndeclare void @b2a_poll() addrspace(1)\n");
    let dir = Path::new("build/board-gpio");
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(if optimize {
            "optimized-ir.ll"
        } else {
            "plain-ir.ll"
        }),
        text,
    )
    .unwrap();
}
