use std::fs::{File, remove_file};
use std::io::Write;
use std::process::{exit, Command};
use colored::Colorize;
use crate::backend::microcontroller::Microcontroller;

/// Assembles the produced assembly language with avra and uploads it via AVR-Dude
// TODO: Errors instead of exits and panics
pub fn assemble_and_load(assembly_language: String, microcontroller: &Microcontroller) {
    let output_name = "output";
    let tmp_file_path = format!("./{output_name}.asm");
    let mut tmp_file = File::create(&tmp_file_path).expect("Can't create ./output.asm");
    tmp_file.write_all(assembly_language.as_bytes()).expect("Unable to write to ./output.asm");

    let avra_result = Command::new("avra")
        .args([&tmp_file_path])
        .output()
        .expect("Failed to execute avra");

    if !avra_result.status.success() {
        eprintln!("{}", "Couldn't assemble using avra".red());
        match std::str::from_utf8(&avra_result.stderr) {
            Ok(err) => eprintln!("{}", err.red()),
            Err(_) => eprintln!("Output from avra wasn't utf8, can't read")
        }
        
        exit(-1);
    }

    println!("{}", "Uploading...".yellow());
    
    let flash_arg = format!("flash:w:{output_name}.hex");
    let avr_dude_result = Command::new("avrdude")
        .args(["-p", microcontroller.avr_dude_name, "-c", "USBasp", "-U", &flash_arg])
        .output()
        .expect("Failed to execute avr_dude");
    
    if !avr_dude_result.status.success() {
        eprintln!("{}", "Couldn't upload using avr_dude".red());
        match std::str::from_utf8(&avr_dude_result.stderr) {
            Ok(err) => eprintln!("{}", err.red()),
            Err(_) => eprintln!("Output from avr_dude wasn't utf8, can't read")
        }

        exit(-1);
    }

    println!("{}", "Upload complete".green());
}