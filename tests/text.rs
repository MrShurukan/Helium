use helium::{
    backend::microcontroller::ATMEGA328P,
    ir::{self, Op, Options, TextSpace, Type},
};

fn options() -> Options {
    Options {
        cpu_hz: Some(16_000_000),
        ..Options::default()
    }
}
fn compile(s: &str) -> ir::Module {
    helium::compile_typed_ir(s, "atmega328p", options()).unwrap_or_else(|e| panic!("{e:#?}"))
}

#[test]
fn text_bytes_lengths_and_safe_fallback_execute_in_interpreter() {
    let m = compile(
        r#"
        fn read(text: &str, i: u16): u8 { return text.byte_or(i, 99); }
        fn test(): u16 {
            var s: &str = "Я\0";
            if s.len() == 3 and read(s, 0) == 208 and read(s, 1) == 175
                and read(s, 2) == 0 and read(s, 3) == 99 and read(s, 65535) == 99 {
                return 42;
            }
            return 0;
        }
        loop {}
    "#,
    );
    let id = m.functions.iter().find(|f| f.name == "test").unwrap().id;
    let mut vm = ir::interpret::Interpreter::new(&m, 1000).unwrap();
    assert_eq!(vm.invoke(id, &[]).unwrap().unwrap().bits, 42);
}

#[test]
fn reference_restrictions_and_invalid_operations_are_diagnosed() {
    for s in [
        "var s=\"text\"; loop{}",
        "fn escape(s:&str):&str{return s;} loop{}",
        "loop{var s:&str;}",
        "loop{var s=\"x\";var b=s.byte_or(0);}",
        "loop{var s=\"x\";var b=s.byte_or(true,0);}",
        "loop{var s:u8=1;var n=s.len();}",
        "loop{var s=\"x\";var n=s.len(1);}",
        "loop{var s=\"x\";var eq=s==s;}",
        "loop{var s=\"bad\\q\";}",
    ] {
        assert!(
            helium::compile_llvm(s, "atmega328p", options()).is_err(),
            "{s}"
        );
    }
}

#[test]
fn utf8_byte_limit_and_truncated_reference_syntax_are_checked() {
    let source =
        "fn size(s: &str):u16{return s.len();} setup{var s=\"Я\\\"\\0\";var n=size(s);} loop{}";
    compile(source);
    for (end, _) in source.char_indices() {
        let _ = helium::analyze(&source[..end]);
    }
    let oversized = format!("setup{{var text=\"{}\";}}loop{{}}", "Я".repeat(32768));
    let errors = helium::compile_llvm(&oversized, "atmega328p", options()).unwrap_err();
    assert!(errors.iter().any(|e| e.message().contains("65535")));
}

#[test]
fn empty_text_never_reads_and_fallback_arguments_are_evaluated() {
    let m = compile(
        r#"
        var calls:u8=0;
        fn fallback():u8{calls=calls+1;return 90;}
        fn test():u8{
            var empty="";var full="A";
            var a=empty.byte_or(0,fallback());
            var b=full.byte_or(0,fallback());
            if a==90 and b==65 and calls==2 {return 1;}
            return 0;
        }loop{}
    "#,
    );
    let id = m.functions.iter().find(|f| f.name == "test").unwrap().id;
    let mut vm = ir::interpret::Interpreter::new(&m, 1000).unwrap();
    assert_eq!(vm.invoke(id, &[]).unwrap().unwrap().bits, 1);
}

#[test]
fn text_forwards_through_modules_and_preserves_uart_effects() {
    let mut p = helium::modules::Project::memory(
        "main.he",
        r#"
        import app::log;
        device debug=UART0{baud:38400,mode:TX,}
        timer t=Timer1{every:1ms,interrupt{app::log::send(debug,"tick");}}
        loop{}
    "#,
    );
    p.modules.insert(
        "app::log".into(),
        "import extension std::uart::UartText; pub fn send(device u:UartTx,s:&str){u.println(s);}"
            .into(),
    );
    let m = helium::compile_project_ir(&mut p, &ATMEGA328P, options()).unwrap();
    assert_eq!(m.warnings.len(), 1);
    let send = m
        .functions
        .iter()
        .find(|f| f.name.contains("app::log::send<debug>"))
        .unwrap();
    assert_eq!(send.parameters.len(), 1);
    assert_eq!(send.parameters[0].1, Type::Text);
}

#[test]
fn llvm_text_has_explicit_address_space_and_no_terminator() {
    let m = compile(include_str!("../examples/uart-text.he"));
    let llvm = helium::backend::llvm::emit(&m, true).unwrap();
    assert!(llvm.contains("addrspace(1) constant [5 x i8] c\"\\48\\65\\6C\\6C\\6F\""));
    assert!(llvm.contains("{ i16, i16, i8 }"));
    assert!(llvm.contains("load i8, ptr addrspace(1)"));
    assert!(llvm.contains("load i8, ptr %dp"));
}

#[test]
fn verifier_rejects_forged_text_and_bad_operands() {
    let m = compile("fn len(s:&str):u16{return s.len();} setup{var n=len(\"hi\");} loop{}");
    for mutation in 0..3 {
        let mut bad = m.clone();
        let i = bad
            .functions
            .iter_mut()
            .flat_map(|f| &mut f.blocks)
            .flat_map(|b| &mut b.instructions)
            .find(|i| matches!(i.op, Op::TextLiteral { .. }))
            .unwrap();
        match mutation {
            0 => i.op = Op::Const(ir::Constant::new(Type::Text, 0)),
            1 => {
                i.op = Op::TextLiteral {
                    text: "a".repeat(65536),
                    space: TextSpace::Flash,
                }
            }
            _ => i.result.as_mut().unwrap().1 = Type::U16,
        }
        assert!(ir::verify::verify(&bad).is_err());
    }
}

fn machine_program(scenario: usize) -> ir::Module {
    let setup = match scenario {
        0 => "print_n(debug,\"Hello\",2);print_n(debug,\"Я\\0\",1);print_n(debug,\"\",1);print_n(debug,\"skip\",0);",
        1 => "print_n(debug,\"RAM\",2);",
        2 => "var s:&str=\"Flash\";if input==LOW{s=\"RAM\";}print_n(debug,s,2);",
        3 => "var s:&str=\"Flash\";if input==HIGH{s=\"RAM\";}print_n(debug,s,2);",
        _ => "var empty:&str=\"\";var text:&str=\"Q\";debug.write_byte(empty.byte_or(0,33));debug.write_byte(text.byte_or(65535,34));",
    };
    let mut m = compile(&format!(
        r#"
        import extension std::uart::UartText;
        device debug=UART0{{baud:38400,mode:TX,}}
        def input=PortB.0 INPUT;
        fn print_n(device uart:UartTx,s:&str,n:u16){{
            var i:u16=0;while i<n{{uart.println(s);i=i+1;}}
        }}
        setup{{{setup}debug.flush();}}loop{{}}
    "#
    ));
    // Until SRAM owners exist in source, a typed IR fixture supplies immutable
    // SRAM storage. Selection still comes from a real volatile GPIO read.
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
    ir::verify::verify(&m).unwrap();
    m
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain, Node and AVR8js; no flashing"]
fn universal_text_print_n_executes_flash_sram_and_runtime_selection() {
    use helium::backend::{
        llvm,
        llvm_toolchain::{self, Optimization},
    };
    use std::{path::Path, process::Command};
    for scenario in 0..5 {
        for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
            let m = machine_program(scenario);
            let llvm = llvm::emit(&m, true).unwrap();
            let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("build/text-tests/{scenario}-{level:?}.hex"));
            let build =
                llvm_toolchain::build(&llvm, &ATMEGA328P, &destination, "hex", level).unwrap();
            let simulation =
                Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
                    .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/simulate-avr.cjs"))
                    .arg(&build.directory)
                    .arg(if scenario == 2 || scenario == 3 {
                        "00"
                    } else {
                        ""
                    })
                    .env("HELIUM_UART_CYCLES", "200000")
                    .output()
                    .unwrap();
            assert!(
                simulation.status.success(),
                "{}",
                String::from_utf8_lossy(&simulation.stderr)
            );
            let actual = String::from_utf8(simulation.stdout).unwrap();
            let bytes = match scenario {
                0 => "[72,101,108,108,111,10,72,101,108,108,111,10,208,175,0,10,10]",
                1 | 2 => "[82,65,77,10,82,65,77,10]",
                3 => "[70,108,97,115,104,10,70,108,97,115,104,10]",
                _ => "[33,34]",
            };
            assert!(
                actual.contains(&format!("\"uartBytes\":{bytes}")),
                "scenario {scenario} {level:?}: {actual}"
            );
            if matches!(level, Optimization::Speed) && scenario < 2 {
                let optimized =
                    std::fs::read_to_string(build.directory.join("optimized.ll")).unwrap();
                let setup = m.lifecycle.as_ref().unwrap().setup.0;
                let marker = format!("define void @helium_f{setup}()");
                let body = optimized
                    .split(&marker)
                    .nth(1)
                    .unwrap()
                    .split("\n}")
                    .next()
                    .unwrap();
                assert!(
                    !body.contains("extractvalue { i16, i16, i8 }"),
                    "known placement survived in optimized setup: {body}"
                );
            }
            let memory = build.memory.unwrap();
            if scenario == 0 {
                assert_eq!(memory.static_sram, 1, "Flash literals must not occupy SRAM");
            }
            println!(
                "text scenario={scenario}, opt={level:?}: Flash={}, static SRAM={}, artifacts={}",
                memory.flash_extent,
                memory.static_sram,
                build.directory.display()
            );
        }
    }
}
