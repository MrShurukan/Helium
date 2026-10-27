use helium::{
    backend::{llvm, microcontroller::ATMEGA328P},
    compile_llvm, compile_typed_ir,
    ir::{self, Options},
    timer,
};

#[test]
fn source_verifier_and_emitter_agree_on_every_gpio_boundary() {
    for port in ["A", "B", "C", "D", "E"] {
        for bit in 0..=8 {
            let expected = match port {
                "B" | "D" => bit < 8,
                "C" => bit < 7,
                _ => false,
            };
            let source = format!("def Pin=Port{port}.{bit} INPUT; loop {{Pin=OUTPUT; Pin=!Pin;}}");
            assert_eq!(
                compile_llvm(&source, "atmega328p", Options::default()).is_ok(),
                expected,
                "{port}.{bit}"
            );
            // Forge the same pin in otherwise valid IR to exercise independent boundaries.
            let mut module = compile_typed_ir(
                "def Pin=PortB.0 INPUT; loop {Pin=OUTPUT; Pin=!Pin;}",
                "atmega328p",
                Options::default(),
            )
            .unwrap();
            module.pins[0].port = port.into();
            module.pins[0].bit = bit;
            assert_eq!(
                ir::verify::verify(&module).is_ok(),
                expected,
                "verifier {port}.{bit}"
            );
            assert_eq!(
                llvm::emit(&module, false).is_ok(),
                expected,
                "emitter {port}.{bit}"
            );
        }
    }
    assert!(!ATMEGA328P.has_digital_pin("B", usize::MAX));
}

#[test]
fn gpio_operations_use_correct_input_direction_and_latch_addresses() {
    // Independent hardware expectations, not values read back from the descriptor.
    for (port, input, direction, output) in
        [("B", 35, 36, 37), ("C", 38, 39, 40), ("D", 41, 42, 43)]
    {
        let text = compile_llvm(
            &format!("def Pin=Port{port}.0 INPUT; loop {{Pin=OUTPUT; Pin=!Pin;}}"),
            "atmega328p",
            Options::default(),
        )
        .unwrap();
        assert!(text.contains(&format!(
            "load volatile i8, ptr inttoptr (i16 {input} to ptr)"
        )));
        assert!(!text
            .lines()
            .any(|l| l.contains("store volatile") && l.contains(&format!("i16 {input} to ptr"))));
        for address in [direction, output] {
            let io = address - 32;
            assert!(
                text.contains(&format!("sbi {io}, 0")) || text.contains(&format!("cbi {io}, 0"))
            );
            assert!(!text
                .lines()
                .any(|l| l.contains("store volatile")
                    && l.contains(&format!("i16 {address} to ptr"))));
        }
    }
}

#[test]
fn timer_limits_encodings_and_register_write_order_remain_exact() {
    for (divisor, control) in [(1, 9), (8, 10), (64, 11), (256, 12), (1024, 13)] {
        let period = divisor * 65_536 * 1000u64;
        let source = format!("timer clock=Timer1 {{every:{period}ns, interrupt {{}}}} loop {{}}");
        let module = compile_typed_ir(
            &source,
            "atmega328p",
            Options {
                cpu_hz: Some(1_000_000),
                ..Options::default()
            },
        )
        .unwrap();
        let config = module.timer.as_ref().unwrap();
        assert_eq!(u64::from(config.prescaler), divisor);
        assert_eq!(config.ticks, 65_536);
        let text = llvm::emit(&module, false).unwrap();
        assert!(text.contains("define avr_signalcc void @__vector_11()"));
        let writes = text
            .lines()
            .filter(|l| l.contains("store volatile i8"))
            .map(str::trim)
            .collect::<Vec<_>>();
        let expected = [
            (129, 0),
            (111, 0),
            (128, 0),
            (130, 0),
            (133, 0),
            (132, 0),
            (137, 255),
            (136, 255),
            (54, 39),
            (111, 2),
            (129, control),
        ]
        .map(|(address, value)| {
            format!("store volatile i8 {value}, ptr inttoptr (i16 {address} to ptr), align 1")
        });
        assert_eq!(writes, expected);
        let mut broken = module;
        broken.timer.as_mut().unwrap().prescaler = 2;
        assert!(ir::verify::verify(&broken).is_err());
        assert!(llvm::emit(&broken, false).is_err());
    }
    assert!(timer::plan(1_000_000, 67_108_864_001, 0).is_err());
    assert!(timer::validate_clock(20_000_000).is_ok());
    assert!(timer::validate_clock(20_000_001).is_err());
    assert!(timer::validate_clock(0).is_err());
}
