use helium::ir::{self, Constant, FunctionId, Op, Options, TextSpace, Type};

fn compile(source: &str) -> ir::Module {
    helium::compile_typed_ir(
        source,
        "atmega328p",
        Options {
            cpu_hz: Some(16_000_000),
            ..Options::default()
        },
    )
    .unwrap()
}
fn ops(f: &ir::Function) -> impl Iterator<Item = &Op> {
    f.blocks.iter().flat_map(|b| &b.instructions).map(|i| &i.op)
}

#[test]
fn print_chain_dispatches_outside_loops_and_known_calls_bypass_wrappers() {
    let raw = compile(include_str!("../examples/uart-text.he"));
    let m = ir::text::specialize(&raw).unwrap();
    let wrapper = m
        .functions
        .iter()
        .find(|f| f.name == "print_n<debug>")
        .unwrap();
    assert_eq!(wrapper.locals.len(), 0);
    assert_eq!(
        ops(wrapper)
            .filter(|op| matches!(op, Op::TextIsFlash(_)))
            .count(),
        1
    );
    assert_eq!(wrapper.blocks.len(), 3);
    assert!(wrapper.blocks.iter().all(|b| b
        .terminator
        .successors()
        .iter()
        .all(|s| s.index > b.id.index)));
    let concrete: Vec<_> = m
        .functions
        .iter()
        .filter(|f| f.name.ends_with("[Flash]") || f.name.ends_with("[Sram]"))
        .collect();
    assert!(concrete.iter().any(|f| f.name.contains("print_n")));
    assert!(concrete.iter().any(|f| f.name.contains("UartText::print<")));
    for f in concrete {
        assert!(
            !ops(f).any(|op| matches!(
                op,
                Op::TextIsFlash(_) | Op::TextNarrow(..) | Op::TextErase(_)
            )),
            "{}",
            f.name
        );
        assert!(f.parameters.iter().all(|(_, t)| *t != Type::Text));
        for op in ops(f) {
            if let Op::Call(id, _) = op {
                assert!(m.functions[id.0]
                    .parameters
                    .iter()
                    .all(|(_, t)| *t != Type::Text));
            }
        }
    }
    let setup = &m.functions[m.lifecycle.as_ref().unwrap().setup.0];
    assert!(!ops(setup).any(|op| matches!(op, Op::Call(id, _) if *id == wrapper.id)));
    let llvm = helium::backend::llvm::emit(&m, true).unwrap();
    for suffix in ["flash", "sram"] {
        let helper = llvm
            .split(&format!("define internal i8 @helium_text_byte_{suffix}("))
            .nth(1)
            .unwrap()
            .split("\n}")
            .next()
            .unwrap();
        assert!(!helper.contains("%space"));
        assert_eq!(helper.matches("br i1").count(), 1); // Bounds only.
    }
    assert_eq!(m.dump(), ir::text::specialize(&m).unwrap().dump());
}

fn fixture() -> ir::Module {
    let mut m = compile(
        r#"
        fn pair(a:&str,b:&str):u16 {return a.len()*100+b.len();}
        fn run(first:bool, second:bool):u16 {
            var a="F";var b="F";
            if first {a="RAM";}
            if second {b="RAM";}
            return pair(a,b);
        }
        loop{}
    "#,
    );
    for i in m
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
    {
        if let Op::TextLiteral { text, space } = &mut i.op {
            if text == "RAM" {
                *space = TextSpace::Sram;
            }
        }
    }
    m
}

#[test]
fn two_independent_parameters_dispatch_all_combinations_and_preserve_results() {
    let raw = fixture();
    let m = ir::text::specialize(&raw).unwrap();
    for name in [
        "pair[Flash,Flash]",
        "pair[Flash,Sram]",
        "pair[Sram,Flash]",
        "pair[Sram,Sram]",
    ] {
        assert!(m.functions.iter().any(|f| f.name == name));
    }
    let run = raw.functions.iter().find(|f| f.name == "run").unwrap().id;
    for first in [false, true] {
        for second in [false, true] {
            let args = [
                Constant::new(Type::Bool, u64::from(first)),
                Constant::new(Type::Bool, u64::from(second)),
            ];
            let before = ir::interpret::Interpreter::new(&raw, 1000)
                .unwrap()
                .invoke(run, &args)
                .unwrap();
            let after = ir::interpret::Interpreter::new(&m, 1000)
                .unwrap()
                .invoke(run, &args)
                .unwrap();
            assert_eq!(before, after);
            assert_eq!(
                after.unwrap().bits,
                (if first { 300 } else { 100 }) + if second { 3 } else { 1 }
            );
        }
    }
}

#[test]
fn reassignment_does_not_keep_stale_parameter_placement() {
    let mut raw = compile(
        r#"
        fn read(s:&str):u8{return s.byte_or(0,0);}
        fn change(s:&str,flag:bool):u8 {if flag{s="RAM";}return read(s);}
        fn run(flag:bool):u8{return change_text("Flash",flag);}
        fn change_text(s:&str,flag:bool):u8{return change(s,flag);}
        loop{}
    "#,
    );
    for i in raw
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
    {
        if let Op::TextLiteral { text, space } = &mut i.op {
            if text == "RAM" {
                *space = TextSpace::Sram;
            }
        }
    }
    let m = ir::text::specialize(&raw).unwrap();
    let run = m.functions.iter().find(|f| f.name == "run").unwrap().id;
    for (arg, expected) in [(false, 70), (true, 82)] {
        assert_eq!(
            ir::interpret::Interpreter::new(&m, 1000)
                .unwrap()
                .invoke(run, &[Constant::new(Type::Bool, u64::from(arg))])
                .unwrap()
                .unwrap()
                .bits,
            expected
        );
    }
    let changed = m
        .functions
        .iter()
        .find(|f| f.name == "change[Flash]")
        .unwrap();
    assert!(ops(changed).any(|op| matches!(op, Op::TextErase(_))));
    assert!(ops(changed).any(|op| matches!(op,Op::Call(id,_) if m.functions[id.0].name=="read")));
}

#[test]
fn verifier_rejects_unguarded_narrowing_wrong_space_and_concrete_call_mismatch() {
    let m = ir::text::specialize(&fixture()).unwrap();
    for mutation in 0..3 {
        let mut bad = m.clone();
        let wrapper = bad.functions.iter_mut().find(|f| f.name == "pair").unwrap();
        if mutation == 0 {
            let leaf = wrapper
                .blocks
                .iter()
                .find(|b| {
                    b.instructions
                        .iter()
                        .any(|i| matches!(i.op, Op::TextNarrow(..)))
                })
                .unwrap()
                .id;
            wrapper.blocks[0].terminator = ir::Terminator::Jump(leaf);
        } else {
            let i = wrapper
                .blocks
                .iter_mut()
                .flat_map(|b| &mut b.instructions)
                .find(|i| matches!(i.op, Op::TextNarrow(..)))
                .unwrap();
            if mutation == 1 {
                if let Op::TextNarrow(_, space) = &mut i.op {
                    *space = TextSpace::Sram;
                }
            } else {
                i.result.as_mut().unwrap().1 = Type::Text;
            }
        }
        assert!(ir::verify::verify(&bad).is_err());
    }
}

#[test]
fn protected_call_contract_survives_dispatch_and_specialization() {
    let raw=compile("noInterrupt fn byte(s:&str):u8{return s.byte_or(0,0);} loop{noInterrupt{var b=byte(\"x\");}}");
    let m = ir::text::specialize(&raw).unwrap();
    assert!(m
        .functions
        .iter()
        .filter(|f| f.name.starts_with("byte"))
        .all(|f| f.requires_no_interrupt));
}

#[test]
fn specialization_limit_is_explicit_and_does_not_mutate_input() {
    let source = "fn many(a:&str,b:&str,c:&str,d:&str,e:&str,f:&str,g:&str){}loop{}";
    let errors = helium::compile_llvm(source, "atmega328p", Options::default()).unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.message().contains("6 text parameters")));
    let mut raw = compile("fn many(a:&str,b:&str,c:&str,d:&str,e:&str,f:&str){}loop{}");
    let f = raw.functions.iter_mut().find(|f| f.name == "many").unwrap();
    let next = f
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(|i| i.result.map(|(v, _)| v.index))
        .chain(f.parameters.iter().map(|(v, _)| v.index))
        .max()
        .unwrap()
        + 1;
    f.parameters.push((
        ir::ValueId {
            function: f.id,
            index: next,
        },
        Type::Text,
    ));
    let before = raw.dump();
    assert!(ir::text::specialize(&raw)
        .unwrap_err()
        .contains("6 text parameters"));
    assert_eq!(before, raw.dump());
    assert_eq!(raw.entry, FunctionId(0));
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain, Node and AVR8js; no flashing"]
fn two_runtime_text_arguments_execute_on_avr_without_llvm_specialization() {
    use helium::backend::{
        llvm,
        llvm_toolchain::{self, Optimization},
        microcontroller::ATMEGA328P,
    };
    use std::{path::Path, process::Command};
    let mut raw = compile(
        r#"
        import extension std::uart::UartText;
        device debug=UART0{baud:38400,mode:TX,}
        def input=PortB.0 INPUT;
        fn pair(device u:UartTx,a:&str,b:&str){
            var n:u16=0;while n<2{u.print(a);u.println(b);n=n+1;}
        }
        setup{
            var a="F";var b="F";
            if input==HIGH{a="RAM";}
            if input==HIGH{b="RAM";}
            pair(debug,a,b);debug.flush();
        }loop{}
    "#,
    );
    for i in raw
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
    {
        if let Op::TextLiteral { text, space } = &mut i.op {
            if text == "RAM" {
                *space = TextSpace::Sram;
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
        let destination = root.join(format!("build/text-specialization/pair-{level:?}.hex"));
        // Normal internal linkage: unused variants need not survive machine linking.
        let build = llvm_toolchain::build(
            &llvm::emit(&raw, false).unwrap(),
            &ATMEGA328P,
            &destination,
            "hex",
            level,
        )
        .unwrap();
        std::fs::write(
            destination.with_extension("ir"),
            ir::text::specialize(&raw).unwrap().dump(),
        )
        .unwrap();
        for a in [false, true] {
            for b in [false, true] {
                let input = format!("{:02x},{:02x}", u8::from(a), u8::from(b));
                let simulation =
                    Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
                        .arg(root.join("tools/simulate-avr.cjs"))
                        .arg(&build.directory)
                        .arg(input)
                        .env("HELIUM_UART_CYCLES", "200000")
                        .output()
                        .unwrap();
                assert!(
                    simulation.status.success(),
                    "{}",
                    String::from_utf8_lossy(&simulation.stderr)
                );
                let text = format!(
                    "{}{}\n",
                    if a { "RAM" } else { "F" },
                    if b { "RAM" } else { "F" }
                )
                .repeat(2);
                let bytes = text
                    .as_bytes()
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                assert!(String::from_utf8_lossy(&simulation.stdout)
                    .contains(&format!("\"uartBytes\":[{bytes}]")));
            }
        }
        println!("pair runtime {level:?}: {}", build.memory.unwrap().render());
    }
}
