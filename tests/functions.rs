use helium::diagnostic::{DiagnosticCode, DiagnosticKind};
use helium::{
    analyze, compile_typed_ir,
    ir::{self, interpret::Interpreter, *},
};

fn compile(source: &str, optimize: bool) -> Module {
    compile_typed_ir(
        source,
        "atmega328p",
        Options {
            verify: true,
            optimize,
        },
    )
    .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"))
}

fn run(source: &str, optimize: bool) -> Vec<u64> {
    let mut module = compile(source, optimize);
    module.functions[0].blocks[1].terminator = Terminator::Return(None);
    let mut runner = Interpreter::new(&module, 10000).unwrap();
    runner.run_entry().unwrap();
    runner.globals.iter().map(|value| value.bits).collect()
}

#[test]
fn forward_functions_mutable_parameter_copies_and_widened_returns() {
    let source = "fn twice(value:u8):u8 { value=value+value; return value; }
        var argument:u8=130; var wrapped:u16=0; var negative:i16=0;
        setup { wrapped=twice(argument); negative=widen(-2); }
        loop {} fn widen(value:i8):i16 { return value; }";
    for optimize in [false, true] {
        assert_eq!(run(source, optimize), [130, 4, 65534]);
    }
}

#[test]
fn calls_snapshot_arguments_and_operands_left_to_right() {
    let source = "var x:u8=1; var pair:u8=0; var sum:u8=0;
        fn change():u8 { x=x+1; return x; }
        fn combine(a:u8,b:u8):u8 { return a*10+b; }
        loop { pair=combine(x,change()); sum=change()+change(); }";
    for optimize in [false, true] {
        assert_eq!(run(source, optimize), [4, 12, 7]);
    }
}

#[test]
fn early_returns_close_cfg_branches_loops_and_nested_blocks() {
    let source = "var a:i16=0; var b:i16=0; var calls:u8=0;
        fn choose(x:i16):i16 {
            if x<0 { { return -1; } } else { if x==0 { return 0; } else { return x; } }
            calls=99;
        }
        fn find(x:i16):i16 { while x>0 { if x==2 { return x; } x=x-1; } return -1; }
        fn stop() { calls=calls+1; return; calls=99; }
        loop { a=choose(-5); b=find(5); stop(); choose(0); }";
    for optimize in [false, true] {
        assert_eq!(run(source, optimize), [65535, 2, 1]);
    }
}

#[test]
fn function_effects_obey_short_circuit_and_ternary() {
    let source = "var effects:u8=0; var result:bool=false;
        fn effect():bool { effects=effects+1; return true; }
        loop { result=false and effect(); result=true or effect();
            result=true ? effect() : effect(); }";
    for optimize in [false, true] {
        assert_eq!(run(source, optimize), [1, 1]);
    }
}

#[test]
fn thermostat_preserves_hysteresis_state() {
    for optimize in [false, true] {
        assert_eq!(
            run(
                include_str!("../examples/functions-thermostat.he"),
                optimize
            ),
            [0, 1, 1, 0]
        );
    }
}

#[test]
fn function_types_arity_and_returns_have_structured_diagnostics() {
    let cases = [
        ("fn f(a:u8) {} loop { f(); }", DiagnosticCode::Call),
        ("fn f() {} loop { f(1); }", DiagnosticCode::Call),
        ("fn f() {} loop { var x=f(); }", DiagnosticCode::Call),
        ("fn f() {} loop { var x=f; }", DiagnosticCode::Call),
        ("var x:u8=0; loop { x(); }", DiagnosticCode::Call),
        ("loop { missing(); }", DiagnosticCode::Undefined),
        (
            "fn f(a:u8) {} loop { f(256); }",
            DiagnosticCode::LiteralRange,
        ),
        (
            "fn f(a:u8) {} loop { var x:u16=1; f(x); }",
            DiagnosticCode::Type,
        ),
        (
            "fn f():u8 { return 256; } loop {}",
            DiagnosticCode::LiteralRange,
        ),
        ("fn f():u8 { return true; } loop {}", DiagnosticCode::Type),
        ("fn f():u8 { return; } loop {}", DiagnosticCode::Return),
        ("fn f() { return 1; } loop {}", DiagnosticCode::Return),
        ("loop { return; }", DiagnosticCode::Return),
        ("setup { return; } loop {}", DiagnosticCode::Return),
        (
            "fn f(a:bool):u8 { if a { return 1; } } loop {}",
            DiagnosticCode::Return,
        ),
        (
            "fn f():u8 { while true { return 1; } } loop {}",
            DiagnosticCode::Return,
        ),
        (
            "fn f():u8 { return 1; } var x:u8=f(); loop {}",
            DiagnosticCode::Call,
        ),
        ("fn f(a:string) {} loop {}", DiagnosticCode::Unsupported),
    ];
    for (source, code) in cases {
        let errors = analyze(source).expect_err(source);
        assert!(
            errors.iter().any(|e| e.kind.code() == code),
            "{source}: {errors:#?}"
        );
        assert!(
            errors.iter().all(|e| e.location.is_some()),
            "{source}: {errors:#?}"
        );
    }
}

#[test]
fn all_visible_names_remain_unshadowable() {
    for source in [
        "fn f() {} fn f() {} loop {}",
        "fn f(a:u8,a:u8) {} loop {}",
        "fn f(a:u8) { var a:u8=1; } loop {}",
        "fn f(f:u8) {} loop {}",
        "fn f(x:u8) {} var x:u8=1; loop {}",
        "var f:u8=1; fn f() {} loop {}",
        "def Led=PortB.5 OUTPUT; fn f(Led:bool) {} loop {}",
        "fn f() {} loop { var f:u8=1; }",
    ] {
        assert!(
            analyze(source)
                .unwrap_err()
                .iter()
                .any(|e| e.kind.code() == DiagnosticCode::Duplicate),
            "{source}"
        );
    }
    assert!(analyze("fn f(a:u8) {} fn g(a:u8) {} loop {}").is_ok());
    assert!(analyze("fn f() { var hidden:u8=1; } loop { hidden=2; }").is_err());
}

#[test]
fn recursion_is_rejected_even_in_unused_or_unreachable_code() {
    for source in [
        "fn f() { f(); } loop {}",
        "fn a() { b(); } fn b() { a(); } loop {}",
        "fn a() { return; b(); } fn b() { a(); } loop {}",
    ] {
        let errors = analyze(source).unwrap_err();
        let error = errors
            .iter()
            .find(|e| e.kind.code() == DiagnosticCode::Recursion)
            .unwrap();
        let DiagnosticKind::RecursiveCalls { chain } = &*error.kind else {
            panic!()
        };
        assert_eq!(chain.first(), chain.last());
        assert!(chain.len() >= 2);
    }
}

#[test]
fn default_compilation_accepts_functions() {
    helium::check("fn f() {} loop { f(); }", "atmega328p").unwrap();
}

#[test]
fn malformed_function_syntax_and_prefixes_do_not_panic() {
    let source = "fn f(a:u8,b:i16):i16 { if a>0 { return b; } return -1; } loop { f(1,-2); }";
    for end in 0..source.len() {
        let result = std::panic::catch_unwind(|| analyze(&source[..end]));
        assert!(result.is_ok(), "prefix {end}");
    }
    for source in [
        "fn () {} loop {}",
        "fn f(a) {} loop {}",
        "fn f(a:u8,) {} loop {}",
        "fn f(): {} loop {}",
        "fn f() { return 1 } loop {}",
        "fn f() {} loop { f(,); }",
        "loop { fn inner() {} }",
        "fun f() {} loop {}",
        "fn f() {} loop { f()(); }",
    ] {
        assert!(analyze(source).is_err(), "{source}");
    }
}

#[test]
fn functions_preserve_deterministic_ids_and_parameter_origins() {
    use helium::frontend::{statement::StatementType, static_type::StaticType};
    let ast = analyze("fn ready() {} loop { ready(); }").unwrap();
    let StatementType::LoopDeclaration(body) = &ast[1].statement_type else {
        panic!()
    };
    let StatementType::Expression(call) = &body[0].statement_type else {
        panic!()
    };
    assert_eq!(call.static_type, StaticType::NoValue);

    let source = "fn z(x:u8):u8 { return x; } loop { z(1); } fn a() {}";
    let module = compile(source, false);
    assert_eq!(module.dump(), compile(source, false).dump());
    assert_eq!(module.functions[3].name, "z");
    assert_eq!(module.functions[4].name, "a");
    assert_eq!(module.functions[3].parameters.len(), 1);
    assert!(matches!(module.functions[3].origin, ir::Origin::Source(_)));
}
