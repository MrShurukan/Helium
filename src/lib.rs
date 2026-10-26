//! Helium compilation stages. None of these functions starts an external process.
pub mod backend;
pub mod diagnostic;
pub mod frontend;
pub mod ir;
pub mod middle_end;
mod support;
pub mod timer;

use diagnostic::Diagnostic;
use frontend::{statement::Statement, Parser};

/// Parse and type-check the language, including constructs not yet lowered to AVR.
pub fn analyze(source: &str) -> Result<Vec<Statement>, Vec<Diagnostic>> {
    let mut statements = Parser::new(source).parse().map_err(|errors| {
        use unicode_segmentation::UnicodeSegmentation;
        let line = source.rsplit('\n').next().unwrap_or("");
        let eof = frontend::utils::SourceLocation {
            line_start: source.split('\n').count(),
            line_end: source.split('\n').count(),
            start_offset: line.graphemes(true).count(),
            end_offset: line.graphemes(true).count(),
        };
        let mut diagnostics: Vec<Diagnostic> = errors.into_iter().map(Diagnostic::from).collect();
        for error in &mut diagnostics {
            if error.location.is_none() {
                error.location = Some(eof);
            }
        }
        diagnostics.dedup_by(|a, b| a.location == b.location && a.message() == b.message());
        diagnostics
    })?;

    middle_end::Analyzer::new(source).analyze(&mut statements)?;

    Ok(statements)
}

/// Validate the executable language subset without starting external tools.
pub fn check(source: &str, mcu: &str) -> Result<(), Vec<Diagnostic>> {
    compile_llvm(source, mcu, ir::Options::default()).map(|_| ())
}

pub fn compile_llvm(
    source: &str,
    mcu: &str,
    options: ir::Options,
) -> Result<String, Vec<Diagnostic>> {
    let statements = analyze(source)?;
    let module = typed_ir_from_ast(&statements, mcu, options, true)?;
    backend::llvm::emit(&module, false).map_err(|detail| {
        vec![Diagnostic::new(
            diagnostic::DiagnosticKind::InvalidIr {
                stage: "LLVM emission",
                detail,
            },
            None,
        )]
    })
}

pub fn compile_typed_ir(
    source: &str,
    mcu: &str,
    options: ir::Options,
) -> Result<ir::Module, Vec<Diagnostic>> {
    let statements = analyze(source)?;
    typed_ir_from_ast(&statements, mcu, options, false)
}

fn typed_ir_from_ast(
    statements: &[Statement],
    mcu: &str,
    options: ir::Options,
    machine: bool,
) -> Result<ir::Module, Vec<Diagnostic>> {
    support::validate_pins(statements, mcu)?;
    support::validate_features(statements, machine)?;
    let timer = timer::resolve(statements, options.cpu_hz)?;
    let mut module = ir::lower(statements, mcu, timer).map_err(|error| vec![error])?;
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
