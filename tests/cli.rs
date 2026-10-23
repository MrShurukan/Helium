use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
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
        fs::write(&source, include_str!("../goal.he")).unwrap();
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
            child.env("PATH", "");
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
fn check_and_emit_need_no_external_tools_and_are_quiet() {
    let fixture = Fixture::new();
    for (command, extra) in [
        ("check", vec![]),
        ("build", vec!["--emit", "asm"]),
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
        .join("build/goal.asm")
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
fn build_reports_missing_avra_without_panicking() {
    let fixture = Fixture::new();
    let result = fixture.run("build", &[], false);
    assert!(!result.status.success());
    assert!(stderr(&result).contains("Cannot start avra"));
    assert!(!stderr(&result).contains("panicked"));
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
#[ignore = "requires AVRA on PATH; does not flash hardware"]
fn avra_builds_hex_from_a_different_directory() {
    let fixture = Fixture::new();
    let result = fixture.run("build", &[], true);
    assert!(result.status.success(), "{}", stderr(&result));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("Upload"));
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
    assert_eq!(count, 40, "historical GPIO sample code size");
}
#[test]
#[ignore = "requires AVRA on PATH; does not flash hardware"]
fn avra_failure_is_returned_as_an_error() {
    let fixture = Fixture::new();
    let assembly = fixture.root.join("invalid.asm");
    fs::write(&assembly, "this_is_not_an_instruction\n").unwrap();
    let error = helium::backend::assembler::assemble(&assembly, &fixture.root.join("invalid.hex"))
        .unwrap_err();
    assert!(error.contains("avra failed"), "{error}");
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
        .args(["--mcu", "atmega328p", "--emit", "asm"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", stderr(&result));
    assert!(source
        .parent()
        .unwrap()
        .join("build/tank.control.asm")
        .is_file());
}
