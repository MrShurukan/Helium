use helium::diagnostic::DiagnosticCode;
use helium::frontend::{
    expression::ExpressionType as E, statement::StatementType as S, static_type::StaticType as T,
};
use helium::ir::{
    self,
    interpret::{Event, Interpreter},
    *,
};
use helium::{analyze, check, compile_typed_ir};
use std::collections::HashMap;

fn errors(source: &str) -> Vec<DiagnosticCode> {
    analyze(source)
        .unwrap_err()
        .iter()
        .map(|d| d.code())
        .collect()
}
fn body(program: &str) -> String {
    format!("loop {{ {program} }}")
}
const GPIO: &str = "def Key = PortD.2 INPUT; def Led = PortC.5 OUTPUT; setup { Led = LOW; } loop { if Key == LOW { Led = HIGH; } else { Led = LOW; } }";

#[test]
fn duplicate_and_shadowed_names_are_rejected() {
    for source in [
        "var x=1; loop { var x=2; }",
        "loop { var x=1; var x=2; }",
        "def Led=PortC.5 OUTPUT; loop { var Led=1; }",
        "loop { var x=1; {var x=2;} }",
    ] {
        assert!(
            errors(source).contains(&DiagnosticCode::Duplicate),
            "{source}"
        );
    }
    analyze("loop { {var x=1;} {var x=2;} }").unwrap();
}
#[test]
fn scope_and_self_initialization() {
    for source in [
        "loop { var x:u8=x; }",
        "setup { var x=1; } loop { x=2; }",
        "loop { {var x=1;} x=2; }",
        "loop { for i=0 to 3 {} i=2; }",
    ] {
        assert!(
            errors(source).contains(&DiagnosticCode::Undefined),
            "{source}"
        );
    }
}
#[test]
fn literal_ranges_include_signed_minimum_and_unsigned_maximum() {
    for declaration in [
        "var x:i8=-128;",
        "var x:u8=255;",
        "var x:i64=-9223372036854775808;",
        "var x:u64=18446744073709551615;",
        "var x:i8=(-128);",
    ] {
        analyze(&body(declaration)).unwrap();
    }
    for declaration in [
        "var x:i8=128;",
        "var x:i8=-129;",
        "var x:u8=-1;",
        "var x:u8=256;",
        "var x=2147483648;",
        "var x:u8=300+1;",
    ] {
        assert!(
            errors(&body(declaration)).contains(&DiagnosticCode::LiteralRange),
            "{declaration}"
        );
    }
}
#[test]
fn assignments_check_range_and_only_widen_losslessly() {
    assert!(errors(&body("var x:u8=0; x=256;")).contains(&DiagnosticCode::LiteralRange));
    assert!(errors(&body("var x:u16=0; var y:u8=x;")).contains(&DiagnosticCode::Type));
    analyze(&body("var x:u8=0; var y:u16=x;")).unwrap();
    assert!(errors(&body("var x:i64=0; var y:f32=x;")).contains(&DiagnosticCode::Type));
}
#[test]
fn boolean_operators_and_pin_inversion_have_explicit_types() {
    for expression in ["!1;", "true and 1;", "HIGH < LOW;", "true == 1;"] {
        assert!(analyze(&body(expression)).is_err(), "{expression}");
    }
    assert!(errors(&body("!missing;")).contains(&DiagnosticCode::Undefined));
    let ast =
        analyze("def Led=PortC.5 OUTPUT; loop { Led=!Led; var state=!HIGH; var flag=!false; }")
            .unwrap();
    let S::LoopDeclaration(statements) = &ast.statements()[1].statement_type else {
        panic!()
    };
    let S::Expression(expr) = &statements[0].statement_type else {
        panic!()
    };
    assert_eq!(expr.static_type, T::PinState);
    let S::VariableDeclaration {
        static_type: Some(ty),
        ..
    } = &statements[2].statement_type
    else {
        panic!()
    };
    assert_eq!(*ty, T::Boolean);
    check("def Led=PortC.5 OUTPUT; loop { Led=!Led; }", "atmega328p").unwrap();
}
#[test]
fn ternary_and_default_initializer_are_typed() {
    let ast = analyze(&body("var x:u8; var b=true?false:true;")).unwrap();
    let S::LoopDeclaration(statements) = &ast.statements()[0].statement_type else {
        panic!()
    };
    for statement in statements {
        let S::VariableDeclaration {
            static_type: Some(ty),
            initializer: Some(expr),
            ..
        } = &statement.statement_type
        else {
            panic!()
        };
        assert_eq!(*ty, expr.static_type);
        assert!(expr.source_location.line_start > 0);
    }
}
#[test]
fn parser_precedence_and_left_associativity() {
    let ast = analyze(&body("var x=10-3-2*2;")).unwrap();
    let S::LoopDeclaration(body) = &ast.statements()[0].statement_type else {
        panic!()
    };
    let S::VariableDeclaration {
        initializer: Some(expr),
        ..
    } = &body[0].statement_type
    else {
        panic!()
    };
    let E::Binary { left, right, .. } = &expr.expression_type else {
        panic!()
    };
    assert!(matches!(left.expression_type, E::Binary { .. }));
    assert!(matches!(right.expression_type, E::Binary { .. }));
}
#[test]
fn nested_comments_unicode_and_crlf() {
    analyze("/* outer /* inner */ end */\r\nloop {var температура:u8=20;}").unwrap();
    let diagnostics = analyze("loop { var температура:u8=300; }").unwrap_err();
    assert!(diagnostics[0]
        .render("loop { var температура:u8=300; }")
        .contains("^^^"));
}
#[test]
fn malformed_input_returns_errors_without_panicking() {
    for source in [
        "",
        "loop {",
        "loop { var x=; }",
        "def {} loop {}",
        "loop { if true {",
        "/*",
        "loop { var x=(1+2; }",
        "loop { var x=\"unterminated",
        "loop {}\n@",
        "loop {}\n",
        "loop { var x = \"abc\n",
        "loop { for =0 to 3 {} }",
    ] {
        let result = std::panic::catch_unwind(|| analyze(source));
        assert!(result.is_ok(), "panicked for {source:?}");
        if source != "loop {}\n" {
            assert!(result.unwrap().is_err(), "accepted {source:?}");
        }
    }
}
#[test]
fn target_and_support_errors_are_explicit() {
    assert_eq!(
        check(GPIO, "unknown").unwrap_err()[0].code(),
        DiagnosticCode::Target
    );
    for pin in ["PortA.0", "PortB.8", "PortC.7"] {
        assert_eq!(
            check(&format!("def Led={pin} OUTPUT; loop {{}}"), "atmega328p").unwrap_err()[0].code(),
            DiagnosticCode::Pin
        );
    }
    assert_eq!(
        check(
            "def A=PortB.0 OUTPUT; def B=PortB.0 INPUT; loop {}",
            "atmega328p"
        )
        .unwrap_err()[0]
            .code(),
        DiagnosticCode::PinDuplicate
    );
    for source in [
        "loop {var x:f32=1.5;}",
        "loop {for i=0 to 3 {}}",
        "loop {var x:u8=4/2;}",
    ] {
        let errors = check(source, "atmega328p").unwrap_err();
        assert!(errors.iter().all(|d| d.location.is_some()), "{source}");
    }
}
fn compile(source: &str) -> Module {
    compile_typed_ir(
        source,
        "atmega328p",
        ir::Options {
            cpu_hz: None,
            verify: true,
            optimize: false,
        },
    )
    .unwrap()
}

#[test]
fn gpio_names_multiple_defs_and_deterministic_ir() {
    let module = compile(GPIO);
    assert_eq!(module.dump(), compile(GPIO).dump());
    assert_eq!(module.pins[0].name, "Key");
    assert_eq!(module.pins[1].name, "Led");
    check(GPIO, "atmega328p").unwrap();
}

#[test]
fn repeated_pin_reads_are_not_cached_in_or_across_branches() {
    let source="def Key=PortD.2 INPUT; loop { if Key==LOW { if Key==LOW {} } else { if Key==LOW {} } if Key==LOW {} }";
    let module = compile(source);
    let mut runner = Interpreter::new(&module, 1000).unwrap();
    runner.inputs.insert(PinId(0), [true, false, true].into());
    runner.invoke(FunctionId(2), &[]).unwrap();
    assert_eq!(runner.events.len(), 3);
}

fn evaluate(source: &str) -> HashMap<String, u64> {
    let mut module = compile(source);
    module.lifecycle = None;
    module.functions[0].blocks[1].terminator = Terminator::Return(None);
    let mut runner = Interpreter::new(&module, 10000).unwrap();
    runner.run_entry().unwrap();
    module
        .globals
        .iter()
        .zip(&runner.globals)
        .map(|(g, value)| (g.name.clone(), value.bits))
        .collect()
}
#[test]
fn assignments_reads_and_scopes_preserve_values() {
    let memory = evaluate("var out=0; loop {var x=1; var y=x; x=2; out=y+x;}");
    assert_eq!(memory["out"], 3);
    let memory = evaluate("var out=0; loop {var x=1; out=x+(x=2);}");
    assert_eq!(memory["out"], 3);
    let memory = evaluate("var out=0; setup {var x=1; out=x;} loop {var x=2; out=x;}");
    assert_eq!(memory["out"], 2);
    let memory = evaluate("var x=1; var out=0; loop {out=x+1; x=3; out=x+1;}");
    assert_eq!(memory["out"], 4);
}
#[test]
fn chained_pin_assignments_keep_both_effects() {
    let module = compile("def A=PortB.0 OUTPUT; def B=PortB.1 OUTPUT; loop {A=B=HIGH;}");
    let mut runner = Interpreter::new(&module, 1000).unwrap();
    runner.invoke(FunctionId(2), &[]).unwrap();
    assert_eq!(
        runner.events,
        [Event::Write(PinId(1), true), Event::Write(PinId(0), true)]
    );
}
#[test]
fn integer_to_float_literals_must_be_exact() {
    analyze(&body("var x:f32=16777216; var y:f64=9007199254740992;")).unwrap();
    for declaration in ["var x:f32=16777217;", "var y:f64=9007199254740993;"] {
        assert!(errors(&body(declaration)).contains(&DiagnosticCode::LiteralRange));
    }
}
#[test]
fn setup_must_precede_loop() {
    assert!(analyze("loop {} setup {}").is_err());
    assert!(analyze("setup {} setup {} loop {}").is_err());
}
#[test]
fn latch_is_supported_by_the_only_backend() {
    check(include_str!("../examples/functions-latch.he"), "atmega328p").unwrap();
}

#[test]
fn malformed_token_combinations_do_not_panic() {
    // Deterministic small corpus catches error-recovery interactions, not just handpicked valid programs.
    let tokens = [
        "var", "if", "while", "for", "def", "fun", "return", "{", "}", "(", ")", "=", ";", "!",
        "1", "\"x\"", "unknown",
    ];
    for a in tokens {
        for b in tokens {
            let source = format!("loop {{ {a} {b} }}");
            assert!(
                std::panic::catch_unwind(|| analyze(&source)).is_ok(),
                "{source}"
            );
        }
    }
}
#[test]
fn grouped_and_reversed_pin_comparisons_use_the_correct_pin() {
    for condition in ["(Key)==LOW", "LOW==(Key)"] {
        let source = format!("def Key=PortD.2 INPUT; loop {{ if {condition} {{}} }}");
        let module = compile(&source);
        let mut runner = Interpreter::new(&module, 1000).unwrap();
        runner.invoke(FunctionId(2), &[]).unwrap();
        assert_eq!(runner.events, [Event::Read(PinId(0), false)]);
    }
}
#[test]
fn contextual_negation_does_not_bypass_range_checks() {
    assert!(analyze(&body("var x:u8=-(1+2);")).is_err());
    assert!(analyze(&body("var x:i8=-(300+1);")).is_err());
}
#[test]
fn reserved_keywords_do_not_trigger_error_recovery_loops() {
    for source in [
        "loop { return; }",
        "loop { fun f {} }",
        "loop { def A=PortB.0 OUTPUT; }",
    ] {
        let errors = analyze(source).unwrap_err();
        assert!(
            errors.len() < 10,
            "{} diagnostics for {source}",
            errors.len()
        );
        assert!(!errors
            .iter()
            .any(|error| error.message().to_lowercase().contains("infinite loop")));
    }
}
#[test]
fn unfinished_program_prefixes_return_diagnostics_without_panics() {
    for source in [
        GPIO,
        "loop { for counter = 0 to 10 { var x:u8=1; } }",
        "loop {var x=\"text\"; if true {x=\"other\";} }",
        "def { Led=PortC.5 OUTPUT; } setup {} loop {}",
    ] {
        for (end, _) in source.char_indices() {
            let prefix = &source[..end];
            assert!(
                std::panic::catch_unwind(|| analyze(prefix)).is_ok(),
                "panicked for {prefix:?}"
            );
        }
    }
}
