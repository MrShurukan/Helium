//! Helium compilation stages. None of these functions starts an external process.
pub mod backend;
pub mod diagnostic;
pub mod effects;
pub mod frontend;
pub mod intrinsics;
pub mod ir;
pub mod middle_end;
pub mod modules;
pub mod resources;
pub mod semantic;
mod support;
pub use semantic::AnalyzedProgram;
pub mod timer;
pub mod uart;
pub mod values;

use backend::microcontroller::{self, Microcontroller};
use diagnostic::Diagnostic;

#[derive(Debug)]
pub struct Compilation {
    pub output: String,
    pub warnings: Vec<Diagnostic>,
}
impl std::ops::Deref for Compilation {
    type Target = str;
    fn deref(&self) -> &str {
        &self.output
    }
}

fn target_named(mcu: &str) -> Result<&'static Microcontroller, Vec<Diagnostic>> {
    microcontroller::get(mcu).ok_or_else(|| {
        vec![Diagnostic::new(
            diagnostic::DiagnosticKind::UnsupportedTarget { mcu: mcu.into() },
            None,
        )]
    })
}

/// Parse and type-check the language, including constructs not yet lowered to AVR.
pub fn analyze(source: &str) -> Result<AnalyzedProgram, Vec<Diagnostic>> {
    modules::Project::memory("<input>", source).analyze()
}

/// Validate the executable language subset without starting external tools.
pub fn check(source: &str, mcu: &str) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    check_with_options(source, mcu, ir::Options::default())
}

/// Configurable check, including the CPU clock required by timer programs.
/// Uses the same compilation path and options as CLI check and compile_llvm.
pub fn check_with_options(
    source: &str,
    mcu: &str,
    options: ir::Options,
) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    compile_llvm(source, mcu, options).map(|result| result.warnings)
}

pub fn compile_llvm(
    source: &str,
    mcu: &str,
    options: ir::Options,
) -> Result<Compilation, Vec<Diagnostic>> {
    compile_llvm_with_target(source, target_named(mcu)?, options)
}

/// Compile with the already selected descriptor; no stage selects another MCU.
pub fn compile_llvm_with_target(
    source: &str,
    target: &'static Microcontroller,
    options: ir::Options,
) -> Result<Compilation, Vec<Diagnostic>> {
    let statements = analyze(source)?;
    let module = project_ir(&statements, target, options, true)?;
    backend::llvm::emit(&module, false)
        .map(|output| Compilation {
            output,
            warnings: module.warnings.clone(),
        })
        .map_err(|detail| {
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
    compile_typed_ir_with_target(source, target_named(mcu)?, options)
}

pub fn compile_typed_ir_with_target(
    source: &str,
    target: &'static Microcontroller,
    options: ir::Options,
) -> Result<ir::Module, Vec<Diagnostic>> {
    let statements = analyze(source)?;
    project_ir(&statements, target, options, false)
}

fn typed_ir_from_ast(
    program: &AnalyzedProgram,
    target: &'static Microcontroller,
    options: ir::Options,
    machine: bool,
) -> Result<ir::Module, Vec<Diagnostic>> {
    if let Some(cpu_hz) = options.cpu_hz {
        timer::validate_clock(target, cpu_hz).map_err(|error| {
            vec![Diagnostic::new(
                diagnostic::DiagnosticKind::Timer { error },
                None,
            )]
        })?;
    }
    let statements = program.statements();
    let mut reservations = resources::Reservations::default();
    support::validate_pins(statements, target, &mut reservations)?;
    support::validate_features(statements, machine)?;
    support::validate_features(&program.template_checks, machine)?;
    let timer = timer::resolve(target, statements, options.cpu_hz, &mut reservations)?;
    let uarts = uart::resolve(target, statements, options.cpu_hz, &mut reservations)?;
    let mut module = ir::lower(program, target, timer, uarts).map_err(|error| vec![error])?;
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

/// Compile an explicit project/module graph with file-aware diagnostics.
pub fn compile_project_llvm(
    project: &mut modules::Project,
    target: &'static Microcontroller,
    options: ir::Options,
) -> Result<Compilation, Vec<Diagnostic>> {
    compile_project_artifacts(project, target, options).map(|artifacts| artifacts.llvm)
}

/// Textual artifacts from one analysis and lowering, with machine-subset validation.
pub struct ProjectArtifacts {
    pub llvm: Compilation,
    /// Same pre-machine-preparation view as build --emit ir.
    pub ir: String,
}

pub fn compile_project_artifacts(
    project: &mut modules::Project,
    target: &'static Microcontroller,
    options: ir::Options,
) -> Result<ProjectArtifacts, Vec<Diagnostic>> {
    let program = project.analyze()?;
    let module = project_ir(&program, target, options, true)?;
    backend::llvm::emit(&module, false)
        .map(|output| ProjectArtifacts {
            ir: module.dump(),
            llvm: Compilation {
                output,
                warnings: module.warnings.clone(),
            },
        })
        .map_err(|detail| {
            vec![Diagnostic::new(
                diagnostic::DiagnosticKind::InvalidIr {
                    stage: "LLVM emission",
                    detail,
                },
                None,
            )]
        })
}
pub fn compile_project_ir(
    project: &mut modules::Project,
    target: &'static Microcontroller,
    options: ir::Options,
) -> Result<ir::Module, Vec<Diagnostic>> {
    let program = project.analyze()?;
    project_ir(&program, target, options, false)
}
fn project_ir(
    program: &AnalyzedProgram,
    target: &'static Microcontroller,
    options: ir::Options,
    machine: bool,
) -> Result<ir::Module, Vec<Diagnostic>> {
    typed_ir_from_ast(program, target, options, machine).map_err(|mut errors| {
        if program.sources.files.len() > 1 {
            for d in &mut errors {
                d.sources = Some(program.sources.clone());
            }
        }
        errors
    })
}
