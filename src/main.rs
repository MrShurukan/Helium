use helium::diagnostic::{Diagnostic, DiagnosticKind, RenderOptions};
use helium::{
    analyze, backend::assembler, compile_assembly_with_options, compile_ir, compile_typed_ir, ir,
};
use std::io::IsTerminal;
use std::{env, fs, path::PathBuf, process::ExitCode};

const USAGE: &str = concat!(
    "Usage:\n",
    "  helium check <source.he> --mcu atmega328p\n",
    "  helium build <source.he> --mcu atmega328p [--emit hex|asm|ir|legacy-ir|ast] [-o <file>]\n",
    "  helium flash <file.hex> --mcu atmega328p --programmer <name>\n",
    "Options: --color auto|always|never (default: auto; NO_COLOR disables automatic color)\n",
    "  --verify-ir (check/build; default enabled in debug, disabled in release)\n",
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
    if helium::backend::microcontroller::get(&mcu).is_none() {
        return Err(
            Diagnostic::new(DiagnosticKind::UnsupportedTarget { mcu }, None)
                .render_with("", RenderOptions { color }),
        );
    }

    if command == "flash" {
        assembler::flash(
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
    if explicit_verify && matches!(emit.as_deref(), Some("ast" | "legacy-ir")) {
        return Err("--verify-ir requires typed IR or AVR compilation".into());
    }

    let diagnostics = |errors: Vec<helium::diagnostic::Diagnostic>| {
        format!(
            "{}\n{}",
            input.display(),
            errors
                .iter()
                .map(|e| e.render_with(&source, RenderOptions { color }))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    if command == "check" {
        compile_assembly_with_options(&source, &mcu, ir_options).map_err(diagnostics)?;
        return Ok(format!("Checked {}", input.display()));
    }

    let emit = emit.as_deref().unwrap_or("hex");
    let content = match emit {
        "hex" | "asm" => {
            compile_assembly_with_options(&source, &mcu, ir_options).map_err(diagnostics)?
        }
        "ir" => compile_typed_ir(&source, &mcu, ir_options)
            .map_err(diagnostics)?
            .dump(),
        "legacy-ir" => {
            compile_ir(&source, &mcu)
                .map_err(diagnostics)?
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        }
        "ast" => format!("{:#?}\n", analyze(&source).map_err(diagnostics)?),
        _ => {
            return Err(format!(
                "Unknown emit format '{emit}'; expected hex, asm, ir, legacy-ir or ast"
            ))
        }
    };

    let destination = output.map(PathBuf::from).unwrap_or_else(|| {
        input
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("build")
            .join(format!(
                "{}.{emit}",
                input.file_stem().unwrap_or_default().to_string_lossy()
            ))
    });

    let source_path = input.canonicalize().map_err(|e| e.to_string())?;
    if destination.canonicalize().ok().as_ref() == Some(&source_path) {
        return Err("Output must not overwrite the source file".into());
    }

    if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("Cannot create output directory: {e}"))?;
    }

    let assembly_path = if emit == "hex" {
        destination.with_extension("asm")
    } else {
        destination.clone()
    };
    if assembly_path.canonicalize().ok().as_ref() == Some(&source_path) {
        return Err("Assembly output must not overwrite the source file".into());
    }
    if emit == "hex" && assembly_path == destination {
        return Err("HEX output must have a different path from the intermediate .asm file".into());
    }

    fs::write(&assembly_path, content)
        .map_err(|e| format!("Cannot write {}: {e}", assembly_path.display()))?;

    if emit == "hex" {
        assembler::assemble(&assembly_path, &destination)?;
    }

    Ok(format!("Built {}", destination.display()))
}
