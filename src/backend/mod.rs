pub mod assembler;
pub mod microcontroller;
use crate::{
    diagnostic::Diagnostic,
    middle_end::linear_ir::{IRType, IntermediateRepresentation, LoadType},
};

/// Only called after support validation of the GPIO subset. A condition's scratch
/// value is consumed immediately, before either branch body can overwrite r16.
pub(crate) fn produce(ir: &[IntermediateRepresentation]) -> Result<String, Diagnostic> {
    let mut lines = vec![
        format!(
            ".nolist\n{}\n.list",
            microcontroller::ATMEGA328P.definitions
        ),
        "jmp setup".into(),
    ];
    let mut scratch = None;
    for instruction in ir {
        lines.push(match &instruction.ir_type {
            IRType::Label(label) => format!("{label}:"), IRType::Jump(label) => format!("JMP {label}"),
            IRType::Load(LoadType::ImmediateInt(value),reg) if *value <= 1 => { scratch = Some(*reg); format!("LDI r16, {value}") }
            IRType::JumpIfZero(reg,label) if scratch == Some(*reg) => { scratch = None; format!("TST r16\nBREQ {label}") }
            IRType::SetPinLow(pin) => format!("CBI PORT{},{}",pin.port,pin.pin_number),
            IRType::SetPinHigh(pin) => format!("SBI PORT{},{}",pin.port,pin.pin_number),
            IRType::SetInputPinMode(pin) => format!("CBI DDR{},{}",pin.port,pin.pin_number),
            IRType::SetOutputPinMode(pin) => format!("SBI DDR{},{}",pin.port,pin.pin_number),
            IRType::SkipIfPinLow(pin) => format!("SBIC PIN{},{}",pin.port,pin.pin_number),
            IRType::SkipIfPinHigh(pin) => format!("SBIS PIN{},{}",pin.port,pin.pin_number),
            _ => return Err(Diagnostic::new("E_BACKEND", "Instruction requires general register/storage lowering, which is not implemented yet",None)),
        });
    }
    Ok(lines.join("\n") + "\n")
}
