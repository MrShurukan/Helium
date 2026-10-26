use helium::frontend::expression::{Expression, ExpressionType as E};
use helium::middle_end::shared::{AccessKind, Protection, ResourceId};
use helium::{
    analyze, compile_typed_ir,
    ir::{self, Options},
    semantic::{BodyId, SymbolKind},
};

#[test]
fn sibling_names_have_distinct_identities_and_correct_storage() {
    let source = "var result:u8=0;
        fn first(x:u8):u8 {return x+1;} fn second(x:u8):u8 {return x+2;}
        setup {{var x:u8=3; result=first(x);} {var x:u8=5; result=result+second(x);}}
        loop {}";
    let program = analyze(source).unwrap();
    let xs = program
        .symbols()
        .iter()
        .filter(|(_, s)| s.name == "x")
        .collect::<Vec<_>>();
    assert_eq!(xs.len(), 4);
    for (i, (id, _)) in xs.iter().enumerate() {
        for (other, _) in &xs[i + 1..] {
            assert_ne!(id, other);
        }
    }
    assert_eq!(
        xs.iter()
            .filter(|(_, s)| s.kind == SymbolKind::Parameter)
            .count(),
        2
    );
    for optimize in [false, true] {
        let module = compile_typed_ir(
            source,
            "atmega328p",
            Options {
                optimize,
                ..Options::default()
            },
        )
        .unwrap();
        let mut runner = ir::interpret::Interpreter::new(&module, 200).unwrap();
        assert_eq!(runner.run_entry(), Err(ir::interpret::Error::StepLimit));
        assert_eq!(runner.globals[0].bits, 11);
    }
}

#[test]
fn all_name_uses_are_resolved_and_ids_are_deterministic() {
    let source = "def Led=PortB.0 OUTPUT; var count:u8=0;
        fn take(x:u8):u8 {return x;}
        fn early():u8 {return later(count);}
        fn later(x:u8):u8 {var local:u8=x; return local;}
        setup {count=early();}
        loop {{var local:u8=take(count); Led=!Led;} for i=0 to 2 {var copy=i;}}
        timer clock=Timer1 {every:1s, interrupt {var private:u8=1; private=take(private);}}";
    let program = analyze(source).unwrap();
    fn inspect(e: &Expression, program: &helium::AnalyzedProgram) {
        if let E::Identifier(name)
        | E::Call {
            identifier: name, ..
        }
        | E::Assignment {
            identifier: name, ..
        } = &e.expression_type
        {
            let symbol = program.symbols().get(e.symbol.expect("resolved use"));
            assert_eq!(&symbol.name, name);
            if matches!(e.expression_type, E::Call { .. }) {
                assert!(matches!(symbol.kind, SymbolKind::Function { .. }));
            }
        }
        e.for_each_child(|child| inspect(child, program));
    }
    for s in program.statements() {
        s.for_each_expression(&mut |e| inspect(e, &program));
    }
    let again = analyze(source).unwrap();
    let identities = |p: &helium::AnalyzedProgram| {
        p.symbols()
            .iter()
            .map(|(id, s)| (id, s.name.clone(), s.owner))
            .collect::<Vec<_>>()
    };
    assert_eq!(identities(&program), identities(&again));
    assert!(program
        .symbols()
        .iter()
        .any(|(_, s)| s.name == "private" && matches!(s.owner, Some(BodyId::Interrupt(_)))));
}

#[test]
fn shared_graph_uses_resource_ids_and_preserves_protection_and_calls() {
    let source = "def A=PortB.0 OUTPUT; def B=PortB.1 INPUT; var counter:u16=0;
        noInterrupt fn tick() {counter=counter+1; A=HIGH;}
        timer clock=Timer1 {every:1s, interrupt {tick();}}
        setup {B=INPUT_PULLUP;}
        loop {noInterrupt {var now=counter; var level=!B;}}";
    let program = analyze(source).unwrap();
    let graph = program.shared_analysis();
    let counter = program
        .symbols()
        .iter()
        .find(|(_, s)| s.name == "counter")
        .unwrap()
        .0;
    assert!(graph.interrupt_globals().contains(&counter));
    let pins = program
        .symbols()
        .iter()
        .filter_map(|(_, s)| match s.kind {
            SymbolKind::Pin { port, .. } => Some(port),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(pins[0], pins[1]);
    assert_eq!(program.symbols().port_name(pins[0]), "B");
    let setup = &graph.bodies()[&BodyId::Setup];
    assert_eq!(setup.accesses[0].kind, AccessKind::ConfigurePin);
    assert_eq!(setup.accesses[0].protection, Protection::Setup);
    let main = &graph.bodies()[&BodyId::MainLoop];
    assert!(main
        .accesses
        .iter()
        .all(|a| a.protection == Protection::Block));
    assert!(main
        .accesses
        .iter()
        .any(|a| a.resource == ResourceId::GpioPort(pins[0]) && a.kind == AccessKind::ReadPin));
    let isr = graph
        .bodies()
        .iter()
        .find(|(id, _)| matches!(id, BodyId::Interrupt(_)))
        .unwrap()
        .1;
    assert_eq!(isr.calls[0].protection, Protection::Interrupt);
    let callee = &graph.bodies()[&BodyId::Function(isr.calls[0].callee)];
    assert!(callee
        .accesses
        .iter()
        .all(|a| a.protection == Protection::FunctionContract));
    let module = compile_typed_ir(
        source,
        "atmega328p",
        Options {
            cpu_hz: Some(16_000_000),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(
        module.interrupt_globals.len(),
        graph.interrupt_globals().len()
    );
}
