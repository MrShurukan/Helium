//! Exact Timer1 CTC planning. Floating point is used only to present the report.
use crate::diagnostic::{Diagnostic, DiagnosticKind, LabelKind};
use crate::frontend::{
    statement::{Statement, StatementType},
    utils::SourceLocation,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerConfig {
    pub cpu_hz: u32,
    pub requested_ns: u64,
    pub tolerance_ns: u64,
    pub prescaler: u16,
    pub ticks: u32,
}

#[derive(Debug, Clone)]
pub enum TimerError {
    InvalidClock {
        hz: u32,
    },
    ZeroPeriod,
    MissingClock,
    Unsupported {
        hardware: String,
    },
    AlreadyReserved,
    NotValue {
        name: String,
    },
    Unrepresentable {
        cpu_hz: u32,
        requested_ns: u64,
        tolerance_ns: u64,
        shorter: Option<TimerConfig>,
        longer: Option<TimerConfig>,
        minimum_tolerance_ns: u128,
    },
}

impl std::fmt::Display for TimerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for TimerError {}

impl TimerConfig {
    pub fn cycles(&self) -> u64 {
        u64::from(self.prescaler) * u64::from(self.ticks)
    }

    pub fn clock_select(&self) -> u8 {
        match self.prescaler {
            1 => 1,
            8 => 2,
            64 => 3,
            256 => 4,
            1024 => 5,
            _ => unreachable!(),
        }
    }

    fn numerator(&self) -> u128 {
        u128::from(self.cycles()) * 1_000_000_000
    }

    fn error(&self) -> u128 {
        self.numerator()
            .abs_diff(u128::from(self.requested_ns) * u128::from(self.cpu_hz))
    }

    pub fn report(&self) -> String {
        let actual = self.cycles() as f64 / f64::from(self.cpu_hz);
        let delta =
            self.numerator() as i128 - i128::from(self.requested_ns) * i128::from(self.cpu_hz);
        let error = delta as f64 / f64::from(self.cpu_hz) / 1e9;
        format!("Timer1: CPU={} Hz, period={actual:.12}s, error={:+.6}ns ({:+.6}%), prescaler={}, OCR1A={}",
            self.cpu_hz, error * 1e9, error * 1e11 / self.requested_ns as f64, self.prescaler, self.ticks - 1)
    }
}

pub fn duration(value: u64, unit: &str) -> Option<u64> {
    value.checked_mul(match unit {
        "ns" => 1,
        "us" => 1000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        _ => return None,
    })
}

pub fn parse_duration(text: &str) -> Result<u64, String> {
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .ok_or("Expected a duration such as 1s, 10ms or 50us")?;
    let value = text[..split]
        .parse()
        .map_err(|_| "Expected an unsigned integer duration")?;
    duration(value, &text[split..])
        .ok_or_else(|| "Invalid duration unit or duration overflow; use integer ns/us/ms/s".into())
}

pub fn candidates(cpu_hz: u32, requested_ns: u64) -> Result<Vec<TimerConfig>, TimerError> {
    if cpu_hz == 0 || cpu_hz > 20_000_000 {
        return Err(TimerError::InvalidClock { hz: cpu_hz });
    }
    if requested_ns == 0 {
        return Err(TimerError::ZeroPeriod);
    }
    let desired = u128::from(requested_ns) * u128::from(cpu_hz);
    let mut result = Vec::new();
    for prescaler in [1, 8, 64, 256, 1024] {
        let quantum = u128::from(prescaler) * 1_000_000_000;
        let lower = desired / quantum;
        for ticks in [lower, lower + 1] {
            let config = TimerConfig {
                cpu_hz,
                requested_ns,
                tolerance_ns: 0,
                prescaler,
                ticks: ticks.clamp(1, 65536) as u32,
            };
            if !result.contains(&config) {
                result.push(config);
            }
        }
    }
    result.sort_by_key(|c| (c.error(), c.prescaler, c.ticks));
    Ok(result)
}

pub fn plan(cpu_hz: u32, requested_ns: u64, tolerance_ns: u64) -> Result<TimerConfig, TimerError> {
    let options = candidates(cpu_hz, requested_ns)?;
    let mut best = options[0].clone();
    best.tolerance_ns = tolerance_ns;
    if best.error() <= u128::from(tolerance_ns) * u128::from(cpu_hz) {
        return Ok(best);
    }

    let desired = u128::from(requested_ns) * u128::from(cpu_hz);
    let lower = options
        .iter()
        .filter(|c| c.numerator() < desired)
        .max_by_key(|c| c.numerator());
    let upper = options
        .iter()
        .filter(|c| c.numerator() > desired)
        .min_by_key(|c| c.numerator());
    let minimum = best.error().div_ceil(u128::from(cpu_hz));
    Err(TimerError::Unrepresentable {
        cpu_hz,
        requested_ns,
        tolerance_ns,
        shorter: lower.cloned(),
        longer: upper.cloned(),
        minimum_tolerance_ns: minimum,
    })
}

pub(crate) fn resolve(
    statements: &[Statement],
    cpu_hz: Option<u32>,
) -> Result<Option<TimerConfig>, Vec<Diagnostic>> {
    let mut selected = None;
    let mut previous: Option<SourceLocation> = None;
    let mut errors = Vec::new();
    for statement in statements {
        let StatementType::TimerDeclaration(timer) = &statement.statement_type else {
            continue;
        };
        let mut fail = |error: TimerError, location| {
            errors.push(Diagnostic::new(
                DiagnosticKind::Timer { error },
                Some(location),
            ))
        };
        if timer.hardware != "Timer1" {
            fail(
                TimerError::Unsupported {
                    hardware: timer.hardware.clone(),
                },
                timer.hardware_location,
            );
            continue;
        }
        if let Some(location) = previous {
            errors.push(
                Diagnostic::new(
                    DiagnosticKind::Timer {
                        error: TimerError::AlreadyReserved,
                    },
                    Some(timer.hardware_location),
                )
                .with_label(location, LabelKind::PreviousDeclaration),
            );
            continue;
        }
        previous = Some(timer.hardware_location);
        let Some(cpu_hz) = cpu_hz else {
            fail(TimerError::MissingClock, timer.period_location);
            continue;
        };
        match plan(cpu_hz, timer.every_ns, timer.tolerance_ns) {
            Ok(config) => selected = Some(config),
            Err(detail) => fail(detail, timer.period_location),
        }
    }
    if errors.is_empty() {
        Ok(selected)
    } else {
        Err(errors)
    }
}
