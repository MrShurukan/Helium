use helium::diagnostic::{Diagnostic, DiagnosticKind, RenderOptions};
use helium::{
    backend::{flash, llvm_toolchain},
    compile_project_ir, compile_project_llvm, ir,
};
use std::io::IsTerminal;
use std::{env, fs, path::PathBuf, process::ExitCode};

const USAGE: &str = concat!(
    "Usage:\n",
    "  helium check <source.he> --mcu atmega328p\n",
    "  helium build <source.he> --mcu atmega328p [--emit hex|elf|asm|llvm|ir|ast] [-o <file>]\n",
    "  helium flash <file.hex> --mcu atmega328p --programmer <name>\n",
    "  helium timer-options --mcu atmega328p --cpu-hz <Hz> --timer Timer1 --every <duration>\n",
    "  --cpu-hz <Hz> (check/build; required for timers, does not change oscillator settings)\n",
    "Options: --color auto|always|never (default: auto; NO_COLOR disables automatic color)\n",
    "  --verify-ir (check/build; enabled by default in debug and release)\n",
    "  --opt none|speed|size (machine output only; default: none)\n",
    "  --intermediates [directory] (machine build only; retain IR, LLVM and toolchain artifacts)\n",
    "  --optimize-ir (build --emit ir only; constant folding, CFG cleanup and dead values)"
);

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
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

fn run(args: Vec<String>) -> Result<String, String> {
    if args.first().is_some_and(|arg| arg == "timer-options") {
        return timer_options(&args[1..]);
    }
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        return Ok(USAGE.into());
    }
    if args.len() < 2 {
        return Err(USAGE.into());
    }

    let command = args[0].as_str();
    if !matches!(command, "check" | "build" | "flash") {
        return Err(USAGE.into());
    }

    let input = PathBuf::from(&args[1]);
    let mut mcu = None;
    let mut emit = None;
    let mut output = None;
    let mut programmer = None;
    let mut color = None;
    let mut optimization = None;
    let mut cpu_hz = None;
    let mut ir_options = ir::Options::default();
    let mut explicit_verify = false;
    let mut intermediates: Option<Option<PathBuf>> = None;

    let mut options = args[2..].iter().peekable();
    while let Some(option) = options.next() {
        if (option == "--intermediates" || option.starts_with("--intermediates="))
            && command == "build"
        {
            if intermediates.is_some() {
                return Err("Repeated option --intermediates".into());
            }
            let directory = if let Some((_, path)) = option.split_once('=') {
                if path.is_empty() {
                    return Err("Intermediates directory must not be empty".into());
                }
                Some(PathBuf::from(path))
            } else if options.peek().is_some_and(|next| !next.starts_with('-')) {
                Some(PathBuf::from(options.next().unwrap()))
            } else {
                None
            };
            intermediates = Some(directory);
            continue;
        }
        if option == "--verify-ir" && command != "flash" {
            if explicit_verify {
                return Err("Repeated option --verify-ir".into());
            }
            explicit_verify = true;
            ir_options.verify = true;
            continue;
        }
        if option == "--optimize-ir" && command == "build" {
            if ir_options.optimize {
                return Err("Repeated option --optimize-ir".into());
            }
            ir_options.optimize = true;
            continue;
        }
        let value = options
            .next()
            .ok_or_else(|| format!("Missing value for {option}\n{USAGE}"))?;
        let slot = match option.as_str() {
            "--mcu" => &mut mcu,
            "--emit" if command == "build" => &mut emit,
            "-o" | "--output" if command == "build" => &mut output,
            "--programmer" if command == "flash" => &mut programmer,
            "--color" => &mut color,
            "--opt" if command == "build" => &mut optimization,
            "--cpu-hz" if command != "flash" => &mut cpu_hz,
            _ => return Err(format!("Unknown or inapplicable option {option}\n{USAGE}")),
        };

        if slot.replace(value.clone()).is_some() {
            return Err(format!("Repeated option {option}"));
        }
    }

    let color = match color.as_deref().unwrap_or("auto") {
        "auto" => std::io::stderr().is_terminal() && env::var_os("NO_COLOR").is_none(),
        "always" => true,
        "never" => false,
        value => {
            return Err(format!(
                "Unknown color mode '{value}'; expected auto, always or never"
            ))
        }
    };

    let mcu = mcu.ok_or("--mcu is required")?.to_lowercase();
    ir_options.cpu_hz = cpu_hz
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| "--cpu-hz must be a positive integer".to_owned())
        })
        .transpose()?;
    let Some(target) = helium::backend::microcontroller::get(&mcu) else {
        return Err(
            Diagnostic::new(DiagnosticKind::UnsupportedTarget { mcu }, None)
                .render_with("", RenderOptions { color }),
        );
    };
    if let Some(cpu_hz) = ir_options.cpu_hz {
        helium::timer::validate_clock(target, cpu_hz).map_err(|e| e.to_string())?;
    }

    if command == "flash" {
        flash::flash(
            &input,
            &mcu,
            &programmer.ok_or("--programmer is required for flash")?,
        )?;
        return Ok(format!("Flashed {}", input.display()));
    }

    let source =
        fs::read_to_string(&input).map_err(|e| format!("Cannot read {}: {e}", input.display()))?;

    if ir_options.optimize && emit.as_deref() != Some("ir") {
        return Err("--optimize-ir requires build --emit ir".into());
    }
    if explicit_verify && matches!(emit.as_deref(), Some("ast")) {
        return Err("--verify-ir requires typed IR or AVR compilation".into());
    }

    let mut project = helium::modules::Project::from_file(&input).map_err(|e| e.to_string())?;
    let source_name = input.display().to_string();
    let source_file = helium::diagnostic::source::SourceFile {
        id: helium::diagnostic::source::FileId(0),
        name: &source_name,
        text: &source,
    };
    let diagnostics = |errors: Vec<helium::diagnostic::Diagnostic>| {
        errors
            .iter()
            .map(|e| e.render_in(&source_file, RenderOptions { color }))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let emit = emit.as_deref().unwrap_or("hex");
    if !matches!(emit, "ast" | "ir" | "llvm" | "asm" | "elf" | "hex") {
        return Err(format!(
            "Unknown emit format '{emit}'; expected ast, ir, llvm, asm, elf or hex"
        ));
    }
    if optimization.is_some() && matches!(emit, "ast" | "ir" | "llvm") {
        return Err("--opt applies only to machine output (asm, elf, hex)".into());
    }
    if intermediates.is_some() && !matches!(emit, "asm" | "elf" | "hex") {
        return Err("--intermediates requires machine output (asm, elf, hex)".into());
    }
    let level = llvm_toolchain::Optimization::parse(optimization.as_deref().unwrap_or("none"))?;
    if command == "check" {
        let llvm = compile_project_llvm(&mut project, target, ir_options).map_err(diagnostics)?;
        if !llvm.warnings.is_empty() {
            eprintln!("{}", diagnostics(llvm.warnings.clone()));
        }
        return Ok(format!(
            "Checked {}{}",
            input.display(),
            device_report(&llvm)
        ));
    }

    let mut internal_ir = None;
    let content = match emit {
        "ast" => {
            let program = project.analyze().map_err(diagnostics)?;
            if !program.warnings().is_empty() {
                eprintln!("{}", diagnostics(program.warnings().to_vec()));
            }
            format!("{program:#?}\n")
        }
        "ir" => {
            let module =
                compile_project_ir(&mut project, target, ir_options).map_err(diagnostics)?;
            if !module.warnings.is_empty() {
                eprintln!("{}", diagnostics(module.warnings.clone()));
            }
            module.dump()
        }
        _ => {
            let artifacts = helium::compile_project_artifacts(&mut project, target, ir_options)
                .map_err(diagnostics)?;
            internal_ir = Some(artifacts.ir);
            let result = artifacts.llvm;
            if !result.warnings.is_empty() {
                eprintln!("{}", diagnostics(result.warnings));
            }
            result.output
        }
    };
    let report = device_report(&content);

    let extension = if emit == "llvm" { "ll" } else { emit };
    let destination = output.map(PathBuf::from).unwrap_or_else(|| {
        input
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("build")
            .join(format!(
                "{}.{extension}",
                input.file_stem().unwrap_or_default().to_string_lossy()
            ))
    });

    let protected_paths: Vec<_> = project
        .sources
        .files
        .iter()
        .map(|f| PathBuf::from(&f.name))
        .collect();
    helium::backend::output::check(&destination, &protected_paths)?;

    if matches!(emit, "asm" | "elf" | "hex") {
        let retention = match &intermediates {
            None => llvm_toolchain::Intermediates::Discard,
            Some(None) => llvm_toolchain::Intermediates::Generated,
            Some(Some(path)) => llvm_toolchain::Intermediates::Directory(path),
        };
        let result = llvm_toolchain::build_with_options(
            &content,
            target,
            &destination,
            emit,
            level,
            llvm_toolchain::BuildOptions {
                intermediates: retention,
                internal_ir: internal_ir.as_deref(),
                protected_paths: &protected_paths,
            },
        )?;
        let retained = result
            .retained_directory
            .as_ref()
            .map(|p| format!("\nIntermediates: {}", p.display()))
            .unwrap_or_default();
        return Ok(format!(
            "Built {} (external stages: {} ms){}\n{}",
            destination.display(),
            result.elapsed_ms,
            retained,
            result.memory.map(|m| m.render()).unwrap_or_else(|| {
                "Memory: unavailable for assembly output; link ELF/HEX for a final report\n".into()
            }) + &report
        ));
    }

    helium::backend::output::write(&destination, content.as_bytes(), &protected_paths)?;
    Ok(format!("Built {}{report}", destination.display()))
}

fn device_report(content: &str) -> String {
    content
        .lines()
        .filter_map(|line| line.strip_prefix("; "))
        .filter(|line| line.starts_with("Timer1:") || line.starts_with("UART0:"))
        .map(|line| format!("\n{line}"))
        .collect()
}

fn timer_options(args: &[String]) -> Result<String, String> {
    let mut values = std::collections::BTreeMap::new();
    let mut args = args.iter();
    while let Some(option) = args.next() {
        if !matches!(
            option.as_str(),
            "--mcu" | "--cpu-hz" | "--timer" | "--every"
        ) {
            return Err(format!("Unknown timer-options option {option}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {option}"))?;
        if values.insert(option.as_str(), value.as_str()).is_some() {
            return Err(format!("Repeated option {option}"));
        }
    }
    let required = |key| {
        values
            .get(key)
            .copied()
            .ok_or_else(|| format!("{key} is required"))
    };
    let target = helium::backend::microcontroller::get(required("--mcu")?)
        .ok_or("Unsupported MCU for timer-options")?;
    if required("--timer")? != target.timer.name {
        return Err(format!(
            "timer-options currently supports {} {} only",
            target.name, target.timer.name
        ));
    }
    let hz = required("--cpu-hz")?
        .parse()
        .map_err(|_| "--cpu-hz must be an integer")?;
    let period = helium::timer::parse_duration(required("--every")?)?;
    let candidates =
        helium::timer::candidates(target, hz, period).map_err(|error| error.to_string())?;
    Ok(format!("Requested: {period}ns; candidates ordered by absolute error, then prescaler.\n{}\nCalculated periods assume the stated CPU clock; oscillator error and ISR latency are not included.", candidates.iter().map(helium::timer::TimerConfig::report).collect::<Vec<_>>().join("\n")))
}
