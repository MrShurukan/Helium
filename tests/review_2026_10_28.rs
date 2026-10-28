//! Independent review probes and regressions for the resolved CLI findings.
use helium::ir::{self, interpret::Interpreter, Options};
use std::{fs, process::Command};

fn options(optimize: bool) -> Options {
    Options {
        optimize,
        verify: true,
        cpu_hz: Some(16_000_000),
    }
}

#[test]
fn review_nested_loans_moves_and_global_effects() {
    for body in [
        "both(&mut a,a.clone());",
        "take(a); var b=a.clone();",
        "take(a); var n=a.len();",
        "both(&mut a, (a=[2;2]));",
        "read2(&a, mutate(&mut a));",
        "a[index(&mut a)]=a[1];",
        "var n=a[index(&mut a)];",
        "while true { var b=a.clone(); take(a); }",
    ] {
        let source = format!("fn both(x:&mut [u8],y:[u8;2]){{}} fn take(x:[u8;2]){{}} fn read2(x:&[u8],y:u8){{}} fn mutate(x:&mut [u8]):u8{{x[0]=9;return 0;}} fn index(x:&mut [u8]):u16{{x[0]=9;return 0;}} setup{{var a:[u8;2]=[1;2];{body}}}loop{{}}");
        let errors = helium::analyze(&source).expect_err(&source);
        assert!(
            errors
                .iter()
                .any(|e| e.message().contains("borrow") || e.message().contains("moved")),
            "{source}: {errors:#?}"
        );
    }
    for reference in ["&[u8;2]", "&[u8]", "&mut [u8]"] {
        let borrow = if reference.contains("mut") {
            "&mut storage"
        } else {
            "&storage"
        };
        let source = format!("var storage:[u8;2]=[0;2];fn change(){{storage[0]=1;}}fn relay(){{change();}}fn use_ref(x:{reference}){{relay();}}setup{{noInterrupt{{use_ref({borrow});}}}}loop{{}}");
        assert!(helium::analyze(&source).is_err(), "{source}");
    }
}

#[test]
fn review_snapshots_reinitialization_and_text_remapping() {
    let sources = [
        ("fn pair(x:[u8;2],y:[u8;2]):u8{return x[0]+y[0];}fn run():u8{var a:[u8;2]=[3;2];return pair(a.clone(),(a=[7;2]));}setup{}loop{}", 10),
        ("fn take(x:[u8;2]){}fn run():u8{var a:[u8;2]=[3;2];var i:u8=0;while i<2{take(a);a=[7;2];i=i+1;}return a[0];}setup{}loop{}", 7),
        ("fn inner(t:&str,u:&str,a:&mut [u16]):u16{a[0]=t.len()+u.len();return a[0];}fn outer(t:&str,u:&str,a:[u16;2]):[u16;2]{var b=a.clone();var n=inner(t,u,&mut b);b[1]=n;return b;}fn run():u16{var a:[u16;2]=[0;2];var b=outer(\"abc\",\"de\",a);return b[0]+b[1];}setup{}loop{}", 10),
    ];
    for (source, expected) in sources {
        for optimize in [false, true] {
            let module = helium::compile_typed_ir(source, "atmega328p", options(optimize)).unwrap();
            for module in [&module, &ir::text::specialize(&module).unwrap()] {
                let run = module
                    .functions
                    .iter()
                    .find(|f| f.name == "run")
                    .unwrap()
                    .id;
                assert_eq!(
                    Interpreter::new(module, 10000)
                        .unwrap()
                        .invoke(run, &[])
                        .unwrap()
                        .unwrap()
                        .bits,
                    expected
                );
                helium::backend::llvm::emit(module, false).unwrap();
            }
        }
    }
}

#[test]
fn review_output_must_preserve_imported_source() {
    let root = std::env::temp_dir().join(format!("helium-review-import-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let main = root.join("main.he");
    let helper = root.join("helper.he");
    let original = "pub fn value():u8{return 7;}";
    fs::write(
        &main,
        "import app::helper;var result:u8=0;setup{result=app::helper::value();}loop{}",
    )
    .unwrap();
    fs::write(&helper, original).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_helium"))
        .arg("build")
        .arg(&main)
        .args(["--mcu", "atmega328p", "--emit", "llvm", "-o"])
        .arg(&helper)
        .output()
        .unwrap();
    let preserved = fs::read_to_string(&helper).unwrap() == original;
    fs::remove_dir_all(&root).unwrap();
    assert!(
        preserved,
        "Imported source was overwritten; exit={:?}",
        result.status.code()
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("source file"));
}

#[test]
#[ignore = "requires LLVM AVR and AVR toolchain; no flashing"]
fn review_output_must_preserve_hard_linked_source() {
    let root = std::env::temp_dir().join(format!("helium-review-hardlink-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let main = root.join("main.he");
    let intermediate = root.join("inspect");
    fs::create_dir_all(&intermediate).unwrap();
    let output = intermediate.join("program.hex");
    let original = "setup{}loop{}";
    fs::write(&main, original).unwrap();
    fs::hard_link(&main, &output).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_helium"))
        .arg("build")
        .arg(&main)
        .args(["--mcu", "atmega328p", "--emit", "hex", "--intermediates"])
        .arg(&intermediate)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    let preserved = fs::read_to_string(&main).unwrap() == original;
    fs::remove_dir_all(&root).unwrap();
    assert!(
        preserved,
        "Hard-linked source was overwritten; exit={:?}",
        result.status.code()
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
#[ignore = "requires LLVM AVR; no flashing"]
fn review_assembly_can_target_its_own_intermediate() {
    let root = std::env::temp_dir().join(format!("helium-review-assembly-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let main = root.join("main.he");
    let intermediate = root.join("inspect");
    let output = intermediate.join("program.s");
    fs::write(&main, "setup{}loop{}").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_helium"))
        .arg("build")
        .arg(&main)
        .args(["--mcu", "atmega328p", "--emit", "asm", "--intermediates"])
        .arg(&intermediate)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
    let exists = output.is_file();
    fs::remove_dir_all(&root).unwrap();
    assert!(result.status.success() && exists, "{stderr}");
}

#[test]
fn output_publication_preserves_aliases_and_cleans_staging() {
    use helium::backend::output;
    let root = std::env::temp_dir().join(format!("helium-review-publish-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source.he");
    let destination = root.join("output.ll");
    fs::write(&source, "original").unwrap();
    fs::hard_link(&source, &destination).unwrap();
    let protected = [source.clone()];
    output::write(&destination, b"new output", &protected).unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read(&destination).unwrap(), b"new output");
    assert!(output::write(&source, b"bad", &protected).is_err());
    assert!(output::copy(&root.join("missing"), &destination, &protected).is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"new output");
    assert!(!fs::read_dir(&root).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".helium-output-")));
    fs::remove_dir_all(root).unwrap();
}
