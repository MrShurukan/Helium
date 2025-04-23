pub mod microcontroller;
pub mod assembler;

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path;
use crate::backend::microcontroller::Microcontroller;
use crate::middle_end::linear_ir::{IRType, IntermediateRepresentation, LoadType};
use crate::middle_end::utils::EnvironmentStack;

pub struct AssemblyProducer<'a> {
    source: &'a str
}

impl AssemblyProducer<'_> {
    pub fn new(source: &str) -> AssemblyProducer {
        AssemblyProducer { source }
    }

    pub fn produce(&mut self, environment: EnvironmentStack, ir_vec: Vec<IntermediateRepresentation>, 
                   microcontroller: &Microcontroller) -> Result<Vec<String>, AssemblyError> {
        let mut vec: Vec<String> = Vec::with_capacity(ir_vec.len());

        // TODO: Figure out a way to include these files in the final binary?
        vec.push(format!(".nolist\n.include \"./assembly_includes/{}\"\n.list", microcontroller.include_file_name));
        
        // Working with global variables
        environment.
        
        vec.push("jmp setup".to_string());

        for ir in ir_vec {
            vec.push(match ir.ir_type {
                IRType::Label(label) => format!("{label}:"),
                IRType::Jump(label) => format!("JMP {label}"),
                IRType::Load(load_type, reg) => {
                    match load_type {
                        LoadType::ImmediateInt(value) => {
                            // TODO: Register work
                            format!("LDI r16, {value}")
                        },
                        LoadType::Memory(_) => todo!("load memory"),
                        LoadType::DDR(_) => todo!("load ddr"),
                        LoadType::Port(_) => todo!("load port"),
                    }
                },
                // TODO: Optimize for not requiring SRAM when registers are abundant
                IRType::Store(_, _) => todo!("store"),
                IRType::Move { .. } => todo!("move"),
                IRType::IntegerAdd { .. } => todo!("integer add"),
                IRType::LogicalShiftLeft(_) => todo!("logical <<"),
                IRType::LogicalShiftRight(_) => todo!("logical >>"),
                IRType::SetPinLow(pin) => format!("CBI Port{},{}", pin.port, pin.pin_number),
                IRType::SetPinHigh(pin) => format!("SBI Port{},{}", pin.port, pin.pin_number),
                IRType::SetInputPinMode(pin) => format!("CBI DDR{},{}", pin.port, pin.pin_number),
                IRType::SetOutputPinMode(pin) => format!("SBI DDR{},{}", pin.port, pin.pin_number),
                IRType::AndImmediate(_, _) => todo!("and immediate"),
                IRType::OrImmediate(_, _) => todo!("or immediate"),
                IRType::JumpIfZero(reg, label) => {
                    // TODO: Register work
                    format!("TST r16\nBREQ {label}")
                },
                IRType::BranchIfEqual(_) => todo!("branch if equal"),
                IRType::SkipIfPinLow(pin) => format!("SBIC Pin{},{}", pin.port, pin.pin_number),
                IRType::SkipIfPinHigh(pin) => format!("SBIS Pin{},{}", pin.port, pin.pin_number)
            });
        }

        Ok(vec)
    }
}

#[derive(Debug)]
pub struct AssemblyError {
    error_type: AssemblyErrorType
}

#[derive(Debug)]
pub enum AssemblyErrorType {

}

impl Error for AssemblyError {}
impl Display for AssemblyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.error_type {

        }
    }
}