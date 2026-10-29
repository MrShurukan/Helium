use helium::diagnostic::{
    source::{FileId, SourceFile},
    Diagnostic, DiagnosticCode, DiagnosticKind, LabelKind,
};
use helium::{analyze, check};

fn excerpt(source: &str, location: helium::frontend::utils::SourceLocation) -> &str {
    let file = SourceFile {
        id: FileId(7),
        name: "test.he",
        text: source,
    };
    let span = file.span(location).expect("valid source range");
    assert_eq!(span.file, FileId(7));
    &source[span.bytes]
}

fn error(source: &str, code: DiagnosticCode) -> Diagnostic {
    analyze(source)
        .unwrap_err()
        .into_iter()
        .find(|e| e.code() == code)
        .unwrap()
}

#[test]
fn missing_returns_point_to_uncovered_paths_and_result_annotation() {
    let source = "fn choose(flag:bool):u8 {\n if flag { return 1; }\n}\nloop {}";
    let e = error(source, DiagnosticCode::Return);
    assert_eq!(excerpt(source, e.location.unwrap()), "flag");
    assert!(e
        .labels
        .iter()
        .any(|l| l.kind == LabelKind::FalseBranch && excerpt(source, l.location) == "flag"));
    assert!(e.labels.iter().any(
        |l| matches!(l.kind, LabelKind::ExpectedType(_)) && excerpt(source, l.location) == "u8"
    ));
    assert!(!e.render(source).contains("^^^^^^"));

    let source = "fn choose(flag:bool):u8 { if flag { return 1; } else { var x:u8=0; } } loop {}";
    let e = error(source, DiagnosticCode::Return);
    assert_eq!(excerpt(source, e.location.unwrap()), "}");
    assert_eq!(
        e.labels
            .iter()
            .filter(|l| l.kind == LabelKind::Fallthrough)
            .count(),
        1
    );
}

#[test]
fn following_return_discharges_branches_but_loop_return_is_not_sufficient() {
    for source in [
        "fn f(b:bool):u8 { if b {} return 1; } loop {}",
        "fn f(b:bool):u8 { if b { return 1; } else { return 2; } } loop {}",
        "fn f(b:bool):u8 { while b { return 1; } return 2; } loop {}",
        "fn f(b:bool):u8 { { if b { return 1; } } return 2; } loop {}",
    ] {
        analyze(source).unwrap();
    }

    let source = "fn f(b:bool):u8 { while b { return 1; } } loop {}";
    let e = error(source, DiagnosticCode::Return);
    assert!(e
        .labels
        .iter()
        .any(|l| l.kind == LabelKind::LoopMayNotRun && excerpt(source, l.location) == "b"));

    let source = "fn f(b:bool):u8 { if b {} else {} } loop {}";
    let e = error(source, DiagnosticCode::Return);
    assert_eq!(
        e.labels
            .iter()
            .filter(|l| l.kind == LabelKind::Fallthrough)
            .count(),
        2
    );
}

#[test]
fn duplicate_names_reference_both_exact_identifiers() {
    for source in [
        "var value:u8=0; loop { var value:u8=1; }",
        "fn value() {} fn value() {} loop {}",
        "fn value() {} loop { var value:u8=1; }",
        "var value:u8=0; fn f(value:u8) {} loop {}",
    ] {
        let e = error(source, DiagnosticCode::Duplicate);
        assert_eq!(excerpt(source, e.location.unwrap()), "value");
        assert_eq!(e.labels.len(), 1);
        assert_eq!(excerpt(source, e.labels[0].location), "value");
        assert_ne!(e.location.unwrap(), e.labels[0].location);
    }
}

#[test]
fn repeated_lifecycle_blocks_reference_the_first_keyword() {
    for (source, keyword) in [
        ("setup {} setup {} loop {}", "setup"),
        ("loop {} loop {}", "loop"),
    ] {
        let e = error(source, DiagnosticCode::Parse);
        assert_eq!(excerpt(source, e.location.unwrap()), keyword);
        assert_eq!(excerpt(source, e.labels[0].location), keyword);
        assert_ne!(e.location.unwrap(), e.labels[0].location);
    }
}

#[test]
fn argument_and_return_errors_link_to_the_declared_type() {
    for source in [
        "fn f(value:u8) {} loop { f(300); }",
        "fn f():u8 { return 300; } loop {}",
    ] {
        let e = error(source, DiagnosticCode::LiteralRange);
        assert_eq!(excerpt(source, e.location.unwrap()), "300");
        assert!(e
            .labels
            .iter()
            .any(|l| matches!(l.kind, LabelKind::ExpectedType(_))
                && excerpt(source, l.location) == "u8"));
    }
    let source = "fn f(value:u8) {} loop { f(); }";
    let e = error(source, DiagnosticCode::Call);
    assert_eq!(excerpt(source, e.labels[0].location), "f");
    let source = "fn f():f32 { return 1.5; } loop {}";
    let e = error(source, DiagnosticCode::Unsupported);
    assert_eq!(excerpt(source, e.location.unwrap()), "f32");
    let source = "fn f(value:string) {} loop {}";
    let e = error(source, DiagnosticCode::Unsupported);
    assert_eq!(excerpt(source, e.location.unwrap()), "string");
}

#[test]
fn unsupported_machine_type_is_located_before_tool_execution() {
    let source = "var counter:f32=1.5; loop {}";
    let errors = check(source, "atmega328p").unwrap_err();
    assert_eq!(errors.len(), 1);
    assert!(matches!(
        *errors[0].kind,
        DiagnosticKind::UnsupportedType { machine: false, .. }
    ));
    assert_eq!(excerpt(source, errors[0].location.unwrap()), "f32");
}

#[test]
fn pin_diagnostics_identify_the_pin_or_mode_and_original_binding() {
    let source = "def A=PortB.0 OUTPUT; def B=PortB.0 INPUT; loop {}";
    let errors = check(source, "atmega328p").unwrap_err();
    let e = &errors[0];
    assert_eq!(excerpt(source, e.location.unwrap()), "PortB.0");
    assert_eq!(excerpt(source, e.labels[0].location), "PortB.0");
    let source = "def A=PortB.0 INPUT_PULLDOWN; loop {}";
    let errors = check(source, "atmega328p").unwrap_err();
    assert_eq!(
        excerpt(source, errors[0].location.unwrap()),
        "INPUT_PULLDOWN"
    );
}

#[test]
fn undefined_values_do_not_cascade_into_ternary_or_bound_type_errors() {
    for source in [
        "loop { var x=true?missing:1; }",
        "loop { for i=missing to 3 {} }",
        "loop { var x=missing+1; }",
    ] {
        let errors = analyze(source).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:#?}");
        assert_eq!(errors[0].code(), DiagnosticCode::Undefined);
    }
}

#[test]
fn recursion_has_related_call_sites() {
    let source = "fn a() { b(); } fn b() { a(); } loop {}";
    let e = error(source, DiagnosticCode::Recursion);
    assert!(e.labels.iter().any(|l| l.kind == LabelKind::RecursiveCall));
    assert!(e
        .labels
        .iter()
        .all(|l| !excerpt(source, l.location).is_empty()));
}

#[test]
fn unicode_crlf_and_eof_ranges_are_valid_bytes() {
    let source = "loop {\r\n\tvar температура:u8=300;\r\n}";
    let e = error(source, DiagnosticCode::LiteralRange);
    assert_eq!(excerpt(source, e.location.unwrap()), "300");
    let file = SourceFile {
        id: FileId(3),
        name: "температура.he",
        text: source,
    };
    assert!(e.source_spans(&file).is_some());

    for source in ["", "loop {\r\n", "fn f(x:u8):u8 {\n return"] {
        for e in analyze(source).unwrap_err() {
            let file = SourceFile {
                id: FileId(0),
                name: "incomplete.he",
                text: source,
            };
            assert!(e.location.is_some());
            assert!(e.source_spans(&file).is_some(), "{source:?}: {e:#?}");
            e.render(source);
        }
    }
}

#[test]
fn terminal_width_and_byte_offsets_are_separate() {
    use helium::frontend::utils::SourceLocation;
    let text = "界e\u{301}\tX";
    let span = SourceLocation {
        file: 0,
        line_start: 1,
        line_end: 1,
        start_offset: 3,
        end_offset: 4,
    };
    assert_eq!(excerpt(text, span), "X");
    let e = Diagnostic::new(
        DiagnosticKind::UndefinedName { name: "X".into() },
        Some(span),
    );
    assert!(e.render(text).ends_with("     |     ^"));
}

#[test]
fn operators_are_primary_and_operands_are_related_locations() {
    let source = "loop { true + 1; }";
    let e = error(source, DiagnosticCode::BinaryType);
    assert_eq!(excerpt(source, e.location.unwrap()), "+");
    assert_eq!(e.labels.len(), 2);
    assert_eq!(excerpt(source, e.labels[0].location), "true");
    assert_eq!(excerpt(source, e.labels[1].location), "1");

    let source = "loop { !1; }";
    let e = error(source, DiagnosticCode::UnaryType);
    assert_eq!(excerpt(source, e.location.unwrap()), "!");

    let source = "loop { var x:u8=4 / 0; }";
    let errors = check(source, "atmega328p").unwrap_err();
    assert_eq!(excerpt(source, errors[0].location.unwrap()), "/");
}

#[test]
fn incomplete_unicode_programs_keep_every_range_inside_the_file() {
    for source in [
        "fn тепло(значение:u8):u8 { if значение>0 { return значение; } } loop {}",
        "loop {\r\nvar строка=\"text\"; /* comment\nsecond line */ }",
        "loop { var x=123456789012345678901234567890; }",
    ] {
        for end in source
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(source.len()))
        {
            let prefix = &source[..end];
            if let Err(errors) = analyze(prefix) {
                let file = SourceFile {
                    id: FileId(0),
                    name: "prefix.he",
                    text: prefix,
                };
                for e in errors {
                    assert!(e.location.is_some());
                    assert!(e.source_spans(&file).is_some(), "{prefix:?}: {e:#?}");
                    e.render(prefix);
                }
            }
        }
    }
}

#[test]
fn invalid_constant_arithmetic_is_rejected_even_after_a_return() {
    let source = "fn f() { return; var x:u8=4/0; } loop {}";
    let errors = check(source, "atmega328p").unwrap_err();
    assert_eq!(excerpt(source, errors[0].location.unwrap()), "/");
}
