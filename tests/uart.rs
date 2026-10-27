use helium::{
    backend::microcontroller::ATMEGA328P,
    compile_llvm, compile_typed_ir,
    diagnostic::{DiagnosticCode, Severity},
    ir::{self, DeviceConfig, Op, Options, StartupPhase},
};

const DEVICE: &str = "device debug=UART0 {baud:38400, mode:TX,}";
fn options() -> Options {
    Options {
        cpu_hz: Some(16_000_000),
        verify: true,
        optimize: false,
    }
}
fn source(body: &str) -> String {
    format!("{DEVICE} {body}")
}

#[test]
fn baud_selection_and_clock_contract() {
    let c = helium::uart::plan(&ATMEGA328P, 16_000_000, 38400).unwrap();
    assert_eq!((c.divider, c.double_speed), (25, false));
    let c = helium::uart::plan(&ATMEGA328P, 16_000_000, 57600).unwrap();
    assert_eq!((c.divider, c.double_speed), (34, true));
    for baud in [0, 1, 115200, u64::MAX] {
        assert!(
            helium::uart::plan(&ATMEGA328P, 16_000_000, baud).is_err(),
            "{baud}"
        );
    }
    assert!(
        compile_typed_ir(&source("loop {}"), "atmega328p", Options::default())
            .unwrap_err()
            .iter()
            .any(|d| d.code() == DiagnosticCode::Device)
    );
}

#[test]
fn uart_startup_and_intrinsics_have_explicit_ir() {
    let m=compile_typed_ir(&source("setup {debug.flush(); debug.write_byte(65);} loop {var ready:bool=debug.try_write_byte(66);}"),"atmega328p",options()).unwrap();
    assert!(matches!(m.devices[0].config, DeviceConfig::UartTx(_)));
    assert_eq!(m.devices[0].phase, StartupPhase::BeforeSetup);
    let ops = &m.functions[m.entry.0].blocks[0].instructions;
    assert!(matches!(ops[2].op, Op::DeviceStart(_)));
    assert!(!ops.iter().any(|i| matches!(i.op, Op::InterruptEnable)));
    assert!(m
        .functions
        .iter()
        .flat_map(|f| &f.blocks)
        .flat_map(|b| &b.instructions)
        .any(|i| matches!(i.op, Op::Intrinsic { .. })));
    assert_eq!(m.functions.len(), 4); // No synthetic Helium function bodies.
    let text = helium::backend::llvm::emit(&m, false).unwrap();
    assert!(text.contains("@helium_uart_0_write_byte"));
    assert!(text.contains("store volatile i8 64, ptr inttoptr (i16 192 to ptr)"));
}

#[test]
fn blocking_warning_propagates_through_real_functions_and_survives_optimization() {
    let s=source("fn leaf(){debug.write_byte(65);} fn helper(){leaf();} timer t=Timer1 {every:1ms, interrupt {helper();}} loop {}");
    let analyzed = helium::analyze(&s).unwrap();
    assert_eq!(analyzed.warnings().len(), 1);
    let d = &analyzed.warnings()[0];
    assert_eq!(d.severity(), Severity::Warning);
    assert_eq!(d.code(), DiagnosticCode::IsrBlocking);
    assert_eq!(d.labels.len(), 2);
    assert!(d.render(&s).contains("warning[W_ISR_BLOCKING]"));
    for optimize in [false, true] {
        let c = compile_llvm(
            &s,
            "atmega328p",
            Options {
                optimize,
                ..options()
            },
        )
        .unwrap();
        assert_eq!(c.warnings.len(), 1);
    }
    for body in [
        "setup{debug.flush();} loop{noInterrupt{debug.write_byte(65);}}",
        "timer t=Timer1 {every:1ms, interrupt {var ok:bool=debug.try_write_byte(65);}} loop{}",
    ] {
        assert!(compile_llvm(&source(body), "atmega328p", options())
            .unwrap()
            .warnings
            .is_empty());
    }
}

#[test]
fn uart_signature_resources_and_shared_access_are_checked() {
    for body in [
        "loop{debug.flush(1);}",
        "loop{debug.write_byte(true);}",
        "loop{debug.write_byte(256);}",
        "loop{debug.read_byte();}",
        "loop{var x=debug;}",
        "def tx=PortD.1 OUTPUT; loop{}",
        "device other=UART0 {baud:38400,mode:TX,} loop{}",
        "timer t=Timer1{every:1ms,interrupt{debug.write_byte(65);}} loop{debug.write_byte(66);}",
    ] {
        assert!(
            compile_llvm(&source(body), "atmega328p", options()).is_err(),
            "{body}"
        );
    }
    let shared=source("noInterrupt fn send(){debug.write_byte(65);} timer t=Timer1{every:1ms,interrupt{send();}} loop{noInterrupt{send();}}");
    assert_eq!(
        compile_llvm(&shared, "atmega328p", options())
            .unwrap()
            .warnings
            .len(),
        1
    );
}

#[test]
fn malformed_device_and_truncated_calls_do_not_panic() {
    for invalid in [
        "device d=UART0{}",
        "device d=UART0{baud:1,baud:2,mode:TX,}",
        "device d=UART0{baud:1,mode:RX,}",
        "device d=UART0{baud:1,mode:TX}",
    ] {
        assert!(helium::analyze(invalid).is_err());
    }
    let s = source("setup{debug.write_byte(65);} loop{}");
    for end in 0..s.len() {
        let _ = helium::analyze(&s[..end]);
    }
}

#[test]
fn verifier_rejects_invalid_uart_devices_and_calls() {
    let m = compile_typed_ir(
        &source("setup{debug.write_byte(65);} loop{}"),
        "atmega328p",
        options(),
    )
    .unwrap();
    for mutation in 0..5 {
        let mut bad = m.clone();
        match mutation {
            0 => bad.devices[0].phase = StartupPhase::AfterSetup,
            1 => {
                let DeviceConfig::UartTx(c) = &mut bad.devices[0].config else {
                    unreachable!()
                };
                c.divider += 1;
            }
            2 => bad.devices.push(bad.devices[0].clone()),
            _ => {
                let i = bad
                    .functions
                    .iter_mut()
                    .flat_map(|f| &mut f.blocks)
                    .flat_map(|b| &mut b.instructions)
                    .find(|i| matches!(i.op, Op::Intrinsic { .. }))
                    .unwrap();
                if let Op::Intrinsic {
                    device, arguments, ..
                } = &mut i.op
                {
                    if mutation == 3 {
                        device.0 = 99;
                    } else {
                        arguments.clear();
                    }
                }
            }
        }
        assert!(ir::verify::verify(&bad).is_err(), "mutation {mutation}");
    }
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain, Node and AVR8js; no flashing"]
fn uart_polling_and_isr_execute_at_all_optimization_levels() {
    use helium::backend::{
        llvm,
        llvm_toolchain::{self, Optimization},
    };
    use std::{path::Path, process::Command};
    for scenario in 0..3 {
        let isr = scenario == 1;
        let body = if scenario == 2 {
            "def mark=PortB.0 OUTPUT; var done:u8=0; var ticks:u16=0;
             timer t=Timer1{every:100us,interrupt{ticks=ticks+1;}}
             loop{if done==0 {debug.write_byte(65); debug.flush(); mark=HIGH;
                noInterrupt{debug.write_byte(66);debug.flush();mark=LOW;} done=1;}}"
        } else if isr {
            "def mark=PortB.0 OUTPUT; var done:u8=0; var progress:u16=0; var snapshot:u16=0;
             timer t=Timer1{every:1ms,interrupt{snapshot=progress; if done==0 {debug.write_byte(68); debug.flush(); mark=HIGH; done=1;}}}
             setup{debug.flush(); debug.write_byte(65); debug.flush();}
             loop{noInterrupt{progress=progress+1;}}"
        } else {
            "def mark=PortB.0 OUTPUT; var done:u8=0; var busy:bool=false;
             setup{debug.flush(); debug.write_byte(65); debug.write_byte(66);
               busy=!debug.try_write_byte(99); debug.flush(); mark=HIGH;
               noInterrupt{debug.write_byte(67);debug.flush();mark=LOW;} done=1;}
             loop{}"
        };
        let s = source(body);
        for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
            let module = compile_typed_ir(
                &s,
                "atmega328p",
                Options {
                    optimize: matches!(level, Optimization::Speed),
                    ..options()
                },
            )
            .unwrap();
            assert_eq!(module.warnings.len(), if isr { 2 } else { 0 });
            let text = llvm::emit(&module, true).unwrap();
            let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("build/uart-tests/{scenario}-{level:?}.hex"));
            let build =
                llvm_toolchain::build(&text, &ATMEGA328P, &destination, "hex", level).unwrap();
            let simulation =
                Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
                    .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/simulate-avr.cjs"))
                    .arg(build.directory)
                    .env("HELIUM_UART_CYCLES", "80000")
                    .output()
                    .unwrap();
            assert!(
                simulation.status.success(),
                "{}",
                String::from_utf8_lossy(&simulation.stderr)
            );
            let actual = String::from_utf8(simulation.stdout).unwrap();
            assert!(
                actual.contains(if scenario == 2 {
                    "\"uartBytes\":[65,66]"
                } else if isr {
                    "\"uartBytes\":[65,68]"
                } else {
                    "\"uartBytes\":[65,66,67]"
                }),
                "{actual}"
            );
            assert!(
                actual.contains("helium_g0:1"),
                "flush never completed: {actual}"
            );
            assert!(
                actual.contains("\"complete\":true,\"irq\":false"),
                "flush/IRQ state: {actual}"
            );
            if scenario == 2 {
                assert!(
                    actual.contains("\"complete\":true,\"irq\":true"),
                    "caller IRQ state not restored: {actual}"
                );
                assert!(
                    !actual.contains("helium_g1:0\""),
                    "timer did not run: {actual}"
                );
            } else if isr {
                assert!(
                    !actual.contains("helium_g1:0\""),
                    "main did not resume: {actual}"
                );
            } else {
                assert!(
                    actual.contains("helium_g1:1"),
                    "busy try must fail without writing: {actual}"
                );
            }
            assert!(actual.contains("\"nestedInterrupt\":false"), "{actual}");
        }
    }
}
