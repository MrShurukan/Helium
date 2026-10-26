use helium::diagnostic::{Diagnostic, DiagnosticKind, RenderOptions};
use helium::{
    analyze,
    backend::{flash, llvm_toolchain},
    compile_llvm, compile_typed_ir, ir,
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
    "  --verify-ir (check/build; default enabled in debug, disabled in release)\n",
    "  --opt none|speed|size (machine output only; default: none)\n",
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

    let mut options = args[2..].iter();
    while let Some(option) = options.next() {
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
    if ir_options
        .cpu_hz
        .is_some_and(|hz| hz == 0 || hz > 20_000_000)
    {
        return Err("--cpu-hz must be in 1..=20000000 for ATmega328P".into());
    }
    if helium::backend::microcontroller::get(&mcu).is_none() {
        return Err(
            Diagnostic::new(DiagnosticKind::UnsupportedTarget { mcu }, None)
                .render_with("", RenderOptions { color }),
        );
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
    let level = llvm_toolchain::Optimization::parse(optimization.as_deref().unwrap_or("none"))?;
    if command == "check" {
        let llvm = compile_llvm(&source, &mcu, ir_options).map_err(diagnostics)?;
        return Ok(format!(
            "Checked {}{}",
            input.display(),
            timer_report(&llvm)
        ));
    }

    let content = match emit {
        "ast" => format!("{:#?}\n", analyze(&source).map_err(diagnostics)?),
        "ir" => compile_typed_ir(&source, &mcu, ir_options)
            .map_err(diagnostics)?
            .dump(),
        _ => compile_llvm(&source, &mcu, ir_options).map_err(diagnostics)?,
    };
    let report = timer_report(&content);

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

    if destination.canonicalize().ok() == Some(input.canonicalize().map_err(|e| e.to_string())?) {
        return Err("Output must not overwrite the source file".into());
    }

    if matches!(emit, "asm" | "elf" | "hex") {
        let result = llvm_toolchain::build(&content, &destination, emit, level)?;
        return Ok(format!(
            "Built {} (external stages: {} ms)\nIntermediates: {}\n{}",
            destination.display(),
            result.elapsed_ms,
            result.directory.display(),
            result.sections + &report
        ));
    }

    if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("Cannot create output directory: {e}"))?;
    }

    fs::write(&destination, content)
        .map_err(|e| format!("Cannot write {}: {e}", destination.display()))?;
    Ok(format!("Built {}{report}", destination.display()))
}

fn timer_report(content: &str) -> String {
    content
        .lines()
        .find_map(|line| {
            line.strip_prefix("; Timer1:")
                .map(|report| format!("\nTimer1:{report}"))
        })
        .unwrap_or_default()
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
    if required("--mcu")? != "atmega328p" || required("--timer")? != "Timer1" {
        return Err("timer-options currently supports atmega328p Timer1 only".into());
    }
    let hz = required("--cpu-hz")?
        .parse()
        .map_err(|_| "--cpu-hz must be an integer")?;
    let period = helium::timer::parse_duration(required("--every")?)?;
    let candidates = helium::timer::candidates(hz, period).map_err(|error| error.to_string())?;
    Ok(format!("Requested: {period}ns; candidates ordered by absolute error, then prescaler.\n{}\nCalculated periods assume the stated CPU clock; oscillator error and ISR latency are not included.", candidates.iter().map(helium::timer::TimerConfig::report).collect::<Vec<_>>().join("\n")))
}
