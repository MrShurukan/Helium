use helium::{
    analyze,
    diagnostic::{DiagnosticCode, LabelKind},
    semantic::BodyId,
};

fn shared_error(source: &str) {
    assert!(
        analyze(source)
            .unwrap_err()
            .iter()
            .any(|e| e.code() == DiagnosticCode::SharedAccess),
        "{source}"
    );
}

#[test]
fn setup_only_helpers_are_separate_from_runtime() {
    let source = "var value:u16=0; fn write() {value=1;} fn initialize() {write();}
        timer clock=Timer1 {every:1s, interrupt {var now=value;}}
        setup {initialize();} loop {}";
    let program = analyze(source).unwrap();
    let id = program
        .symbols()
        .iter()
        .find(|(_, s)| s.name == "write")
        .unwrap()
        .0;
    assert!(program
        .shared_analysis()
        .setup_bodies()
        .contains(&BodyId::Function(id)));
    assert!(!program
        .shared_analysis()
        .main_bodies()
        .contains(&BodyId::Function(id)));
    shared_error(&source.replace("loop {}", "loop {initialize();}"));
}

#[test]
fn unused_accesses_do_not_conflict_but_source_and_contracts_are_checked() {
    let source = "var value:u16=0; fn unused() {value=1;}
        timer clock=Timer1 {every:1s, interrupt {value=value+1;}}
        loop {noInterrupt {var now=value;}}";
    analyze(source).unwrap();
    for (body, code) in [
        ("missing=1;", DiagnosticCode::Undefined),
        ("var x:u8=true;", DiagnosticCode::Type),
        ("protected();", DiagnosticCode::InterruptContext),
    ] {
        let source = format!("noInterrupt fn protected() {{}} fn unused() {{{body}}} loop {{}}");
        let errors = analyze(&source).unwrap_err();
        assert!(errors.iter().any(|e| e.code() == code), "{errors:#?}");
    }
    assert!(analyze("noInterrupt fn protected() {} fn initialize() {protected();} setup {initialize();} loop {}").is_err());
}

#[test]
fn setup_writes_do_not_turn_runtime_reads_into_conflicts() {
    analyze(
        "var value:u16=0; fn initialize(){value=2;} fn read():u16{return value;}
        setup {initialize();} timer clock=Timer1 {every:1s, interrupt {read();}}
        loop {var now=read();}",
    )
    .unwrap();
}

#[test]
fn gpio_modes_conflict_only_on_the_same_pin() {
    let source = "def A=PortB.0 INPUT; def B=PortB.1 INPUT;
        timer clock=Timer1 {every:1s, interrupt {var level=!A;}} loop {B=OUTPUT;}";
    analyze(source).unwrap();
    shared_error(&source.replace("loop {B=OUTPUT;}", "loop {A=OUTPUT;}"));
    analyze(&source.replace("PortB.1", "PortC.1")).unwrap();
    analyze(&source.replace("B=OUTPUT;", "var level=!B;")).unwrap();
}

#[test]
fn all_expression_positions_and_syntactic_paths_are_checked() {
    let prefix = "var value:u8=0; fn take(x:u8):u8{return x;}
        timer clock=Timer1 {every:1s, interrupt {value=1;}} ";
    for body in [
        "var x=take(value);",
        "var x:u8=0; x=value;",
        "var x:u8=true ? value : 0;",
        "if false and value==0 {}",
        "if true or value==0 {}",
        "while value==0 {}",
        "for i=0 to value {}",
        "if false {var x=value;}",
    ] {
        shared_error(&format!("{prefix} loop {{{body}}}"));
    }
    shared_error(&format!(
        "{prefix} fn read():u8{{return value;}} loop {{noInterrupt {{read();}}}}"
    ));
}

#[test]
fn diagnostics_include_both_call_paths_and_are_deterministic() {
    let source = "var счёт:u16=0; fn чтение():u16{return счёт;} fn outer():u16{return чтение();}
        noInterrupt fn запись(){счёт=1;} noInterrupt fn tick(){запись();}
        timer clock=Timer1 {every:1s, interrupt {tick();}} loop {outer();}";
    let errors = analyze(source).unwrap_err();
    let error = errors
        .iter()
        .find(|e| e.code() == DiagnosticCode::SharedAccess)
        .unwrap();
    assert_eq!(error.labels[0].kind, LabelKind::InterruptAccess);
    assert_eq!(
        error
            .labels
            .iter()
            .filter(|l| l.kind == LabelKind::MainCall)
            .count(),
        2
    );
    assert_eq!(
        error
            .labels
            .iter()
            .filter(|l| l.kind == LabelKind::InterruptCall)
            .count(),
        2
    );
    assert_eq!(
        format!("{errors:?}"),
        format!("{:?}", analyze(source).unwrap_err())
    );
}

#[test]
fn initialization_accesses_and_isr_only_globals_are_recorded() {
    use helium::middle_end::shared::{AccessKind, Protection};
    let program = analyze(
        "def Led=PortB.0 OUTPUT; var value:u16=0;
        fn tick(){value=value+1;} timer clock=Timer1 {every:1s, interrupt {tick();}} loop {}",
    )
    .unwrap();
    let graph = program.shared_analysis();
    let init = &graph.bodies()[&BodyId::Initialization];
    assert_eq!(init.accesses.len(), 2);
    assert_eq!(init.accesses[0].kind, AccessKind::ConfigurePin);
    assert_eq!(init.accesses[1].kind, AccessKind::WriteMemory);
    assert!(init
        .accesses
        .iter()
        .all(|a| a.protection == Protection::Initialization));
    assert_eq!(graph.interrupt_globals().len(), 1);
}
