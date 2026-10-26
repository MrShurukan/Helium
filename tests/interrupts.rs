use helium::diagnostic::{
    source::{FileId, SourceFile},
    DiagnosticCode,
};
use helium::ir::{interpret::Interpreter, *};
use helium::{analyze, compile_typed_ir};

fn compile(source: &str, optimize: bool) -> Module {
    compile_typed_ir(
        source,
        "atmega328p",
        Options {
            cpu_hz: None,
            verify: true,
            optimize,
        },
    )
    .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"))
}

#[test]
fn requirements_are_explicit_and_checked_in_every_expression_context() {
    for body in [
        "restricted();",
        "var x:u8=restricted();",
        "if restricted()==1 {}",
        "while restricted()==1 {}",
        "if false { restricted(); }",
    ] {
        let source = format!("noInterrupt fn restricted():u8 {{ return 1; }} loop {{ {body} }}");
        let errors = analyze(&source).unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.code() == DiagnosticCode::InterruptContext));
    }

    // A protected caller does not silently change an ordinary helper's contract.
    let source = "noInterrupt fn restricted() {} fn helper() { restricted(); } loop { noInterrupt { helper(); } }";
    assert!(analyze(source)
        .unwrap_err()
        .iter()
        .any(|e| e.code() == DiagnosticCode::InterruptContext));

    compile("noInterrupt fn outer() { inner(); } noInterrupt fn inner() {} setup { outer(); } loop { noInterrupt { outer(); } }", false);
    compile("fn helper() { noInterrupt { restricted(); } } noInterrupt fn restricted() {} loop { helper(); }", false);
}

#[test]
fn requirement_diagnostics_point_to_call_and_modifier_in_unicode_source() {
    let source = "// РџСЂРµСЂС‹РІР°РЅРёСЏ\nnoInterrupt fn restricted() {}\nloop { restricted(); }";
    let error = analyze(source)
        .unwrap_err()
        .into_iter()
        .find(|e| e.code() == DiagnosticCode::InterruptContext)
        .unwrap();
    let file = SourceFile {
        id: FileId(0),
        name: "test.he",
        text: source,
    };
    assert_eq!(
        &source[file.span(error.location.unwrap()).unwrap().bytes],
        "restricted()"
    );
    assert_eq!(
        &source[file.span(error.labels[0].location).unwrap().bytes],
        "noInterrupt"
    );
    assert!(error
        .render(source)
        .contains("Wrap the call in noInterrupt"));
}

#[test]
fn nested_scopes_restore_both_initial_states_on_all_return_paths() {
    let source = "fn choose(x:u8):u16 {
        noInterrupt {
            if x==0 { return 513; }
            noInterrupt { if x==1 { return 1027; } }
            while x>2 { noInterrupt { return 1541; } }
        }
        return 2055;
    } loop {}";

    for optimize in [false, true] {
        let module = compile(source, optimize);
        for enabled in [false, true] {
            for (input, expected) in [(0, 513), (1, 1027), (2, 2055), (3, 1541)] {
                let mut runner = Interpreter::new(&module, 1000).unwrap();
                runner.interrupts_enabled = enabled;
                let result = runner
                    .invoke(FunctionId(3), &[Constant::new(Type::U8, input)])
                    .unwrap()
                    .unwrap();
                assert_eq!(result.bits, expected);
                assert_eq!(runner.interrupts_enabled, enabled);
            }
        }
    }
}

#[test]
fn setup_protects_calls_and_modifier_does_not_insert_a_block() {
    let module = compile(
        "noInterrupt fn restricted() {} setup { restricted(); } loop {}",
        false,
    );
    assert!(module.functions[3].requires_no_interrupt);
    assert!(module.functions[3]
        .blocks
        .iter()
        .all(|b| b.instructions.is_empty()));
    let mut runner = Interpreter::new(&module, 1000).unwrap();
    runner.interrupts_enabled = true;
    assert!(runner.invoke(FunctionId(3), &[]).is_err());
    runner.invoke(FunctionId(1), &[]).unwrap();
    assert!(runner.interrupts_enabled);
}

#[test]
fn callee_return_preserves_the_callers_protection() {
    let source = "fn outer():u8 { noInterrupt { inner(); restricted(); return 9; } }
        fn inner() { noInterrupt { return; } }
        noInterrupt fn restricted() {}
        loop {}";

    for optimize in [false, true] {
        let module = compile(source, optimize);
        let mut runner = Interpreter::new(&module, 1000).unwrap();
        runner.interrupts_enabled = true;
        assert_eq!(runner.invoke(FunctionId(3), &[]).unwrap().unwrap().bits, 9);
        assert!(runner.interrupts_enabled);
    }
}

#[test]
fn verifier_rejects_scope_disagreement_at_a_branch_merge() {
    let mut module = compile(
        "fn f(flag:bool) { if flag { noInterrupt {} } } loop {}",
        false,
    );
    let branch = module.functions[3]
        .blocks
        .iter_mut()
        .find(|b| {
            b.instructions
                .iter()
                .any(|i| matches!(i.op, Op::InterruptSave))
        })
        .unwrap();
    branch
        .instructions
        .retain(|i| !matches!(i.op, Op::InterruptRestore(_)));
    assert_eq!(
        verify::verify(&module).unwrap_err().kind,
        verify::ErrorKind::InterruptState
    );
}

#[test]
fn scopes_obey_variable_visibility_and_return_analysis() {
    compile("fn f():u8 { noInterrupt { return 7; } } loop {}", false);
    assert!(analyze("loop { noInterrupt { var local:u8=1; } local=2; }").is_err());
    assert!(analyze("loop { noInterrupt { return; } }").is_err());
    assert!(
        analyze("fn f(flag:bool):u8 { noInterrupt { if flag { return 1; } } } loop {}").is_err()
    );
}

#[test]
fn malformed_and_truncated_interrupt_syntax_never_panics() {
    let source =
        "noInterrupt fn f():u8 { noInterrupt { return 1; } } loop { noInterrupt { f(); } }";
    for (end, _) in source.char_indices() {
        let _ = analyze(&source[..end]);
    }
    for source in [
        "noInterrupt {} loop {}",
        "noInterrupt noInterrupt fn f() {} loop {}",
        "loop { noInterrupt f(); }",
        "loop { noInterrupt fn f() {} }",
    ] {
        assert!(analyze(source).is_err(), "{source}");
    }
}

#[test]
fn verifier_rejects_missing_forged_and_out_of_order_restores() {
    let source = "fn f() { noInterrupt { noInterrupt {} } } loop {}";
    let original = compile(source, false);
    for mode in 0..3 {
        let mut module = original.clone();
        let instructions = &mut module.functions[3].blocks[0].instructions;
        match mode {
            0 => {
                instructions.pop();
            }
            1 => {
                instructions.swap(2, 3);
            }
            _ => {
                instructions[0].op = Op::Const(Constant::new(Type::U8, 1));
            }
        }
        assert_eq!(
            verify::verify(&module).unwrap_err().kind,
            verify::ErrorKind::InterruptState
        );
    }
    let mut module = compile(
        "noInterrupt fn restricted() {} loop { noInterrupt { restricted(); } }",
        false,
    );
    module.functions[2].blocks[0]
        .instructions
        .retain(|i| matches!(i.op, Op::Call(..)));
    assert_eq!(
        verify::verify(&module).unwrap_err().kind,
        verify::ErrorKind::InterruptState
    );
}

#[test]
fn llvm_interrupt_effects_have_memory_clobbers_and_no_automatic_sei() {
    let module = compile("fn f() { noInterrupt { noInterrupt {} } } loop {}", true);
    let llvm = helium::backend::llvm::emit(&module, false).unwrap();
    assert!(llvm.contains("in $0, 63\\0Acli"));
    assert!(llvm.contains("out 63, $0"));
    assert_eq!(
        llvm.matches("asm sideeffect").count(),
        llvm.matches("~{memory}").count()
    );
    assert!(!llvm.contains("sei"));
}

#[test]
#[ignore = "requires LLVM AVR, AVR GCC/binutils, Node and AVR8js; no hardware flashing"]
fn machine_restores_interrupt_state_and_return_values_at_all_optimization_levels() {
    use helium::backend::{
        llvm,
        llvm_toolchain::{self, Optimization},
    };
    use std::{fs, path::Path, process::Command};

    let source = "def Led=PortB.0 OUTPUT;
        fn scenario(early:bool):u16 {
            Led=LOW;
            noInterrupt {
                Led=HIGH;
                noInterrupt { Led=LOW; if early { return 513; } }
                Led=HIGH;
            }
            Led=LOW;
            return 1027;
        }
        loop {}";

    for optimize_ir in [false, true] {
        let module = compile(source, optimize_ir);
        for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
            for enabled in [false, true] {
                let text = llvm::emit(&module, true).unwrap();
                let (definitions, _) = text.split_once("\ndefine i16 @main()").unwrap();
                let definitions = definitions.replace(
                    "@helium_f3(i1 %v0) addrspace(1) {",
                    "@helium_f3(i1 %v0) addrspace(1) noinline {",
                );
                // Only the test harness enables interrupts. Observe SREG immediately
                // after each call, before any arithmetic can change its other flags.
                let toggle = if enabled { "sei" } else { "cli" };
                let harness = format!(
                    r#"
@helium_g0 = global i16 0
@helium_g1 = global i16 0
@helium_g2 = global i8 0
@helium_g3 = global i8 0
define i16 @main() addrspace(1) {{
entry:
    call void asm sideeffect "{toggle}", "~{{memory}}"()
    %early = call addrspace(1) i16 @helium_f3(i1 true)
    %state1 = call i8 asm sideeffect "in $0, 63", "=r,~{{memory}}"()
    %bit1 = and i8 %state1, 128
    store volatile i16 %early, ptr @helium_g0
    store volatile i8 %bit1, ptr @helium_g2
    %normal = call addrspace(1) i16 @helium_f3(i1 false)
    %state2 = call i8 asm sideeffect "in $0, 63", "=r,~{{memory}}"()
    %bit2 = and i8 %state2, 128
    store volatile i16 %normal, ptr @helium_g1
    store volatile i8 %bit2, ptr @helium_g3
    ret i16 0
}}
"#
                );
                let destination = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                    "build/interrupt-tests/{level:?}-{optimize_ir}-{enabled}.hex"
                ));
                let build =
                    llvm_toolchain::build(&(definitions + &harness), &destination, "hex", level)
                        .unwrap();
                let simulation =
                    Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
                        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/simulate-avr.cjs"))
                        .arg(&build.directory)
                        .env("HELIUM_TRACE_INTERRUPTS", "1")
                        .output()
                        .unwrap();
                assert!(
                    simulation.status.success(),
                    "{}",
                    String::from_utf8_lossy(&simulation.stderr)
                );
                let actual = String::from_utf8(simulation.stdout).unwrap();
                let bit = if enabled { 128 } else { 0 };
                assert!(actual.contains(&format!("\"globals\":[\"helium_g0:513\",\"helium_g1:1027\",\"helium_g2:{bit}\",\"helium_g3:{bit}\"]")), "{actual}");
                assert!(actual.contains(&format!("\"interruptStates\":[{enabled},false,false,{enabled},false,false,false,{enabled}]")), "{actual}");
                let asm = fs::read_to_string(build.directory.join("disassembly.txt")).unwrap();
                assert!(asm.contains("cli"));
                assert!(asm.contains("<helium_f3>"));
            }
        }
    }
}
