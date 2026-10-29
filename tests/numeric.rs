use helium::ir::{
    self,
    interpret::{Error, Interpreter},
    Constant, Options, PanicReason, Type,
};

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

fn run(source: &str, arguments: &[Constant], expected: Result<u64, Error>) {
    for optimize in [false, true] {
        let m = compile(source, optimize);
        let f = m.functions.iter().find(|f| f.name == "run").unwrap().id;
        assert_eq!(
            Interpreter::new(&m, 10000)
                .unwrap()
                .invoke(f, arguments)
                .map(|v| v.unwrap().bits),
            expected,
            "{source}, optimize={optimize}"
        );
        helium::backend::llvm::emit(&m, false).unwrap();
    }
}

#[test]
fn numeric_casts_preserve_source_width_and_evaluate_once() {
    for (expr, expected) in [
        ("(u8)300", 44),
        ("(u8)-1", 255),
        ("(u16)(i8)255", 65535),
        ("(u16)((u8)255+1)", 0),
        ("(u16)(u8)255+1", 256),
    ] {
        run(
            &format!("fn run():u16{{return {expr};}}setup{{}}loop{{}}"),
            &[],
            Ok(expected),
        );
    }
    run("var n:u16=0;fn next():i32{n=n+1;return 300;}fn run():u16{var x=(u8)next();return x+n;}setup{}loop{}", &[], Ok(45));
    for source in [
        "setup{var x=(u8)true;}loop{}",
        "setup{var a:[u8;2]=[1;2];var x=(u8)a;}loop{}",
        "setup{var x=(bool)1;}loop{}",
    ] {
        assert!(helium::compile_typed_ir(source, "atmega328p", Options::default()).is_err());
    }
}

#[test]
fn integer_arithmetic_all_widths() {
    for (ty, signed, bits) in [
        ("u8", false, 8),
        ("i8", true, 8),
        ("u16", false, 16),
        ("i16", true, 16),
        ("u32", false, 32),
        ("i32", true, 32),
        ("u64", false, 64),
        ("i64", true, 64),
    ] {
        let src = format!("fn run(a:{ty},b:{ty}):{ty}{{return (a/b)*b+a%b;}}setup{{}}loop{{}}");
        let m = compile(&src, false);
        let t = m
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .return_type
            .unwrap();
        let a = if signed {
            Constant::new(t, (-73i64) as u64)
        } else {
            Constant::new(t, 201)
        };
        run(&src, &[a, Constant::new(t, 7)], Ok(a.bits));
        run(
            &format!("fn run(a:{ty},b:{ty}):{ty}{{return a/b;}}setup{{}}loop{{}}"),
            &[a, Constant::new(t, 0)],
            Err(Error::Panic(PanicReason::DivisionByZero)),
        );
        if signed {
            let min = Constant::new(t, 1u64 << (bits - 1));
            let negone = Constant::new(t, u64::MAX);
            run(
                &format!("fn run(a:{ty},b:{ty}):{ty}{{return a/b;}}setup{{}}loop{{}}"),
                &[min, negone],
                Err(Error::Panic(PanicReason::DivisionOverflow)),
            );
            run(
                &format!("fn run(a:{ty},b:{ty}):{ty}{{return a%b;}}setup{{}}loop{{}}"),
                &[min, negone],
                Ok(0),
            );
        }
    }
}

#[test]
fn shifts_bits_and_unused_panics() {
    run("fn run():u8{return ~0;}setup{}loop{}", &[], Ok(255));
    run(
        "fn run():u8{return ((3<<2)|1)^((~(u8)0)&3);}setup{}loop{}",
        &[],
        Ok(14),
    );
    run(
        "fn run(a:i8,b:i8):i8{return a>>b;}setup{}loop{}",
        &[Constant::new(Type::I8, 248), Constant::new(Type::I8, 2)],
        Ok(254),
    );
    for count in [8, 255] {
        run(
            "fn run(a:u8,b:u8):u8{a<<b;return 7;}setup{}loop{}",
            &[Constant::new(Type::U8, 1), Constant::new(Type::U8, count)],
            Err(Error::Panic(PanicReason::InvalidShift)),
        );
    }
    run(
        "fn run(a:u8,b:u8):u8{a/b;return 7;}setup{}loop{}",
        &[Constant::new(Type::U8, 1), Constant::new(Type::U8, 0)],
        Err(Error::Panic(PanicReason::DivisionByZero)),
    );
}

#[test]
fn numeric_ir_verifier_rejects_forged_casts_and_operands() {
    let source = "fn run(x:u16):u8{return (u8)x;}setup{}loop{}";
    let mut m = compile(source, false);
    let f = m.functions.iter_mut().find(|f| f.name == "run").unwrap();
    let cast = f
        .blocks
        .iter_mut()
        .flat_map(|b| &mut b.instructions)
        .find(|i| matches!(i.op, ir::Op::NumericCast(..)))
        .unwrap();
    let ir::Op::NumericCast(v, _) = cast.op else {
        unreachable!()
    };
    cast.op = ir::Op::Widen(v, Type::U8);
    assert!(ir::verify::verify(&m).is_err());
    let mut m = compile(source, false);
    let f = m.functions.iter_mut().find(|f| f.name == "run").unwrap();
    let cast = f
        .blocks
        .iter_mut()
        .flat_map(|b| &mut b.instructions)
        .find(|i| matches!(i.op, ir::Op::NumericCast(..)))
        .unwrap();
    cast.op = ir::Op::NumericCast(v, Type::Bool);
    assert!(ir::verify::verify(&m).is_err());
}

#[test]
fn numeric_diagnostics_and_mixed_widths() {
    run(
        "fn run(a:i16,b:u16):i32{return a+b;}setup{}loop{}",
        &[
            Constant::new(Type::I16, 65535),
            Constant::new(Type::U16, 65535),
        ],
        Ok(65534),
    );
    run(
        "fn run(a:u8,b:u64):u8{return a<<b;}setup{}loop{}",
        &[Constant::new(Type::U8, 3), Constant::new(Type::U64, 7)],
        Ok(128),
    );
    run(
        "fn run(a:u8,b:u64):u8{return a<<b;}setup{}loop{}",
        &[Constant::new(Type::U8, 3), Constant::new(Type::U64, 256)],
        Err(Error::Panic(PanicReason::InvalidShift)),
    );
    for statement in [
        "var x:u8=1/0;",
        "var x:u8=1%0;",
        "var x:i8=-128/-1;",
        "var x:u8=1<<8;",
        "var x:u8=1<<-1;",
        "var x:u8=1<<(u16)-1;",
    ] {
        for prefix in ["", "return;"] {
            let source = format!("fn f(){{{prefix}{statement}}}setup{{}}loop{{}}");
            let errors =
                helium::compile_typed_ir(&source, "atmega328p", Options::default()).unwrap_err();
            assert!(
                errors.iter().any(|e| matches!(
                    *e.kind,
                    helium::diagnostic::DiagnosticKind::Numeric { .. }
                ) && e.location.is_some()),
                "{source}: {errors:?}"
            );
        }
    }
    assert!(helium::analyze("fn f(a:i64,b:u64){a+b;}setup{}loop{}").is_err());
    // Parsing incomplete casts and new operators must remain diagnostic-only.
    let sample = "fn run(x:u16):u8{return (u8)((x<<2)^~x)%7;}setup{}loop{}";
    for end in 0..sample.len() {
        let _ = helium::analyze(&sample[..end]);
    }
}

#[test]
#[ignore = "requires LLVM AVR, avr-gcc and AVR8js"]
fn numeric_panics_and_widening_execute_on_avr() {
    for (name, expression, parameter, valid, invalid) in [
        ("shift", "var x:u8=1;x<<n;", "u64", "0", "256"),
        (
            "overflow",
            "var x:i64=-9223372036854775808;x/n;",
            "i64",
            "1",
            "-1",
        ),
        (
            "remainder",
            "var x:u64=18446744073709551615;x%n;",
            "u64",
            "7",
            "0",
        ),
    ] {
        let source=format!("device debug=UART0{{baud:38400,mode:TX,}}def input=PortB.0 INPUT;fn guarded(n:{parameter}){{{expression}}}setup{{var n:{parameter}={valid};if input==HIGH{{n={invalid};}}debug.write_byte(65);debug.flush();guarded(n);debug.write_byte(66);debug.flush();}}loop{{}}");
        run_avr_matrix(&source, &format!("numeric-{name}"), "65,66", "65");
    }
    let source="device debug=UART0{baud:38400,mode:TX,}def input=PortB.0 INPUT;fn test(a:i8,b:i16,c:i32,device out:UartTx){var x:i64=a;var y:i64=b;var z=(u64)c;if x==-73 and y==-300 and z==18446744073709481616{out.write_byte(84);}else{out.write_byte(70);}var v:u64=4294967297;var n:u8=32;var high:bool=input==HIGH;if high{n=33;}if v*v==8589934593 and (v<<n)>>n==1{out.write_byte(66);}else{out.write_byte(70);}var min:i64=-9223372036854775808;var neg:i64=-1;if high{neg=-2;}out.write_byte((u8)(min%neg));}setup{test(-73,-300,-70000,debug);debug.flush();}loop{}";
    run_avr_matrix(source, "numeric-extensions", "84,66,0", "84,66,0");
}

#[test]
#[ignore = "requires LLVM AVR, avr-gcc and AVR8js"]
fn numeric_executes_on_avr() {
    run_avr_matrix(
        include_str!("../examples/uart-numeric.he"),
        "numeric",
        "67,49,50,51,52,53,54,55,56,66",
        "67",
    );
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
