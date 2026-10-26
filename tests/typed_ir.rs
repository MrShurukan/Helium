use helium::ir::interpret::{Event, Interpreter};
use helium::ir::verify::{verify, ErrorKind};
use helium::{compile_typed_ir, ir::*};

fn compile(source: &str) -> Module {
    compile_typed_ir(
        source,
        "atmega328p",
        Options {
            cpu_hz: None,
            verify: true,
            optimize: false,
        },
    )
    .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"))
}

fn optimized(source: &str) -> Module {
    compile_typed_ir(
        source,
        "atmega328p",
        Options {
            cpu_hz: None,
            verify: true,
            optimize: true,
        },
    )
    .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"))
}

fn run(module: &Module, readings: &[bool]) -> (Vec<Constant>, Vec<Event>) {
    let mut runner = Interpreter::new(module, 10000).unwrap();
    runner
        .inputs
        .insert(PinId(0), readings.iter().copied().collect());
    // Invoke finite lifecycle bodies to compare complete traces, not prefixes cut at
    // different instruction counts before and after optimization.
    runner.invoke(FunctionId(1), &[]).unwrap();
    runner.invoke(FunctionId(2), &[]).unwrap();
    (runner.globals, runner.events)
}

#[test]
fn wrapping_arithmetic_and_signed_comparisons_match_language_rules() {
    let source = "var a:u8=0; var b:i8=0; var c:u16=0; var d:i16=0; var less:bool=false;
        setup { a=255; b=127; c=65535; d=-32768; }
        loop { a=a+1; b=b+1; c=c*2; d=-d; less=b<0; }";
    for module in [compile(source), optimized(source)] {
        let (globals, _) = run(&module, &[]);
        assert_eq!(
            globals.iter().map(|v| v.bits).collect::<Vec<_>>(),
            [0, 128, 65534, 32768, 1]
        );
        assert_eq!(globals[1].ty.numeric_value(globals[1].bits), -128);
    }
}

#[test]
fn loops_ternaries_and_widening_preserve_values() {
    let source = "var result:u16=0; var negative:i16=0; loop {
        var i:u8=0; while i<5 { i=i+1; }
        var a:u8=250; var b:u8=10;
        var wrapped:u16=a+b;
        var selected:u16=true ? a : b;
        var signed:i8=-2; negative=signed;
        result=wrapped+selected+i;
    }";
    let module = compile(source);
    let (globals, _) = run(&module, &[]);
    assert_eq!(globals[0].bits, 259);
    assert_eq!(globals[1].ty.numeric_value(globals[1].bits), -2);
    assert_eq!(run(&optimized(source), &[]), run(&module, &[]));
}

#[test]
fn pin_inversion_reads_physical_level_once_and_does_not_read_output_latch() {
    let source = "def Led=PortC.5 OUTPUT; loop { Led=HIGH; Led=!Led; }";
    for module in [compile(source), optimized(source)] {
        assert_eq!(
            run(&module, &[false]).1,
            [
                Event::Write(PinId(0), true),
                Event::Read(PinId(0), false),
                Event::Write(PinId(0), true)
            ]
        );
    }
}

#[test]
fn short_circuit_and_ternary_only_execute_selected_effects() {
    let source = "def Key=PortD.2 INPUT; def Led=PortC.5 OUTPUT;
        loop {
            false and Key==HIGH;
            true or Key==HIGH;
            true and Key==HIGH;
            false or Key==HIGH;
            Led = true ? !Key : !!Key;
        }";
    let expected = vec![
        Event::Read(PinId(0), true),
        Event::Read(PinId(0), false),
        Event::Read(PinId(0), false),
        Event::Write(PinId(1), true),
    ];
    for module in [compile(source), optimized(source)] {
        assert_eq!(run(&module, &[true, false, false]).1, expected);
    }
}

#[test]
fn left_to_right_evaluation_snapshots_mutable_variables() {
    let source = "var result:u8=0; loop { var x:u8=2; result=x+(x=7); }";
    assert_eq!(run(&compile(source), &[]).0[0].bits, 9);
    assert_eq!(run(&optimized(source), &[]).0[0].bits, 9);
}

#[test]
fn chained_assignments_and_unused_pin_reads_keep_effect_order() {
    let source = "def A=PortB.0 OUTPUT; def B=PortB.1 OUTPUT;
        loop { A=B=HIGH; A; A; }";
    let expected = vec![
        Event::Write(PinId(1), true),
        Event::Write(PinId(0), true),
        Event::Read(PinId(0), false),
        Event::Read(PinId(0), true),
    ];
    for module in [compile(source), optimized(source)] {
        assert_eq!(run(&module, &[false, true]).1, expected);
    }
}

#[test]
fn entry_initializes_globals_then_pins_then_setup_then_loop() {
    let module = compile(
        "def Led=PortC.5 OUTPUT; var x:u8=4; setup {x=x+1; Led=LOW;} loop {x=x+1; Led=HIGH;}",
    );
    let mut runner = Interpreter::new(&module, 100).unwrap();
    assert_eq!(runner.run_entry(), Err(interpret::Error::StepLimit));
    assert!(runner.globals[0].bits > 5);
    assert_eq!(runner.events[0], Event::Mode(PinId(0), PinMode::Output));
    assert_eq!(runner.events[1], Event::Write(PinId(0), false));
    assert!(runner.events[2..]
        .iter()
        .all(|e| *e == Event::Write(PinId(0), true)));
}

#[test]
fn optimizations_remove_only_dead_pure_work_and_unreachable_effects() {
    let source = "def Key=PortD.2 INPUT; loop { 1+2*3; Key; if false { Key; } }";
    let original = compile(source);
    let simplified = optimized(source);
    let instructions = |m: &Module| {
        m.functions
            .iter()
            .flat_map(|f| &f.blocks)
            .map(|b| b.instructions.len())
            .sum::<usize>()
    };
    assert!(instructions(&simplified) < instructions(&original));
    assert_eq!(run(&simplified, &[true]), run(&original, &[true]));
    assert_eq!(run(&simplified, &[true]).1, [Event::Read(PinId(0), true)]);
}

#[test]
fn unsupported_features_fail_with_source_diagnostics() {
    for source in [
        "loop { var x:f32=1.5; }",
        "loop {var x=4/2;}",
        "loop {for i=0 to 3 {}}",
        "loop { var mode:PinMode=INPUT; }",
    ] {
        let errors = compile_typed_ir(source, "atmega328p", Options::default()).unwrap_err();
        assert!(errors
            .iter()
            .all(|e| e.code() != helium::diagnostic::DiagnosticCode::Internal));
        assert!(errors.iter().any(|e| e.location.is_some()));
    }
}

#[test]
fn verifier_default_follows_build_profile_and_can_be_forced() {
    assert_eq!(Options::default().verify, cfg!(debug_assertions));
    let source = "loop {var a:u8=1;}";
    assert_eq!(
        compile(source).dump(),
        compile_typed_ir(
            source,
            "atmega328p",
            Options {
                cpu_hz: None,
                verify: false,
                optimize: false
            }
        )
        .unwrap()
        .dump()
    );
}

fn bare() -> Module {
    let mut module = compile("loop {}");
    module.entry = FunctionId(0);
    module.functions.truncate(1);
    module.functions[0].blocks.truncate(1);
    module.functions[0].blocks[0].instructions.clear();
    module.functions[0].blocks[0].terminator = Terminator::Return(None);
    module
}

fn value(index: usize) -> ValueId {
    ValueId {
        function: FunctionId(0),
        index,
    }
}
fn block(index: usize) -> BlockId {
    BlockId {
        function: FunctionId(0),
        index,
    }
}
fn local(index: usize) -> LocalId {
    LocalId {
        function: FunctionId(0),
        index,
    }
}
fn instruction(result: Option<(ValueId, Type)>, op: Op) -> Instruction {
    Instruction {
        result,
        op,
        origin: Origin::Generated("test"),
    }
}
fn constant(index: usize, ty: Type, bits: u64) -> Instruction {
    instruction(Some((value(index), ty)), Op::Const(Constant::new(ty, bits)))
}

#[test]
fn verifier_catches_a_value_defined_in_only_one_branch() {
    let mut module = bare();
    let blocks = &mut module.functions[0].blocks;
    blocks[0].instructions.push(constant(0, Type::Bool, 0));
    blocks[0].terminator = Terminator::Branch {
        condition: value(0),
        yes: block(1),
        no: block(2),
    };
    blocks.push(Block {
        id: block(1),
        instructions: vec![constant(1, Type::U8, 30)],
        terminator: Terminator::Jump(block(2)),
        origin: Origin::Generated("test"),
    });
    blocks.push(Block {
        id: block(2),
        instructions: vec![instruction(
            Some((value(2), Type::U8)),
            Op::Binary(Binary::Add, value(1), value(1)),
        )],
        terminator: Terminator::Return(None),
        origin: Origin::Generated("test"),
    });
    assert_eq!(
        verify(&module).unwrap_err().kind,
        ErrorKind::UnavailableValue
    );
}

#[test]
fn verifier_checks_local_definite_initialization_across_branches_and_backedges() {
    let mut module = bare();
    module.functions[0].locals.push(Storage {
        name: "x".into(),
        ty: Type::U8,
        origin: Origin::Generated("test"),
    });
    let blocks = &mut module.functions[0].blocks;
    blocks[0].instructions.push(constant(0, Type::Bool, 0));
    blocks[0].terminator = Terminator::Branch {
        condition: value(0),
        yes: block(1),
        no: block(2),
    };
    blocks.push(Block {
        id: block(1),
        instructions: vec![
            constant(1, Type::U8, 30),
            instruction(None, Op::StoreLocal(local(0), value(1))),
        ],
        terminator: Terminator::Jump(block(2)),
        origin: Origin::Generated("test"),
    });
    blocks.push(Block {
        id: block(2),
        instructions: vec![instruction(
            Some((value(2), Type::U8)),
            Op::LoadLocal(local(0)),
        )],
        terminator: Terminator::Jump(block(1)),
        origin: Origin::Generated("test"),
    });
    assert_eq!(
        verify(&module).unwrap_err().kind,
        ErrorKind::UninitializedLocal
    );

    module.functions[0].blocks[0].instructions.extend([
        constant(3, Type::U8, 20),
        instruction(None, Op::StoreLocal(local(0), value(3))),
    ]);
    verify(&module).unwrap();
}

#[test]
fn verifier_rejects_structural_and_type_corruption() {
    let mut module = bare();
    module.functions[0].blocks[0].terminator = Terminator::Jump(block(999));
    assert_eq!(verify(&module).unwrap_err().kind, ErrorKind::InvalidId);

    let mut module = bare();
    module.functions[0].blocks[0].instructions =
        vec![constant(0, Type::U8, 1), constant(0, Type::U8, 2)];
    assert_eq!(
        verify(&module).unwrap_err().kind,
        ErrorKind::DuplicateDefinition
    );

    let mut module = bare();
    module.functions[0].blocks[0].instructions = vec![
        constant(0, Type::U8, 1),
        constant(1, Type::Bool, 0),
        instruction(
            Some((value(2), Type::U8)),
            Op::Binary(Binary::Add, value(0), value(1)),
        ),
    ];
    assert_eq!(verify(&module).unwrap_err().kind, ErrorKind::TypeMismatch);

    let mut module = bare();
    module.functions[0].blocks[0].instructions = vec![instruction(
        Some((value(0), Type::U8)),
        Op::LoadLocal(LocalId {
            function: FunctionId(1),
            index: 0,
        }),
    )];
    assert_eq!(verify(&module).unwrap_err().kind, ErrorKind::InvalidId);
}

#[test]
fn function_calls_check_signatures_and_execute_results() {
    let mut module = bare();
    let arg = ValueId {
        function: FunctionId(1),
        index: 0,
    };
    let next = ValueId {
        function: FunctionId(1),
        index: 1,
    };
    let entry = BlockId {
        function: FunctionId(1),
        index: 0,
    };
    module.functions.push(Function {
        requires_no_interrupt: false,
        id: FunctionId(1),
        name: "double".into(),
        parameters: vec![(arg, Type::U8)],
        return_type: Some(Type::U8),
        locals: vec![],
        entry,
        origin: Origin::Generated("test"),
        blocks: vec![Block {
            id: entry,
            instructions: vec![instruction(
                Some((next, Type::U8)),
                Op::Binary(Binary::Add, arg, arg),
            )],
            terminator: Terminator::Return(Some(next)),
            origin: Origin::Generated("test"),
        }],
    });
    module.globals.push(Storage {
        name: "result".into(),
        ty: Type::U8,
        origin: Origin::Generated("test"),
    });
    module.functions[0].blocks[0].instructions = vec![
        constant(0, Type::U8, 130),
        instruction(
            Some((value(1), Type::U8)),
            Op::Call(FunctionId(1), vec![value(0)]),
        ),
        instruction(None, Op::StoreGlobal(GlobalId(0), value(1))),
    ];
    verify(&module).unwrap();
    let mut runner = Interpreter::new(&module, 100).unwrap();
    runner.run_entry().unwrap();
    assert_eq!(runner.globals[0].bits, 4);

    module.functions[0].blocks[0].instructions[1].op = Op::Call(FunctionId(1), vec![]);
    assert_eq!(verify(&module).unwrap_err().kind, ErrorKind::TypeMismatch);
}

#[test]
fn recursive_calls_are_rejected() {
    let mut module = bare();
    module.functions[0].blocks[0]
        .instructions
        .push(instruction(None, Op::Call(FunctionId(0), vec![])));
    assert_eq!(verify(&module).unwrap_err().kind, ErrorKind::RecursiveCall);
}

#[test]
fn all_integer_widths_have_explicit_wrapping_and_literal_ranges() {
    for (name, max) in [
        ("u8", 255u64),
        ("u16", 65535),
        ("u32", u64::from(u32::MAX)),
        ("u64", u64::MAX),
    ] {
        let source = format!("var x:{name}=0; setup {{x={max};}} loop {{x=x+1;}}");
        for module in [compile(&source), optimized(&source)] {
            assert_eq!(run(&module, &[]).0[0].bits, 0);
        }
    }
    for (name, max) in [
        ("i8", 127i64),
        ("i16", 32767),
        ("i32", i64::from(i32::MAX)),
        ("i64", i64::MAX),
    ] {
        let source = format!("var x:{name}=0; setup {{x={max};}} loop {{x=x+1;}}");
        let module = compile(&source);
        let globals = run(&module, &[]).0;
        assert_eq!(
            globals[0].ty.numeric_value(globals[0].bits),
            -i128::from(max) - 1
        );
        assert_eq!(run(&module, &[]), run(&optimized(&source), &[]));
    }

    // Unconstrained integer expressions default to i32; they must not silently truncate.
    let errors =
        compile_typed_ir("loop {4294967296==0;}", "atmega328p", Options::default()).unwrap_err();
    assert_eq!(
        errors[0].code(),
        helium::diagnostic::DiagnosticCode::LiteralRange
    );
}

#[test]
fn optimized_branching_preserves_effects_for_every_input_combination() {
    let source = "def Key=PortD.2 INPUT; def Led=PortC.5 OUTPUT; loop {
        var i:u8=0;
        while i<3 {
            if Key==HIGH and (Key==LOW or Key==HIGH) {Led=HIGH;} else {Led=LOW;}
            i=i+1;
        }
    }";
    let original = compile(source);
    let simplified = optimized(source);
    for pattern in 0..512 {
        let readings = (0..9)
            .map(|bit| pattern & (1 << bit) != 0)
            .collect::<Vec<_>>();
        assert_eq!(
            run(&original, &readings),
            run(&simplified, &readings),
            "pattern {pattern}"
        );
    }
}

#[test]
fn verifier_rejects_wrong_results_returns_pins_and_constants() {
    let mut module = bare();
    module.functions[0].blocks[0].instructions.push(instruction(
        Some((value(0), Type::Bool)),
        Op::Const(Constant::new(Type::U8, 1)),
    ));
    assert_eq!(verify(&module).unwrap_err().kind, ErrorKind::TypeMismatch);

    let mut module = bare();
    module.functions[0].blocks[0]
        .instructions
        .push(constant(0, Type::U8, 1));
    module.functions[0].blocks[0].terminator = Terminator::Return(Some(value(0)));
    assert_eq!(verify(&module).unwrap_err().kind, ErrorKind::TypeMismatch);

    let mut module = bare();
    module.functions[0].blocks[0].instructions.push(instruction(
        Some((value(0), Type::PinState)),
        Op::ReadPin(PinId(8)),
    ));
    assert_eq!(verify(&module).unwrap_err().kind, ErrorKind::InvalidPin);

    let mut module = bare();
    module.functions[0].blocks[0].instructions.push(instruction(
        Some((value(0), Type::Bool)),
        Op::Const(Constant {
            ty: Type::Bool,
            bits: 2,
        }),
    ));
    assert_eq!(verify(&module).unwrap_err().kind, ErrorKind::TypeMismatch);
}

#[test]
fn source_origins_and_dump_are_preserved_deterministically() {
    let source = "def Led=PortC.5 OUTPUT; loop {Led=!Led;}";
    let module = compile(source);
    let body = &module.functions[2].blocks[0];
    assert!(body
        .instructions
        .iter()
        .all(|instruction| matches!(instruction.origin, Origin::Source(_))));
    assert_eq!(module.dump(), compile(source).dump());
    assert_eq!(
        module.dump().trim_end(),
        include_str!("snapshots/typed-gpio.ir")
            .replace("\r\n", "\n")
            .trim_end()
    );
}
