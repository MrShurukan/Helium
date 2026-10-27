use helium::{
    analyze, compile_typed_ir,
    ir::{self, Options},
    timer::{self, TimerError, ToleranceDirection as D},
};

#[test]
fn direction_filters_before_selecting_the_nearest_period() {
    // 16 MHz: 62.5 ns per tick. The closest period overall is 125 ns,
    // but a negative tolerance must choose 62.5 ns for every=100 ns.
    let either = timer::plan(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        100,
        38,
    )
    .unwrap();
    let down = timer::plan_with_direction(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        100,
        38,
        D::NotLonger,
    )
    .unwrap();
    let up = timer::plan_with_direction(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        100,
        25,
        D::NotShorter,
    )
    .unwrap();
    assert_eq!((either.ticks, down.ticks, up.ticks), (2, 1, 2));
    assert!(timer::plan_with_direction(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        100,
        37,
        D::NotLonger
    )
    .is_err());
    assert!(timer::plan_with_direction(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        100,
        24,
        D::NotShorter
    )
    .is_err());
    for direction in [D::Either, D::NotLonger, D::NotShorter] {
        assert_eq!(
            timer::plan_with_direction(
                &helium::backend::microcontroller::ATMEGA328P,
                16_000_000,
                125,
                0,
                direction
            )
            .unwrap()
            .ticks,
            2
        );
    }
}

#[test]
fn impossible_side_is_distinct_from_insufficient_tolerance() {
    for (period, direction) in [(1, D::NotLonger), (u64::MAX, D::NotShorter)] {
        let error = timer::plan_with_direction(
            &helium::backend::microcontroller::ATMEGA328P,
            16_000_000,
            period,
            u64::MAX,
            direction,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            TimerError::Unrepresentable {
                minimum_tolerance_ns: None,
                ..
            }
        ));
        assert!(error
            .to_string()
            .contains("increasing tolerance cannot help"));
    }
    let error = timer::plan_with_direction(
        &helium::backend::microcontroller::ATMEGA328P,
        16_000_000,
        100,
        30,
        D::NotLonger,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        TimerError::Unrepresentable {
            minimum_tolerance_ns: Some(38),
            ..
        }
    ));
    assert!(error.to_string().contains("tolerance: -30ns"));
}

#[test]
fn planner_matches_exhaustive_hardware_search() {
    // Independent brute-force oracle over every counter value and divisor.
    for (hz, period) in [
        (16_000_000, 100),
        (16_000_000, 125),
        (3_000_000, 1001),
        (16_000_000, 4_096_001),
        (16_000_000, 4_194_304_001),
        (1, u64::MAX),
    ] {
        for direction in [D::Either, D::NotLonger, D::NotShorter] {
            let target = u128::from(period) * u128::from(hz);
            let mut best = None;
            for divisor in [1u16, 8, 64, 256, 1024] {
                for ticks in 1..=65_536u32 {
                    let value = u128::from(divisor) * u128::from(ticks) * 1_000_000_000;
                    if (direction == D::NotLonger && value > target)
                        || (direction == D::NotShorter && value < target)
                    {
                        continue;
                    }
                    let key = (value.abs_diff(target), divisor, ticks);
                    if best.is_none_or(|old| key < old) {
                        best = Some(key);
                    }
                }
            }
            match (
                best,
                timer::plan_with_direction(
                    &helium::backend::microcontroller::ATMEGA328P,
                    hz,
                    period,
                    u64::MAX,
                    direction,
                ),
            ) {
                (Some((_, d, t)), Ok(actual)) => {
                    assert_eq!((actual.prescaler, actual.ticks), (d, t))
                }
                (
                    None,
                    Err(TimerError::Unrepresentable {
                        minimum_tolerance_ns: None,
                        ..
                    }),
                ) => {}
                other => panic!("oracle mismatch: {other:?}"),
            }
        }
    }
}

#[test]
fn signed_tolerance_survives_source_ir_reports_and_verification() {
    for (sign, expected, direction) in [
        ("", 2, D::Either),
        ("-", 1, D::NotLonger),
        ("+", 2, D::NotShorter),
    ] {
        let source = format!(
            "timer clock=Timer1 {{every:100ns, tolerance:{sign}38ns, interrupt {{}}}} loop {{}}"
        );
        let program = analyze(&source).unwrap();
        let displayed = program.statements()[0].statement_type.to_string();
        assert!(displayed.contains(&format!("tolerance: {sign}38ns")));
        analyze(&format!("{displayed} loop {{}}")).unwrap();
        for optimize in [false, true] {
            let mut module = compile_typed_ir(
                &source,
                "atmega328p",
                Options {
                    cpu_hz: Some(16_000_000),
                    optimize,
                    ..Options::default()
                },
            )
            .unwrap();
            let config = module.devices[0].timer();
            assert_eq!((config.ticks, config.direction), (expected, direction));
            assert!(config.report().contains(&format!("tolerance={sign}38ns")));
            if direction == D::NotLonger {
                module.devices[0].timer_mut().ticks = 2;
                assert!(ir::verify::verify(&module).is_err());
            }
        }
    }
    for tolerance in ["-0ns", "+0ns"] {
        compile_typed_ir(
            &format!(
                "timer t=Timer1 {{every:125ns, tolerance:{tolerance}, interrupt {{}}}} loop {{}}"
            ),
            "atmega328p",
            Options {
                cpu_hz: Some(16_000_000),
                ..Options::default()
            },
        )
        .unwrap();
    }
}

#[test]
fn signs_are_allowed_only_once_and_only_for_tolerance() {
    for tolerance in [
        "--1ns",
        "+-1ns",
        "-+1ns",
        "++1ns",
        "-",
        "+",
        "-1.5ms",
        "+18446744073709551615s",
    ] {
        assert!(analyze(&format!(
            "timer t=Timer1 {{every:1ms, tolerance:{tolerance}, interrupt {{}}}} loop {{}}"
        ))
        .is_err());
    }
    for period in ["-1ms", "+1ms"] {
        assert!(analyze(&format!(
            "timer t=Timer1 {{every:{period}, interrupt {{}}}} loop {{}}"
        ))
        .is_err());
    }
    for sign in ["-", "+"] {
        let source = format!(
            "timer t=Timer1 {{tolerance: {sign} 1ms, every:1ms, interrupt {{}}}} loop {{}}"
        );
        analyze(&source).unwrap();
        for (end, _) in source.char_indices() {
            let _ = analyze(&source[..end]);
        }
    }
}
