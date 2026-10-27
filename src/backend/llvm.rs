//! Textual LLVM adapter. The caller owns IR verification policy.
//! Hardware accesses are volatile; ordinary storage remains optimizable.
use super::microcontroller::{self, GpioPort, Microcontroller};
use crate::ir::*;
use std::collections::HashMap;
use std::fmt::Write;

fn ty(value: Type) -> &'static str {
    match value {
        Type::Bool | Type::PinState => "i1",
        Type::U8 | Type::I8 => "i8",
        Type::U16 | Type::I16 => "i16",
        Type::U32 | Type::I32 => "i32",
        Type::U64 | Type::I64 => "i64",
    }
}

fn result_type(value: Option<Type>) -> &'static str {
    value.map(ty).unwrap_or("void")
}

/// Produce unoptimized LLVM IR from a well-formed Helium module.
/// `export_symbols` is for differential tests, not a public C ABI contract.
pub fn emit(module: &Module, export_symbols: bool) -> Result<String, String> {
    let target = microcontroller::get(&module.target)
        .ok_or_else(|| format!("Unsupported LLVM target {}", module.target))?;

    // Reject wider integers until their machine-level behavior is covered by tests.
    let types = module
        .globals
        .iter()
        .map(|s| s.ty)
        .chain(module.functions.iter().flat_map(|f| {
            f.locals
                .iter()
                .map(|s| s.ty)
                .chain(f.parameters.iter().map(|(_, t)| *t))
                .chain(f.return_type)
                .chain(
                    f.blocks
                        .iter()
                        .flat_map(|b| b.instructions.iter())
                        .filter_map(|i| i.result.map(|(_, t)| t)),
                )
        }));
    if types
        .into_iter()
        .any(|t| t.integer().is_some_and(|(w, _)| w > 16))
    {
        return Err(
            "LLVM prototype currently supports only bool, PinState and 8/16-bit integers".into(),
        );
    }

    let linkage = if export_symbols { "" } else { "internal " };
    let mut out = format!("; Helium LLVM prototype: wrapping arithmetic, explicit hardware effects\ntarget triple = \"{}\"\n\n", target.llvm_triple);
    if let Some(timer) = &module.timer {
        writeln!(out, "; {}", timer.report()).unwrap();
    }
    for line in super::timing::inventory(module) {
        writeln!(out, "; helium-timing: {line}").unwrap();
    }
    for (index, storage) in module.globals.iter().enumerate() {
        writeln!(
            out,
            "@helium_g{index} = {linkage}global {} 0, align 1",
            ty(storage.ty)
        )
        .unwrap();
    }

    for function in &module.functions {
        let mut emitter = FunctionEmitter {
            module,
            target,
            types: HashMap::new(),
            out: String::new(),
            temporary: 0,
        };
        for (id, kind) in &function.parameters {
            emitter.types.insert(*id, *kind);
        }
        for instruction in function.blocks.iter().flat_map(|b| &b.instructions) {
            if let Some((id, kind)) = instruction.result {
                emitter.types.insert(id, kind);
            }
        }

        let parameters = function
            .parameters
            .iter()
            .map(|(id, t)| format!("{} %v{}", ty(*t), id.index))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            out,
            "\ndefine {linkage}{} @helium_f{}({parameters}) addrspace(1) {{\nentry:",
            result_type(function.return_type),
            function.id.0
        )
        .unwrap();
        for (index, local) in function.locals.iter().enumerate() {
            writeln!(out, "  %l{index} = alloca {}, align 1", ty(local.ty)).unwrap();
        }
        writeln!(out, "  br label %b{}", function.entry.index).unwrap();

        for block in &function.blocks {
            writeln!(emitter.out, "b{}:", block.id.index).unwrap();
            for instruction in &block.instructions {
                emitter.instruction(instruction)?;
            }
            let terminator = match block.terminator {
                Terminator::Jump(b) => format!("br label %b{}", b.index),
                Terminator::Branch { condition, yes, no } => format!(
                    "br i1 %v{}, label %b{}, label %b{}",
                    condition.index, yes.index, no.index
                ),
                Terminator::Return(Some(v)) => format!("ret {}", emitter.value(v)),
                Terminator::Return(None) => "ret void".into(),
            };
            emitter.line(&terminator);
        }
        out.push_str(&emitter.out);
        out.push_str("}\n");
    }

    if let Some(handler) = module.interrupt_handler {
        writeln!(out, "\ndefine avr_signalcc void @{}() addrspace(1) {{\nentry:\n  call addrspace(1) void @helium_f{}()\n  ret void\n}}", target.timer.vector, handler.0).unwrap();
    }
    writeln!(out, "\ndefine i16 @main() addrspace(1) {{\nentry:\n  call addrspace(1) void @helium_f{}()\n  ret i16 0\n}}", module.entry.0).unwrap();
    Ok(out)
}

struct FunctionEmitter<'a> {
    module: &'a Module,
    target: &'static Microcontroller,
    types: HashMap<ValueId, Type>,
    out: String,
    temporary: usize,
}

impl FunctionEmitter<'_> {
    fn line(&mut self, text: &str) {
        writeln!(self.out, "  {text}").unwrap();
    }

    fn temp(&mut self, expression: &str) -> String {
        let name = format!("%t{}", self.temporary);
        self.temporary += 1;
        self.line(&format!("{name} = {expression}"));
        name
    }

    fn value(&self, id: ValueId) -> String {
        format!("{} %v{}", ty(self.types[&id]), id.index)
    }

    fn address(&self, id: PinId) -> Result<(&'static GpioPort, u8), String> {
        let pin = &self.module.pins[id.0];
        if !self.target.has_digital_pin(&pin.port, pin.bit) {
            return Err(format!("Invalid GPIO pin {}.{}", pin.port, pin.bit));
        }
        let port = self.target.port(&pin.port).unwrap();
        Ok((port, 1 << pin.bit))
    }

    fn read_register(&mut self, address: u16) -> String {
        self.temp(&format!(
            "load volatile i8, ptr inttoptr (i16 {address} to ptr), align 1"
        ))
    }

    fn write_register(&mut self, address: u16, value: &str) {
        self.line(&format!(
            "store volatile i8 {value}, ptr inttoptr (i16 {address} to ptr), align 1"
        ));
    }

    fn set_bit(&mut self, address: u16, bit: usize, on: bool) -> Result<(), String> {
        let io = microcontroller::atomic_bit_io(address)
            .ok_or("GPIO register has no supported atomic bit-write implementation")?;
        self.line(&format!(
            "call void asm sideeffect \"{} {io}, {bit}\", \"~{{memory}}\"()",
            if on { "sbi" } else { "cbi" }
        ));
        Ok(())
    }

    fn instruction(&mut self, instruction: &Instruction) -> Result<(), String> {
        let expression = match &instruction.op {
            Op::Const(c) => format!("or {} 0, {}", ty(c.ty), c.bits),
            Op::Unary(Unary::Negate, v) => format!("sub {} 0, %v{}", ty(self.types[v]), v.index),
            Op::Unary(Unary::Not, v) => format!("xor {}, 1", self.value(*v)),
            Op::Binary(op, a, b) => {
                let signed = self.types[a].integer().is_some_and(|(_, signed)| signed);
                let name = match op {
                    Binary::Add => "add",
                    Binary::Subtract => "sub",
                    Binary::Multiply => "mul",
                    Binary::Equal => "icmp eq",
                    Binary::NotEqual => "icmp ne",
                    Binary::Less => {
                        if signed {
                            "icmp slt"
                        } else {
                            "icmp ult"
                        }
                    }
                    Binary::LessEqual => {
                        if signed {
                            "icmp sle"
                        } else {
                            "icmp ule"
                        }
                    }
                    Binary::Greater => {
                        if signed {
                            "icmp sgt"
                        } else {
                            "icmp ugt"
                        }
                    }
                    Binary::GreaterEqual => {
                        if signed {
                            "icmp sge"
                        } else {
                            "icmp uge"
                        }
                    }
                };
                format!("{name} {}, %v{}", self.value(*a), b.index)
            }
            Op::Widen(v, target) => {
                if self.types[v] == *target {
                    format!("or {}, 0", self.value(*v))
                } else {
                    let signed = self.types[v].integer().is_some_and(|(_, signed)| signed);
                    format!(
                        "{} {} to {}",
                        if signed { "sext" } else { "zext" },
                        self.value(*v),
                        ty(*target)
                    )
                }
            }
            Op::InterruptDisable => "call void asm sideeffect \"cli\", \"~{memory}\"()".into(),
            Op::TimerStart => {
                let timer = self
                    .module
                    .timer
                    .as_ref()
                    .ok_or("missing timer configuration")?;
                let compare = timer.ticks - 1;
                let hardware = &self.target.timer;
                let control = hardware.ctc_mode
                    | hardware
                        .clock_select(timer.prescaler)
                        .ok_or("invalid timer prescaler")?;
                // Stop and mask first. 16-bit registers are written high byte first.
                self.write_register(hardware.control_b, "0");
                self.write_register(hardware.interrupt_mask, "0");
                self.write_register(hardware.control_a, "0");
                self.write_register(hardware.control_c, "0");
                self.write_register(hardware.counter_high, "0");
                self.write_register(hardware.counter_low, "0");
                self.write_register(hardware.compare_high, &(compare >> 8).to_string());
                self.write_register(hardware.compare_low, &(compare & 255).to_string());
                self.write_register(hardware.interrupt_flags, &hardware.clear_flags.to_string()); // W1C.
                self.write_register(
                    hardware.interrupt_mask,
                    &hardware.compare_interrupt.to_string(),
                );
                self.write_register(hardware.control_b, &control.to_string());
                "call void asm sideeffect \"sei\", \"~{memory}\"()".into()
            }
            Op::InterruptSave => {
                // Keep save + CLI in one assembly operation. The memory clobber
                // orders memory accesses across the boundary, not pure arithmetic.
                format!(
                    "call i8 asm sideeffect \"in $0, {}\\0Acli\", \"=r,~{{memory}}\"()",
                    self.target.sreg_io
                )
            }
            Op::InterruptRestore(value) => {
                format!(
                    "call void asm sideeffect \"out {}, $0\", \"r,~{{memory}}\"({})",
                    self.target.sreg_io,
                    self.value(*value)
                )
            }
            Op::LoadLocal(local) => format!(
                "load {}, ptr %l{}, align 1",
                ty(instruction.result.unwrap().1),
                local.index
            ),
            Op::LoadGlobal(global) => format!(
                "load {}{}, ptr @helium_g{}, align 1",
                if self.module.interrupt_globals.contains(global) {
                    "volatile "
                } else {
                    ""
                },
                ty(self.module.globals[global.0].ty),
                global.0
            ),
            Op::StoreLocal(local, v) => {
                format!("store {}, ptr %l{}, align 1", self.value(*v), local.index)
            }
            Op::StoreGlobal(global, v) => format!(
                "store {}{}, ptr @helium_g{}, align 1",
                if self.module.interrupt_globals.contains(global) {
                    "volatile "
                } else {
                    ""
                },
                self.value(*v),
                global.0
            ),
            Op::ReadPin(pin) => {
                let (port, mask) = self.address(*pin)?;
                let read = self.read_register(port.input);
                let masked = self.temp(&format!("and i8 {read}, {mask}"));
                format!("icmp ne i8 {masked}, 0")
            }
            Op::WritePin(pin, v) => {
                let (port, mask) = self.address(*pin)?;
                // Exactly one SBI or CBI executes. Interrupts between the skip
                // instructions are safe: ISR preserves registers, and each write
                // touches only this bit. No whole-port load/store is emitted.
                let io = microcontroller::atomic_bit_io(port.output)
                    .ok_or("GPIO register has no supported atomic bit-write implementation")?;
                let bit = mask.trailing_zeros();
                let value = self.temp(&format!("zext {} to i8", self.value(*v)));
                self.line(&format!(
                    "call void asm sideeffect \"sbrc $0, 0\\0Asbi {io}, {bit}\\0Asbrs $0, 0\\0Acbi {io}, {bit}\", \"r,~{{memory}}\"(i8 {value})"
                ));
                return Ok(());
            }
            Op::SetPinMode(pin, mode) => {
                let (port, mask) = self.address(*pin)?;
                self.set_bit(
                    port.direction,
                    mask.trailing_zeros() as usize,
                    *mode == PinMode::Output,
                )?;
                if *mode != PinMode::Output {
                    self.set_bit(
                        port.output,
                        mask.trailing_zeros() as usize,
                        *mode == PinMode::InputPullUp,
                    )?;
                }
                return Ok(());
            }
            Op::Call(id, arguments) => {
                let args = arguments
                    .iter()
                    .map(|v| self.value(*v))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "call addrspace(1) {} @helium_f{}({args})",
                    result_type(self.module.functions[id.0].return_type),
                    id.0
                )
            }
        };

        match instruction.result {
            Some((v, _)) => self.line(&format!("%v{} = {expression}", v.index)),
            None => self.line(&expression),
        }
        Ok(())
    }
}
