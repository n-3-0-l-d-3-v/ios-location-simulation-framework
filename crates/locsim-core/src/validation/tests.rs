use super::*;
use crate::consistency::{KinematicsDeriver, ObservationNoise};
use crate::domain::{
    Coordinate, LocationSource, MovementMode, MovementParameters, NoiseParameters,
    PlaybackParameters, SimulationState, CURRENT_SCHEMA_VERSION,
};

const SEC: i64 = 1_000_000_000;

fn origin() -> Coordinate {
    Coordinate::new(12.9352, 77.6245).unwrap()
}

fn sample(nanos: i64) -> SyntheticLocation {
    SyntheticLocation {
        timestamp: Timestamp::from_nanos(nanos),
        coordinate: origin(),
        altitude_m: 920.0,
        horizontal_accuracy_m: 5.0,
        vertical_accuracy_m: 8.0,
        speed_mps: Some(0.0),
        course_deg: None,
        source: LocationSource::Simulation,
        simulation_state: SimulationState::Running,
    }
}

/// A sample `metres` along `bearing` from the origin at `nanos`.
fn at_ns(nanos: i64, bearing: f64, metres: f64) -> SyntheticLocation {
    SyntheticLocation {
        coordinate: geographic::destination(origin(), bearing, metres).unwrap(),
        ..sample(nanos)
    }
}

fn at(seconds: i64, bearing: f64, metres: f64) -> SyntheticLocation {
    at_ns(seconds * SEC, bearing, metres)
}

/// Like [`at`] with explicit (not necessarily consistent) speed and course.
/// Used with the consistency stage switched off, to probe one stage at a time.
fn moving(seconds: i64, bearing: f64, metres: f64, speed: f64, course: f64) -> SyntheticLocation {
    SyntheticLocation {
        speed_mps: Some(speed),
        course_deg: Some(course),
        ..at(seconds, bearing, metres)
    }
}

/// Generous limits with no noise and no consistency stage, so each test
/// tightens only what it probes.
const QUIET: SampleLimits = SampleLimits {
    max_speed_mps: 50.0,
    position_noise_rate_mps: 0.0,
    boundary: None,
    max_acceleration_mps2: 1e9,
    max_deceleration_mps2: 1e9,
    max_heading_rate_dps: 1e9,
    speed_noise_reach_mps: 0.0,
    heading_noise_reach_deg: 0.0,
    consistency: None,
};

fn fenced() -> SampleValidator {
    SampleValidator::with_limits(SampleLimits {
        max_speed_mps: 3.0,
        boundary: Some(Boundary {
            center: origin(),
            radius_m: 100.0,
        }),
        ..QUIET
    })
}

// --- Stages 1 and 2 -----------------------------------------------------------

#[test]
fn accepts_increasing_timestamps() {
    let mut v = SampleValidator::new();
    for n in [0, 1, 2, 1_000_000_000] {
        assert_eq!(v.validate(&sample(n)), Ok(()));
    }
}

#[test]
fn rejects_equal_or_earlier_timestamps() {
    let mut v = SampleValidator::new();
    v.validate(&sample(100)).unwrap();
    for n in [100, 99] {
        assert_eq!(
            v.validate(&sample(n)),
            Err(ValidationError::NonMonotonicTimestamp {
                previous: Timestamp::from_nanos(100),
                current: Timestamp::from_nanos(n),
            })
        );
    }
}

#[test]
fn rejects_invalid_fields_without_advancing_state() {
    let mut v = SampleValidator::new();
    v.validate(&sample(100)).unwrap();
    let bad = SyntheticLocation {
        altitude_m: f64::NAN,
        ..sample(500)
    };
    assert_eq!(
        v.validate(&bad),
        Err(ValidationError::Location(LocationError::NonFinite(
            "altitude"
        )))
    );
    // 200 would be rejected had the bad sample at 500 been recorded.
    assert_eq!(v.validate(&sample(200)), Ok(()));
}

#[test]
fn reset_starts_a_new_stream() {
    let mut v = SampleValidator::new();
    v.validate(&sample(100)).unwrap();
    v.reset();
    assert_eq!(v.validate(&sample(50)), Ok(()));
}

#[test]
fn without_limits_any_displacement_passes() {
    let mut v = SampleValidator::new();
    v.validate(&at(0, 0.0, 0.0)).unwrap();
    assert_eq!(v.validate(&at(1, 0.0, 50_000.0)), Ok(()));
}

// --- Boundary and displacement --------------------------------------------------

#[test]
fn boundary_is_enforced() {
    let mut v = fenced();
    assert_eq!(v.validate(&at(0, 90.0, 99.999)), Ok(()));
    match v.validate(&at(1, 90.0, 100.001)) {
        Err(ValidationError::OutsideBoundary {
            distance_m,
            radius_m,
        }) => {
            assert!((distance_m - 100.001).abs() < 1e-6);
            assert_eq!(radius_m, 100.0);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn displacement_limit_scales_with_elapsed_time() {
    let mut v = fenced();
    v.validate(&at(0, 0.0, 0.0)).unwrap();
    // 3 m/s for 2 s allows 6 m.
    assert_eq!(v.validate(&at(2, 0.0, 5.999)), Ok(()));
    // From 5.999 m, one more second allows up to 8.999 m.
    match v.validate(&at(3, 0.0, 9.1)) {
        Err(ValidationError::ImpossibleDisplacement {
            distance_m,
            limit_m,
        }) => {
            assert!((distance_m - 3.101).abs() < 1e-6);
            assert_eq!(limit_m, 3.0);
        }
        other => panic!("unexpected {other:?}"),
    }
    // The rejected sample did not become the reference point.
    assert_eq!(v.validate(&at(3, 0.0, 8.9)), Ok(()));
}

#[test]
fn position_noise_rate_adds_to_the_displacement_limit() {
    let limits = SampleLimits {
        max_speed_mps: 2.0,
        position_noise_rate_mps: 1.0,
        ..QUIET
    };
    assert_eq!(limits.max_displacement_rate_mps(), 3.0);
    let mut v = SampleValidator::with_limits(limits);
    v.validate(&at(0, 0.0, 0.0)).unwrap();
    assert_eq!(v.validate(&at(1, 0.0, 2.99)), Ok(()));
    assert!(matches!(
        v.validate(&at(2, 0.0, 6.0)),
        Err(ValidationError::ImpossibleDisplacement { .. })
    ));
}

#[test]
fn zero_rate_requires_an_exactly_stationary_stream() {
    let mut v = SampleValidator::with_limits(SampleLimits {
        max_speed_mps: 0.0,
        ..QUIET
    });
    v.validate(&at(0, 0.0, 0.0)).unwrap();
    assert_eq!(v.validate(&at(1, 0.0, 0.0)), Ok(()));
    assert!(matches!(
        v.validate(&at(2, 0.0, 0.001)),
        Err(ValidationError::ImpossibleDisplacement { .. })
    ));
}

// --- Speed ------------------------------------------------------------------------

#[test]
fn reported_speed_is_bounded_by_motion_plus_noise() {
    // 2 m/s of motion, 1 m/s of position noise, 0.5 m/s of observation noise.
    let limits = SampleLimits {
        max_speed_mps: 2.0,
        position_noise_rate_mps: 1.0,
        speed_noise_reach_mps: 0.5,
        ..QUIET
    };
    let mut v = SampleValidator::with_limits(limits);
    // Checked on a first sample too.
    let first = SyntheticLocation {
        speed_mps: Some(3.6),
        course_deg: Some(0.0),
        ..sample(0)
    };
    assert!(matches!(
        v.validate(&first),
        Err(ValidationError::SpeedAboveMaximum { .. })
    ));
    v.validate(&moving(0, 0.0, 0.0, 3.4, 0.0)).unwrap();
    assert_eq!(v.validate(&moving(1, 0.0, 1.0, 3.5, 0.0)), Ok(()));
    match v.validate(&moving(2, 0.0, 2.0, 3.51, 0.0)) {
        Err(ValidationError::SpeedAboveMaximum { speed_mps, max_mps }) => {
            assert_eq!(speed_mps, 3.51);
            assert!((max_mps - 3.5).abs() < 1e-12);
        }
        other => panic!("unexpected {other:?}"),
    }
    // Unknown speed is not a violation.
    let unknown = SyntheticLocation {
        speed_mps: None,
        ..at(2, 0.0, 2.0)
    };
    assert_eq!(v.validate(&unknown), Ok(()));
}

// --- Acceleration and deceleration -------------------------------------------------

fn kinematic(acceleration: f64, deceleration: f64) -> SampleLimits {
    SampleLimits {
        max_acceleration_mps2: acceleration,
        max_deceleration_mps2: deceleration,
        // No turning allowed, so chords are as long as paths.
        max_heading_rate_dps: 0.0,
        ..QUIET
    }
}

#[test]
fn acceleration_is_judged_between_interval_midpoints() {
    // Speeds are interval means, so the time over which one may differ from
    // the next is the distance between the interval midpoints.
    let mut v = SampleValidator::with_limits(kinematic(1.0, 3.0));
    v.validate(&moving(0, 0.0, 0.0, 10.0, 0.0)).unwrap();
    // First comparison: no earlier interval is known, this one stands in. 2 s.
    assert_eq!(v.validate(&moving(2, 0.0, 20.0, 12.0, 0.0)), Ok(()));
    // Intervals of 2 s then 1 s: midpoints 1.5 s apart, so +1.5 m/s is fine…
    assert_eq!(v.validate(&moving(3, 0.0, 32.0, 13.5, 0.0)), Ok(()));
    // …where a naive `a × dt` would have refused it. Then 1 s and 1 s: 1 m/s.
    match v.validate(&moving(4, 0.0, 45.0, 14.6, 0.0)) {
        Err(ValidationError::AccelerationExceeded {
            change_mps,
            limit_mps,
        }) => {
            assert!((change_mps - 1.1).abs() < 1e-9);
            // 1.0 plus the resolution of two speeds derived over 1 s each.
            assert!(limit_mps > 1.0 && limit_mps < 1.0 + 1e-8, "{limit_mps}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(v.validate(&moving(4, 0.0, 45.0, 14.5, 0.0)), Ok(()));
    // A short interval after a long one is judged leniently, a long one
    // after a short one strictly: 0.5 s after 1 s gives 0.75 s.
    let half = SyntheticLocation {
        timestamp: Timestamp::from_nanos(4 * SEC + SEC / 2),
        ..moving(0, 0.0, 52.0, 15.25, 0.0)
    };
    assert_eq!(v.validate(&half), Ok(()));
}

#[test]
fn braking_has_its_own_limit() {
    let mut v = SampleValidator::with_limits(kinematic(1.0, 3.0));
    v.validate(&moving(0, 0.0, 0.0, 10.0, 0.0)).unwrap();
    v.validate(&moving(1, 0.0, 10.0, 10.0, 0.0)).unwrap();
    // −3 m/s in 1 s passes, −3.5 does not.
    assert_eq!(v.validate(&moving(2, 0.0, 17.0, 7.0, 0.0)), Ok(()));
    match v.validate(&moving(3, 0.0, 20.0, 3.5, 0.0)) {
        Err(ValidationError::DecelerationExceeded {
            change_mps,
            limit_mps,
        }) => {
            assert_eq!(change_mps, 3.5);
            assert!(limit_mps > 3.0 && limit_mps < 3.0 + 1e-8);
        }
        other => panic!("unexpected {other:?}"),
    }
    // A standing start to full speed in one sample is caught too.
    let mut v = SampleValidator::with_limits(kinematic(1.0, 3.0));
    v.validate(&at(0, 0.0, 0.0)).unwrap();
    assert!(matches!(
        v.validate(&moving(1, 0.0, 1.0, 30.0, 0.0)),
        Err(ValidationError::AccelerationExceeded { .. })
    ));
}

#[test]
fn turning_within_an_interval_may_shorten_the_chord() {
    // No braking allowed at all, but 60°/s of turning: over 1 s the path may
    // turn 60°, and its chord may then be cos(30°) of its length.
    let limits = SampleLimits {
        max_speed_mps: 10.0,
        max_acceleration_mps2: 0.0,
        max_deceleration_mps2: 0.0,
        max_heading_rate_dps: 60.0,
        ..QUIET
    };
    let shortfall = 10.0 * (1.0 - 30f64.to_radians().cos());
    assert!((limits.chord_shortfall_mps(1.0) - shortfall).abs() < 1e-12);
    assert!((shortfall - 1.3397).abs() < 1e-4);
    // Half a turn or more within the interval: the chord can vanish.
    assert_eq!(limits.chord_shortfall_mps(10.0), 10.0);
    assert_eq!(QUIET.chord_shortfall_mps(1.0), 50.0);
    assert_eq!(kinematic(1.0, 1.0).chord_shortfall_mps(1.0), 0.0);

    let mut v = SampleValidator::with_limits(limits);
    v.validate(&moving(0, 0.0, 0.0, 10.0, 0.0)).unwrap();
    v.validate(&moving(1, 0.0, 10.0, 10.0, 0.0)).unwrap();
    assert_eq!(v.validate(&moving(2, 0.0, 18.7, 8.7, 0.0)), Ok(()));
    let mut v = SampleValidator::with_limits(limits);
    v.validate(&moving(0, 0.0, 0.0, 10.0, 0.0)).unwrap();
    v.validate(&moving(1, 0.0, 10.0, 10.0, 0.0)).unwrap();
    assert!(matches!(
        v.validate(&moving(2, 0.0, 18.6, 8.6, 0.0)),
        Err(ValidationError::DecelerationExceeded { .. })
    ));
}

#[test]
fn noise_adds_exactly_its_reach_to_the_speed_change_limit() {
    // 0.5 m/s of position noise can add 0.5 to each derived speed, and
    // observation noise of reach 0.2 another 0.2 to each: 1.4 in total.
    let limits = SampleLimits {
        position_noise_rate_mps: 0.5,
        speed_noise_reach_mps: 0.2,
        ..kinematic(1.0, 1.0)
    };
    let mut v = SampleValidator::with_limits(limits);
    v.validate(&moving(0, 0.0, 0.0, 10.0, 0.0)).unwrap();
    v.validate(&moving(1, 0.0, 10.0, 10.0, 0.0)).unwrap();
    assert_eq!(v.validate(&moving(2, 0.0, 20.0, 12.39, 0.0)), Ok(()));
    assert!(matches!(
        v.validate(&moving(3, 0.0, 30.0, 14.8, 0.0)),
        Err(ValidationError::AccelerationExceeded { .. })
    ));
}

#[test]
fn coordinate_resolution_bounds_what_can_be_said_at_high_rates() {
    // At 1 MHz a derived speed is only known to 4 nm / 1 µs = 4 mm/s, so two
    // of them may differ by 8 mm/s with no acceleration at all.
    let mut v = SampleValidator::with_limits(kinematic(0.0, 0.0));
    let micro = |n: i64, speed: f64| SyntheticLocation {
        speed_mps: Some(speed),
        course_deg: Some(0.0),
        ..at_ns(n * 1_000, 0.0, n as f64 * 1e-5)
    };
    v.validate(&micro(0, 10.0)).unwrap();
    v.validate(&micro(1, 10.0)).unwrap();
    assert_eq!(v.validate(&micro(2, 10.007)), Ok(()));
    match v.validate(&micro(3, 10.016)) {
        Err(ValidationError::AccelerationExceeded { limit_mps, .. }) => {
            assert!((limit_mps - 0.008).abs() < 1e-9, "{limit_mps}");
        }
        other => panic!("unexpected {other:?}"),
    }
    // At 1 Hz the same allowance is 8 nm/s: irrelevant.
    let mut v = SampleValidator::with_limits(kinematic(0.0, 0.0));
    v.validate(&moving(0, 0.0, 0.0, 10.0, 0.0)).unwrap();
    v.validate(&moving(1, 0.0, 10.0, 10.0, 0.0)).unwrap();
    assert!(v.validate(&moving(2, 0.0, 20.0, 10.000001, 0.0)).is_err());
}

#[test]
fn unknown_speed_or_course_skips_the_rate_checks() {
    let mut v = SampleValidator::with_limits(SampleLimits {
        max_heading_rate_dps: 0.0,
        ..kinematic(0.0, 0.0)
    });
    v.validate(&moving(0, 0.0, 0.0, 1.0, 10.0)).unwrap();
    let unknown = SyntheticLocation {
        speed_mps: None,
        course_deg: None,
        ..at(1, 0.0, 1.0)
    };
    assert_eq!(v.validate(&unknown), Ok(()));
    assert_eq!(v.validate(&moving(2, 0.0, 2.0, 2.0, 200.0)), Ok(()));
}

// --- Heading rate ---------------------------------------------------------------------

fn turning(rate: f64) -> SampleLimits {
    SampleLimits {
        max_heading_rate_dps: rate,
        ..QUIET
    }
}

#[test]
fn heading_change_is_judged_over_both_intervals() {
    // Chord directions lie within the range of directions travelled, so two
    // consecutive chords may differ by the turning possible over both
    // intervals: 10°/s × (1 s + 1 s) = 20°.
    let mut v = SampleValidator::with_limits(turning(10.0));
    v.validate(&moving(0, 0.0, 0.0, 1.0, 350.0)).unwrap();
    v.validate(&moving(1, 0.0, 1.0, 1.0, 350.0)).unwrap();
    // 350° → 9.99° is a 19.99° turn through north.
    assert_eq!(v.validate(&moving(2, 0.0, 2.0, 1.0, 9.99)), Ok(()));
    match v.validate(&moving(3, 0.0, 3.0, 1.0, 30.1)) {
        Err(ValidationError::HeadingRateExceeded {
            turn_deg,
            limit_deg,
        }) => {
            assert!((turn_deg - 20.11).abs() < 1e-6, "{turn_deg}");
            // 20° plus the resolution of two courses over 1 m chords.
            assert!(limit_deg > 20.0 && limit_deg < 20.0 + 1e-6, "{limit_deg}");
        }
        other => panic!("unexpected {other:?}"),
    }
    // An about-turn in one sample is the classic discontinuity.
    assert!(matches!(
        v.validate(&moving(3, 0.0, 3.0, 1.0, 189.0)),
        Err(ValidationError::HeadingRateExceeded { .. })
    ));
    // With half a turn or more available nothing can be a violation.
    assert_eq!(v.validate(&moving(30, 0.0, 3.5, 1.0, 189.0)), Ok(()));
}

#[test]
fn straight_travel_near_a_pole_is_not_mistaken_for_turning() {
    // Drive 600 m along one geodesic 1 km from the north pole, in 100 m
    // steps. The bearing swings by tens of degrees, yet nothing turns.
    let limits = SampleLimits {
        max_speed_mps: 200.0,
        ..turning(0.001)
    };
    let mut v = SampleValidator::with_limits(limits);
    let mut position = Coordinate::new(89.991, 10.0).unwrap();
    let mut course = 90.0;
    let mut swing = 0.0f64;
    for step in 0..7 {
        let fix = SyntheticLocation {
            coordinate: position,
            speed_mps: Some(100.0),
            course_deg: Some(course),
            ..sample(step * SEC)
        };
        assert_eq!(v.validate(&fix), Ok(()), "step {step}");
        let (next, arrival) = geographic::direct(position, course, 100.0).unwrap();
        swing += bearing_difference(course, arrival).abs();
        position = next;
        course = arrival;
    }
    assert!(swing > 30.0, "bearing only swung {swing} degrees");

    // A real 5° turn on top of the same geometry is still caught.
    let (next, arrival) = geographic::direct(position, course, 100.0).unwrap();
    let turned = SyntheticLocation {
        coordinate: next,
        speed_mps: Some(100.0),
        course_deg: Some(geographic::normalize_bearing(arrival + 5.0)),
        ..sample(8 * SEC)
    };
    match v.validate(&turned) {
        Err(ValidationError::HeadingRateExceeded { turn_deg, .. }) => {
            assert!((turn_deg - 5.0).abs() < 1e-6, "{turn_deg}");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn position_noise_can_deflect_a_chord_only_so_far() {
    // Noise of 0.5 m/s over 1 s moves each end by up to 0.5 m.
    let limits = SampleLimits {
        position_noise_rate_mps: 0.5,
        ..turning(0.0)
    };
    // A 10 m chord can be swung by asin(0.5 / 9.5) = 3.017°.
    assert!((limits.noise_deflection_deg(10.0, 1.0) - 3.0170).abs() < 1e-3);
    // A 1 m chord is within reach of the noise: any direction.
    assert_eq!(limits.noise_deflection_deg(1.0, 1.0), 180.0);
    assert_eq!(limits.noise_deflection_deg(0.0, 1.0), 180.0);
    assert_eq!(turning(0.0).noise_deflection_deg(10.0, 1.0), 0.0);

    // Two 10 m chords: 6.03° of apparent turning is noise, more is not.
    let mut v = SampleValidator::with_limits(limits);
    v.validate(&moving(0, 0.0, 0.0, 10.0, 0.0)).unwrap();
    v.validate(&moving(1, 0.0, 10.0, 10.0, 0.0)).unwrap();
    assert_eq!(v.validate(&moving(2, 0.0, 20.0, 10.0, 6.0)), Ok(()));
    assert!(matches!(
        v.validate(&moving(3, 0.0, 30.0, 10.0, 12.1)),
        Err(ValidationError::HeadingRateExceeded { .. })
    ));
    // Short chords: the direction says nothing, so nothing is rejected.
    let mut v = SampleValidator::with_limits(limits);
    v.validate(&moving(0, 0.0, 0.0, 0.9, 0.0)).unwrap();
    v.validate(&moving(1, 0.0, 0.9, 0.9, 0.0)).unwrap();
    assert_eq!(v.validate(&moving(2, 0.0, 1.8, 0.9, 170.0)), Ok(()));
}

#[test]
fn heading_observation_noise_adds_twice_its_reach() {
    let limits = SampleLimits {
        heading_noise_reach_deg: 2.0,
        ..turning(10.0)
    };
    let mut v = SampleValidator::with_limits(limits);
    v.validate(&moving(0, 0.0, 0.0, 1.0, 100.0)).unwrap();
    v.validate(&moving(1, 0.0, 1.0, 1.0, 100.0)).unwrap();
    assert_eq!(v.validate(&moving(2, 0.0, 2.0, 1.0, 123.9)), Ok(()));
    assert!(matches!(
        v.validate(&moving(3, 0.0, 3.0, 1.0, 148.0)),
        Err(ValidationError::HeadingRateExceeded { .. })
    ));
}

// --- Consistency stage -------------------------------------------------------------------

/// Limits of a 2 m/s walker with the consistency stage on.
fn consistent_limits() -> SampleLimits {
    SampleLimits {
        max_speed_mps: 2.0,
        max_acceleration_mps2: 1.0,
        max_deceleration_mps2: 1.0,
        max_heading_rate_dps: 45.0,
        consistency: Some(ConsistencyTolerance::EXACT),
        ..QUIET
    }
}

/// Fixes with metadata derived from the positions themselves.
fn derived(fixes: &[(i64, f64, f64)]) -> Vec<SyntheticLocation> {
    let mut deriver = KinematicsDeriver::new();
    fixes
        .iter()
        .map(|&(seconds, bearing, metres)| {
            let fix = at(seconds, bearing, metres);
            let k = deriver
                .derive(fix.timestamp, fix.coordinate, ObservationNoise::NONE)
                .unwrap();
            deriver.accept(fix.timestamp, fix.coordinate);
            SyntheticLocation {
                speed_mps: k.speed_mps,
                course_deg: k.course_deg,
                ..fix
            }
        })
        .collect()
}

#[test]
fn a_stream_with_derived_metadata_passes_every_stage() {
    // Accelerate north, cruise, stop.
    let stream = derived(&[
        (0, 0.0, 0.0),
        (1, 0.0, 0.5),
        (2, 0.0, 1.9),
        (3, 0.0, 3.8),
        (4, 0.0, 5.7),
        (5, 0.0, 6.9),
        (6, 0.0, 7.3),
        (7, 0.0, 7.3),
    ]);
    assert_eq!((stream[0].speed_mps, stream[0].course_deg), (None, None));
    assert_eq!(
        (stream[7].speed_mps, stream[7].course_deg),
        (Some(0.0), None)
    );
    let mut v = SampleValidator::with_limits(consistent_limits());
    for s in &stream {
        assert_eq!(v.validate(s), Ok(()));
    }
}

#[test]
fn inconsistent_metadata_is_rejected_by_the_gate() {
    let stream = derived(&[(0, 0.0, 0.0), (1, 0.0, 0.5), (2, 0.0, 1.9), (3, 0.0, 3.8)]);
    let run_to = |n: usize| {
        let mut v = SampleValidator::with_limits(consistent_limits());
        for s in &stream[..n] {
            v.validate(s).unwrap();
        }
        v
    };
    // A first sample claiming a speed.
    let first = SyntheticLocation {
        speed_mps: Some(0.0),
        ..stream[0]
    };
    assert_eq!(
        run_to(0).validate(&first),
        Err(ValidationError::Inconsistent(
            ConsistencyError::FirstSampleHasKinematics
        ))
    );
    // Speed that the positions do not support.
    let fast = SyntheticLocation {
        speed_mps: Some(1.5),
        ..stream[2]
    };
    assert!(matches!(
        run_to(2).validate(&fast),
        Err(ValidationError::Inconsistent(
            ConsistencyError::SpeedMismatch { .. }
        ))
    ));
    // Course that the positions do not support.
    let turned = SyntheticLocation {
        course_deg: Some(90.0),
        ..stream[3]
    };
    assert!(matches!(
        run_to(3).validate(&turned),
        Err(ValidationError::Inconsistent(
            ConsistencyError::CourseMismatch { .. }
        ))
    ));
    // The untouched samples are still fine afterwards.
    let mut v = run_to(3);
    assert!(v.validate(&turned).is_err());
    assert_eq!(v.validate(&stream[3]), Ok(()));
}

#[test]
fn a_teleport_with_honest_metadata_is_still_a_teleport() {
    // Metadata derived from an impossible jump is consistent with it; the
    // displacement stage, which comes first, is what refuses the jump.
    let stream = derived(&[(0, 0.0, 0.0), (1, 0.0, 1.0), (2, 0.0, 500.0)]);
    let mut v = SampleValidator::with_limits(consistent_limits());
    v.validate(&stream[0]).unwrap();
    v.validate(&stream[1]).unwrap();
    assert!(matches!(
        v.validate(&stream[2]),
        Err(ValidationError::ImpossibleDisplacement { .. })
    ));
}

#[test]
fn consistent_metadata_does_not_excuse_impossible_kinematics() {
    // Positions that imply 0 → 1.9 m/s in a second, honestly reported.
    let stream = derived(&[(0, 0.0, 0.0), (1, 0.0, 0.0), (2, 0.0, 1.9)]);
    let mut v = SampleValidator::with_limits(consistent_limits());
    v.validate(&stream[0]).unwrap();
    v.validate(&stream[1]).unwrap();
    assert!(matches!(
        v.validate(&stream[2]),
        Err(ValidationError::AccelerationExceeded { .. })
    ));
    // Positions that double back at walking pace, honestly reported.
    let stream = derived(&[(0, 0.0, 0.0), (1, 0.0, 0.5), (2, 0.0, 1.5), (3, 0.0, 0.5)]);
    let mut v = SampleValidator::with_limits(SampleLimits {
        max_acceleration_mps2: 10.0,
        max_deceleration_mps2: 10.0,
        ..consistent_limits()
    });
    for s in &stream[..3] {
        v.validate(s).unwrap();
    }
    match v.validate(&stream[3]) {
        Err(ValidationError::HeadingRateExceeded { turn_deg, .. }) => {
            assert!((turn_deg - 180.0).abs() < 1e-6);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn limits_are_derived_from_the_scenario() {
    let mut movement = MovementParameters::walking_preset();
    movement.radius_m = Some(75.0);
    movement.max_displacement_per_sample_m = Some(0.5);
    let scenario = Scenario {
        schema_version: CURRENT_SCHEMA_VERSION,
        name: "limits".into(),
        origin: origin(),
        altitude_m: 0.0,
        mode: MovementMode::Walking,
        movement,
        noise: NoiseParameters {
            position_noise_m: 1.0,
            max_position_offset_m: 6.0,
            speed_noise_mps: 0.1,
            heading_noise_deg: 2.0,
            accuracy_noise_m: 0.5,
            drift_rate_mps: 0.0,
            position_correlation_time_s: 3.0,
            max_offset_rate_mps: 0.7,
        },
        horizontal_accuracy_m: 5.0,
        vertical_accuracy_m: 8.0,
        update_interval_s: 0.5,
        seed: 1,
        route: None,
        playback: PlaybackParameters::REAL_TIME,
    };
    assert_eq!(scenario.validate(), Ok(()));
    let limits = SampleLimits::for_scenario(&scenario);
    // 0.5 m per 0.5 s caps the 1.8 m/s walker at 1 m/s.
    assert_eq!(limits.max_speed_mps, 1.0);
    assert_eq!(limits.position_noise_rate_mps, 0.7);
    assert_eq!(limits.max_displacement_rate_mps(), 1.7);
    assert_eq!(limits.boundary.unwrap().radius_m, 75.0);
    assert_eq!(
        (
            limits.max_acceleration_mps2,
            limits.max_deceleration_mps2,
            limits.max_heading_rate_dps
        ),
        (0.8, 1.2, 60.0)
    );
    assert!((limits.speed_noise_reach_mps - 0.3).abs() < 1e-12);
    assert_eq!(limits.heading_noise_reach_deg, 6.0);
    let tolerance = limits.consistency.unwrap();
    assert!((tolerance.speed_noise_reach_mps - 0.3).abs() < 1e-12);
    let band = tolerance.accuracy.unwrap();
    assert_eq!(
        (band.horizontal_m, band.vertical_m, band.reach_m),
        (5.0, 8.0, 1.5)
    );

    // Without position noise its rate does not count.
    let mut quiet = scenario.clone();
    quiet.noise = NoiseParameters::NONE;
    assert_eq!(
        SampleLimits::for_scenario(&quiet).position_noise_rate_mps,
        0.0
    );
}

#[test]
fn a_fix_held_at_the_fence_has_an_unbounded_noise_step() {
    // With position noise and a fence, the noise engine may have to pull a
    // fix back onto the fence; such a fix says nothing about turning.
    let fence = Boundary {
        center: origin(),
        radius_m: 50.0,
    };
    let limits = SampleLimits {
        position_noise_rate_mps: 0.1,
        boundary: Some(fence),
        ..turning(1.0)
    };
    // 49.99994 m is where the engine parks a pulled fix (one margin inside).
    let parked = 50.0 * (1.0 - KINEMATIC_MARGIN) - 1e-7;
    assert!(held_at_fence(&limits, Some(parked)));
    assert!(held_at_fence(&limits, Some(50.0)));
    assert!(!held_at_fence(&limits, Some(49.99)));
    assert!(!held_at_fence(&limits, None));
    // Without position noise nothing is ever "held".
    let quiet = SampleLimits {
        position_noise_rate_mps: 0.0,
        ..limits
    };
    assert!(!held_at_fence(&quiet, Some(50.0)));

    // Away from the fence a sharp turn is rejected…
    let mut v = SampleValidator::with_limits(limits);
    v.validate(&moving(0, 90.0, 10.0, 5.0, 90.0)).unwrap();
    v.validate(&moving(1, 90.0, 15.0, 5.0, 90.0)).unwrap();
    assert!(matches!(
        v.validate(&moving(2, 90.0, 20.0, 5.0, 200.0)),
        Err(ValidationError::HeadingRateExceeded { .. })
    ));
    // …but not when the fix sits on the fence, nor for the sample after it.
    let mut v = SampleValidator::with_limits(limits);
    v.validate(&moving(0, 90.0, 40.0, 5.0, 90.0)).unwrap();
    v.validate(&moving(1, 90.0, 45.0, 5.0, 90.0)).unwrap();
    assert_eq!(v.validate(&moving(2, 90.0, parked, 5.0, 200.0)), Ok(()));
    assert_eq!(v.validate(&moving(3, 90.0, 45.0, 5.0, 20.0)), Ok(()));
    // After that the stage applies again.
    assert!(matches!(
        v.validate(&moving(4, 90.0, 40.0, 5.0, 170.0)),
        Err(ValidationError::HeadingRateExceeded { .. })
    ));
}
