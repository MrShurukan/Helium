use helium::{
    compile_typed_ir,
    ir::{self, DeviceId, Op, Options, StartupPhase},
};

fn timer_module() -> ir::Module {
    compile_typed_ir(
        "timer clock=Timer1 {every:1s, interrupt {}} setup {} loop {}",
        "atmega328p",
        Options {
            cpu_hz: Some(16_000_000),
            ..Options::default()
        },
    )
    .unwrap()
}

#[test]
fn startup_has_separate_device_and_cpu_irq_effects() {
    let module = timer_module();
    let roles = module.lifecycle.as_ref().unwrap();
    let instructions = &module.functions[module.entry.0].blocks[0].instructions;
    assert_eq!(instructions.len(), 5);
    assert!(matches!(instructions[0].op, Op::InterruptDisable));
    assert!(matches!(instructions[1].op, Op::Call(id, _) if id == roles.initialize));
    assert!(matches!(instructions[2].op, Op::Call(id, _) if id == roles.setup));
    assert!(matches!(instructions[3].op, Op::DeviceStart(DeviceId(0))));
    assert!(matches!(instructions[4].op, Op::InterruptEnable));
    assert_eq!(module.interrupts[0].device, DeviceId(0));
    assert_eq!(module.interrupts[0].vector, "__vector_11");
    let text = helium::backend::llvm::emit(&module, false).unwrap();
    assert_eq!(text.matches("asm sideeffect \"sei\"").count(), 1);
    let start = text
        .find("store volatile i8 12, ptr inttoptr (i16 129 to ptr)")
        .unwrap();
    assert!(start < text.find("asm sideeffect \"sei\"").unwrap());
    // No artificial device or IRQ enable is added to ordinary programs.
    let plain = compile_typed_ir("loop {}", "atmega328p", Options::default()).unwrap();
    assert!(plain.devices.is_empty() && plain.interrupts.is_empty());
    assert_eq!(
        plain.functions[plain.entry.0].blocks[0].instructions.len(),
        2
    );
}

#[test]
fn verifier_rejects_resource_binding_and_lifecycle_corruption() {
    let module = timer_module();
    for mutation in 0..12 {
        let mut broken = module.clone();
        match mutation {
            0 => {
                broken.devices.push(broken.devices[0].clone());
            }
            1 => broken.interrupts.clear(),
            2 => {
                broken.interrupts.push(broken.interrupts[0].clone());
            }
            3 => broken.interrupts[0].device = DeviceId(99),
            4 => broken.interrupts[0].vector = "__vector_12",
            5 => broken.devices[0].phase = StartupPhase::BeforeSetup,
            6 => {
                broken.functions[broken.entry.0].blocks[0]
                    .instructions
                    .pop();
            }
            7 => broken.functions[broken.entry.0].blocks[0]
                .instructions
                .swap(2, 4),
            8 => {
                broken.functions[broken.entry.0].blocks[0].instructions[3].op =
                    Op::DeviceStart(DeviceId(99))
            }
            9 => {
                let op = broken.functions[broken.entry.0].blocks[0].instructions[3].clone();
                broken.functions[broken.entry.0].blocks[1]
                    .instructions
                    .push(op);
            }
            10 => broken.lifecycle = None,
            _ => {
                let setup = broken.lifecycle.as_ref().unwrap().setup;
                broken.functions[setup.0].blocks[0]
                    .instructions
                    .push(ir::Instruction {
                        result: None,
                        op: Op::Call(broken.interrupts[0].handler, vec![]),
                        origin: ir::Origin::Generated("invalid handler call"),
                    });
            }
        }
        assert!(ir::verify::verify(&broken).is_err(), "mutation {mutation}");
    }
}

#[test]
fn reservations_preserve_the_first_owner_across_resource_consumers() {
    use helium::resources::{HardwareResource as H, Reservations};
    let mut owners = Reservations::default();
    let tx_pin = H::Pin {
        port: "D".into(),
        bit: 1,
    };
    assert_eq!(owners.claim(tx_pin.clone(), "gpio"), Ok(()));
    assert_eq!(owners.claim(tx_pin.clone(), "peripheral"), Err("gpio"));
    assert_eq!(owners.claim(tx_pin, "third owner"), Err("gpio"));
    assert_eq!(
        owners.claim(H::Peripheral("Timer1".into()), "clock"),
        Ok(())
    );
    assert_eq!(
        owners.claim(H::Peripheral("Timer1".into()), "other clock"),
        Err("clock")
    );
    assert_eq!(
        owners.claim(H::InterruptVector("__vector_11".into()), "clock"),
        Ok(())
    );
    assert_eq!(
        owners.claim(H::InterruptVector("__vector_11".into()), "other irq"),
        Err("clock")
    );
}
