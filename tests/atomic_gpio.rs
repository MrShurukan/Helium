use helium::{
    analyze, compile_typed_ir,
    diagnostic::{DiagnosticCode, DiagnosticKind, SharedAccessReason},
    ir::Options,
};

#[test]
fn pin_conflicts_are_pairwise_and_explain_the_actual_operation() {
    let prefix = "def A=PortB.0 OUTPUT; def B=PortB.1 OUTPUT;";
    for (main, irq, reason) in [
        ("A=HIGH;", "A=LOW;", SharedAccessReason::SamePin),
        ("A=!A;", "A=HIGH;", SharedAccessReason::SamePin),
        (
            "A=INPUT;",
            "var level=!A;",
            SharedAccessReason::PinConfiguration,
        ),
        (
            "var level=!A;",
            "A=OUTPUT;",
            SharedAccessReason::PinConfiguration,
        ),
    ] {
        let source =
            format!("{prefix} timer t=Timer1 {{every:1ms, interrupt {{{irq}}}}} loop {{{main}}}");
        let errors = analyze(&source).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(*e.kind, DiagnosticKind::SharedAccess{reason:r,..} if r==reason)),
            "{errors:?}"
        );
        analyze(&source.replace(
            &format!("loop {{{main}}}"),
            &format!("loop {{noInterrupt {{{main}}}}}"),
        ))
        .unwrap();
    }
    // A's read/read pair remains safe even though B has a write in main.
    analyze(&format!(
        "{prefix} timer t=Timer1 {{every:1ms, interrupt {{var a=!A;}}}} loop {{var a=!A; B=HIGH;}}"
    ))
    .unwrap();
    // Distinct pins remain independent through helpers, including mode changes.
    analyze(&format!("{prefix} fn mainWork(){{A=INPUT_PULLUP; A=OUTPUT; A=!A;}} fn irqWork(){{B=OUTPUT; B=!B;}} timer t=Timer1 {{every:1ms, interrupt {{irqWork();}}}} loop {{mainWork();}}")).unwrap();
}

#[test]
fn byte_rmw_and_multibyte_snapshots_still_require_protection() {
    for (ty, reason) in [
        ("u8", SharedAccessReason::SharedMemory),
        ("u16", SharedAccessReason::MultiByteMemory),
    ] {
        for body in ["value=value+1;", "var snapshot=value;"] {
            let source=format!("var value:{ty}=0; timer t=Timer1 {{every:1ms, interrupt {{value=value+1;}}}} loop {{{body}}}");
            let errors = analyze(&source).unwrap_err();
            assert!(errors
                .iter()
                .any(|e| matches!(*e.kind,DiagnosticKind::SharedAccess{reason:r,..} if r==reason)));
        }
    }
    let source="def A=PortB.0 OUTPUT; fn helper(){A=HIGH;} timer t=Timer1 {every:1ms, interrupt {A=LOW;}} loop {noInterrupt {helper();}}";
    assert!(analyze(source)
        .unwrap_err()
        .iter()
        .any(|e| e.code() == DiagnosticCode::SharedAccess));
}

#[test]
fn atomic_io_encoding_refuses_unsupported_addresses() {
    use helium::backend::microcontroller::atomic_bit_io;
    assert_eq!(atomic_bit_io(0x20), Some(0));
    assert_eq!(atomic_bit_io(0x3f), Some(31));
    for address in [0, 0x1f, 0x40, 0x80, u16::MAX] {
        assert_eq!(atomic_bit_io(address), None);
    }
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain, Node and AVR8js; no flashing"]
fn gpio_updates_survive_isr_at_every_main_instruction_boundary() {
    use helium::backend::{
        llvm,
        llvm_toolchain::{self, Optimization},
    };
    use std::{fs, path::Path, process::Command};
    for port in ["B", "C", "D"] {
        let source=format!("def A=Port{port}.0 OUTPUT; def B=Port{port}.1 OUTPUT;
            def M=Port{port}.2 INPUT; def N=Port{port}.3 INPUT;
            var mainPhase:bool=false; var irqPhase:bool=false;
            fn tick() {{irqPhase=!irqPhase; B=irqPhase ? HIGH : LOW; N=INPUT_PULLUP; N=OUTPUT; N=INPUT;}}
            timer clock=Timer1 {{every:1ms, interrupt {{tick();}}}}
            loop {{mainPhase=!mainPhase; A=mainPhase ? HIGH : LOW; M=INPUT_PULLUP; M=OUTPUT; M=INPUT;}}");
        for optimize in [false, true] {
            let module = compile_typed_ir(
                &source,
                "atmega328p",
                Options {
                    optimize,
                    cpu_hz: Some(16_000_000),
                    ..Options::default()
                },
            )
            .unwrap();
            for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
                let text = llvm::emit(&module, true).unwrap();
                let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("build/gpio-atomic/{port}-{optimize}-{level:?}.hex"));
                let build = llvm_toolchain::build(
                    &text,
                    &helium::backend::microcontroller::ATMEGA328P,
                    &destination,
                    "hex",
                    level,
                )
                .unwrap();
                let asm = fs::read_to_string(build.directory.join("disassembly.txt")).unwrap();
                assert!(asm.contains("sbi") && asm.contains("cbi"), "{asm}");
                let (ddr_io, port_io) = match port {
                    "B" => (4, 5),
                    "C" => (7, 8),
                    _ => (10, 11),
                };
                for line in asm.lines() {
                    let fields = line.split('\t').collect::<Vec<_>>();
                    if fields.len() < 4 {
                        continue;
                    }
                    let mnemonic = fields[2].trim();
                    let address = fields[3]
                        .split(',')
                        .next()
                        .unwrap()
                        .trim()
                        .strip_prefix("0x")
                        .and_then(|n| u16::from_str_radix(n, 16).ok());
                    if let Some(address) = address {
                        assert!(
                            !(mnemonic == "out" && [ddr_io, port_io].contains(&address)),
                            "whole-port OUT: {line}"
                        );
                        assert!(
                            !(mnemonic == "sts" && [ddr_io + 32, port_io + 32].contains(&address)),
                            "whole-port STS: {line}"
                        );
                    }
                }
                let run = |directory: &Path| {
                    Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
                        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/simulate-avr.cjs"))
                        .arg(directory)
                        .env("HELIUM_TIMER_CYCLES", "160000")
                        .env("HELIUM_GPIO_RACE_PORT", port)
                        .output()
                        .unwrap()
                };
                let output = run(&build.directory);
                assert!(
                    output.status.success(),
                    "{port} {optimize} {level:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let actual = String::from_utf8(output.stdout).unwrap();
                for field in ["mainWrites", "isrWrites", "mainChanges", "isrChanges"] {
                    let prefix = format!("\"{field}\":");
                    let count = actual
                        .split_once(&prefix)
                        .unwrap()
                        .1
                        .split(|c: char| !c.is_ascii_digit())
                        .next()
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    assert!(count > 20, "insufficient exercise {field}: {actual}");
                }
                assert!(actual.contains("\"nestedInterrupt\":false"));
                fs::write(build.directory.join("simulation.json"), &actual).unwrap();
                // Negative control: a stale whole-port read/write must fail this
                // same scheduler. Uses a real IN/OUT pair, not a forced hook error.
                if port == "B" && !optimize && matches!(level, Optimization::None) {
                    let mut injected = false;
                    let broken=text.lines().map(|line| {
                        if !injected && line.contains("asm sideeffect \"sbrc") {
                            injected=true;
                            "  call void asm sideeffect \"in r24, 5\\0Aout 5, r24\", \"~{r24},~{memory}\"()".to_owned()
                        } else {line.to_owned()}
                    }).collect::<Vec<_>>().join("\n");
                    assert!(injected);
                    let negative = llvm_toolchain::build(
                        &broken,
                        &helium::backend::microcontroller::ATMEGA328P,
                        &destination.with_file_name("negative.hex"),
                        "hex",
                        level,
                    )
                    .unwrap();
                    let failure = run(&negative.directory);
                    assert!(!failure.status.success());
                    assert!(
                        String::from_utf8_lossy(&failure.stderr).contains("GPIO lost update"),
                        "{}",
                        String::from_utf8_lossy(&failure.stderr)
                    );
                }
            }
        }
    }
}
