use crate::{
    backend::microcontroller::{Microcontroller, Uart},
    diagnostic::{Diagnostic, DiagnosticKind, LabelKind},
    frontend::{
        statement::{Statement, StatementType},
        utils::SourceLocation,
    },
    resources::{HardwareResource, Reservations},
    semantic::SymbolId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UartConfig {
    pub hardware: &'static Uart,
    pub cpu_hz: u32,
    pub baud: u32,
    pub divider: u16,
    pub double_speed: bool,
}
impl UartConfig {
    pub fn denominator(&self) -> u64 {
        (u64::from(self.divider) + 1) * if self.double_speed { 8 } else { 16 }
    }
    pub fn status_bits(&self) -> u8 {
        if self.double_speed {
            self.hardware.double_speed
        } else {
            0
        }
    }
    pub fn report(&self) -> String {
        let actual = f64::from(self.cpu_hz) / self.denominator() as f64;
        format!("{}: TX 8N1, requested={} baud, actual={actual:.6} baud, error={:+.6}%, UBRR={}, U2X={}", self.hardware.name, self.baud, (actual / f64::from(self.baud) - 1.) * 100., self.divider, u8::from(self.double_speed))
    }
}

pub fn plan(
    target: &'static Microcontroller,
    cpu_hz: u32,
    baud: u64,
) -> Result<UartConfig, String> {
    crate::timer::validate_clock(target, cpu_hz).map_err(|e| e.to_string())?;
    let baud = u32::try_from(baud)
        .ok()
        .filter(|b| *b != 0)
        .ok_or("Baud must be a positive 32-bit integer")?;
    let mut best: Option<(u64, u64, UartConfig)> = None;
    for double_speed in [false, true] {
        for divider in 0..=target.uart.divider_max {
            let config = UartConfig {
                hardware: &target.uart,
                cpu_hz,
                baud,
                divider,
                double_speed,
            };
            let denominator = config.denominator();
            let error = u64::from(cpu_hz).abs_diff(u64::from(baud) * denominator);
            if best.as_ref().is_none_or(|(previous, den, _)| {
                u128::from(error) * u128::from(*den)
                    < u128::from(*previous) * u128::from(denominator)
            }) {
                best = Some((error, denominator, config));
            }
        }
    }
    let (error, denominator, config) = best.ok_or("UART has no baud configurations")?;
    if u128::from(error) * 100 > u128::from(baud) * u128::from(denominator) * 2 {
        return Err(format!(
            "UART baud error exceeds the supported 2% limit: {}",
            config.report()
        ));
    }
    Ok(config)
}

pub(crate) fn resolve(
    target: &'static Microcontroller,
    statements: &[Statement],
    cpu_hz: Option<u32>,
    reservations: &mut Reservations<SourceLocation>,
) -> Result<Vec<(SymbolId, UartConfig, SourceLocation)>, Vec<Diagnostic>> {
    let mut result = vec![];
    let mut errors = vec![];
    for statement in statements {
        let StatementType::DeviceDeclaration(device) = &statement.statement_type else {
            continue;
        };
        if device.hardware != target.uart.name {
            errors.push(Diagnostic::new(
                DiagnosticKind::Device {
                    detail: format!(
                        "Unsupported device '{}'; expected {}",
                        device.hardware, target.uart.name
                    ),
                },
                Some(device.hardware_location),
            ));
            continue;
        }
        for key in [
            HardwareResource::Peripheral(device.hardware.clone()),
            HardwareResource::Pin {
                port: target.uart.tx_port.into(),
                bit: target.uart.tx_bit,
            },
        ] {
            if let Err(previous) = reservations.claim(key, device.hardware_location) {
                errors.push(
                    Diagnostic::new(
                        DiagnosticKind::Device {
                            detail: "UART or its TX pin is already reserved".into(),
                        },
                        Some(device.hardware_location),
                    )
                    .with_label(previous, LabelKind::PreviousDeclaration),
                );
            }
        }
        let planned = cpu_hz
            .ok_or_else(|| "UART requires an explicit --cpu-hz".to_string())
            .and_then(|hz| plan(target, hz, device.baud));
        match planned {
            Ok(config) => {
                result.push((statement.symbol.unwrap(), config, statement.source_location))
            }
            Err(detail) => errors.push(Diagnostic::new(
                DiagnosticKind::Device { detail },
                Some(device.baud_location),
            )),
        }
    }
    if errors.is_empty() {
        Ok(result)
    } else {
        Err(errors)
    }
}
