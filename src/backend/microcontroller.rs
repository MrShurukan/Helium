//! The hardware subset currently supported by Helium. Data-space addresses are
//! explicit; SREG uses an I/O-space address for IN/OUT. See docs/TARGET.md.

pub use crate::backend::microcontroller_def::{SupportedMicrocontroller, ATMEGA328P};

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub uart: Uart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uart {
    pub name: &'static str,
    pub tx_port: &'static str,
    pub tx_bit: usize,
    pub status: u16,
    pub control: u16,
    pub format: u16,
    pub baud_low: u16,
    pub baud_high: u16,
    pub data: u16,
    pub divider_max: u16,
    pub tx_enable: u8,
    pub format_8n1: u8,
    pub empty: u8,
    pub complete: u8,
    pub double_speed: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpioPort {
    pub name: &'static str,
    pub digital_mask: u8,
    pub input: u16,
    pub direction: u16,
    pub output: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

pub fn get(microcontroller: SupportedMicrocontroller) -> &'static Microcontroller {
    match microcontroller {
        SupportedMicrocontroller::Atmega328P => &ATMEGA328P,
    }
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
