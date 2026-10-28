use helium::ir::{self, interpret::Interpreter, Options};

fn compile(source: &str, optimize: bool) -> ir::Module {
    helium::compile_typed_ir(
        source,
        "atmega328p",
        Options {
            optimize,
            verify: true,
            cpu_hz: Some(16_000_000),
        },
    )
    .unwrap_or_else(|e| panic!("{source}\n{e:#?}"))
}

#[test]
fn scalar_references_reborrow_and_preserve_caller_storage() {
    let source = "fn inc(x:&mut u16){*x=*x+1;}fn read(x:&u16):u16{return *x;}fn twice(x:&mut u16){inc(x);inc(&mut *x);var n=read(x);*x=n+1;}fn run():u16{var n:u16=1000;twice(&mut n);return n;}setup{}loop{}";
    run(source, 1003);
    run("fn wide(x:&u8):u16{return *x;}fn write(x:&mut u8):u16{return *x=255;}fn run():u16{var n:u8=1;var a=write(&mut n);return a+wide(&n);}setup{}loop{}",510);
    run(
        "fn run():u8{var x:u8=3;*(&mut x)=*(&x)+1;return x;}setup{}loop{}",
        4,
    );
}

fn run(source: &str, expected: u64) {
    for optimize in [false, true] {
        let m = compile(source, optimize);
        for m in [&m, &ir::text::specialize(&m).unwrap()] {
            let id = m.functions.iter().find(|f| f.name == "run").unwrap().id;
            assert_eq!(
                Interpreter::new(m, 10000)
                    .unwrap()
                    .invoke(id, &[])
                    .unwrap()
                    .unwrap()
                    .bits,
                expected
            );
            helium::backend::llvm::emit(m, false).unwrap();
        }
    }
}

#[test]
fn references_to_elements_check_bounds_and_whole_array_replacement() {
    run("fn copy(a:&[u8;2]):[u8;2]{return (*a).clone();}fn run():u8{var a:[u8;2]=[3;2];var b=copy(&a);b[0]=7;return a[0]+b[0];}setup{}loop{}",10);
    run("fn set(x:&mut u16){*x=1000;}fn fill(a:&mut [u16;2]){*a=[3;2];set(&mut a[1]);}fn via(a:&mut [u16]){set(&mut a[0]);}fn run():u16{var a:[u16;2]=[0;2];fill(&mut a);via(&mut a);return a[0]+a[1];}setup{}loop{}",2000);
    let m = compile(
        "fn set(x:&mut u8){*x=1;}fn run(i:u16){var a:[u8;2]=[0;2];set(&mut a[i]);}setup{}loop{}",
        false,
    );
    let id = m.functions.iter().find(|f| f.name == "run").unwrap().id;
    assert_eq!(
        Interpreter::new(&m, 1000)
            .unwrap()
            .invoke(id, &[ir::Constant::new(ir::Type::U16, 2)]),
        Err(ir::interpret::Error::Bounds)
    );
}

#[test]
fn references_reject_aliases_escapes_upgrades_and_moves() {
    for source in [
        "fn both(x:&mut u8,y:u8){}setup{var n:u8=0;both(&mut n,n);}loop{}",
        "fn both(x:&mut u8,y:&mut u8){}setup{var a:[u8;2]=[0;2];both(&mut a[0],&mut a[1]);}loop{}",
        "fn set(x:&mut u8){}fn bad(x:&u8){set(&mut *x);}setup{}loop{}",
        "fn set(x:&mut u8){}fn bad(x:&[u8]){set(&mut x[0]);}setup{}loop{}",
        "fn set(x:&mut u8){}fn bad(x:&[u8]){set(&mut (x[0]));}setup{}loop{}",
        "fn set(x:&mut u8){}fn bad(x:&u8){set(&mut (*x));}setup{}loop{}",
        "fn index(a:&mut [u8]):u16{return 0;}setup{var a:[u8;2]=[0;2];var n=*(&a[index(&mut a)]);}loop{}",
        "fn index(a:&mut [u8]):u16{return 0;}fn set(x:&mut u8){}setup{var a:[u8;2]=[0;2];set(&mut a[index(&mut a)]);}loop{}",
        "fn bad(x:&u8){*x=1;}setup{}loop{}",
        "fn bad(x:&mut u8){x=x;}setup{}loop{}",
        "fn bad(x:&u8):&u8{return x;}setup{}loop{}",
        "setup{var x:u8=0;var r=&x;}loop{}",
        "fn take(x:[u8;2]){}fn bad(a:&mut [u8;2]){take(*a);}setup{}loop{}",
        "fn bad(a:&mut [u8;2]):[u8;2]{return *a;}setup{}loop{}",
        "fn bad(a:&mut [u8;2]){var b=(*a=[0;2]);}setup{}loop{}",
        "fn bad(a:&mut [u8]){*a=[0;2];}setup{}loop{}",
        "fn bad(x:&mut u8){var r=x.clone();}setup{}loop{}",
        "fn both(a:&mut [u8;2],b:[u8;2]){}fn bad(a:&mut [u8;2]){both(a,(*a).clone());}setup{}loop{}",
        "fn set(x:&mut u8){}setup{set(&mut 1);}loop{}",
    ] {
        assert!(helium::analyze(source).is_err(), "{source}");
    }
}

#[test]
fn scalar_global_aliases_and_isr_keep_existing_protection() {
    let source="var g:u16=0;fn change(){g=1;}fn relay(){change();}fn read(x:&u16){relay();}setup{noInterrupt{read(&g);}}loop{}";
    assert!(helium::analyze(source).is_err());
    let prefix =
        "var g:u16=0;fn inc(x:&mut u16){*x=*x+1;}timer t=Timer1{every:1ms,interrupt{g=g+1;}}";
    assert!(helium::analyze(&format!("{prefix}loop{{inc(&mut g);}}")).is_err());
    compile(
        &format!("{prefix}loop{{noInterrupt{{inc(&mut g);}}}}"),
        false,
    );
    let source="var a:[u8;2]=[0;2];fn index():u16{a[0]=1;return 0;}fn set(x:&mut u8){}setup{set(&mut a[index()]);}loop{}";
    assert!(helium::analyze(source).is_err());
}

#[test]
fn verifier_rejects_forged_references_writes_upgrades_and_uninitialized_addresses() {
    let raw = compile(
        "fn run():u8{var x:u8=0;*(&mut x)=1;return *(&x);}setup{}loop{}",
        false,
    );
    let mut forged = raw.clone();
    let i = forged
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
        .find(|i| matches!(i.op, ir::Op::AddressLocal(..)))
        .unwrap();
    i.op = ir::Op::Const(ir::Constant::new(i.result.unwrap().1, 0));
    assert!(ir::verify::verify(&forged).is_err());
    let mut shared = raw.clone();
    for i in shared
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
    {
        if let ir::Op::AddressLocal(_, m) = &mut i.op {
            *m = false;
            if let Some((_, ir::Type::Ref(_, m))) = &mut i.result {
                *m = false;
            }
        }
    }
    assert!(ir::verify::verify(&shared).is_err());
    let mut uninitialized = raw;
    for b in &mut uninitialized
        .functions
        .iter_mut()
        .find(|f| f.name == "run")
        .unwrap()
        .blocks
    {
        b.instructions
            .retain(|i| !matches!(i.op, ir::Op::StoreLocal(..)));
    }
    assert!(ir::verify::verify(&uninitialized).is_err());
    let mut upgraded = compile(
        "fn run(){var a:[u8;2]=[0;2];var n=*(&a[0]);}setup{}loop{}",
        false,
    );
    for i in upgraded
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
    {
        if let ir::Op::ElementAddress(_, _, m) = &mut i.op {
            *m = true;
            if let Some((_, ir::Type::Ref(_, m))) = &mut i.result {
                *m = true;
            }
        }
    }
    assert!(ir::verify::verify(&upgraded).is_err());
}

#[test]
fn reference_parser_handles_incomplete_input_and_rejects_unsupported_places() {
    let source = include_str!("../examples/uart-references.he");
    for (i, _) in source.char_indices() {
        let _ = helium::analyze(&source[..i]);
    }
    for source in [
        "fn bad(x:&&u8){}loop{}",
        "fn bad(x:&mut str){}loop{}",
        "loop{*1=2;}",
        "fn f(x:&u8,y:&u8,b:bool):u8{return *(b?x:y);}loop{}",
    ] {
        assert!(helium::analyze(source).is_err(), "{source}");
    }
}

#[test]
fn element_index_runs_once_and_text_specialization_remaps_references() {
    run("var calls:u16=0;fn index():u16{calls=calls+1;return 0;}fn set(t:&str,u:&str,x:&mut u16){*x=t.len()+u.len();}fn run():u16{var a:[u16;2]=[0;2];set(\"abc\",\"de\",&mut a[index()]);return a[0]+calls;}setup{}loop{}",6);
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain and AVR8js; no flashing"]
fn references_execute_on_avr_at_all_optimization_levels() {
    let source = include_str!("../examples/uart-references.he");
    run_avr_matrix(source, "references", "84,76,90,33", "84");
}

fn run_avr_matrix(source: &str, folder: &str, valid: &str, invalid: &str) {
    use helium::backend::{
        llvm,
        llvm_toolchain::{self, Optimization},
        microcontroller::ATMEGA328P,
    };
    use std::{path::Path, process::Command};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for optimize_ir in [false, true] {
        let raw = compile(source, optimize_ir);
        for level in [Optimization::None, Optimization::Speed, Optimization::Size] {
            let destination = root.join(format!("build/{folder}/{optimize_ir}-{level:?}.hex"));
            let llvm = llvm::emit(&raw, false).unwrap();
            let build =
                llvm_toolchain::build(&llvm, &ATMEGA328P, &destination, "hex", level).unwrap();
            std::fs::write(
                destination.with_extension("ir"),
                ir::text::specialize(&raw).unwrap().dump(),
            )
            .unwrap();
            for (input, expected) in [("00", valid), ("01", invalid)] {
                let output =
                    Command::new(std::env::var_os("HELIUM_NODE").unwrap_or_else(|| "node".into()))
                        .arg(root.join("tools/simulate-avr.cjs"))
                        .arg(&build.directory)
                        .arg(input)
                        .env("HELIUM_UART_CYCLES", "200000")
                        .output()
                        .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    String::from_utf8_lossy(&output.stdout)
                        .contains(&format!("\"uartBytes\":[{expected}]")),
                    "{optimize_ir} {level:?} {input}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
        }
    }
}
