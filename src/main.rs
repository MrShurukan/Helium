//! Welcome to Helium compiler! This language is an attempt to bring modern practices to
//! microcontrollers of different types (mainly AVR). Language is designed to be simple to approach
//! and safe at compile time, rather than at runtime. Things like I/O, Uart and Timers are recognized
//! at language level, which reduces needs for workarounds like in other languages, while also
//! reducing and simplifying std.
//!
//! This file is the entry point to helium, also covers the CLI usage. The orchestrator of the whole
//! compilation process.

mod cli;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::{env, fs};

use clap::Parser;

use helium::backend::microcontroller::Microcontroller;
use helium::backend::{flash as flash_backend, llvm_toolchain};
use helium::diagnostic::{Diagnostic, DiagnosticKind, RenderOptions};
use helium::{compile_project_ir, compile_project_llvm, ir};

use crate::cli::Command;

fn main() -> ExitCode {
    match run(cli::CliArgs::parse()) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

static COLOR_MODE: OnceLock<cli::ColorMode> = OnceLock::new();

fn is_color() -> bool {
    match COLOR_MODE.get_or_init(|| cli::ColorMode::Auto) {
        cli::ColorMode::Auto => {
            std::io::stderr().is_terminal() && env::var_os("NO_COLOR").is_none()
        }
        cli::ColorMode::Always => true,
        cli::ColorMode::Never => false,
    }
}

fn run(args: cli::CliArgs) -> Result<String, String> {
    COLOR_MODE
        .set(args.color)
        .expect("value was never set before");

    match args.command {
        Command::Check(args) => check(args),
        Command::Build(args) => build(args),
        Command::Flash(args) => flash(args),
        Command::TimerOptions(args) => timer_options(args),
    }
}

/// Verifies target mcu and cpu-hz for that target (if specified). Returns the selected target
fn verify_common_args(args: &cli::CommonArgs) -> Result<&'static Microcontroller, String> {
    let Some(target) = helium::backend::microcontroller::get(&args.mcu) else {
        return Err(
            Diagnostic::new(
                DiagnosticKind::UnsupportedTarget {
                    mcu: args.mcu.clone(),
                },
                None,
            )
                .render_with(
                    "",
                    RenderOptions {
                        color: is_color(),
                    },
                ),
        );
    };

    if let Some(cpu_hz) = args.cpu_hz {
        helium::timer::validate_clock(target, cpu_hz).map_err(|e| e.to_string())?;
    }

    Ok(target)
}

/// Performs a lowering to llvm to check if any language level problems arise.
/// Useful to check if the code compiles and has warnings or errors.
///
/// Sticks to default IR options except passed in --cpu-hz
fn check(args: cli::CheckArgs) -> Result<String, String> {
    let input = args.source.path().to_path_buf();

    let source = fs::read_to_string(&input)
        .map_err(|e| format!("Cannot read {}: {e}", input.display()))?;

    let mut project =
        helium::modules::Project::from_file(&input).map_err(|e| e.to_string())?;

    // TODO: Get this from project somehow. I don't like how there are separate source sets
    let source_name = input.display().to_string();
    let source_file = helium::diagnostic::source::SourceFile {
        id: helium::diagnostic::source::FileId(0),
        name: &source_name,
        text: &source,
    };

    let target = verify_common_args(&args.common)?;

    let options = ir::Options {
        cpu_hz: args.common.cpu_hz,
        ..Default::default()
    };

    let llvm = compile_project_llvm(&mut project, target, options)
        .map_err(|errs| source_file.render_diagnostics(errs, is_color()))?;

    if !llvm.warnings.is_empty() {
        // TODO: Remove this clone. Compilation should just give the warnings instead of storing
        eprintln!(
            "{}",
            source_file.render_diagnostics(llvm.warnings.clone(), is_color())
        );
    }

    Ok(format!(
        "Checked {}{}",
        input.display(),
        device_report(&llvm)
    ))
}

fn build(args: cli::BuildArgs) -> Result<String, String> {
    // TODO: This block is identical in check function and this has to get unified
    let input = args.source.path().to_path_buf();

    let source = fs::read_to_string(&input)
        .map_err(|e| format!("Cannot read {}: {e}", input.display()))?;

    let mut project =
        helium::modules::Project::from_file(&input).map_err(|e| e.to_string())?;

    // TODO: Get this from project somehow. I don't like how there are separate source sets
    let source_name = input.display().to_string();
    let source_file = helium::diagnostic::source::SourceFile {
        id: helium::diagnostic::source::FileId(0),
        name: &source_name,
        text: &source,
    };

    let target = verify_common_args(&args.common)?;

    // Check and validate options make sense together

    if args.optimize_ir && args.emit != cli::Emit::Ir {
        return Err("--optimize-ir requires build --emit ir".into());
    }

    if args.opt.is_some() && !is_machine_output(args.emit) {
        return Err("--opt applies only to machine output (asm, elf, hex)".into());
    }

    if args.intermediates.is_some() && !is_machine_output(args.emit) {
        return Err("--intermediates requires machine output (asm, elf, hex)".into());
    }

    let options = ir::Options {
        cpu_hz: args.common.cpu_hz,
        optimize: args.optimize_ir,
        ..Default::default()
    };

    let optimization= args.opt.unwrap_or(cli::Optimization::None).into();

    /*
     * AST and internal IR don't require LLVM output.
     *
     * LLVM / ASM / ELF / HEX all start from the same project artifact compilation.
     * Machine output then continues through the external LLVM / AVR toolchain.
     */

    let mut internal_ir = None;

    let content = match args.emit {
        cli::Emit::Ast => {
            let program = project
                .analyze()
                .map_err(|errs| source_file.render_diagnostics(errs, is_color()))?;

            // TODO: Inconsistencies in warnings. This is a slice, while module has a vec
            if !program.warnings().is_empty() {
                eprintln!(
                    "{}",
                    source_file.render_diagnostics(
                        program.warnings().to_vec(),
                        is_color()
                    )
                );
            }

            format!("{program:#?}\n")
        }

        cli::Emit::Ir => {
            let module = compile_project_ir(&mut project, target, options)
                .map_err(|errs| source_file.render_diagnostics(errs, is_color()))?;

            if !module.warnings.is_empty() {
                // TODO: Remove this clone. Compilation should just give the warnings instead of storing
                eprintln!(
                    "{}",
                    source_file.render_diagnostics(
                        module.warnings.clone(),
                        is_color()
                    )
                );
            }

            module.dump()
        }

        cli::Emit::Llvm
        | cli::Emit::Asm
        | cli::Emit::Elf
        | cli::Emit::Hex => {
            let artifacts =
                helium::compile_project_artifacts(&mut project, target, options)
                    .map_err(|errs| {
                        source_file.render_diagnostics(errs, is_color())
                    })?;

            internal_ir = Some(artifacts.ir);

            if !artifacts.llvm.warnings.is_empty() {
                // TODO: Remove this clone. Compilation should just give the warnings instead of storing
                eprintln!(
                    "{}",
                    source_file.render_diagnostics(
                        artifacts.llvm.warnings.clone(),
                        is_color()
                    )
                );
            }

            artifacts.llvm.output
        }
    };

    let report = device_report(&content);

    let destination = args
        .output
        .as_ref()
        .map(|output| output.path().path().to_path_buf())
        .unwrap_or_else(|| default_output_path(&input, args.emit));

    let protected_paths: Vec<_> = project
        .sources
        .files
        .iter()
        .map(|file| PathBuf::from(&file.name))
        .collect();

    helium::backend::output::check(&destination, &protected_paths)?;

    /*
     * ASM / ELF / HEX require the external toolchain.
     *
     * AST / IR / LLVM are already final textual outputs and can simply be
     * written to the selected destination.
     */
    if is_machine_output(args.emit) {
        let retention = match &args.intermediates {
            None => llvm_toolchain::Intermediates::Discard,
            Some(None) => llvm_toolchain::Intermediates::Generated,
            Some(Some(path)) => {
                llvm_toolchain::Intermediates::Directory(path)
            }
        };

        let result = llvm_toolchain::build_with_options(
            &content,
            target,
            &destination,
            emit_name(args.emit),
            optimization,
            llvm_toolchain::BuildOptions {
                intermediates: retention,
                internal_ir: internal_ir.as_deref(),
                protected_paths: &protected_paths,
            },
        )?;

        let retained = result
            .retained_directory
            .as_ref()
            .map(|path| format!("\nIntermediates: {}", path.display()))
            .unwrap_or_default();

        let memory = result.memory.map_or_else(
            || {
                "Memory: unavailable for assembly output; \
                 link ELF/HEX for a final report\n"
                    .into()
            },
            |memory| memory.render(),
        );

        return Ok(format!(
            "Built {} (external stages: {} ms){}\n{}{}",
            destination.display(),
            result.elapsed_ms,
            retained,
            memory,
            report,
        ));
    }

    helium::backend::output::write(
        &destination,
        content.as_bytes(),
        &protected_paths,
    )?;

    Ok(format!(
        "Built {}{}",
        destination.display(),
        report
    ))
}

fn flash(args: cli::FlashArgs) -> Result<String, String> {
    let input = args.file.path().to_path_buf();

    let _target = verify_common_args(&args.common)?;

    flash_backend::flash(
        &input,
        &args.common.mcu,
        &args.programmer,
    )?;

    Ok(format!("Flashed {}", input.display()))
}

fn emit_name(emit: cli::Emit) -> &'static str {
    match emit {
        cli::Emit::Ast => "ast",
        cli::Emit::Ir => "ir",
        cli::Emit::Llvm => "llvm",
        cli::Emit::Asm => "asm",
        cli::Emit::Elf => "elf",
        cli::Emit::Hex => "hex",
    }
}

fn emit_extension(emit: cli::Emit) -> &'static str {
    match emit {
        cli::Emit::Llvm => "ll",
        other => emit_name(other),
    }
}

fn is_machine_output(emit: cli::Emit) -> bool {
    matches!(
        emit,
        cli::Emit::Asm | cli::Emit::Elf | cli::Emit::Hex
    )
}

fn default_output_path(input: &Path, emit: cli::Emit) -> PathBuf {
    input
        .parent()
        .unwrap_or(Path::new("."))
        .join("build")
        .join(format!(
            "{}.{}",
            input
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy(),
            emit_extension(emit),
        ))
}

// TODO: Yuck. Manual filters? This has to be done from structs
fn device_report(content: &str) -> String {
    content
        .lines()
        .filter_map(|line| line.strip_prefix("; "))
        .filter(|line| {
            line.starts_with("Timer1:")
                || line.starts_with("UART0:")
        })
        .map(|line| format!("\n{line}"))
        .collect()
}

fn timer_options(args: cli::TimerOptionsArgs) -> Result<String, String> {
    let target = verify_common_args(&args.common)?;

    if args.timer != target.timer.name {
        return Err(format!(
            "timer-options currently supports {} {} only",
            target.name,
            target.timer.name,
        ));
    }

    let hz = args
        .common
        .cpu_hz
        .expect("--cpu-hz is required for timer-options");

    let period = helium::timer::parse_duration(&args.every)?;

    let candidates =
        helium::timer::candidates(target, hz, period)
            .map_err(|error| error.to_string())?;

    Ok(format!(
        "Requested: {period}ns; candidates ordered by absolute error, then prescaler.\n{}\n\
         Calculated periods assume the stated CPU clock; oscillator error and ISR latency are not included.",
        candidates
            .iter()
            .map(helium::timer::TimerConfig::report)
            .collect::<Vec<_>>()
            .join("\n")
    ))
}