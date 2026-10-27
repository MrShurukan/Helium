use helium::{
    backend::microcontroller::ATMEGA328P,
    diagnostic::DiagnosticCode,
    ir::{Op, Options},
    modules::Project,
};
const DEVICE: &str = "device debug=UART0{baud:38400,mode:TX,}";
fn options() -> Options {
    Options {
        cpu_hz: Some(16_000_000),
        ..Options::default()
    }
}
fn project(body: &str) -> Project {
    Project::memory("main.he", &format!("{DEVICE} {body}"))
}
fn library(p: &mut Project, name: &str, text: &str) {
    p.modules.insert(name.into(), text.into());
}
fn compile(p: &mut Project) -> helium::ir::Module {
    helium::compile_project_ir(p, &ATMEGA328P, options())
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
}

#[test]
fn std_newline_specializes_without_runtime_receiver_or_phantom_hardware() {
    let mut p = project("import extension std::uart::UartText; setup{debug.newline();} loop{}");
    let m = compile(&mut p);
    assert_eq!(m.devices.len(), 1);
    let method = m
        .functions
        .iter()
        .find(|f| f.name.contains("UartText::newline<debug>"))
        .unwrap();
    assert!(method.parameters.is_empty());
    assert!(method
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .any(|i| matches!(i.op, Op::Intrinsic { .. })));
    assert!(!m.functions.iter().any(|f| f.name.contains("$check")));
    assert!(m.warnings.is_empty());
}

#[test]
fn static_helpers_forward_receivers_and_preserve_runtime_parameters() {
    let mut p=project("fn send(device out:UartTx, byte:u8){out.write_byte(byte);} extension X for UartTx{pub fn send_it(byte:u8){send(self,byte);}} setup{debug.send_it(42);} loop{}");
    let m = compile(&mut p);
    for f in m.functions.iter().filter(|f| f.name.contains("<debug>")) {
        assert_eq!(f.parameters.len(), 1);
    }
    for invalid in [
        "fn send(device out:Bad){} loop{}",
        "fn send(device out:UartTx){out=1;} loop{}",
        "fn send(device out:UartTx){var out:u8=1;} loop{}",
        "fn send(device out:UartTx){} loop{send(1);}",
    ] {
        assert!(project(invalid).analyze().is_err(), "{invalid}");
    }
}

#[test]
fn ordinary_imports_and_extension_imports_have_separate_visibility() {
    let text="pub fn number():u8{return 7;} pub extension X for UartTx{pub fn newline(){self.write_byte(10);}}";
    let mut p = project("import app::helpers; loop{var n:u8=app::helpers::number();}");
    library(&mut p, "app::helpers", text);
    compile(&mut p);
    let mut p = project("import app::helpers; loop{debug.newline();}");
    library(&mut p, "app::helpers", text);
    assert!(p.analyze().is_err());
    let mut p = project("import extension app::helpers::X; loop{X::newline(debug);}");
    library(&mut p, "app::helpers", text);
    compile(&mut p);
    let mut p = project("import extension app::helpers::X; loop{var n=app::helpers::number();}");
    library(&mut p, "app::helpers", text);
    assert!(p.analyze().is_err());
}

#[test]
fn extension_collision_is_independent_of_import_order_and_explicit_call_selects_one() {
    for imports in [
        "import extension app::a::A; import extension app::b::B;",
        "import extension app::b::B; import extension app::a::A;",
    ] {
        for call in ["debug.mark();", "A::mark(debug);"] {
            let mut p = project(&format!("{imports} loop{{{call}}}"));
            library(
                &mut p,
                "app::a",
                "pub extension A for UartTx{pub fn mark(){self.write_byte(1);}}",
            );
            library(
                &mut p,
                "app::b",
                "pub extension B for UartTx{pub fn mark(){self.write_byte(2);}}",
            );
            let result = p.analyze();
            if call.starts_with("debug") {
                assert!(result.unwrap_err()[0].message().contains("Ambiguous"));
            } else {
                result.unwrap();
            }
        }
    }
    for name in ["write_byte", "try_write_byte", "flush"] {
        let mut p = project(&format!(
            "extension X for UartTx{{fn {name}(){{}}}} loop{{}}"
        ));
        assert!(p.analyze().unwrap_err()[0].message().contains("built-in"));
    }
}

#[test]
fn private_methods_and_module_functions_are_not_accessible_from_callers() {
    for call in [
        "debug.hidden();",
        "X::hidden(debug);",
        "app::helpers::hidden();",
    ] {
        let mut p = project(&format!(
            "import app::helpers; import extension app::helpers::X; loop{{{call}}}"
        ));
        library(&mut p,"app::helpers","fn hidden(){} pub extension X for UartTx{fn hidden(){} pub fn visible(){self.hidden();}}");
        assert!(p.analyze().is_err(), "{call}");
    }
    let mut p = project("import extension app::helpers::X; loop{debug.visible();}");
    library(&mut p,"app::helpers","pub extension X for UartTx{fn hidden(){self.write_byte(1);} pub fn visible(){self.hidden();}}");
    compile(&mut p);
}

#[test]
fn imports_do_not_activate_transitive_extensions_and_libraries_cannot_capture_app_globals() {
    for body in [
        "pub fn helper(){debug.write_byte(1);}",
        "pub fn helper(){secret=1;}",
        "var x:u8=0;",
        "device uart=UART0{baud:38400,mode:TX,}",
        "setup{}",
        "loop{}",
    ] {
        let mut p = project("import app::bad; var secret:u8=0; loop{}");
        library(&mut p, "app::bad", body);
        assert!(p.analyze().is_err(), "{body}");
    }
    let mut p = project("import app::a; loop{debug.newline();}");
    library(
        &mut p,
        "app::a",
        "import extension std::uart::UartText; pub fn send(device d:UartTx){d.newline();}",
    );
    assert!(p.analyze().is_err());
}

#[test]
fn unused_generic_bodies_are_checked_even_without_any_real_devices() {
    let mut p = Project::memory("main.he", "import extension app::bad::X; loop{}");
    library(
        &mut p,
        "app::bad",
        "pub extension X for UartTx{pub fn unused(){self.write_byte(true);}}",
    );
    assert!(p.analyze().is_err());
    let mut p = Project::memory("main.he", "import extension std::uart::UartText; loop{}");
    assert!(compile(&mut p).devices.is_empty());
    let mut p = Project::memory("main.he", "fn unused(device d:UartTx){var x:u32=1;} loop{}");
    assert!(helium::compile_project_llvm(&mut p, &ATMEGA328P, options()).is_err());
}

#[test]
fn cyclic_imports_and_recursive_static_calls_are_diagnosed() {
    let mut p = project("import app::a; loop{}");
    library(&mut p, "app::a", "import app::b;");
    library(&mut p, "app::b", "import app::a;");
    assert!(p.analyze().unwrap_err()[0]
        .message()
        .contains("Import cycle"));
    let mut p = project("fn recurse(device d:UartTx){recurse(d);} loop{recurse(debug);}");
    assert!(p
        .analyze()
        .unwrap_err()
        .iter()
        .any(|d| d.code() == DiagnosticCode::Recursion));
}

#[test]
fn cross_file_warning_paths_and_shared_access_survive_specialization() {
    let mut p=project("import extension std::uart::UartText; timer t=Timer1{every:1ms,interrupt{debug.newline();}} loop{}");
    let m = compile(&mut p);
    assert_eq!(m.warnings.len(), 1);
    let d = &m.warnings[0];
    assert_eq!(d.location.unwrap().file, 0);
    assert!(d.labels.iter().any(|l| l.location.file == 1));
    let rendered = d.render_sources(&p.sources, Default::default());
    assert!(rendered.contains("main.he:1:"), "{rendered}");
    assert!(rendered.contains("<std>/uart.he:4:"), "{rendered}");
    for l in &d.labels {
        assert!(p.sources.span(l.location).is_some());
    }
    let mut p=project("import extension std::uart::UartText; timer t=Timer1{every:1ms,interrupt{debug.newline();}} loop{debug.newline();}");
    assert!(p
        .analyze()
        .unwrap_err()
        .iter()
        .any(|d| d.code() == DiagnosticCode::SharedAccess));
}

#[test]
fn receiver_binding_is_per_call_and_never_leaks_into_machine_storage() {
    let mut p=Project::memory("main.he","device a=UART0{baud:38400,mode:TX,} device b=UART0{baud:38400,mode:TX,} fn send(device d:UartTx){d.write_byte(1);} loop{send(a);send(b);}");
    let program = p.analyze().unwrap(); // Physical duplicate claims are checked later, independently.
    let functions: Vec<_> = program
        .symbols()
        .iter()
        .map(|(_, s)| s.name.as_str())
        .collect();
    assert!(functions.contains(&"send<a>"));
    assert!(functions.contains(&"send<b>"));
    let mut p =
        project("fn send(device d:UartTx){d.write_byte(1);} loop{send(debug);send(debug);}");
    let m = compile(&mut p);
    assert_eq!(
        m.functions
            .iter()
            .filter(|f| f.name == "send<debug>")
            .count(),
        1
    );
}

#[test]
fn malformed_module_and_extension_prefixes_never_panic() {
    let text="import extension std::uart::UartText; extension X for UartTx{pub noInterrupt fn send(device d:UartTx, b:u8){d.write_byte(b);}} loop{}";
    for end in 0..text.len() {
        assert!(
            std::panic::catch_unwind(|| helium::analyze(&text[..end])).is_ok(),
            "{end}"
        );
    }
}

#[test]
fn specialized_diagnostics_keep_original_file_ranges_and_shadow_rules() {
    for source in [
        "fn f(device d:UartTx){} var f:u8=1; loop{}",
        "fn f(device d:UartTx){} fn g(){var f:u8=1;} loop{}",
        "fn f(device d:UartTx){} loop{var f:u8=1;}",
    ] {
        assert!(helium::analyze(source)
            .unwrap_err()
            .iter()
            .any(|d| d.code() == DiagnosticCode::Duplicate));
    }
    for text in [
        "pub extension X for UartTx{pub fn go(){self.missing();}}",
        "pub fn f(device d:UartTx){x=1;}",
        "pub fn f(device d:UartTx){missing();}",
        "pub fn f(device d:UartTx){var x=missing;}",
        "pub fn f(device d:UartTx){for d=0 to 1{}}",
        "pub fn f(f:u8){}",
    ] {
        let mut p = project("import app::bad; loop{}");
        library(&mut p, "app::bad", text);
        let errors = p.analyze().unwrap_err();
        for d in errors {
            for location in d
                .location
                .into_iter()
                .chain(d.labels.iter().map(|l| l.location))
            {
                assert_eq!(location.file, 1, "{text}: {d:?}");
                assert!(p.sources.span(location).is_some(), "{text}: {d:?}");
            }
        }
    }
    let long = "device_name_that_is_much_longer_than_the_receiver";
    let source=format!("device {long}=UART0{{baud:38400,mode:TX,}} import extension app::e::X; timer t=Timer1{{every:1ms,interrupt{{{long}.go();}}}} loop{{{long}.go();}}");
    let mut p = Project::memory("main.he", &source);
    library(
        &mut p,
        "app::e",
        "pub extension X for UartTx{pub fn go(){self.write_byte(1);}}",
    );
    let errors = p.analyze().unwrap_err();
    assert!(errors
        .iter()
        .any(|d| d.code() == DiagnosticCode::SharedAccess));
    for d in errors {
        for l in d.labels {
            assert!(p.sources.span(l.location).is_some());
        }
    }
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain, Node and AVR8js; no flashing"]
fn imported_extension_executes_in_setup_and_isr_at_all_optimization_levels() {
    use helium::backend::{
        llvm,
        llvm_toolchain::{self, Optimization},
    };
    use std::{path::Path, process::Command};
    let body="import extension std::uart::UartText; var done:u8=0;
        fn send(device d:UartTx,b:u8){d.write_byte(b);}
        setup{send(debug,65);debug.newline();debug.flush();}
        timer t=Timer1{every:1ms,interrupt{if done==0{debug.newline();debug.flush();done=1;}}} loop{}";
    for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
        let mut p = project(body);
        let module = compile(&mut p);
        assert_eq!(module.warnings.len(), 2);
        let text = llvm::emit(&module, true).unwrap();
        let output = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("build/extension-tests/{level:?}.hex"));
        let build = llvm_toolchain::build(&text, &ATMEGA328P, &output, "hex", level).unwrap();
        let simulation =
            Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/simulate-avr.cjs"))
                .arg(build.directory)
                .env("HELIUM_UART_CYCLES", "80000")
                .output()
                .unwrap();
        assert!(
            simulation.status.success(),
            "{}",
            String::from_utf8_lossy(&simulation.stderr)
        );
        let actual = String::from_utf8(simulation.stdout).unwrap();
        assert!(actual.contains("\"uartBytes\":[65,10,10]"), "{actual}");
        assert!(actual.contains("helium_g0:1"), "{actual}");
        assert!(actual.contains("\"nestedInterrupt\":false"), "{actual}");
    }
}
