use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
#[cfg(windows)]
fn full_test_gate_fails_when_toolchain_is_missing() {
    let fixture = Fixture::new();
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/test-all.ps1"))
        .arg("-UseConfiguredEnvironment")
        .env("HELIUM_LLVM_BIN", fixture.root.join("missing-toolchain"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("llc.exe"), "{}", stderr(&output));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Full gate passed"));
}
struct Fixture {
    root: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "helium-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("project with spaces")).unwrap();
        fs::create_dir_all(root.join("elsewhere")).unwrap();
        let source = root.join("project with spaces").join("goal.he");
        fs::write(&source, include_str!("../examples/functions-latch.he")).unwrap();
        Self { root, source }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_helium"));
        command
            .current_dir(self.root.join("elsewhere"))
            .env("NO_COLOR", "1");
        command
    }
    fn run(&self, command: &str, extra: &[&str], tools: bool) -> Output {
        let mut child = self.command();
        child
            .arg(command)
            .arg(&self.source)
            .args(["--mcu", "atmega328p"])
            .args(extra);
        if !tools {
            child
                .env("PATH", "")
                .env_remove("HELIUM_LLVM_BIN")
                .env_remove("HELIUM_AVR_BIN");
        }
        child.output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn project_imports_resolve_from_entry_directory_and_diagnostics_name_library_files() {
    let fixture = Fixture::new();
    let helper = fixture.source.parent().unwrap().join("helper.he");
    fs::write(
        &helper,
        "import extension std::uart::UartText;\npub fn send(device d:UartTx){d.newline();}\n",
    )
    .unwrap();
    fs::write(&fixture.source,"import app::helper; device debug=UART0{baud:38400,mode:TX,} timer t=Timer1{every:1ms,interrupt{app::helper::send(debug);}} loop{}").unwrap();
    let output = fixture.run("check", &["--cpu-hz", "16000000"], false);
    assert!(output.status.success(), "{}", stderr(&output));
    let message = stderr(&output);
    assert_eq!(message.matches("warning[W_ISR_BLOCKING]").count(), 1);
    assert!(message.contains("helper.he:2:"), "{message}");
    assert!(message.contains("<std>/uart.he:4:"), "{message}");
    fs::write(
        &helper,
        "pub fn send(device d:UartTx){\n d.write_byte(true);\n}",
    )
    .unwrap();
    let output = fixture.run("check", &["--cpu-hz", "16000000"], false);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("helper.he:2:"),
        "{}",
        stderr(&output)
    );
    assert!(!stderr(&output).contains("internal compiler error"));
}

#[test]
fn warnings_preserve_success_and_are_emitted_once_on_stderr() {
    let fixture = Fixture::new();
    fs::write(&fixture.source,"device debug=UART0{baud:38400,mode:TX,} fn send(){debug.write_byte(65);} timer t=Timer1{every:1ms,interrupt{send();}} loop{}").unwrap();
    for emit in [None, Some("ast"), Some("ir"), Some("llvm")] {
        let mut args = vec!["--cpu-hz", "16000000", "--color", "never"];
        if let Some(emit) = emit {
            args.extend(["--emit", emit]);
        }
        let output = fixture.run(if emit.is_some() { "build" } else { "check" }, &args, false);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            stderr(&output).matches("warning[W_ISR_BLOCKING]").count(),
            1
        );
        assert!(stderr(&output).contains("effect propagated through this call"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("warning["));
    }
}

#[test]
#[ignore = "requires LLVM AVR and AVR toolchain; no flashing"]
fn warning_only_build_produces_hex_and_reports_baud() {
    let fixture = Fixture::new();
    fs::write(&fixture.source,"device debug=UART0{baud:38400,mode:TX,} timer t=Timer1{every:1ms,interrupt{debug.write_byte(65);}} loop{}").unwrap();
    let destination = fixture.root.join("warning.hex");
    let output = fixture.run(
        "build",
        &[
            "--cpu-hz",
            "16000000",
            "--emit",
            "hex",
            "-o",
            destination.to_str().unwrap(),
        ],
        true,
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stderr(&output).matches("warning[W_ISR_BLOCKING]").count(),
        1
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("UART0: TX 8N1"));
    assert!(fs::read_to_string(destination)
        .unwrap()
        .contains(":00000001FF"));
}

#[test]
fn diagnostic_color_policy_is_explicit_and_auto_is_plain_when_captured() {
    let fixture = Fixture::new();
    fs::write(&fixture.source, "loop { @ }").unwrap();

    for (options, expect_color) in [
        (vec![], false),
        (vec!["--color", "auto"], false),
        (vec!["--color", "never"], false),
        (vec!["--color", "always"], true),
    ] {
        let output = fixture.run("check", &options, false);
        assert!(!output.status.success());
        assert_eq!(stderr(&output).contains('\x1b'), expect_color);
        assert!(stderr(&output).contains("error[E_PARSE]"));
    }

    let output = fixture
        .command()
        .arg("check")
        .arg(&fixture.source)
        .args(["--mcu", "atmega328p", "--color", "always"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        stderr(&output).contains('\x1b'),
        "explicit flag overrides NO_COLOR"
    );

    let invalid = fixture.run("check", &["--color", "sometimes"], false);
    assert!(!invalid.status.success());
    assert!(stderr(&invalid).contains("Unknown color mode"));
}

#[test]
fn inspection_flags_and_removed_options_are_checked() {
    let fixture = Fixture::new();
    fs::write(&fixture.source, "def Led=PortC.5 OUTPUT; loop {Led=!Led;}").unwrap();

    let output = fixture.run(
        "build",
        &["--emit", "ir", "--verify-ir", "--optimize-ir"],
        false,
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let ir = fs::read_to_string(fixture.source.parent().unwrap().join("build/goal.ir")).unwrap();
    assert!(ir.starts_with("helium-ir v1"));
    assert!(ir.contains("read_pin p0"));
    assert!(ir.contains("not v"));

    for args in [
        vec!["--emit", "asm", "--optimize-ir"],
        vec!["--emit", "ast", "--verify-ir"],
        vec!["--emit", "ir", "--verify-ir", "--verify-ir"],
        vec!["--backend", "llvm"],
        vec!["--backend", "legacy"],
        vec!["--emit", "legacy-ir"],
    ] {
        assert!(!fixture.run("build", &args, false).status.success());
    }
}
#[test]
fn check_and_emit_need_no_external_tools_and_are_quiet() {
    let fixture = Fixture::new();
    for (command, extra) in [
        ("check", vec![]),
        ("build", vec!["--emit", "llvm"]),
        ("build", vec!["--emit", "ir"]),
        ("build", vec!["--emit", "ast"]),
    ] {
        let result = fixture.run(command, &extra, false);
        assert!(result.status.success(), "{}", stderr(&result));
        assert_eq!(String::from_utf8_lossy(&result.stdout).lines().count(), 1);
    }
    assert!(fixture
        .source
        .parent()
        .unwrap()
        .join("build/goal.ll")
        .is_file());
    assert!(fixture
        .source
        .parent()
        .unwrap()
        .join("build/goal.ir")
        .is_file());
    assert!(!fixture.root.join("elsewhere/build").exists());
}
#[test]
fn build_reports_missing_llvm_without_panicking() {
    let fixture = Fixture::new();
    let result = fixture.run("build", &[], false);
    assert!(!result.status.success());
    assert!(stderr(&result).contains("Cannot run"));
    assert!(!stderr(&result).contains("panicked"));
    assert_eq!(
        fs::read_dir(fixture.source.parent().unwrap().join("build"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn intermediates_options_and_failed_rebuild_preserve_existing_files() {
    let fixture = Fixture::new();
    for args in [
        vec!["--intermediates", "--intermediates"],
        vec!["--intermediates="],
        vec!["--emit", "ir", "--intermediates"],
        vec!["--emit", "llvm", "--intermediates"],
    ] {
        let result = fixture.run("build", &args, false);
        assert!(!result.status.success());
        assert!(
            !stderr(&result).contains("Cannot run"),
            "{}",
            stderr(&result)
        );
    }
    assert!(!fixture
        .run("check", &["--intermediates"], false)
        .status
        .success());
    let folder = fixture.root.join("fixed intermediates");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("module.ir"), "old IR").unwrap();
    fs::write(folder.join("notes.txt"), "keep").unwrap();
    let output = fixture.root.join("firmware.hex");
    fs::write(&output, "old firmware").unwrap();
    let result = fixture.run(
        "build",
        &[
            "--intermediates",
            folder.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ],
        false,
    );
    assert!(!result.status.success());
    assert_eq!(
        fs::read_to_string(folder.join("module.ir")).unwrap(),
        "old IR"
    );
    assert_eq!(fs::read_to_string(output).unwrap(), "old firmware");
    assert_eq!(
        fs::read_to_string(folder.join("notes.txt")).unwrap(),
        "keep"
    );
    assert!(!fs::read_dir(&fixture.root).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("llvm-")));
}

#[test]
fn intermediates_cannot_replace_source_or_use_a_directory_as_a_file() {
    let fixture = Fixture::new();
    let folder = fixture.root.join("artifacts");
    fs::create_dir(&folder).unwrap();
    let source = folder.join("module.ir");
    fs::write(&source, "loop{}").unwrap();
    let result = fixture
        .command()
        .arg("build")
        .arg(&source)
        .args(["--mcu", "atmega328p", "--intermediates"])
        .arg(&folder)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        stderr(&result).contains("source file"),
        "{}",
        stderr(&result)
    );
    assert_eq!(fs::read_to_string(source).unwrap(), "loop{}");
    fs::create_dir(folder.join("optimized.ll")).unwrap();
    let result = fixture.run(
        "build",
        &["--intermediates", folder.to_str().unwrap()],
        false,
    );
    assert!(!result.status.success());
    assert!(
        stderr(&result).contains("regular file"),
        "{}",
        stderr(&result)
    );
}

#[test]
#[ignore = "requires LLVM and AVR toolchain; does not flash hardware"]
fn machine_build_intermediates_are_opt_in_and_repeatable() {
    let fixture = Fixture::new();
    fs::write(&fixture.source, "var result:u8=0;setup{result=7;}loop{}").unwrap();
    let default = fixture.run("build", &["--opt", "size"], true);
    assert!(default.status.success(), "{}", stderr(&default));
    assert!(!String::from_utf8_lossy(&default.stdout).contains("Intermediates:"));
    let parent = fixture.source.parent().unwrap().join("build");
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    let generated = fixture.run("build", &["--intermediates", "--opt", "size"], true);
    assert!(generated.status.success(), "{}", stderr(&generated));
    let dir = fs::read_dir(&parent)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.is_dir())
        .unwrap();
    for name in ["module.ir", "module.ll", "optimized.ll", "program.hex"] {
        assert!(dir.join(name).is_file(), "{name}");
    }
    let inspection = fixture.root.join("inspect.ir");
    let emitted = fixture.run(
        "build",
        &["--emit", "ir", "-o", inspection.to_str().unwrap()],
        false,
    );
    assert!(emitted.status.success());
    assert_eq!(
        fs::read(dir.join("module.ir")).unwrap(),
        fs::read(inspection).unwrap()
    );
    let folder = fixture.root.join("fixed intermediates");
    let output = folder.join("program.hex");
    let custom = fixture.run(
        "build",
        &[
            "--intermediates",
            folder.to_str().unwrap(),
            "--opt",
            "size",
            "-o",
            output.to_str().unwrap(),
        ],
        true,
    );
    assert!(custom.status.success(), "{}", stderr(&custom));
    fs::write(folder.join("notes.txt"), "keep").unwrap();
    let old_ir = fs::read(folder.join("module.ir")).unwrap();
    fs::write(folder.join("optimized.ll"), "stale").unwrap();
    fs::write(&fixture.source, "var result:u8=0;setup{result=8;}loop{}").unwrap();
    let custom = fixture.run(
        "build",
        &[
            &format!("--intermediates={}", folder.display()),
            "--opt",
            "size",
            "-o",
            output.to_str().unwrap(),
        ],
        true,
    );
    assert!(custom.status.success(), "{}", stderr(&custom));
    assert_ne!(fs::read(folder.join("module.ir")).unwrap(), old_ir);
    assert_ne!(
        fs::read_to_string(folder.join("optimized.ll")).unwrap(),
        "stale"
    );
    assert_eq!(
        fs::read_to_string(folder.join("notes.txt")).unwrap(),
        "keep"
    );
    let assembly = fixture.root.join("output.s");
    let custom = fixture.run(
        "build",
        &[
            "--emit",
            "asm",
            "--intermediates",
            folder.to_str().unwrap(),
            "-o",
            assembly.to_str().unwrap(),
        ],
        true,
    );
    assert!(custom.status.success(), "{}", stderr(&custom));
    assert!(!folder.join("program.hex").exists());
    assert!(folder.join("program.s").is_file());
    assert!(!fs::read_dir(&folder)
        .unwrap()
        .any(|e| e.unwrap().path().is_dir()));
}
#[test]
fn input_and_cli_errors_are_actionable() {
    let fixture = Fixture::new();
    for extra in [
        vec!["--emit", "unknown"],
        vec!["--emit"],
        vec!["--mcu", "other"],
        vec!["--programmer", "usbasp"],
    ] {
        let result = fixture.run("build", &extra, false);
        assert!(!result.status.success());
        assert!(!stderr(&result).contains("panicked"));
    }
    let result = fixture
        .command()
        .args(["flash", "missing.hex", "--mcu", "atmega328p"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(stderr(&result).contains("--programmer is required"));
    let result = fixture
        .command()
        .args([
            "flash",
            "missing.hex",
            "--mcu",
            "atmega328p",
            "--programmer",
            "usbasp",
        ])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(stderr(&result).contains("HEX file does not exist"));
    fs::write(&fixture.source, "loop {var x:u8=300;}").unwrap();
    let result = fixture.run("check", &[], false);
    assert!(stderr(&result).contains("E_LITERAL_RANGE"));
    assert!(stderr(&result).contains("^^^"));
}
#[test]
fn source_file_is_not_overwritten_by_output() {
    let fixture = Fixture::new();
    let original = fs::read(&fixture.source).unwrap();
    let result = fixture.run(
        "build",
        &["--emit", "asm", "-o", fixture.source.to_str().unwrap()],
        false,
    );
    assert!(!result.status.success());
    assert_eq!(fs::read(&fixture.source).unwrap(), original);
}
#[test]
#[ignore = "requires LLVM and AVR toolchain; does not flash hardware"]
fn llvm_builds_hex_from_a_different_directory() {
    let fixture = Fixture::new();
    let result = fixture.run("build", &[], true);
    assert!(result.status.success(), "{}", stderr(&result));
    let hex = fs::read_to_string(fixture.source.parent().unwrap().join("build/goal.hex")).unwrap();
    let mut count = 0;
    for line in hex.lines() {
        let data = (1..line.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&line[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            data.iter().fold(0u8, |sum, value| sum.wrapping_add(*value)),
            0,
            "HEX checksum"
        );
        if data[3] == 0 {
            count += usize::from(data[0]);
        }
    }
    assert!(count > 0);
}
#[test]
fn dotted_source_names_keep_the_full_stem() {
    let fixture = Fixture::new();
    let source = fixture.source.with_file_name("tank.control.he");
    fs::copy(&fixture.source, &source).unwrap();
    let result = fixture
        .command()
        .arg("build")
        .arg(&source)
        .args(["--mcu", "atmega328p", "--emit", "llvm"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", stderr(&result));
    assert!(source
        .parent()
        .unwrap()
        .join("build/tank.control.ll")
        .is_file());
}
