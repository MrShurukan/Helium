use helium::{
    compile_typed_ir,
    ir::{self, interpret::Interpreter, Options},
};

fn compile(source: &str, optimize: bool) -> ir::Module {
    compile_typed_ir(
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
fn run(source: &str) -> u64 {
    let m = compile(source, false);
    let id = m.functions.iter().find(|f| f.name == "run").unwrap().id;
    Interpreter::new(&m, 10000)
        .unwrap()
        .invoke(id, &[])
        .unwrap()
        .unwrap()
        .bits
}

#[test]
fn arrays_reborrow_across_helpers_and_preserve_caller_storage() {
    let source="fn clear(data: &mut [u8; 4]) { var i:u16=0; while i<data.len() { data[i]=0; i=i+1; } }
        fn put(data: &mut [u8;4]) { clear(data); data[2]=7; }
        fn sum(data: &[u8;4]):u8 { return data[0]+data[1]+data[2]+data[3]; }
        fn run():u8 { var buffer:[u8;4]=[3;4]; put(&mut buffer); return sum(&buffer); } setup {} loop {}";
    assert_eq!(run(source), 7);
    for optimize in [false, true] {
        let m = compile(source, optimize);
        helium::backend::llvm::emit(&m, false).unwrap();
    }
}

#[test]
fn array_values_move_and_returns_are_independent_snapshots() {
    let source="fn change(data:[u8;3]):[u8;3] { data[1]=9; return data; }
        fn run():u8 { var first:[u8;3]=[1,2,3]; var second=change(first); first=[4;3]; second[0]=first[0]; return second[0]+second[1]+second[2]; } setup {} loop {}";
    assert_eq!(run(source), 16);
}

#[test]
fn moves_and_borrow_conflicts_are_source_diagnostics() {
    let prefix = "fn pair(a:&[u8;2],b:&mut [u8;2]) {} fn take(a:[u8;2]) {}";
    for body in [
        "take(buf); take(buf);",
        "pair(&buf,&mut buf);",
        "take(buf); buf[0]=1;",
        "while true { take(buf); }",
        "var r=&buf;",
    ] {
        let source = format!("{prefix} setup {{ var buf:[u8;2]=[0;2]; {body} }} loop {{}}");
        assert!(helium::analyze(&source).is_err(), "{source}");
    }
}

#[test]
fn global_aliases_through_helpers_conflict_even_inside_no_interrupt() {
    let source =
        "var buffer:[u8;2]=[0;2]; fn touch() { buffer[0]=1; } fn helper(x:&[u8;2]) { touch(); }
        setup { noInterrupt { helper(&buffer); } } loop {}";
    let errors = helium::analyze(source).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.message().contains("Conflicting access")),
        "{errors:#?}"
    );
}

#[test]
fn constant_bounds_and_reference_escapes_are_rejected() {
    for source in [
        "setup { var a:[u8;2]=[0;2]; a[2]=1; } loop {}",
        "setup { var a:[u8;2]=[0;2]; a[1+1]=1; } loop {}",
        "fn bad(a:&[u8;2]):&[u8;2] { return a; } setup {} loop {}",
        "fn bad(a:&[u8;2]) { a[0]=1; } setup {} loop {}",
        "setup { var a:[u8;2]=[1,2,3]; } loop {}",
        "setup { var a:[u8;2]; } loop {}",
        "setup { var a:[u8;2]=[256;2]; } loop {}",
    ] {
        assert!(helium::analyze(source).is_err(), "{source}");
    }
}

#[test]
fn dynamic_bounds_fail_before_out_of_range_memory_access() {
    let m = compile(
        "fn run(i:u16):u8 { var a:[u8;2]=[1,2]; return a[i]; } setup {} loop {}",
        false,
    );
    let id = m.functions.iter().find(|f| f.name == "run").unwrap().id;
    let mut interpreter = Interpreter::new(&m, 1000).unwrap();
    assert_eq!(
        interpreter.invoke(id, &[ir::Constant::new(ir::Type::U16, 2)]),
        Err(ir::interpret::Error::Panic(ir::PanicReason::Bounds))
    );
}

#[test]
fn contextual_elements_repeat_once_and_source_owner_control_flow() {
    assert_eq!(
        run("fn run():u16{var small:u8=3;var a:[u16;2]=[small,1000];return a[1];}setup{}loop{}"),
        1000
    );
    assert_eq!(run("var calls:u8=0;fn next():u8{calls=calls+1;return calls;}fn run():u8{var a:[u8;4]=[next();4];return a[0]+a[3]+calls;}setup{}loop{}"),3);
    assert_eq!(run("fn both(a:&[u8;2],b:&[u8;2]):u8{return a[0]+b[1];}fn run():u8{var a:[u8;2]=[3,4];return both(&a,&a);}setup{}loop{}"),7);
    assert_eq!(run("fn take(a:[u8;2]){} fn run():u8{var a:[u8;2]=[1;2];var n:u8=0;while n<2{take(a);a=[9;2];n=n+1;}return a[0];}setup{}loop{}"),9);
    assert_eq!(
        run("fn run():u8{var a:[u8;2]=[7,8];return a[65535+1];}setup{}loop{}"),
        7
    );
    let bad="fn take(a:[u8;2]){}fn bad(flag:bool){var a:[u8;2]=[0;2];if flag{take(a);}var x=a[0];}setup{}loop{}";
    assert!(helium::analyze(bad).is_err());
}

#[test]
fn globals_operand_reservations_and_mutable_to_shared_reborrows() {
    let source = "var storage:[u8;2]=[0;2]; fn read(data:&[u8;2]):u8 {return data[0];}
        fn update(data:&mut [u8;2]):u8 {data[0]=5; var n=read(data); data[1]=n; return data[1];}
        fn run():u8 {return update(&mut storage);} setup {} loop {}";
    assert_eq!(run(source), 5);
    assert_eq!(
        run("fn run():u8 {var a:[u8;2]=[3,4]; a[0]=a[1]; return a[0];} setup {} loop {}"),
        4
    );
    for body in [
        "use_both(&mut a, read(&a));",
        "a[0]=mutate(&mut a);",
        "a[index(&mut a)]=1;",
    ] {
        let source=format!("fn use_both(data:&mut [u8;2],n:u8){{}} fn read(data:&[u8;2]):u8{{return data[0];}} fn mutate(data:&mut [u8;2]):u8{{data[0]=1;return 0;}} fn index(data:&mut [u8;2]):u16{{return 0;}} setup{{var a:[u8;2]=[0;2];{body}}}loop{{}}");
        assert!(helium::analyze(&source).is_err(), "{body}");
    }
}

#[test]
fn shared_array_access_with_isr_requires_existing_irq_protection() {
    let base="var storage:[u8;2]=[0;2]; fn put(data:&mut [u8;2]) {data[0]=1;} timer t=Timer1{every:1ms,interrupt{storage[0]=2;}}";
    let bad = format!("{base} loop{{put(&mut storage);}}");
    assert!(helium::analyze(&bad).is_err());
    let good = format!("{base} loop{{noInterrupt{{put(&mut storage);}}}}");
    compile(&good, false);
}

#[test]
fn array_parser_errors_and_truncated_inputs_do_not_panic() {
    let source = include_str!("../examples/uart-array.he");
    for (i, _) in source.char_indices() {
        let _ = helium::analyze(&source[..i]);
    }
    for source in [
        "setup{var a:[u8;0]=[0;0];}loop{}",
        "setup{var a=[1;65536];}loop{}",
        "setup{var a:[[u8;2];2];}loop{}",
        "fn f(a:&mut str){}loop{}",
        "setup{var a=[];}loop{}",
    ] {
        assert!(helium::analyze(source).is_err(), "{source}");
    }
}

#[test]
fn verifier_rejects_array_shape_mutability_and_initialization_errors() {
    let raw = compile(
        "fn run():u8{var a:[u8;2]=[1,2];a[0]=3;return a[0];}setup{}loop{}",
        false,
    );
    let mut m = raw.clone();
    let array = m
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
        .find(|i| matches!(i.op, ir::Op::ArrayLiteral(_)))
        .unwrap();
    if let ir::Op::ArrayLiteral(v) = &mut array.op {
        v.pop();
    }
    assert!(ir::verify::verify(&m).is_err());
    let mut m = raw.clone();
    for i in m
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
    {
        if let ir::Op::AddressLocal(_, mutable) = &mut i.op {
            *mutable = false;
            if let Some((_, ir::Type::ArrayRef(_, _, mutable))) = &mut i.result {
                *mutable = false;
            }
        }
    }
    assert!(ir::verify::verify(&m).is_err());
    let mut m = raw;
    let function = m.functions.iter_mut().find(|f| f.name == "run").unwrap();
    for b in &mut function.blocks {
        b.instructions
            .retain(|i| !matches!(i.op, ir::Op::StoreLocal(..)));
    }
    assert!(ir::verify::verify(&m).is_err());
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain and AVR8js; no flashing"]
fn source_arrays_execute_on_avr_at_all_optimization_levels() {
    let source = r#"
        import extension std::uart::UartText;
        device debug=UART0{baud:38400,mode:TX,}
        def input=PortB.0 INPUT;
        var global:[u8;4]=[1,2,3,4];
        fn fill(data:&mut [u8;4]){var i:u16=0;while i<data.len(){data[i]=65;i=i+1;}}
        fn changed(data:[u8;4]):[u8;4]{data[1]=66;return data;}
        fn read(data:&[u8;4],index:u16):u8{return data[index];}
        fn report(device out:UartTx,label:&str,data:&[u8;4]){out.print(label);var i:u16=0;while i<data.len(){out.write_byte(data[i]);i=i+1;}}
        setup{
            var data:[u8;4]=[0;4];fill(&mut data);var moved=changed(data);
            moved[2]=67;moved[3]=68;global[0]=69;
            report(debug,"R:",&moved);debug.write_byte(global[0]);
            var flags:[bool;2]=[true,false];flags[1]=flags[0];
            var signed:[i16;2]=[-300,400];signed[0]=signed[1]-1;
            var wide:[u16;2]=[1000;2];var states:[PinState;2]=[LOW,HIGH];
            if flags[1] and signed[0]==399 and wide[1]==1000 and states[1]==HIGH {debug.write_byte(84);}else{debug.write_byte(70);}
            var index:u16=2;if input==HIGH{index=4;}
            debug.flush();debug.write_byte(read(&moved,index));debug.write_byte(33);debug.flush();
        }loop{}
    "#;
    run_avr_matrix(
        source,
        "arrays",
        "82,58,65,66,67,68,69,84,67,33",
        "82,58,65,66,67,68,69,84",
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
#[test]
fn slices_accept_different_lengths_and_reborrow_without_losing_storage() {
    let source = "fn sum(a:&[u8]):u16 {var i:u16=0;var total:u16=0;while i<a.len(){total=total+a[i];i=i+1;}return total;}
        fn fill(a:&mut [u8]){var i:u16=0;while i<a.len(){a[i]=7;i=i+1;}}
        fn twice(a:&mut [u8]):u16{fill(a);fill(&mut a);return sum(a)+sum(&a);}
        fn fixed(a:&mut [u8;3]):u16{return twice(a);}
        fn run():u16{var a:[u8;3]=[1;3];var b:[u8;5]=[2;5];return fixed(&mut a)+sum(&b);} setup{}loop{}";
    assert_eq!(run(source), 52);
    for opt in [false, true] {
        helium::backend::llvm::emit(&compile(source, opt), false).unwrap();
    }
}

#[test]
fn clone_arrays_is_independent_and_does_not_move_the_receiver() {
    let source = "var global:[u8;2]=[9,8];
        fn copy(a:[u8;2]):[u8;2]{return a.clone();}
        fn run():u8 {global=[9,8];var a:[u8;2]=[1,2];var b=a.clone();b[0]=4;var c=copy(a.clone());a[1]=3;
        var d=global.clone();d[0]=5;var scalar:u8=6;var other=scalar.clone();
        return a[0]+a[1]+b[0]+b[1]+c[0]+c[1]+d[0]+global[0]+other;}setup{}loop{}";
    assert_eq!(run(source), 33);
    for opt in [false, true] {
        helium::backend::llvm::emit(&compile(source, opt), false).unwrap();
    }
}

#[test]
fn slices_and_clone_reject_aliases_moves_and_escapes() {
    for source in [
        "fn bad(a:&[u8]){a[0]=1;}setup{}loop{}",
        "fn bad(a:&[u8]):&[u8]{return a;}setup{}loop{}",
        "fn bad(a:&mut [u8]){var b=a.clone();}setup{}loop{}",
        "fn bad(a:&mut [u8;2]){var b=a.clone();}setup{}loop{}",
        "fn bad(a:&[u8]){var b:&[u8]=a;}setup{}loop{}",
        "fn bad(a:&[u8]){a=a;}setup{}loop{}",
        "fn bad(a:&mut [u8]){}fn shared(a:&[u8]){bad(a);}setup{}loop{}",
        "fn pair(a:&mut [u8],b:&[u8]){}setup{var a:[u8;2]=[1;2];pair(&mut a,&a);}loop{}",
        "fn pair(a:&mut [u8],b:[u8;2]){}setup{var a:[u8;2]=[1;2];pair(&mut a,a.clone());}loop{}",
        "fn take(a:[u8;2]){}setup{var a:[u8;2]=[1;2];take(a);var b=a.clone();}loop{}",
        "setup{var a:[u8;2]=[1;2];var b=a.clone(1);}loop{}",
        "var a:[u8;2]=[1;2];fn touch(){a[0]=2;}fn inspect(b:&[u8]){touch();}setup{inspect(&a);}loop{}",
    ] { assert!(helium::analyze(source).is_err(),"{source}"); }
}

#[test]
fn slice_bounds_are_dynamic_and_checked() {
    let m=compile("fn read(a:&[u16],i:u16):u16{return a[i];}fn run():u16{var a:[u16;2]=[1000;2];return read(&a,2);}setup{}loop{}",false);
    let id = m.functions.iter().find(|f| f.name == "run").unwrap().id;
    assert!(matches!(
        Interpreter::new(&m, 1000).unwrap().invoke(id, &[]),
        Err(ir::interpret::Error::Panic(ir::PanicReason::Bounds))
    ));
}

#[test]
#[ignore = "requires LLVM AVR, AVR toolchain and AVR8js; no flashing"]
fn source_slices_and_clone_execute_on_avr_at_all_optimization_levels() {
    let source = r#"
        import extension std::uart::UartText;
        device debug=UART0{baud:38400,mode:TX,}
        def input=PortB.0 INPUT;
        var global:[u8;2]=[71,72];
        fn fill(a:&mut [u8]){var i:u16=0;while i<a.len(){a[i]=65;i=i+1;}}
        fn helper(a:&mut [u8;4]){fill(a);fill(&mut a);}
        fn copy(a:[u8;4]):[u8;4]{return a.clone();}
        fn read(a:&[u8],i:u16):u8{return a[i];}
        fn sum(a:&[u8]):u16{var i:u16=0;var s:u16=0;while i<a.len(){s=s+a[i];i=i+1;}return s;}
        fn check(a:&[u16],b:&[i16],c:&[bool],d:&[PinState]):bool{return a[1]==1000 and b[0]==-300 and c[1] and d[0]==HIGH;}
        fn report(device out:UartTx,label:&str,a:&[u8]){out.print(label);var i:u16=0;while i<a.len(){out.write_byte(a[i]);i=i+1;}}
        setup{
            var a:[u8;4]=[0;4];helper(&mut a);var b=copy(a.clone());
            b[1]=66;b[2]=67;b[3]=68;a[0]=90;
            report(debug,"R:",&b);
            var small:[u8;2]=[69,70];report(debug,"",&small);
            var g=global.clone();g[0]=73;report(debug,"",&global);report(debug,"",&g);
            var wide:[u16;2]=[1000;2];var signed:[i16;2]=[-300;2];
            var flags:[bool;2]=[true;2];var states:[PinState;2]=[HIGH;2];
            var flags_copy=flags.clone();flags_copy[0]=false;
            if check(&wide,&signed,&flags,&states) and flags[0] and sum(&b)==266 {debug.write_byte(84);}else{debug.write_byte(70);}
            var index:u16=2;if input==HIGH{index=4;}
            debug.flush();debug.write_byte(read(&b,index));debug.write_byte(33);debug.flush();
        }loop{}
    "#;
    run_avr_matrix(
        source,
        "slices-clone",
        "82,58,65,66,67,68,69,70,71,72,73,72,84,67,33",
        "82,58,65,66,67,68,69,70,71,72,73,72,84",
    );
}
#[test]
fn verifier_rejects_slice_upgrades_and_invalid_lengths() {
    let raw=compile("fn read(a:&[u8]):u16{return a.len();}fn run():u16{var a:[u8;2]=[1;2];return read(&a);}setup{}loop{}",false);
    let mut m = raw.clone();
    for i in m
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
    {
        if let ir::Op::Slice(_, mutable) = &mut i.op {
            *mutable = true;
        }
    }
    assert!(ir::verify::verify(&m).is_err());
    let mut m = raw;
    for i in m
        .functions
        .iter_mut()
        .flat_map(|f| &mut f.blocks)
        .flat_map(|b| &mut b.instructions)
    {
        if matches!(i.op, ir::Op::SliceLen(_)) {
            i.result.as_mut().unwrap().1 = ir::Type::U8;
        }
    }
    assert!(ir::verify::verify(&m).is_err());
}

#[test]
fn slice_and_clone_accesses_preserve_irq_rules() {
    let prefix="var a:[u8;2]=[0;2];fn clear(b:&mut [u8]){b[0]=0;}timer tick=Timer1{every:1ms,interrupt{a[0]=1;}}";
    for access in ["clear(&mut a);", "var b=a.clone();"] {
        assert!(helium::analyze(&format!("{prefix}setup{{}}loop{{{access}}}")).is_err());
        compile(
            &format!("{prefix}setup{{}}loop{{noInterrupt{{{access}}}}}"),
            false,
        );
    }
}

#[test]
fn sequence_and_text_results_widen_after_native_operations() {
    assert_eq!(run("fn length(a:&[u8]):u32{return a.len();}fn run():u32{var a:[u8;2]=[1;2];var s=\"abc\";return length(&a)+a.len()+a[0]+s.len()+s.byte_or(0,0);}setup{}loop{}"),105);
}
