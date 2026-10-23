//! Helium compilation stages. None of these functions starts an external process.
pub mod backend;
pub mod diagnostic;
pub mod frontend;
pub mod ir;
pub mod middle_end;
mod support;

use diagnostic::Diagnostic;
use frontend::{
    statement::{SingleDefDeclaration, Statement, StatementType},
    Parser,
};
use middle_end::linear_ir::{IRBuilder, IntermediateRepresentation};

pub(crate) struct Program {
    def_declarations: Vec<SingleDefDeclaration>,
    global_var_declarations: Vec<Statement>,
    setup_block: Vec<Statement>,
    loop_block: Vec<Statement>,
}

/// Parse and type-check the language, including constructs not yet lowered to AVR.
pub fn analyze(source: &str) -> Result<Vec<Statement>, Vec<Diagnostic>> {
    let mut statements = Parser::new(source)
        .parse()
        .map_err(|errors| errors.into_iter().map(Diagnostic::from).collect::<Vec<_>>())?;

    middle_end::Analyzer::new(source).analyze(&mut statements)?;

    Ok(statements)
}

/// Check the exact subset supported by the selected backend, without invoking AVRA.
pub fn check(source: &str, mcu: &str) -> Result<(), Vec<Diagnostic>> {
    compile_assembly(source, mcu).map(|_| ())
}

/// Emit the legacy, untyped IR for inspection. It is not a portable executable IR.
pub fn compile_ir(
    source: &str,
    mcu: &str,
) -> Result<Vec<IntermediateRepresentation>, Vec<Diagnostic>> {
    let statements = analyze(source)?;
    support::validate_pins(&statements, mcu)?;

    IRBuilder::new(source)
        .produce_ir(structure(statements))
        .map_err(|e| vec![e])
}

pub fn compile_assembly(source: &str, mcu: &str) -> Result<String, Vec<Diagnostic>> {
    compile_assembly_with_options(source, mcu, ir::Options::default())
}

/// Typed IR inspection/execution is independent of the narrower legacy AVR backend.
pub fn compile_typed_ir(
    source: &str,
    mcu: &str,
    options: ir::Options,
) -> Result<ir::Module, Vec<Diagnostic>> {
    let statements = analyze(source)?;
    typed_ir_from_ast(&statements, mcu, options)
}

fn typed_ir_from_ast(
    statements: &[Statement],
    mcu: &str,
    options: ir::Options,
) -> Result<ir::Module, Vec<Diagnostic>> {
    support::validate_pins(statements, mcu)?;
    let mut module = ir::lower(statements, mcu).map_err(|error| vec![error])?;
    let internal_error = |stage, error: ir::verify::Error| {
        vec![Diagnostic::new(
            diagnostic::DiagnosticKind::InvalidIr {
                stage,
                detail: error.to_string(),
            },
            None,
        )]
    };

    if options.verify {
        ir::verify::verify(&module).map_err(|error| internal_error("construction", error))?;
    }
    if options.optimize {
        ir::optimize::optimize(&mut module, options.verify)
            .map_err(|error| internal_error("optimization", error))?;
    }
    Ok(module)
}

pub fn compile_assembly_with_options(
    source: &str,
    mcu: &str,
    options: ir::Options,
) -> Result<String, Vec<Diagnostic>> {
    if options.optimize {
        return Err(vec![Diagnostic::new(
            diagnostic::DiagnosticKind::TypedIrUnsupported {
                feature: "optimizing the legacy AVR backend; use typed IR output".into(),
            },
            None,
        )]);
    }

    let statements = analyze(source)?;
    support::validate(&statements, mcu)?;

    // During the transition, verify typed lowering alongside the legacy generator
    // only when requested. Release AVR builds do not construct a throwaway IR.
    if options.verify {
        typed_ir_from_ast(&statements, mcu, options)?;
    }

    let ir = IRBuilder::new(source)
        .produce_ir(structure(statements))
        .map_err(|e| vec![e])?;

    backend::produce(&ir).map_err(|e| vec![e])
}

fn structure(statements: Vec<Statement>) -> Program {
    let mut program = Program {
        def_declarations: vec![],
        global_var_declarations: vec![],
        setup_block: vec![],
        loop_block: vec![],
    };

    for statement in statements {
        match statement.statement_type {
            StatementType::DefDeclaration(mut declarations) => {
                program.def_declarations.append(&mut declarations)
            }
            StatementType::VariableDeclaration { .. } => {
                program.global_var_declarations.push(statement)
            }
            StatementType::SetupDeclaration(body) => program.setup_block = body,
            StatementType::LoopDeclaration(body) => program.loop_block = body,
            _ => unreachable!("parser only produces top-level declarations"),
        }
    }

    program
}
