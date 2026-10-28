//! Textual LLVM adapter. The caller owns IR verification policy.
//! Hardware accesses are volatile; ordinary storage remains optimizable.
use super::microcontroller::{self, GpioPort, Microcontroller};
use crate::ir::*;
use std::collections::HashMap;
use std::fmt::Write;

fn ty(value: Type) -> String {
    match value {
        Type::Array(e, n) => return format!("[{n} x {}]", ty(Type::element(e))),
        Type::ArrayRef(..) => "ptr",
        Type::SliceRef(..) => "{ ptr, i16 }",
        Type::Text => super::text::LLVM_TYPE,
        Type::TextIn(_) => "{ i16, i16 }",
        Type::Bool | Type::PinState => "i1",
        Type::U8 | Type::I8 => "i8",
        Type::U16 | Type::I16 => "i16",
        Type::U32 | Type::I32 => "i32",
        Type::U64 | Type::I64 => "i64",
    }
    .into()
}

fn result_type(value: Option<Type>) -> String {
    value.map(ty).unwrap_or_else(|| "void".into())
}

/// Produce unoptimized LLVM IR from a well-formed Helium module.
/// `export_symbols` is for differential tests, not a public C ABI contract.
pub fn emit(module: &Module, export_symbols: bool) -> Result<String, String> {
    let prepared = crate::ir::text::specialize(module)?;
    let module = &prepared;
    let target = module.target;
    if module.devices.iter().any(|device| match &device.config {
        DeviceConfig::PeriodicTimer(t) => t.hardware != &target.timer,
        DeviceConfig::UartTx(u) => u.hardware != &target.uart,
    }) {
        return Err("Device configuration belongs to a different target description".into());
    }

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
    if types.into_iter().any(|t| {
        t.sequence()
            .map_or(t, |(e, _)| Type::element(e))
            .integer()
            .is_some_and(|(w, _)| w > 16)
    }) {
        return Err(
            "LLVM prototype currently supports only bool, PinState and 8/16-bit integers".into(),
        );
    }

    let linkage = if export_symbols { "" } else { "internal " };
    let mut out = format!("; Helium LLVM prototype: wrapping arithmetic, explicit hardware effects\ntarget triple = \"{}\"\n\n", target.llvm_triple);
    for device in &module.devices {
        writeln!(
            out,
            "; {}",
            match &device.config {
                DeviceConfig::PeriodicTimer(t) => t.report(),
                DeviceConfig::UartTx(u) => u.report(),
            }
        )
        .unwrap();
    }
    for line in super::timing::inventory(module) {
        writeln!(out, "; helium-timing: {line}").unwrap();
    }
    super::text::emit(module, &mut out);
    if module
        .functions
        .iter()
        .flat_map(|f| &f.blocks)
        .flat_map(|b| &b.instructions)
        .any(|i| matches!(i.op, Op::ArrayRead(..) | Op::ArrayWrite(..)))
    {
        out.push_str(
            r#"
define internal void @helium_array_bounds(i16 %index, i16 %length) addrspace(1) alwaysinline {
entry:
  %valid = icmp ult i16 %index, %length
  br i1 %valid, label %ok, label %fail
ok:
  ret void
fail:
  call void asm sideeffect "cli", "~{memory}"()
  br label %halt
halt:
  br label %halt
}
"#,
        );
    }
    for (index, storage) in module.globals.iter().enumerate() {
        writeln!(
            out,
            "@helium_g{index} = {linkage}global {} zeroinitializer, align 1",
            ty(storage.ty)
        )
        .unwrap();
    }

    for (index, device) in module.devices.iter().enumerate() {
        if let DeviceConfig::UartTx(config) = &device.config {
            emit_uart(&mut out, index, config, target.sreg_io);
        }
    }
    for function in &module.functions {
        writeln!(out, "; Helium function: {}", function.name).unwrap();
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

    for binding in &module.interrupts {
        writeln!(out, "\ndefine avr_signalcc void @{}() addrspace(1) {{\nentry:\n  call addrspace(1) void @helium_f{}()\n  ret void\n}}", binding.vector, binding.handler.0).unwrap();
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

// Polling implementations have typed frontend contracts, but no synthetic Helium
// bodies. Only the check/write transaction masks interrupts; waits never do.
fn emit_uart(out: &mut String, id: usize, config: &crate::uart::UartConfig, sreg: u8) {
    let h = config.hardware;
    let status = h.status;
    let data = h.data;
    let empty = h.empty;
    let complete = h.complete;
    let clear = config.status_bits() | complete;
    writeln!(
        out,
        r#"
@helium_uart_sent_{id} = internal global i8 0, align 1
define internal i1 @helium_uart_{id}_try_write_byte(i8 %byte) addrspace(1) {{
entry:
  %saved = call i8 asm sideeffect "in $0, {sreg}\0Acli", "=r,~{{memory}}"()
  %status = load volatile i8, ptr inttoptr (i16 {status} to ptr), align 1
  %mask = and i8 %status, {empty}
  %ready = icmp ne i8 %mask, 0
  br i1 %ready, label %send, label %done
send:
  store volatile i8 %byte, ptr inttoptr (i16 {data} to ptr), align 1
  store volatile i8 {clear}, ptr inttoptr (i16 {status} to ptr), align 1
  store volatile i8 1, ptr @helium_uart_sent_{id}, align 1
  br label %done
done:
  call void asm sideeffect "out {sreg}, $0", "r,~{{memory}}"(i8 %saved)
  ret i1 %ready
}}
define internal void @helium_uart_{id}_write_byte(i8 %byte) addrspace(1) {{
entry:
  br label %wait
wait:
  %ready = call addrspace(1) i1 @helium_uart_{id}_try_write_byte(i8 %byte)
  br i1 %ready, label %done, label %wait
done:
  ret void
}}
define internal void @helium_uart_{id}_flush() addrspace(1) {{
entry:
  %sent = load volatile i8, ptr @helium_uart_sent_{id}, align 1
  %idle = icmp eq i8 %sent, 0
  br i1 %idle, label %done, label %wait
wait:
  %status = load volatile i8, ptr inttoptr (i16 {status} to ptr), align 1
  %mask = and i8 %status, {complete}
  %ready = icmp ne i8 %mask, 0
  br i1 %ready, label %done, label %wait
done:
  ret void
}}
"#
    )
    .unwrap();
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

    fn text_literal_result(
        &mut self,
        instruction: &Instruction,
        id: usize,
        pointer: &str,
        length: usize,
    ) -> Result<(), String> {
        let v = instruction.result.unwrap().0.index;
        self.line(&format!("%v{v} = select i1 true, {{ i16, i16 }} {{ i16 ptrtoint ({pointer} @helium_text{id} to i16), i16 {length} }}, {{ i16, i16 }} zeroinitializer"));
        Ok(())
    }

    fn instruction(&mut self, instruction: &Instruction) -> Result<(), String> {
        let expression = match &instruction.op {
            Op::ArrayLiteral(values) => self.array_literal(instruction.result.unwrap().1, values),
            Op::ArrayRepeat(v, n) => {
                self.array_literal(instruction.result.unwrap().1, &vec![*v; usize::from(*n)])
            }
            Op::AddressLocal(l, _) => format!("getelementptr i8, ptr %l{}, i16 0", l.index),
            Op::AddressGlobal(g, _) => format!("getelementptr i8, ptr @helium_g{}, i16 0", g.0),
            Op::SliceLen(v) => format!("extractvalue {}, 1", self.value(*v)),
            Op::Slice(v, _) => {
                if let Type::ArrayRef(_, n, _) = self.types[v] {
                    let p = self.temp(&format!(
                        "insertvalue {{ ptr, i16 }} poison, ptr %v{}, 0",
                        v.index
                    ));
                    format!("insertvalue {{ ptr, i16 }} {p}, i16 {n}, 1")
                } else {
                    format!(
                        "select i1 true, {}, {} zeroinitializer",
                        self.value(*v),
                        ty(self.types[v])
                    )
                }
            }
            Op::Reborrow(v, _) => format!("getelementptr i8, ptr %v{}, i16 0", v.index),
            Op::ArrayRead(a, i) | Op::ArrayWrite(a, i, _) => {
                let (e, n) = self.types[a].sequence().ok_or("array reference type")?;
                let (base, len) = if let Some(n) = n {
                    (format!("%v{}", a.index), n.to_string())
                } else {
                    (
                        self.temp(&format!("extractvalue {}, 0", self.value(*a))),
                        self.temp(&format!("extractvalue {}, 1", self.value(*a))),
                    )
                };
                self.line(&format!(
                    "call addrspace(1) void @helium_array_bounds({}, i16 {len})",
                    self.value(*i)
                ));
                let p = self.temp(&format!(
                    "getelementptr {}, ptr {base}, {}",
                    ty(Type::element(e)),
                    self.value(*i)
                ));
                if let Op::ArrayWrite(_, _, v) = instruction.op {
                    format!("store volatile {}, ptr {p}, align 1", self.value(v))
                } else {
                    format!("load volatile {}, ptr {p}, align 1", ty(Type::element(e)))
                }
            }
            Op::TextLiteral { text, space } => {
                let id = super::text::literals(self.module)
                    .iter()
                    .position(|(t, s)| t == text && s == space)
                    .unwrap();
                let pointer = if *space == TextSpace::Flash {
                    "ptr addrspace(1)"
                } else {
                    "ptr"
                };
                let flag = u8::from(*space == TextSpace::Flash);
                if matches!(instruction.result.unwrap().1, Type::TextIn(_)) {
                    return self.text_literal_result(instruction, id, pointer, text.len());
                }
                format!("select i1 true, {{ i16, i16, i8 }} {{ i16 ptrtoint ({pointer} @helium_text{id} to i16), i16 {}, i8 {flag} }}, {{ i16, i16, i8 }} zeroinitializer", text.len())
            }
            Op::TextIsFlash(v) => {
                let temp = self.temporary;
                self.temporary += 1;
                self.line(&format!(
                    "%text_space{temp} = extractvalue {}, 2",
                    self.value(*v)
                ));
                format!("icmp eq i8 %text_space{temp}, 1")
            }
            Op::TextNarrow(v, _) | Op::TextErase(v) => {
                let temp = self.temporary;
                self.temporary += 1;
                self.line(&format!(
                    "%text_addr{temp} = extractvalue {}, 0",
                    self.value(*v)
                ));
                self.line(&format!(
                    "%text_len{temp} = extractvalue {}, 1",
                    self.value(*v)
                ));
                let out_type = ty(instruction.result.unwrap().1);
                self.line(&format!(
                    "%text_a{temp} = insertvalue {out_type} poison, i16 %text_addr{temp}, 0"
                ));
                if let Op::TextErase(_) = instruction.op {
                    self.line(&format!("%text_b{temp} = insertvalue {out_type} %text_a{temp}, i16 %text_len{temp}, 1"));
                    let Type::TextIn(space) = self.types[v] else {
                        unreachable!()
                    };
                    format!(
                        "insertvalue {out_type} %text_b{temp}, i8 {}, 2",
                        u8::from(space == TextSpace::Flash)
                    )
                } else {
                    format!("insertvalue {out_type} %text_a{temp}, i16 %text_len{temp}, 1")
                }
            }
            Op::TextLen(v) => format!("extractvalue {}, 1", self.value(*v)),
            Op::TextByteOr {
                text,
                index,
                fallback,
            } => {
                let suffix = match self.types[text] {
                    Type::TextIn(TextSpace::Flash) => "_flash",
                    Type::TextIn(TextSpace::Sram) => "_sram",
                    _ => "",
                };
                format!(
                    "call i8 @helium_text_byte{suffix}({}, {}, {})",
                    self.value(*text),
                    self.value(*index),
                    self.value(*fallback)
                )
            }
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
            Op::InterruptEnable => "call void asm sideeffect \"sei\", \"~{memory}\"()".into(),
            Op::DeviceStart(id) => {
                let device = self
                    .module
                    .devices
                    .get(id.0)
                    .ok_or("missing device configuration")?;
                if let DeviceConfig::UartTx(config) = &device.config {
                    let h = config.hardware;
                    self.write_register(h.control, "0");
                    self.write_register(h.baud_high, &(config.divider >> 8).to_string());
                    self.write_register(h.baud_low, &(config.divider & 255).to_string());
                    self.write_register(h.format, &h.format_8n1.to_string());
                    self.write_register(h.status, &(config.status_bits() | h.complete).to_string());
                    self.line(&format!(
                        "store volatile i8 0, ptr @helium_uart_sent_{}, align 1",
                        id.0
                    ));
                    self.write_register(h.control, &h.tx_enable.to_string());
                    return Ok(());
                }
                let timer = device.timer().ok_or("missing timer configuration")?;
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
                return Ok(());
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
            Op::Intrinsic {
                device,
                operation,
                arguments,
            } => {
                let args = arguments
                    .iter()
                    .map(|v| self.value(*v))
                    .collect::<Vec<_>>()
                    .join(", ");
                let result = if *operation == crate::intrinsics::Intrinsic::UartTryWrite {
                    "i1"
                } else {
                    "void"
                };
                format!(
                    "call addrspace(1) {result} @helium_uart_{}_{}({args})",
                    device.0,
                    operation.name()
                )
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

    fn array_literal(&mut self, array: Type, values: &[ValueId]) -> String {
        let mut current = "zeroinitializer".to_string();
        for (i, v) in values.iter().enumerate() {
            let expression = format!(
                "insertvalue {} {current}, {}, {i}",
                ty(array),
                self.value(*v)
            );
            if i + 1 == values.len() {
                return expression;
            }
            current = self.temp(&expression);
        }
        unreachable!("verified nonempty array")
    }
}
