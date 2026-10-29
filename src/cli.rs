use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clio::{InputPath, OutputPath};
use helium::backend::llvm_toolchain;

#[derive(Debug, Parser)]
#[command(name = "helium")]
#[command(version)]
#[command(about = "Helium compiler")]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Command,

    /// Controls colored terminal output. Auto mode will disable color when piping
    #[arg(
        long,
        value_enum,
        default_value_t = ColorMode::Auto,
        global = true
    )]
    pub color: ColorMode,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check a Helium source file without producing a binary
    Check(CheckArgs),

    /// Build a Helium source file.
    Build(BuildArgs),

    /// Flash a HEX file to a microcontroller.
    Flash(FlashArgs),

    /// Show timer configuration options. Standalone timer calculator. Note that this is a helper tool,
    /// language can already calculate this for you
    TimerOptions(TimerOptionsArgs),
}

#[derive(Debug, Args)]
pub struct CommonArgs {
    /// Target MCU.
    // TODO: Make this an enum of supported variants
    #[arg(long)]
    pub mcu: String,

    /// CPU frequency in Hz.
    ///
    /// Required for builds/checks when using a timer/uart and for timer-options (standalone timer calculator)
    #[arg(long)]
    pub cpu_hz: Option<u32>,
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Helium source file.
    pub source: InputPath,

    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Helium source file.
    pub source: InputPath,

    #[command(flatten)]
    pub common: CommonArgs,

    /// Output representation.
    #[arg(long, value_enum, default_value_t = Emit::Hex)]
    pub emit: Emit,

    /// Output file or directory.
    #[arg(short = 'o', long)]
    pub output: Option<OutputPath>,

    /// Optimization level. Defaults to none
    #[arg(long, value_enum)]
    pub opt: Option<Optimization>,

    /// Emit compiler intermediate files. Internal IR, LLVM, assembly, optimized versions, etc.
    ///
    /// Optionally accepts the destination directory, otherwise creates it in a unique folder next to
    /// the output file
    #[arg(long, num_args = 0..=1)]
    pub intermediates: Option<Option<PathBuf>>,

    /// Optimize internal IR before emitting it (used with --emit ir).
    #[arg(long, default_value_t = false)]
    pub optimize_ir: bool,
}

#[derive(Debug, Args)]
pub struct FlashArgs {
    /// HEX file to flash.
    pub file: InputPath,

    #[command(flatten)]
    pub common: CommonArgs,

    /// Programmer to use.
    #[arg(long)]
    pub programmer: String,
}

#[derive(Debug, Args)]
pub struct TimerOptionsArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// Timer to inspect.
    #[arg(long)]
    pub timer: String,

    /// Requested timer period. Numbers with a time postfix, i.e. 1s, 1500ms, 300ns, 10ms, 100us.
    #[arg(long)]
    pub every: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Emit {
    Hex,
    Elf,
    Asm,
    Llvm,
    Ir,
    Ast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Optimization {
    None,
    Speed,
    Size,
}

impl Into<llvm_toolchain::Optimization> for Optimization {
    fn into(self) -> llvm_toolchain::Optimization {
        match self {
            Optimization::None => llvm_toolchain::Optimization::None,
            Optimization::Speed => llvm_toolchain::Optimization::Speed,
            Optimization::Size => llvm_toolchain::Optimization::Size,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}