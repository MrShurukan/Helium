//! External tools are explicit operations; compilation never calls this module.
use std::{path::Path, process::Command};

fn run(tool: &str, command: &mut Command) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("Cannot start {tool}: {error}. Install it and add it to PATH."))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{tool} failed ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

pub fn assemble(assembly: &Path, hex: &Path) -> Result<(), String> {
    run(
        "avra",
        Command::new("avra").arg("-o").arg(hex).arg(assembly),
    )
}

pub fn flash(hex: &Path, mcu: &str, programmer: &str) -> Result<(), String> {
    let target =
        super::microcontroller::get(mcu).ok_or_else(|| format!("Unsupported MCU '{mcu}'"))?;

    if !hex.is_file() {
        return Err(format!("HEX file does not exist: {}", hex.display()));
    }

    let hex = hex.canonicalize().map_err(|e| e.to_string())?;

    // Explicit Intel HEX format avoids guessing from the filename.
    run(
        "avrdude",
        Command::new("avrdude").args([
            "-p",
            target.avr_dude_name,
            "-c",
            programmer,
            "-U",
            &format!("flash:w:{}:i", hex.display()),
        ]),
    )
}
