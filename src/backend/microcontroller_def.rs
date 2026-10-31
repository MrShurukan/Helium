use clap::ValueEnum;
use crate::backend::microcontroller::{GpioPort, Microcontroller, Timer1, Uart};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum SupportedMicrocontroller {
    Atmega328P
}

pub const ATMEGA328P: Microcontroller = Microcontroller {
    uart: Uart {
        name: "UART0",
        tx_port: "D",
        tx_bit: 1,
        status: 0xc0,
        control: 0xc1,
        format: 0xc2,
        baud_low: 0xc4,
        baud_high: 0xc5,
        data: 0xc6,
        divider_max: 4095,
        tx_enable: 8,
        format_8n1: 6,
        empty: 32,
        complete: 64,
        double_speed: 2,
    },
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