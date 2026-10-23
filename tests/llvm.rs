use helium::backend::{
    llvm,
    llvm_toolchain::{self, Optimization},
};
use helium::ir::{
    interpret::{Event, Interpreter},
    *,
};
use std::{collections::HashMap, fs, path::Path, process::Command};

fn compile(source: &str) -> Module {
    helium::compile_typed_ir(
        source,
        "atmega328p",
        Options {
            verify: true,
            optimize: false,
        },
    )
    .unwrap()
}

fn finite(mut module: Module) -> Module {
    // Test exactly one full lifecycle, including global and GPIO initialization.
    module.functions[0].blocks[1].terminator = Terminator::Return(None);
    verify::verify(&module).unwrap();
    module
}

#[test]
fn llvm_adapter_preserves_effects_and_wrapping_contract() {
    let module = compile("def Led=PortC.5 OUTPUT; var x:i8=127; loop { x=x+1; Led=!Led; }");
    let text = llvm::emit(&module, false).unwrap();
    assert!(text.contains("load volatile i8, ptr inttoptr (i16 38 to ptr)"));
    assert_eq!(
        text.matches("load volatile i8, ptr inttoptr (i16 38 to ptr)")
            .count(),
        1
    );
    assert!(text.contains("store volatile i8"));
    assert!(!text.contains(" nsw "));
    assert!(!text.contains(" nuw "));
    assert!(!text.contains("mustprogress"));
    assert!(text.contains("addrspace(1)"));
    assert_eq!(text, llvm::emit(&module, false).unwrap());
}

#[test]
fn unsupported_machine_width_is_reported_before_running_tools() {
    let module = compile("var x:u32=1; loop { x=x+1; }");
    assert!(llvm::emit(&module, false).unwrap_err().contains("8/16-bit"));
}

#[test]
fn llvm_cli_emits_ir_without_external_tools_and_protects_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("build/llvm-cli-test");
    fs::create_dir_all(&root).unwrap();
    let source = root.join("input.he");
    let text = "var x:u8=1; loop { x=x+1; }";
    fs::write(&source, text).unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_helium"))
            .args([
                "build",
                source.to_str().unwrap(),
                "--mcu",
                "atmega328p",
                "--backend",
                "llvm",
            ])
            .args(extra)
            .env("PATH", "")
            .env_remove("HELIUM_LLVM_BIN")
            .env_remove("HELIUM_AVR_BIN")
            .output()
            .unwrap()
    };

    let output = root.join("output.ll");
    assert!(run(&["--emit", "llvm", "-o", output.to_str().unwrap()])
        .status
        .success());
    assert!(fs::read_to_string(output)
        .unwrap()
        .contains("target triple"));
    assert!(!run(&["--emit", "llvm", "-o", source.to_str().unwrap()])
        .status
        .success());
    assert_eq!(fs::read_to_string(&source).unwrap(), text);
    assert!(!run(&["--emit", "llvm", "--opt", "size"]).status.success());
    assert!(!run(&["--emit", "ir", "--optimize-ir"]).status.success());

    let hex = root.join("existing.hex");
    fs::write(&hex, "keep existing firmware").unwrap();
    let failure = run(&["--emit", "hex", "-o", hex.to_str().unwrap()]);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("Cannot run"));
    assert_eq!(fs::read_to_string(hex).unwrap(), "keep existing firmware");
}

fn quote_list(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn compare_machine(
    module: &Module,
    pattern: &[bool],
    name: &str,
    level: Optimization,
    export: bool,
) {
    let mut runner = Interpreter::new(module, 10000).unwrap();
    for index in 0..module.pins.len() {
        runner
            .inputs
            .insert(PinId(index), pattern.iter().copied().collect());
    }
    runner.run_entry().unwrap();
    let globals = runner
        .globals
        .iter()
        .enumerate()
        .map(|(i, value)| format!("helium_g{i}:{}", value.bits))
        .collect::<Vec<_>>();

    let mut latch = HashMap::<String, u8>::new();
    let mut input = Vec::new();
    let mut events = Vec::new();
    let mut directions = [0u8; 3];
    for event in &runner.events {
        if let Event::Mode(id, mode) = event {
            let pin = &module.pins[id.0];
            let port = match pin.port.as_str() {
                "B" => 0,
                "C" => 1,
                "D" => 2,
                _ => unreachable!(),
            };
            let mask = 1 << pin.bit;
            directions[port] =
                (directions[port] & !mask) | if *mode == PinMode::Output { mask } else { 0 };
        }
        let (id, value, read) = match event {
            Event::Read(id, value) => (*id, *value, true),
            Event::Write(id, value) => (*id, *value, false),
            Event::Mode(_, PinMode::Output) => continue,
            Event::Mode(id, mode) => (*id, *mode == PinMode::InputPullUp, false),
        };
        let pin = &module.pins[id.0];
        let bits = if value { 1u8 << pin.bit } else { 0 };
        if read {
            input.push(format!("{bits:x}"));
            events.push(format!("read:{}:{bits}", pin.port));
        } else {
            let current = latch.entry(pin.port.clone()).or_default();
            *current = (*current & !(1 << pin.bit)) | bits;
            events.push(format!("write:{}:{current}", pin.port));
        }
    }

    let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("build/llvm-tests")
        .join(format!("{name}-{level:?}.hex"));
    let mut text = llvm::emit(module, export).unwrap();
    if name == "calls" || name.starts_with("source-") {
        // Keep a real mixed-width call even under O2; otherwise constant folding
        // would test the optimizer rather than the AVR calling convention.
        text = text
            .lines()
            .map(|line| {
                if line.starts_with("define ")
                    && module
                        .functions
                        .iter()
                        .any(|f| f.id.0 >= 3 && line.contains(&format!("@helium_f{}(", f.id.0)))
                {
                    line.replace("addrspace(1) {", "addrspace(1) noinline {")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let build = llvm_toolchain::build(&text, &destination, "hex", level).unwrap();
    if matches!(level, Optimization::Size) {
        let raw = fs::read_to_string(build.directory.join("module.ll")).unwrap();
        assert!(raw
            .lines()
            .filter(|line| line.starts_with("define "))
            .all(|line| line.contains("minsize optsize")));
    }
    if name == "calls" || name.starts_with("source-") {
        let disassembly = fs::read_to_string(build.directory.join("disassembly.txt")).unwrap();
        assert!(
            disassembly
                .lines()
                .any(|line| line.contains("call") && line.contains("<helium_f3>")),
            "Missing machine call: {disassembly}"
        );
    }
    let simulation = Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/simulate-avr.cjs"))
        .arg(&build.directory)
        .arg(input.join(","))
        .output()
        .unwrap();
    assert!(
        simulation.status.success(),
        "{}\n{}",
        build.directory.display(),
        String::from_utf8_lossy(&simulation.stderr)
    );
    let actual = String::from_utf8(simulation.stdout).unwrap();
    assert!(
        actual.contains(&format!(
            "\"directions\":[{},{},{}]",
            directions[0], directions[1], directions[2]
        )),
        "{actual}"
    );
    assert!(
        actual.contains(&format!("\"globals\":{}", quote_list(&globals))),
        "{name}: {actual}; expected {globals:?}"
    );
    assert!(
        actual.contains(&format!("\"events\":{}", quote_list(&events))),
        "{name}: {actual}; expected {events:?}"
    );
    fs::write(build.directory.join("simulation.json"), &actual).unwrap();
    println!("{name} {level:?}: {} ms; {actual}", build.elapsed_ms);
}

#[test]
#[ignore = "requires LLVM AVR, AVR GCC/binutils, Node and AVR8js"]
fn source_functions_execute_on_avr_with_register_and_stack_arguments() {
    let mut latch = finite(compile(include_str!("../examples/functions-latch.he")));
    let loop_call = latch.functions[0].blocks[1].instructions[0].clone();
    latch.functions[0].blocks[1]
        .instructions
        .extend(std::iter::repeat_n(loop_call, 4));
    let thermostat = finite(compile(include_str!("../examples/functions-thermostat.he")));
    let stack = finite(compile(
        "var result:i16=0;
        fn sum(a:i16,b:i16,c:i16,d:i16,e:i16,f:i16,g:i16,h:i16,i:i16,j:i16,k:i16,l:i16):i16 {
            return a+b+c+d+e+f+g+h+i+j+k+l;
        }
        fn nested(value:i16):i16 { return value+sum(1,2,3,4,5,6,7,8,9,10,11,12); }
        loop { result=nested(-100); }",
    ));
    let effects = finite(compile(
        "def Led=PortB.5 OUTPUT; var count:u8=0; var result:u16=0;
        fn next():u8 { count=count+1; return count; }
        fn pair(a:u8,b:u8):u16 { var first:u16=a; return first*10+b; }
        fn invert(state:PinState):PinState { return !state; }
        fn drive() { Led=invert(HIGH); if count==4 { return; } Led=HIGH; }
        loop { result=pair(next(),next())+pair(next(),next()); drive(); }",
    ));
    let returns = finite(compile("var a:i16=0; var b:i16=0; var sign:i16=0;
        fn choose(x:i16):i16 { if x<0 { return -1; } else { if x==0 { return 0; } else { return x; } } }
        fn find(x:i16):i16 { while x>0 { if x==2 { return x; } x=x-1; } return -1; }
        fn widen(x:i8):i16 { return x; }
        loop { a=choose(-5); b=find(5); sign=widen(-2); }"));

    for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
        compare_machine(
            &latch,
            &[true, false, false, true, false],
            "source-latch",
            level,
            true,
        );
        compare_machine(&thermostat, &[], "source-thermostat", level, true);
        compare_machine(&stack, &[], "source-stack", level, true);
        compare_machine(&effects, &[], "source-effects", level, true);
        compare_machine(&returns, &[], "source-returns", level, true);
    }
}

fn call_fixture() -> Module {
    let mut module = finite(compile("var result:i16=0; loop {}"));
    let id = FunctionId(3);
    let value = |index| ValueId {
        function: id,
        index,
    };
    let block = BlockId {
        function: id,
        index: 0,
    };
    let origin = Origin::Generated("LLVM call test");
    module.functions.push(Function {
        id,
        name: "sum".into(),
        parameters: vec![(value(0), Type::I16), (value(1), Type::I8)],
        return_type: Some(Type::I16),
        locals: vec![],
        entry: block,
        origin: origin.clone(),
        blocks: vec![Block {
            id: block,
            origin: origin.clone(),
            instructions: vec![
                Instruction {
                    result: Some((value(2), Type::I16)),
                    op: Op::Widen(value(1), Type::I16),
                    origin: origin.clone(),
                },
                Instruction {
                    result: Some((value(3), Type::I16)),
                    op: Op::Binary(Binary::Add, value(0), value(2)),
                    origin: origin.clone(),
                },
            ],
            terminator: Terminator::Return(Some(value(3))),
        }],
    });
    let value = |index| ValueId {
        function: FunctionId(2),
        index,
    };
    module.functions[2].blocks[0].instructions = vec![
        Instruction {
            result: Some((value(0), Type::I16)),
            op: Op::Const(Constant::new(Type::I16, 32767)),
            origin: origin.clone(),
        },
        Instruction {
            result: Some((value(1), Type::I8)),
            op: Op::Const(Constant::new(Type::I8, 1)),
            origin: origin.clone(),
        },
        Instruction {
            result: Some((value(2), Type::I16)),
            op: Op::Call(id, vec![value(0), value(1)]),
            origin: origin.clone(),
        },
        Instruction {
            result: None,
            op: Op::StoreGlobal(GlobalId(0), value(2)),
            origin,
        },
    ];
    verify::verify(&module).unwrap();
    let mut runner = Interpreter::new(&module, 1000).unwrap();
    runner.run_entry().unwrap();
    assert_eq!(runner.globals[0].bits, 32768);
    module
}

#[test]
#[ignore = "requires LLVM AVR, AVR GCC/binutils, Node and AVR8js; see tools/setup-toolchain.ps1"]
fn llvm_machine_code_matches_interpreter_at_all_optimization_levels() {
    let arithmetic = finite(compile(
        "var a:u8=255; var b:i8=127; var c:u16=65535; var d:i16=-32768; var less:bool=false;
        loop { a=a+1; b=b+1; c=c*2; d=-d; less=b<0; }",
    ));
    let control = finite(compile(
        "var result:u16=0; var negative:i16=0; loop {
        var i:u8=0; while i<5 { i=i+1; }
        var a:u8=250; var b:u8=10; var wrapped:u16=a+b;
        var selected:u16=true ? a : b; var signed:i8=-2; negative=signed;
        result=wrapped+selected+i; }",
    ));
    let gpio = finite(compile(
        "def { Led=PortC.5 OUTPUT; Input=PortD.2 INPUT_PULLUP; Other=PortC.4 OUTPUT; }
        setup { Other=HIGH; Led=HIGH; } loop {
        Led=!Led;
        if Input==LOW and Led==HIGH { Led=LOW; }
        if Input==HIGH or Led==LOW { Other=LOW; }
        }",
    ));
    let calls = call_fixture();
    let modes = finite(compile(
        "def Led=PortB.1 OUTPUT; loop { Led=INPUT_PULLUP; Led=OUTPUT; Led=LOW; Led=INPUT; }",
    ));
    let mut unused_read = finite(compile("def Input=PortD.2 INPUT; loop {}"));
    unused_read.functions[2].blocks[0]
        .instructions
        .push(Instruction {
            result: Some((
                ValueId {
                    function: FunctionId(2),
                    index: 0,
                },
                Type::PinState,
            )),
            op: Op::ReadPin(PinId(0)),
            origin: Origin::Generated("unused physical read"),
        });
    verify::verify(&unused_read).unwrap();

    for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
        compare_machine(&arithmetic, &[], "arithmetic", level, true);
        compare_machine(&control, &[], "control", level, true);
        compare_machine(&calls, &[], "calls", level, true);
        compare_machine(&modes, &[], "modes", level, false);
        compare_machine(&unused_read, &[true], "unused-read", level, false);
        compare_machine(&gpio, &[false, true], "production-gpio", level, false);
        for (index, pattern) in [[false, false], [false, true], [true, false], [true, true]]
            .iter()
            .enumerate()
        {
            compare_machine(&gpio, pattern, &format!("gpio-{index}"), level, true);
        }
    }
}
