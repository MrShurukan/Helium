use helium::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticKind, RenderOptions};
use helium::frontend::{utils::SourceLocation, ParserErrorType};
use helium::{analyze, check};

#[test]
fn structured_errors_keep_names_types_and_spans() {
    let errors = analyze("loop { missing; }").unwrap_err();
    let error = &errors[0];

    assert_eq!(error.code(), DiagnosticCode::Undefined);
    assert!(
        matches!(error.kind.as_ref(), DiagnosticKind::UndefinedName { name } if name == "missing")
    );
    let span = error.location.unwrap();
    assert_eq!(
        (span.line_start, span.start_offset, span.end_offset),
        (1, 7, 14)
    );
    assert_eq!(error.code().as_str(), "E_UNDEFINED");

    let errors = analyze("loop { var value:u8=300; }").unwrap_err();
    assert!(matches!(
        errors[0].kind.as_ref(),
        DiagnosticKind::IntegerRange {
            value: 300,
            min: 0,
            max: 255,
            ..
        }
    ));
}

#[test]
fn scanner_parser_and_analyzer_share_plain_rendering() {
    let cases = [
        ("loop { @ }", include_str!("snapshots/scanner-error.txt")),
        (
            "loop { var x = ; }",
            include_str!("snapshots/parser-error.txt"),
        ),
        (
            "loop {\r\n\tvar температура:u8=300;\r\n}",
            include_str!("snapshots/type-error.txt"),
        ),
    ];

    for (source, expected) in cases {
        let errors = analyze(source).unwrap_err();
        let error = &errors[0];
        assert_eq!(
            error.render(source),
            expected.replace("\r\n", "\n").trim_end()
        );
        assert!(error.location.is_some());
        assert!(!error.to_string().contains('\x1b'));
        assert!(!error.message().contains("Happened here"));

        let colored = error.render_with(source, RenderOptions { color: true });
        let uncolored = colored
            .replace("\x1b[1;31m", "")
            .replace("\x1b[34m", "")
            .replace("\x1b[0m", "");
        assert_eq!(uncolored, error.render(source));
        assert!(colored.starts_with("\x1b[1;31merror["));
        assert!(colored.contains("\x1b[34m"));
    }
}

#[test]
fn missing_source_location_and_multiline_spans_render_without_panics() {
    let no_location = Diagnostic::new(
        DiagnosticKind::Parse(ParserErrorType::MissingLoopDeclaration),
        None,
    );
    assert_eq!(
        no_location.render(""),
        "error[E_PARSE]: 'loop' must be defined"
    );

    let error = Diagnostic::new(
        DiagnosticKind::ExpressionLowering,
        Some(SourceLocation {
            line_start: 1,
            line_end: 2,
            start_offset: 1,
            end_offset: 2,
        }),
    );
    assert!(error
        .render("абв\r\nгде")
        .ends_with("   1 | абв\n     |  ^^\n   2 | где\n     | ^^"));
}

#[test]
fn support_errors_use_the_same_renderer() {
    let source = "loop { var x=1; }";
    let errors = check(source, "atmega328p").unwrap_err();
    assert!(matches!(
        errors[0].kind.as_ref(),
        DiagnosticKind::StorageLowering
    ));
    assert!(errors[0]
        .render(source)
        .starts_with("error[E_UNSUPPORTED]: "));
    assert!(errors[0]
        .render(source)
        .contains("   1 | loop { var x=1; }"));
}

#[test]
fn a_trailing_empty_line_can_carry_an_eof_marker() {
    let error = Diagnostic::new(
        DiagnosticKind::Parse(ParserErrorType::RanOutOfTokens),
        Some(SourceLocation {
            line_start: 2,
            line_end: 2,
            start_offset: 0,
            end_offset: 0,
        }),
    );

    assert!(error.render("loop {\r\n").ends_with("   2 | \n     | ^"));
}
