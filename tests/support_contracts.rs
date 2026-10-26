use helium::diagnostic::{DiagnosticCode, DiagnosticKind};
use helium::frontend::{
    expression::Expression,
    static_type::{FloatType, IntegerType, StaticType},
};
use helium::{analyze, check, compile_llvm, compile_typed_ir, ir::Options};

#[test]
fn analyzed_expressions_have_concrete_types_in_every_source_context() {
    let source = "var global:bool=1<2;
        fn take(x:bool):bool {return x;}
        fn f():bool {return take((1+2)<3); if 4<5 {}}
        setup {var min:i8=-128; var wide:u64=18446744073709551615;
            var tiny:i64=-9223372036854775808; var float:f32=2;}
        loop {if 1<2 {} while 3<4 {} var a:bool=true ? 5<6 : 7<8;}
        timer t=Timer1 {every:1s, interrupt {var local:bool=9<10;}}";
    fn assert_concrete(e: &Expression) {
        assert!(
            !matches!(
                e.static_type,
                StaticType::Unknown
                    | StaticType::Integer(IntegerType::Literal)
                    | StaticType::Float(FloatType::Literal)
            ),
            "{e:?}"
        );
        e.for_each_child(assert_concrete);
    }
    for statement in analyze(source).unwrap().statements() {
        statement.for_each_expression(&mut assert_concrete);
    }
}

#[test]
fn unconstrained_literal_comparison_has_a_located_user_error() {
    let source = "loop {if 1 < 2 {}}";
    let errors = check(source, "atmega328p").unwrap_err();
    assert!(matches!(
        *errors[0].kind,
        DiagnosticKind::UnsupportedType { machine: true, .. }
    ));
    let location = errors[0].location.unwrap();
    assert_eq!(location.start_offset, source.find('1').unwrap());
    assert_eq!(location.end_offset, location.start_offset + 1);
    assert!(errors.iter().all(|e| e.code() != DiagnosticCode::Internal));
}

#[test]
fn unsupported_forms_are_rejected_before_dead_code_lowering() {
    for statement in [
        "Led=true ? INPUT : OUTPUT;",
        "INPUT==OUTPUT;",
        "if 1<2 {}",
        "take(1<2);",
        "var test:bool=1<2;",
        "if (true ? Led : Other)==HIGH {}",
        "var mode:PinMode=INPUT;",
        "var quotient:u8=4/2;",
    ] {
        for prefix in ["", "return;"] {
            let source = format!(
                "def Led=PortB.0 OUTPUT; def Other=PortB.1 INPUT;
                fn take(x:bool) {{}} fn f() {{{prefix}{statement}}} loop {{}}"
            );
            for verify in [false, true] {
                for optimize in [false, true] {
                    let errors = compile_llvm(
                        &source,
                        "atmega328p",
                        Options {
                            verify,
                            optimize,
                            cpu_hz: None,
                        },
                    )
                    .unwrap_err();
                    assert!(
                        errors
                            .iter()
                            .all(|e| e.location.is_some() && e.code() != DiagnosticCode::Internal),
                        "{source}: {errors:?}"
                    );
                }
            }
        }
    }
    // PinMode is outside Helium IR itself, not merely outside the machine subset.
    for prefix in ["", "return;"] {
        let source = format!(
            "def Led=PortB.0 OUTPUT; fn f() {{{prefix}Led=true ? INPUT : OUTPUT;}} loop {{}}"
        );
        assert!(compile_typed_ir(&source, "atmega328p", Options::default()).is_err());
    }
    for source in [
        "fn f():bool {return 1<2;} loop {}",
        "fn f():bool {return true; return 1<2;} loop {}",
    ] {
        assert!(check(source, "atmega328p").is_err());
    }
}

#[test]
fn constant_modes_and_contextual_integer_widths_remain_supported() {
    check(
        "def A=PortB.0 OUTPUT; def B=PortB.1 OUTPUT;
        fn minimum():i8 {return -128;}
        loop {A=(B=INPUT_PULLUP); var small:u8=255; var other:u8=1;
            var wrapped:u16=small+other; var result:i16=minimum();}",
        "atmega328p",
    )
    .unwrap();
    for source in ["loop {2147483648;}", "fn f() {return; 2147483648;} loop {}"] {
        let errors = analyze(source).unwrap_err();
        assert_eq!(errors[0].code(), DiagnosticCode::LiteralRange);
    }
}
