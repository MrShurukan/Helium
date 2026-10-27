//! The hardware subset currently supported by Helium. Data-space addresses are
//! explicit; SREG uses an I/O-space address for IN/OUT. See docs/TARGET.md.
#[derive(Debug)]
pub struct Microcontroller {
    pub name: &'static str,
    pub avr_dude_name: &'static str,
    pub llvm_triple: &'static str,
    pub max_cpu_hz: u32,
    pub flash_bytes: u32,
    pub sram_start: u32,
    pub sram_bytes: u32,
    pub sreg_io: u8,
    pub gpio: &'static [GpioPort],
    pub timer: Timer1,
}

#[derive(Debug)]
pub struct GpioPort {
    pub name: &'static str,
    pub digital_mask: u8,
    pub input: u16,
    pub direction: u16,
    pub output: u16,
}

#[derive(Debug)]
pub struct Timer1 {
    pub name: &'static str,
    pub compare_name: &'static str,
    pub vector: &'static str,
    pub max_ticks: u32,
    /// (clock divisor, CS12:0 encoding); external clock sources are unsupported.
    pub prescalers: &'static [(u16, u8)],
    pub control_a: u16,
    pub control_b: u16,
    pub control_c: u16,
    pub counter_high: u16,
    pub counter_low: u16,
    pub compare_high: u16,
    pub compare_low: u16,
    pub interrupt_mask: u16,
    pub interrupt_flags: u16,
    pub ctc_mode: u8,
    pub compare_interrupt: u8,
    /// W1C: ICF1, OCF1B, OCF1A and TOV1. Write directly, never read-modify-write.
    pub clear_flags: u8,
}

impl Timer1 {
    pub fn clock_select(&self, divisor: u16) -> Option<u8> {
        self.prescalers
            .iter()
            .find(|(d, _)| *d == divisor)
            .map(|(_, bits)| *bits)
    }
}

pub const ATMEGA328P: Microcontroller = Microcontroller {
    name: "atmega328p",
    avr_dude_name: "m328p",
    llvm_triple: "avr-unknown-unknown",
    max_cpu_hz: 20_000_000,
    flash_bytes: 32_768,
    sram_start: 0x100,
    sram_bytes: 2_048,
    sreg_io: 0x3f,
    gpio: &[
        GpioPort {
            name: "B",
            digital_mask: 0xff,
            input: 0x23,
            direction: 0x24,
            output: 0x25,
        },
        GpioPort {
            name: "C",
            digital_mask: 0x7f,
            input: 0x26,
            direction: 0x27,
            output: 0x28,
        },
        GpioPort {
            name: "D",
            digital_mask: 0xff,
            input: 0x29,
            direction: 0x2a,
            output: 0x2b,
        },
    ],
    timer: Timer1 {
        name: "Timer1",
        compare_name: "OCR1A",
        vector: "__vector_11",
        max_ticks: 65_536,
        prescalers: &[(1, 1), (8, 2), (64, 3), (256, 4), (1024, 5)],
        control_a: 0x80,
        control_b: 0x81,
        control_c: 0x82,
        counter_high: 0x85,
        counter_low: 0x84,
        compare_high: 0x89,
        compare_low: 0x88,
        interrupt_mask: 0x6f,
        interrupt_flags: 0x36,
        ctc_mode: 1 << 3,
        compare_interrupt: 1 << 1,
        clear_flags: (1 << 5) | (1 << 2) | (1 << 1) | 1,
    },
};

pub fn get(name: &str) -> Option<&'static Microcontroller> {
    (name == ATMEGA328P.name).then_some(&ATMEGA328P)
}
impl Microcontroller {
    pub fn port(&self, name: &str) -> Option<&GpioPort> {
        self.gpio.iter().find(|port| port.name == name)
    }

    pub fn has_digital_pin(&self, port: &str, number: usize) -> bool {
        number < 8
            && self
                .port(port)
                .is_some_and(|p| p.digital_mask & (1 << number) != 0)
    }
}

/// AVR SBI/CBI address encoding for the supported low-I/O GPIO registers.
/// No RMW fallback: pin independence is a language/backend contract.
pub fn atomic_bit_io(data_address: u16) -> Option<u8> {
    (0x20..=0x3f)
        .contains(&data_address)
        .then(|| (data_address - 0x20) as u8)
}
