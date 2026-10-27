use helium::diagnostic::DiagnosticCode;
use helium::{
    analyze, compile_typed_ir,
    ir::{self, Module, Options},
    timer,
};

fn options() -> Options {
    Options {
        cpu_hz: Some(16_000_000),
        verify: true,
        optimize: false,
    }
}
fn compile(source: &str) -> Module {
    compile_typed_ir(source, "atmega328p", options()).unwrap_or_else(|e| panic!("{source}\n{e:#?}"))
}
const TIMER: &str = "timer clock=Timer1 { every: 1s, interrupt { seconds=seconds+1; } }";

#[test]
fn exact_timer_selection_and_tolerance_use_integer_arithmetic() {
    let c = timer::plan(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        1_000_000_000,
        0,
    )
    .unwrap();
    assert_eq!((c.prescaler, c.ticks, c.cycles()), (256, 62500, 16_000_000));
    assert_eq!(
        timer::plan(
            &helium::backend::microcontroller::ATMEGA328P,
            16_000_000,
            1_000_000,
            0
        )
        .unwrap()
        .prescaler,
        1
    );
    assert!(timer::plan(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        100,
        24
    )
    .is_err());
    assert_eq!(
        timer::plan(
            &helium::backend::microcontroller::ATMEGA328P,
            16_000_000,
            100,
            25
        )
        .unwrap()
        .ticks,
        2
    );
    assert!(timer::plan(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        0,
        999
    )
    .is_err());
    assert!(timer::plan(&helium::backend::microcontroller::ATMEGA328P, 0, 1000, 0).is_err());
    assert!(timer::plan(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        u64::MAX,
        0
    )
    .is_err());
    assert!(timer::parse_duration("18446744073709551615s").is_err());
    assert_eq!(timer::parse_duration("500ms").unwrap(), 500_000_000);
    for invalid in ["1", "1.5s", "-1s", "1minute", "s"] {
        assert!(timer::parse_duration(invalid).is_err());
    }
    let error = timer::plan(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        100,
        0,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("Nearest shorter"));
    assert!(error.contains("Nearest longer"));
    assert!(error.contains("at least 25ns"));
}

#[test]
fn timer_declarations_require_clock_unique_resource_and_supported_target() {
    let source = format!("var seconds:u16=0; {TIMER} loop {{}}");
    let error = compile_typed_ir(&source, "atmega328p", Options::default()).unwrap_err();
    assert_eq!(error[0].code(), DiagnosticCode::Timer);
    assert!(error[0].message().contains("--cpu-hz"));
    for declaration in [
        TIMER.replace("Timer1", "Timer2"),
        format!("{TIMER} {}", TIMER.replace("clock", "other")),
    ] {
        let source = format!("var seconds:u16=0; {declaration} loop {{}}");
        assert!(compile_typed_ir(&source, "atmega328p", options())
            .unwrap_err()
            .iter()
            .any(|e| e.code() == DiagnosticCode::Timer));
    }
    assert!(analyze(&format!(
        "var clock:u8=0; var seconds:u16=0; {TIMER} loop {{}}"
    ))
    .is_err());
}

#[test]
fn unsafe_shared_access_is_rejected_in_both_directions_and_through_helpers() {
    for source in [
        format!("var seconds:u16=0; {TIMER} loop {{ var now=seconds; }}"),
        "var seconds:u16=0; timer clock=Timer1 { every:1s, interrupt { var now=seconds; } } loop { seconds=1; }".into(),
        format!("var seconds:u16=0; {TIMER} fn read():u16 {{return seconds;}} loop {{noInterrupt {{read();}}}}"),
        "var seconds:u16=0; fn change() {seconds=seconds+1;} timer clock=Timer1 {every:1s, interrupt {change();}} loop {noInterrupt {var now=seconds;}}".into(),
    ] {
        let errors = analyze(&source).unwrap_err();
        assert!(errors.iter().any(|e| e.code()==DiagnosticCode::SharedAccess), "{errors:#?}");
    }
    compile(&format!("var seconds:u16=0; {TIMER} noInterrupt fn read():u16 {{return seconds;}} loop {{noInterrupt {{read();}}}}"));
    compile("var shared:u8=1; timer clock=Timer1 {every:1s, interrupt { var a=shared; }} loop {var b=shared;}");
    compile(
        "var private:u8=0; timer clock=Timer1 {every:1s, interrupt {private=private+1;}} loop {}",
    );
}

#[test]
fn gpio_conflicts_are_limited_to_the_same_pin() {
    let source = "def A=PortB.0 OUTPUT; def B=PortB.1 OUTPUT;
        timer clock=Timer1 {every:1s, interrupt {A=HIGH;}} loop {B=LOW;}";
    compile(source);
    assert!(analyze(&source.replace("loop {B=LOW;}", "loop {A=LOW;}"))
        .unwrap_err()
        .iter()
        .any(|e| e.code() == DiagnosticCode::SharedAccess));
    compile(&source.replace("loop {B=LOW;}", "loop {noInterrupt {A=LOW;}}"));
}

#[test]
fn malformed_timer_declarations_and_prefixes_do_not_panic() {
    let source = "timer clock=Timer1 {every:1s, tolerance:1ms, interrupt {noInterrupt {}}} loop {}";
    for (end, _) in source.char_indices() {
        let _ = analyze(&source[..end]);
    }
    for declaration in [
        "timer t=Timer1 {}",
        "timer t=Timer1 {every:1s,}",
        "timer t=Timer1 {every:1s, every:2s, interrupt {}}",
        "timer t=Timer1 {every:1s, interrupt {} interrupt {}}",
        "timer t=Timer1 {every:1hour, interrupt {}}",
        "timer t=Timer1 {every:1s, interrupt {return;}}",
    ] {
        assert!(
            analyze(&format!("{declaration} loop {{}}")).is_err(),
            "{declaration}"
        );
    }
}

#[test]
fn ir_and_llvm_preserve_handler_visibility_and_startup_order() {
    let module = compile(include_str!("../examples/timer-seconds.he"));
    let entry = &module.functions[0].blocks[0].instructions;
    assert!(matches!(entry[0].op, ir::Op::InterruptDisable));
    assert!(matches!(entry.last().unwrap().op, ir::Op::InterruptEnable));
    let text = helium::backend::llvm::emit(&module, false).unwrap();
    assert!(text.contains("define avr_signalcc void @__vector_11()"));
    assert!(text.contains("load volatile i16, ptr @helium_g0"));
    assert!(text.contains("store volatile i16"));
    let mut optimized = module.clone();
    ir::optimize::optimize(&mut optimized, true).unwrap();
    assert!(helium::backend::llvm::emit(&optimized, false)
        .unwrap()
        .contains("@__vector_11"));
    let mut broken = module.clone();
    broken.devices[0].timer_mut().ticks = 0;
    assert!(ir::verify::verify(&broken).is_err());
    let mut broken = module.clone();
    broken.functions[0].blocks[0].instructions.swap(0, 1);
    assert!(ir::verify::verify(&broken).is_err());
    let mut broken = module.clone();
    broken.interrupt_globals.clear();
    assert!(ir::verify::verify(&broken).is_err());
}

#[test]
fn branching_initializers_preserve_startup_and_verification_policy() {
    let source = "var flag:bool=true; var value:u8=flag ? 7 : 9;
        var a:bool=flag and false; var b:bool=a or flag;
        timer clock=Timer1 {every:1s, interrupt {}}
        setup {value=value+1;} loop {}";
    for verify in [false, true] {
        for optimize in [false, true] {
            let settings = Options {
                verify,
                optimize,
                ..options()
            };
            let module = compile_typed_ir(source, "atmega328p", settings).unwrap();
            helium::compile_llvm(source, "atmega328p", settings).unwrap();
            ir::verify::verify(&module).unwrap();
            let mut runner = ir::interpret::Interpreter::new(&module, 200).unwrap();
            assert_eq!(runner.run_entry(), Err(ir::interpret::Error::StepLimit));
            assert_eq!(
                runner.globals.iter().map(|v| v.bits).collect::<Vec<_>>(),
                [1, 8, 0, 1]
            );
        }
    }
}

#[test]
fn lifecycle_roles_and_order_are_verified_without_fixed_function_numbers() {
    let source = "var x:u8=true ? 1 : 2; timer t=Timer1 {every:1s, interrupt {}} loop {}";
    let module = compile(source);
    for mutation in 0..5 {
        let mut broken = module.clone();
        let entry = &mut broken.functions[broken.entry.0];
        match mutation {
            0 => entry.blocks[0].instructions.swap(1, 2),
            1 => entry.blocks[0].instructions.swap(2, 3),
            2 => {
                let start = entry.blocks[0].instructions[3].clone();
                entry.blocks[1].instructions.push(start);
            }
            3 => entry.blocks[1].terminator = ir::Terminator::Jump(entry.entry),
            _ => broken.lifecycle.as_mut().unwrap().initialize = broken.entry,
        }
        assert!(ir::verify::verify(&broken).is_err(), "mutation {mutation}");
    }
    let mut relocated = module.clone();
    let roles = relocated.lifecycle.as_mut().unwrap();
    std::mem::swap(&mut roles.setup, &mut roles.loop_body);
    let entry = &mut relocated.functions[relocated.entry.0];
    entry.blocks[0].instructions[2].op = ir::Op::Call(roles.setup, vec![]);
    entry.blocks[1].instructions[0].op = ir::Op::Call(roles.loop_body, vec![]);
    ir::verify::verify(&relocated).unwrap();
}

#[test]
fn configurable_check_matches_cli_clock_policy() {
    let source = include_str!("../examples/timer-seconds.he");
    for cpu_hz in [None, Some(0), Some(16_000_000), Some(20_000_001)] {
        let result = helium::check_with_options(
            source,
            "atmega328p",
            Options {
                cpu_hz,
                ..Options::default()
            },
        );
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_helium"));
        command.args(["check", "examples/timer-seconds.he", "--mcu", "atmega328p"]);
        if let Some(hz) = cpu_hz {
            command.args(["--cpu-hz", &hz.to_string()]);
        }
        let cli = command.output().unwrap();
        assert_eq!(result.is_ok(), cli.status.success(), "clock {cpu_hz:?}");
        assert_eq!(result.is_ok(), cpu_hz == Some(16_000_000));
    }
    assert!(helium::check("loop {}", "atmega328p").is_ok());
    assert!(helium::check_with_options(
        "loop {}",
        "atmega328p",
        Options {
            cpu_hz: Some(0),
            ..Options::default()
        }
    )
    .is_err());
}

#[test]
fn calculator_and_build_reports_are_available_without_external_tools() {
    use std::process::Command;
    let exe = env!("CARGO_BIN_EXE_helium");
    let output = Command::new(exe)
        .args([
            "timer-options",
            "--mcu",
            "atmega328p",
            "--cpu-hz",
            "16000000",
            "--timer",
            "Timer1",
            "--every",
            "1s",
        ])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(output.status.success());
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(report.contains("OCR1A=62499"));
    let output = Command::new(exe)
        .args([
            "check",
            "examples/timer-seconds.he",
            "--mcu",
            "atmega328p",
            "--cpu-hz",
            "16000000",
        ])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("period=1.000000000000s"));
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain, Node and AVR8js; no flashing"]
fn timer_interrupts_execute_and_resume_main_at_all_optimization_levels() {
    use helium::backend::{
        llvm,
        llvm_toolchain::{self, Optimization},
    };
    use std::{fs, path::Path, process::Command};

    // Start near a byte carry to exercise protected 16-bit snapshots.
    let source = "var seconds:u16=true ? 254 : 0; var snapshot:u16=0; var progress:u16=0;
        var busy:bool=(false and true) or false; var failure:u8=0;
        noInterrupt fn tick() { seconds=seconds+1; if busy {failure=failure+1;} }
        timer clock=Timer1 {every:1ms, interrupt {tick();}}
        fn initialize_busy() {busy=true; var i:u16=0; while i<1000 {i=i+1;} busy=false;}
        setup {initialize_busy();}
        loop {noInterrupt {snapshot=seconds;} progress=progress+1;}";
    for optimize in [false, true] {
        let mut settings = options();
        settings.optimize = optimize;
        let module = compile_typed_ir(source, "atmega328p", settings).unwrap();
        for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
            let mut text = llvm::emit(&module, true).unwrap();
            // Keep an actual call from the interrupt context to a user function.
            text = text.replace(
                "@helium_f3() addrspace(1) {",
                "@helium_f3() addrspace(1) noinline {",
            );
            let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("build/timer-tests/{level:?}-{optimize}.hex"));
            let build = llvm_toolchain::build(
                &text,
                &helium::backend::microcontroller::ATMEGA328P,
                &destination,
                "hex",
                level,
            )
            .unwrap();
            let simulation =
                Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
                    .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/simulate-avr.cjs"))
                    .arg(&build.directory)
                    .env("HELIUM_TIMER_CYCLES", "160000")
                    .output()
                    .unwrap();
            assert!(
                simulation.status.success(),
                "{}",
                String::from_utf8_lossy(&simulation.stderr)
            );
            let actual = String::from_utf8(simulation.stdout).unwrap();
            assert!(actual.contains("\"nestedInterrupt\":false"), "{actual}");
            assert!(
                actual.contains("helium_g4:0"),
                "ISR ran during setup protection: {actual}"
            );
            let global = |index| {
                let prefix = format!("helium_g{index}:");
                actual
                    .split_once(&prefix)
                    .unwrap()
                    .1
                    .split('"')
                    .next()
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
            };
            let ticks = global(0) - 254;
            assert!((3..=10).contains(&ticks), "timer failed to tick: {actual}");
            assert!(
                (254..=global(0)).contains(&global(1)),
                "torn snapshot: {actual}"
            );
            assert!(global(2) > 0, "main did not resume: {actual}");
            let cycles = actual
                .split_once("\"interruptCycles\":[")
                .unwrap()
                .1
                .split(']')
                .next()
                .unwrap()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.parse::<u64>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(cycles.len() as u64, ticks);
            for pair in cycles.windows(2) {
                assert!(
                    (pair[1] - pair[0]).abs_diff(16000) < 250,
                    "wrong timer period: {actual}"
                );
            }
            let asm = fs::read_to_string(build.directory.join("disassembly.txt")).unwrap();
            assert!(asm.contains("reti"), "handler must return with RETI");
            assert!(asm.contains("<__vector_11>"));
        }
    }
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain, Node and AVR8js; no flashing"]
fn long_critical_section_defers_and_coalesces_timer_events() {
    use helium::backend::{
        llvm,
        llvm_toolchain::{self, Optimization},
    };
    use std::{path::Path, process::Command};

    // ISR reads spin, making its stores observable and preventing loop removal.
    let source = "var ticks:u16=0; var spin:u16=0; var observed:u16=0;
        var busy:bool=false; var failure:u8=0; var done:bool=false;
        timer clock=Timer1 {every:1ms, interrupt {
            ticks=ticks+1; observed=spin; if busy {failure=failure+1;}
        }}
        loop {if !done {noInterrupt {
            busy=true; while spin<4000 {spin=spin+1;} busy=false;
        } done=true;}}";
    let module = compile(source);
    let text = llvm::emit(&module, true).unwrap();
    let destination = Path::new(env!("CARGO_MANIFEST_DIR")).join("build/timer-tests/coalesced.hex");
    let build = llvm_toolchain::build(
        &text,
        &helium::backend::microcontroller::ATMEGA328P,
        &destination,
        "hex",
        Optimization::Size,
    )
    .unwrap();
    let output = Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/simulate-avr.cjs"))
        .arg(&build.directory)
        .env("HELIUM_TIMER_CYCLES", "320000")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual = String::from_utf8(output.stdout).unwrap();
    assert!(actual.contains("helium_g1:4000"), "{actual}");
    assert!(actual.contains("helium_g2:4000"), "{actual}");
    assert!(
        actual.contains("helium_g4:0"),
        "ISR ran inside the protected region: {actual}"
    );
    assert!(
        actual.contains("helium_g5:1"),
        "main failed to resume: {actual}"
    );
    let ticks = actual
        .split_once("helium_g0:")
        .unwrap()
        .1
        .split('"')
        .next()
        .unwrap()
        .parse::<u32>()
        .unwrap();
    assert!(
        (1..18).contains(&ticks),
        "missed events should coalesce: {actual}"
    );
}

#[test]
fn timer_diagnostics_locate_period_resource_and_shared_access() {
    use helium::diagnostic::source::{FileId, SourceFile};
    let source = "var seconds:u16=0; timer clock=Timer1 {every:1s, interrupt {seconds=seconds+1;}} loop {var now=seconds;}";
    let file = SourceFile {
        id: FileId(0),
        name: "timer.he",
        text: source,
    };
    let error = analyze(source)
        .unwrap_err()
        .into_iter()
        .find(|e| e.code() == DiagnosticCode::SharedAccess)
        .unwrap();
    assert_eq!(
        &source[file.span(error.location.unwrap()).unwrap().bytes],
        "seconds"
    );
    assert_eq!(
        &source[file.span(error.labels[0].location).unwrap().bytes],
        "seconds"
    );
    assert!(error.labels[0].location.start_offset < error.location.unwrap().start_offset);

    let source = "timer clock=Timer1 {every:100ns, interrupt {}} loop {}";
    let file = SourceFile {
        id: FileId(0),
        name: "timer.he",
        text: source,
    };
    let error = compile_typed_ir(source, "atmega328p", options())
        .unwrap_err()
        .remove(0);
    assert_eq!(error.code(), DiagnosticCode::Timer);
    assert_eq!(
        &source[file.span(error.location.unwrap()).unwrap().bytes],
        "100ns"
    );
    for operation in ["var copy=clock;", "clock=1;", "clock();"] {
        let source =
            format!("timer clock=Timer1 {{every:1s, interrupt {{}}}} loop {{{operation}}}");
        assert!(analyze(&source).is_err(), "{source}");
    }
}
