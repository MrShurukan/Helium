//! Optional external toolchain. No tools are needed to emit textual LLVM IR.
use super::microcontroller::Microcontroller;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

#[derive(Debug, Clone, Copy)]
pub enum Optimization {
    None,
    Speed,
    Size,
}

impl Optimization {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "none" => Ok(Self::None),
            "speed" => Ok(Self::Speed),
            "size" => Ok(Self::Size),
            _ => Err(format!(
                "Unknown optimization '{value}'; expected none, speed or size"
            )),
        }
    }

    fn pipeline(self) -> &'static str {
        match self {
            Self::None => "default<O0>",
            Self::Speed => "default<O2>",
            // LLVM 23 expresses size policy through function attributes.
            Self::Size => "default<O2>",
        }
    }
}

#[derive(Debug)]
pub struct Build {
    pub directory: PathBuf,
    pub elapsed_ms: u128,
    pub sections: String,
    pub memory: Option<super::memory::MemoryReport>,
}

/// Overrides are directories containing the executable and its DLL dependencies.
/// Otherwise use PATH; tool installation is always an explicit setup step.
pub fn tool(name: &str) -> PathBuf {
    let variable = if name.starts_with("avr-") {
        "HELIUM_AVR_BIN"
    } else {
        "HELIUM_LLVM_BIN"
    };
    let executable = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    };
    env::var_os(variable)
        .map(PathBuf::from)
        .map(|directory| directory.join(&executable))
        .unwrap_or_else(|| PathBuf::from(executable))
}

fn run(name: &str, args: &[OsString]) -> Result<String, String> {
    let executable = tool(name);
    let output = Command::new(&executable).args(args).output().map_err(|e| {
        format!(
            "Cannot run {}: {e}. See docs/LLVM_PROTOTYPE.md for toolchain setup.",
            executable.display()
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "{} failed ({}):\n{}{}",
            executable.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn arg(value: impl AsRef<OsStr>) -> OsString {
    value.as_ref().to_os_string()
}

/// Keep all intermediates in a newly-created directory; publish only the selected
/// output after every required stage succeeds. No shell command interpolation.
pub fn build(
    ir: &str,
    target: &Microcontroller,
    destination: &Path,
    emit: &str,
    optimization: Optimization,
) -> Result<Build, String> {
    if !matches!(emit, "asm" | "elf" | "hex") {
        return Err(format!(
            "LLVM machine output must be asm, elf or hex, got {emit}"
        ));
    }
    let started = Instant::now();
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let directory = parent.join(format!("llvm-{}-{stamp}", std::process::id()));
    fs::create_dir(&directory).map_err(|e| e.to_string())?;
    // Keep ordinary absolute Windows paths: older AVR binutils do not understand
    // the extended-length prefix produced by fs::canonicalize on Windows.
    let directory = if directory.is_absolute() {
        directory
    } else {
        env::current_dir()
            .map_err(|e| e.to_string())?
            .join(directory)
    };
    let raw = directory.join("module.ll");
    let optimized = directory.join("optimized.ll");
    let object = directory.join("program.o");
    let assembly = directory.join("program.s");
    let elf = directory.join("program.elf");
    let hex = directory.join("program.hex");

    let ir = if matches!(optimization, Optimization::Size) {
        ir.lines()
            .map(|line| {
                if line.starts_with("define ") {
                    line.replace("addrspace(1)", "addrspace(1) minsize optsize")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        ir.to_owned()
    };
    let timing = ir
        .lines()
        .filter_map(|line| line.strip_prefix("; helium-timing: "))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(directory.join("timing.txt"), timing).map_err(|e| e.to_string())?;
    fs::write(&raw, ir).map_err(|e| e.to_string())?;
    let targets = run("llc", &[arg("--version")])?;
    if !targets
        .lines()
        .any(|line| line.trim_start().starts_with("avr "))
    {
        return Err("Installed LLVM has no AVR target; install an AVR-enabled build (see docs/LLVM_PROTOTYPE.md)".into());
    }

    run(
        "opt",
        &[
            arg("-S"),
            arg(format!("-mtriple={}", target.llvm_triple)),
            arg(format!("-mcpu={}", target.name)),
            arg(format!("-passes={}", optimization.pipeline())),
            arg(&raw),
            arg("-o"),
            arg(&optimized),
        ],
    )?;
    let level = if matches!(optimization, Optimization::None) {
        "-O0"
    } else {
        "-O2"
    };
    let machine_args = [
        arg(format!("-mtriple={}", target.llvm_triple)),
        arg(format!("-mcpu={}", target.name)),
        arg(level),
        arg(&optimized),
    ];
    let mut arguments = machine_args.to_vec();
    arguments.extend([arg("-filetype=asm"), arg("-o"), arg(&assembly)]);
    run("llc", &arguments)?;

    let mut sections = String::new();
    let mut memory = None;
    let artifact = if emit == "asm" {
        &assembly
    } else {
        let mut arguments = machine_args.to_vec();
        arguments.extend([arg("-filetype=obj"), arg("-o"), arg(&object)]);
        run("llc", &arguments)?;
        run(
            "avr-gcc",
            &[
                arg(format!("-mmcu={}", target.name)),
                arg(&object),
                arg("-Wl,--gc-sections"),
                arg(format!(
                    "-Wl,-Map={}",
                    directory.join("program.map").display()
                )),
                arg("-o"),
                arg(&elf),
            ],
        )?;

        let report = super::memory::from_elf(&fs::read(&elf).map_err(|e| e.to_string())?, target)?;
        fs::write(directory.join("memory.txt"), report.render()).map_err(|e| e.to_string())?;
        memory = Some(report);
        sections = run("avr-size", &[arg("-A"), arg(&elf)])?;
        fs::write(directory.join("sections.txt"), &sections).map_err(|e| e.to_string())?;
        let symbols = run("avr-nm", &[arg("-S"), arg("-n"), arg(&elf)])?;
        fs::write(directory.join("symbols.txt"), symbols).map_err(|e| e.to_string())?;
        let disassembly = run("avr-objdump", &[arg("-d"), arg(&elf)])?;
        fs::write(directory.join("disassembly.txt"), disassembly).map_err(|e| e.to_string())?;

        if emit == "hex" {
            run(
                "avr-objcopy",
                &[
                    arg("-O"),
                    arg("ihex"),
                    arg("-R"),
                    arg(".eeprom"),
                    arg(&elf),
                    arg(&hex),
                ],
            )?;
            &hex
        } else {
            &elf
        }
    };
    fs::copy(artifact, destination)
        .map_err(|e| format!("Cannot write {}: {e}", destination.display()))?;

    Ok(Build {
        directory,
        elapsed_ms: started.elapsed().as_millis(),
        sections,
        memory,
    })
}
