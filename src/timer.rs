//! Exact Timer1 CTC planning. Floating point is used only to present the report.
use crate::backend::microcontroller::{Microcontroller, Timer1};
use crate::diagnostic::{Diagnostic, DiagnosticKind, LabelKind};
use crate::frontend::{
    statement::{Statement, StatementType},
    utils::SourceLocation,
};

/// Direction of the allowed period error, with exact matches included.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToleranceDirection {
    #[default]
    Either,
    NotLonger,
    NotShorter,
}
impl ToleranceDirection {
    pub fn sign(self) -> &'static str {
        match self {
            Self::Either => "",
            Self::NotLonger => "-",
            Self::NotShorter => "+",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerConfig {
    pub hardware: &'static Timer1,
    pub cpu_hz: u32,
    pub requested_ns: u64,
    pub tolerance_ns: u64,
    pub direction: ToleranceDirection,
    pub prescaler: u16,
    pub ticks: u32,
}

#[derive(Debug, Clone)]
pub enum TimerError {
    InvalidClock {
        hz: u32,
        target: &'static str,
        maximum: u32,
    },
    ZeroPeriod,
    MissingClock,
    Unsupported {
        hardware: String,
        supported: &'static str,
    },
    AlreadyReserved {
        hardware: &'static str,
    },
    NotValue {
        name: String,
    },
    Unrepresentable {
        cpu_hz: u32,
        requested_ns: u64,
        tolerance_ns: u64,
        shorter: Option<Box<TimerConfig>>,
        longer: Option<Box<TimerConfig>>,
        direction: ToleranceDirection,
        minimum_tolerance_ns: Option<u128>,
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
        self.hardware
            .clock_select(self.prescaler)
            .expect("TimerConfig must contain a supported prescaler")
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
        format!(
            "{}: CPU={} Hz, period={actual:.12}s, error={:+.6}ns ({:+.6}%), prescaler={}, {}={}, tolerance={}{}ns",
            self.hardware.name,
            self.cpu_hz,
            error * 1e9,
            error * 1e11 / self.requested_ns as f64,
            self.prescaler,
            self.hardware.compare_name,
            self.ticks - 1,
            self.direction.sign(),
            self.tolerance_ns
        )
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

pub fn validate_clock(target: &Microcontroller, cpu_hz: u32) -> Result<(), TimerError> {
    if cpu_hz == 0 || cpu_hz > target.max_cpu_hz {
        return Err(TimerError::InvalidClock {
            hz: cpu_hz,
            target: target.name,
            maximum: target.max_cpu_hz,
        });
    }
    Ok(())
}

pub fn candidates(
    target: &'static Microcontroller,
    cpu_hz: u32,
    requested_ns: u64,
) -> Result<Vec<TimerConfig>, TimerError> {
    validate_clock(target, cpu_hz)?;
    if requested_ns == 0 {
        return Err(TimerError::ZeroPeriod);
    }
    let desired = u128::from(requested_ns) * u128::from(cpu_hz);
    let mut result = Vec::new();
    for &(prescaler, _) in target.timer.prescalers {
        let quantum = u128::from(prescaler) * 1_000_000_000;
        let lower = desired / quantum;
        for ticks in [lower, lower + 1] {
            let config = TimerConfig {
                hardware: &target.timer,
                cpu_hz,
                requested_ns,
                tolerance_ns: 0,
                direction: ToleranceDirection::Either,
                prescaler,
                ticks: ticks.clamp(1, u128::from(target.timer.max_ticks)) as u32,
            };
            if !result.contains(&config) {
                result.push(config);
            }
        }
    }
    result.sort_by_key(|c| (c.error(), c.prescaler, c.ticks));
    Ok(result)
}

pub fn plan(
    target: &'static Microcontroller,
    cpu_hz: u32,
    requested_ns: u64,
    tolerance_ns: u64,
) -> Result<TimerConfig, TimerError> {
    plan_with_direction(
        target,
        cpu_hz,
        requested_ns,
        tolerance_ns,
        ToleranceDirection::Either,
    )
}

pub fn plan_with_direction(
    target: &'static Microcontroller,
    cpu_hz: u32,
    requested_ns: u64,
    tolerance_ns: u64,
    direction: ToleranceDirection,
) -> Result<TimerConfig, TimerError> {
    let options = candidates(target, cpu_hz, requested_ns)?;
    let desired = u128::from(requested_ns) * u128::from(cpu_hz);
    let best = options.iter().find(|c| match direction {
        ToleranceDirection::Either => true,
        ToleranceDirection::NotLonger => c.numerator() <= desired,
        ToleranceDirection::NotShorter => c.numerator() >= desired,
    });
    if let Some(best) = best {
        if best.error() <= u128::from(tolerance_ns) * u128::from(cpu_hz) {
            let mut selected = best.clone();
            selected.tolerance_ns = tolerance_ns;
            selected.direction = direction;
            return Ok(selected);
        }
    }
    // Candidates are already ordered by error and deterministic tie-breaks.
    let lower = options.iter().find(|c| c.numerator() < desired);
    let upper = options.iter().find(|c| c.numerator() > desired);
    Err(TimerError::Unrepresentable {
        cpu_hz,
        requested_ns,
        tolerance_ns,
        direction,
        shorter: lower.cloned().map(Box::new),
        longer: upper.cloned().map(Box::new),
        minimum_tolerance_ns: best.map(|c| c.error().div_ceil(u128::from(cpu_hz))),
    })
}

pub(crate) fn resolve(
    target: &'static Microcontroller,
    statements: &[Statement],
    cpu_hz: Option<u32>,
    reservations: &mut crate::resources::Reservations<SourceLocation>,
) -> Result<Option<TimerConfig>, Vec<Diagnostic>> {
    let mut selected = None;
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
        if timer.hardware != target.timer.name {
            fail(
                TimerError::Unsupported {
                    hardware: timer.hardware.clone(),
                    supported: target.timer.name,
                },
                timer.hardware_location,
            );
            continue;
        }
        use crate::resources::HardwareResource;
        let conflict = [
            HardwareResource::Peripheral(timer.hardware.clone()),
            HardwareResource::InterruptVector(target.timer.vector.into()),
        ]
        .into_iter()
        .find_map(|key| reservations.claim(key, timer.hardware_location).err());
        if let Some(location) = conflict {
            errors.push(
                Diagnostic::new(
                    DiagnosticKind::Timer {
                        error: TimerError::AlreadyReserved {
                            hardware: target.timer.name,
                        },
                    },
                    Some(timer.hardware_location),
                )
                .with_label(location, LabelKind::PreviousDeclaration),
            );
            continue;
        }
        let Some(cpu_hz) = cpu_hz else {
            fail(TimerError::MissingClock, timer.period_location);
            continue;
        };
        match plan_with_direction(
            target,
            cpu_hz,
            timer.every_ns,
            timer.tolerance_ns,
            timer.tolerance_direction,
        ) {
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
