use super::*;
use crate::domain::{LocationSource, SimulationState};
use crate::geographic::{destination, distance, inverse};
use crate::rng::Rng;

const START: i64 = 1_700_000_000_000_000_000;
const SEC: i64 = 1_000_000_000;

fn origin() -> Coordinate {
    Coordinate::new(12.9352, 77.6245).unwrap()
}

fn t(ns: i64) -> Timestamp {
    Timestamp::from_nanos(START + ns)
}

fn sample(ns: i64, coordinate: Coordinate, k: DerivedKinematics) -> SyntheticLocation {
    SyntheticLocation {
        timestamp: t(ns),
        coordinate,
        altitude_m: 10.0,
        horizontal_accuracy_m: 5.0,
        vertical_accuracy_m: 8.0,
        speed_mps: k.speed_mps,
        course_deg: k.course_deg,
        source: LocationSource::Simulation,
        simulation_state: SimulationState::Running,
    }
}

/// A stream with consistent metadata through the given `(ns, position)` fixes.
fn stream(fixes: &[(i64, Coordinate)]) -> Vec<SyntheticLocation> {
    let mut deriver = KinematicsDeriver::new();
    fixes
        .iter()
        .map(|&(ns, c)| {
            let k = deriver.derive(t(ns), c, ObservationNoise::NONE).unwrap();
            deriver.accept(t(ns), c);
            sample(ns, c, k)
        })
        .collect()
}

/// Three fixes heading north-east at 2 m/s, one second apart.
fn steady() -> Vec<SyntheticLocation> {
    let a = origin();
    let b = destination(a, 45.0, 2.0).unwrap();
    let c = destination(b, 45.0, 2.0).unwrap();
    stream(&[(0, a), (SEC, b), (2 * SEC, c)])
}

// --- Derivation ----------------------------------------------------------------

#[test]
fn first_fix_has_unknown_kinematics() {
    let deriver = KinematicsDeriver::new();
    let k = deriver
        .derive(t(0), origin(), ObservationNoise::NONE)
        .unwrap();
    assert_eq!(k, DerivedKinematics::UNKNOWN);
    assert_eq!(
        (k.speed_mps, k.course_deg, k.displacement_m, k.elapsed_s),
        (None, None, None, None)
    );
}

#[test]
fn speed_is_distance_over_elapsed_time_and_course_the_arrival_bearing() {
    let s = steady();
    for pair in s.windows(2) {
        let line = inverse(pair[0].coordinate, pair[1].coordinate).unwrap();
        assert_eq!(pair[1].speed_mps, Some(line.distance_m / 1.0));
        assert_eq!(pair[1].course_deg, Some(line.final_bearing_deg));
        assert!((pair[1].speed_mps.unwrap() - 2.0).abs() < 1e-8);
        assert!((pair[1].course_deg.unwrap() - 45.0).abs() < 1e-4);
    }
}

#[test]
fn elapsed_time_is_the_actual_timestamp_difference() {
    // The same 3 m step, reached after 0.5 s, 1.5 s and 4 s.
    let a = origin();
    let b = destination(a, 200.0, 3.0).unwrap();
    for (ns, expected) in [(SEC / 2, 6.0), (3 * SEC / 2, 2.0), (4 * SEC, 0.75)] {
        let s = stream(&[(0, a), (ns, b)]);
        assert!((s[1].speed_mps.unwrap() - expected).abs() < 1e-8, "{ns}");
        assert!((s[1].course_deg.unwrap() - 200.0).abs() < 1e-4);
    }
}

#[test]
fn derive_does_not_advance_until_the_fix_is_accepted() {
    let mut deriver = KinematicsDeriver::new();
    let a = origin();
    let b = destination(a, 0.0, 5.0).unwrap();
    deriver.accept(t(0), a);
    let first = deriver.derive(t(SEC), b, ObservationNoise::NONE).unwrap();
    // Asking again, or about a different candidate, is still relative to `a`.
    assert_eq!(
        deriver.derive(t(SEC), b, ObservationNoise::NONE).unwrap(),
        first
    );
    let other = deriver
        .derive(t(2 * SEC), b, ObservationNoise::NONE)
        .unwrap();
    assert!((other.speed_mps.unwrap() - 2.5).abs() < 1e-8);
    deriver.accept(t(SEC), b);
    let after = deriver
        .derive(t(2 * SEC), b, ObservationNoise::NONE)
        .unwrap();
    assert_eq!((after.speed_mps, after.course_deg), (Some(0.0), None));
    deriver.reset();
    assert_eq!(
        deriver
            .derive(t(9 * SEC), b, ObservationNoise::NONE)
            .unwrap(),
        DerivedKinematics::UNKNOWN
    );
}

#[test]
fn stationary_and_short_movement_thresholds() {
    let a = origin();
    let derive = |metres: f64| {
        let b = destination(a, 70.0, metres).unwrap();
        let s = stream(&[(0, a), (SEC, b)]);
        (
            s[1].speed_mps.unwrap(),
            s[1].course_deg,
            distance(a, b).unwrap(),
        )
    };
    // The same point: exactly zero, no course.
    assert_eq!(derive(0.0), (0.0, None, 0.0));
    // 1 nm: below coordinate resolution. Whatever the stored coordinates
    // say, the speed is zero or below 4 nm/s and there is no course.
    let (speed, course, d) = derive(1e-9);
    assert!(d < 1e-8 && speed <= 2.0 * DISTANCE_RESOLUTION_M && course.is_none());
    // 10 µm: it moved, so it has a speed, but no meaningful direction.
    let (speed, course, d) = derive(1e-5);
    assert!((speed - 1e-5).abs() < 1e-8 && d > STATIONARY_DISPLACEMENT_M);
    assert_eq!(course, None);
    // 1 mm: speed and course.
    let (speed, course, _) = derive(1e-3);
    assert!((speed - 1e-3).abs() < 1e-8);
    assert!((course.unwrap() - 70.0).abs() < 0.01);
    // The course threshold is exactly the documented constant.
    assert!(derive(0.99 * MIN_COURSE_DISPLACEMENT_M).1.is_none());
    assert!(derive(1.01 * MIN_COURSE_DISPLACEMENT_M).1.is_some());
}

#[test]
fn no_course_is_manufactured_from_rounding_noise() {
    // A fix that wobbles by a few ulps of longitude around one point.
    let lon = origin().longitude();
    let fixes: Vec<(i64, Coordinate)> = (0..200)
        .map(|k| {
            let wobble = (k % 5) as f64 * 1.5e-14;
            (
                k * SEC,
                Coordinate::new(origin().latitude(), lon + wobble).unwrap(),
            )
        })
        .collect();
    let s = stream(&fixes);
    assert!(s.iter().all(|x| x.course_deg.is_none()));
    assert!(s[1..].iter().all(|x| x.speed_mps.unwrap() < 1e-8));
}

#[test]
fn nanosecond_intervals_do_not_divide_by_zero() {
    let a = origin();
    let b = destination(a, 10.0, 1e-3).unwrap();
    let s = stream(&[(0, a), (1, b)]);
    // 1 mm in 1 ns is a million metres per second: absurd, but finite and
    // exactly what the fixes say. Rejecting it is the gate's job.
    assert!((s[1].speed_mps.unwrap() / 1e6 - 1.0).abs() < 1e-6);
    assert!(speed_resolution_mps(1e-9) > 3.9 && speed_resolution_mps(1.0) < 4.1e-9);

    let mut deriver = KinematicsDeriver::new();
    deriver.accept(t(5), a);
    for bad in [5, 4] {
        assert_eq!(
            deriver.derive(t(bad), b, ObservationNoise::NONE),
            Err(DeriveError::NonIncreasingTime {
                previous: t(5),
                current: t(bad)
            })
        );
    }
}

#[test]
fn resolution_figures() {
    assert_eq!(DISTANCE_RESOLUTION_M, 4e-9);
    assert_eq!(speed_resolution_mps(1e-3), 4e-6);
    // 4 nm across 0.1 mm is 4e-5 rad, about 0.0023°.
    let at_threshold = course_resolution_deg(MIN_COURSE_DISPLACEMENT_M);
    assert!((at_threshold - 0.00229).abs() < 1e-5, "{at_threshold}");
    assert!(course_resolution_deg(100.0) < 1e-8);
}

#[test]
fn observation_noise_is_added_to_the_derived_values() {
    let a = origin();
    let b = destination(a, 350.0, 2.0).unwrap();
    let mut deriver = KinematicsDeriver::new();
    deriver.accept(t(0), a);
    let noisy = |speed_mps: f64, heading_deg: f64| {
        deriver
            .derive(
                t(SEC),
                b,
                ObservationNoise {
                    speed_mps,
                    heading_deg,
                },
            )
            .unwrap()
    };
    let k = noisy(0.25, 15.0);
    assert!((k.speed_mps.unwrap() - 2.25).abs() < 1e-8);
    // 350° + 15° wraps to 5°.
    assert!((k.course_deg.unwrap() - 5.0).abs() < 1e-4);
    // Speed cannot go negative, and at zero speed there is no course.
    let k = noisy(-5.0, 15.0);
    assert_eq!((k.speed_mps, k.course_deg), (Some(0.0), None));
    // A stationary fix gets no noise at all.
    deriver.accept(t(SEC), b);
    let still = deriver
        .derive(
            t(2 * SEC),
            b,
            ObservationNoise {
                speed_mps: 0.4,
                heading_deg: 9.0,
            },
        )
        .unwrap();
    assert_eq!((still.speed_mps, still.course_deg), (Some(0.0), None));
}

#[test]
fn identical_inputs_give_identical_metadata() {
    let mut rng = Rng::from_seed(0x0701);
    let mut fixes = vec![(0i64, origin())];
    for _ in 0..500 {
        let (ns, c) = *fixes.last().unwrap();
        let next = destination(c, rng.uniform(0.0, 360.0), rng.uniform(0.0, 30.0)).unwrap();
        fixes.push((ns + 1 + (rng.next_f64() * 3e9) as i64, next));
    }
    assert_eq!(stream(&fixes), stream(&fixes));
}

// --- Checking --------------------------------------------------------------------

fn exact() -> ConsistencyTolerance {
    ConsistencyTolerance::EXACT
}

#[test]
fn derived_metadata_passes_the_independent_check() {
    let mut rng = Rng::from_seed(0x0702);
    let mut fixes = vec![(0i64, Coordinate::new(-89.999, 10.0).unwrap())];
    for k in 0..3_000 {
        let (ns, c) = *fixes.last().unwrap();
        // Everything from standing still to 300 m steps, 1 µs to 30 s apart.
        let step = if k % 7 == 0 {
            0.0
        } else {
            10f64.powf(rng.uniform(-9.0, 2.5))
        };
        let next = destination(c, rng.uniform(0.0, 360.0), step).unwrap();
        fixes.push((
            ns + (10f64.powf(rng.uniform(3.0, 10.5)) as i64).max(1),
            next,
        ));
    }
    let s = stream(&fixes);
    assert_eq!(check_first(&s[0], &exact()), Ok(()));
    for pair in s.windows(2) {
        assert_eq!(check_pair(&pair[0], &pair[1], &exact()), Ok(()), "{pair:?}");
    }
}

#[test]
fn a_first_sample_may_not_claim_kinematics() {
    let s = steady();
    assert_eq!(check_first(&s[0], &exact()), Ok(()));
    for bad in [
        SyntheticLocation {
            speed_mps: Some(0.0),
            ..s[0]
        },
        SyntheticLocation {
            speed_mps: Some(1.0),
            course_deg: Some(10.0),
            ..s[0]
        },
    ] {
        assert_eq!(
            check_first(&bad, &exact()),
            Err(ConsistencyError::FirstSampleHasKinematics)
        );
    }
}

#[test]
fn speed_corruptions_are_detected() {
    let s = steady();
    let (prev, good) = (s[1], s[2]);
    let speed = good.speed_mps.unwrap();
    let with_speed = |v: Option<f64>| SyntheticLocation {
        speed_mps: v,
        ..good
    };

    // Doubled.
    match check_pair(&prev, &with_speed(Some(2.0 * speed)), &exact()) {
        Err(ConsistencyError::SpeedMismatch {
            reported_mps,
            derived_mps,
            ..
        }) => {
            assert_eq!((reported_mps, derived_mps), (2.0 * speed, speed));
        }
        other => panic!("unexpected {other:?}"),
    }
    // Zero while moving (the course has to go too, or the sample is not even well-formed).
    let stopped = SyntheticLocation {
        speed_mps: Some(0.0),
        course_deg: None,
        ..good
    };
    assert!(matches!(
        check_pair(&prev, &stopped, &exact()),
        Err(ConsistencyError::SpeedMismatch { .. })
    ));
    // Off by one part in a million.
    assert!(matches!(
        check_pair(&prev, &with_speed(Some(speed * (1.0 + 1e-6))), &exact()),
        Err(ConsistencyError::SpeedMismatch { .. })
    ));
    // Missing, NaN, infinite.
    assert_eq!(
        check_pair(&prev, &with_speed(None), &exact()),
        Err(ConsistencyError::MissingSpeed)
    );
    for bad in [f64::NAN, f64::INFINITY] {
        assert!(matches!(
            check_pair(&prev, &with_speed(Some(bad)), &exact()),
            Err(ConsistencyError::SpeedMismatch { .. })
        ));
    }
}

#[test]
fn course_corruptions_are_detected() {
    let s = steady();
    let (prev, good) = (s[1], s[2]);
    let course = good.course_deg.unwrap();
    let with_course = |c: Option<f64>| SyntheticLocation {
        course_deg: c,
        ..good
    };

    // Reversed.
    match check_pair(
        &prev,
        &with_course(Some((course + 180.0) % 360.0)),
        &exact(),
    ) {
        Err(ConsistencyError::CourseMismatch {
            reported_deg,
            derived_deg,
            ..
        }) => {
            assert!((bearing_difference(derived_deg, reported_deg).abs() - 180.0).abs() < 1e-9);
        }
        other => panic!("unexpected {other:?}"),
    }
    // Off by a hundredth of a degree; missing; NaN.
    assert!(matches!(
        check_pair(&prev, &with_course(Some(course + 0.01)), &exact()),
        Err(ConsistencyError::CourseMismatch { .. })
    ));
    assert!(matches!(
        check_pair(&prev, &with_course(None), &exact()),
        Err(ConsistencyError::MissingCourse { .. })
    ));
    assert!(matches!(
        check_pair(&prev, &with_course(Some(f64::NAN)), &exact()),
        Err(ConsistencyError::CourseMismatch { .. })
    ));
}

#[test]
fn a_stale_course_is_detected() {
    // North for a second, then east: keeping the old course is wrong.
    let a = origin();
    let b = destination(a, 0.0, 3.0).unwrap();
    let c = destination(b, 90.0, 3.0).unwrap();
    let s = stream(&[(0, a), (SEC, b), (2 * SEC, c)]);
    assert!((s[1].course_deg.unwrap() - 0.0).abs() < 1e-3);
    assert!((s[2].course_deg.unwrap() - 90.0).abs() < 1e-3);
    let stale = SyntheticLocation {
        course_deg: s[1].course_deg,
        ..s[2]
    };
    assert!(matches!(
        check_pair(&s[1], &stale, &exact()),
        Err(ConsistencyError::CourseMismatch { .. })
    ));
    // …and so is a course kept after stopping.
    let stopped = stream(&[(0, a), (SEC, b), (2 * SEC, b)]);
    let kept = SyntheticLocation {
        course_deg: stopped[1].course_deg,
        ..stopped[2]
    };
    assert!(matches!(
        check_pair(&stopped[1], &kept, &exact()),
        Err(ConsistencyError::CourseWithoutMovement { .. })
    ));
}

#[test]
fn a_shifted_timestamp_is_detected() {
    // Metadata derived for t = 2 s, stamped 2.1 s (or 1.9 s): the positions
    // no longer support the reported speed.
    let s = steady();
    for shift in [SEC / 10, -SEC / 10, 1_000] {
        let moved = SyntheticLocation {
            timestamp: Timestamp::from_nanos(s[2].timestamp.as_nanos() + shift),
            ..s[2]
        };
        assert!(
            matches!(
                check_pair(&s[1], &moved, &exact()),
                Err(ConsistencyError::SpeedMismatch { .. })
            ),
            "shift {shift}"
        );
    }
    // Shifted to or before the previous sample.
    let back = SyntheticLocation {
        timestamp: s[1].timestamp,
        ..s[2]
    };
    assert!(matches!(
        check_pair(&s[1], &back, &exact()),
        Err(ConsistencyError::NonIncreasingTime { .. })
    ));
}

#[test]
fn a_stationary_sample_may_not_claim_movement() {
    let a = origin();
    let s = stream(&[(0, a), (SEC, a), (2 * SEC, a)]);
    assert_eq!(check_pair(&s[1], &s[2], &exact()), Ok(()));
    // Any speed at all, however small, with or without a course.
    for (speed, course) in [(1.5, Some(10.0)), (1e-12, None), (f64::NAN, None)] {
        let moving = SyntheticLocation {
            speed_mps: Some(speed),
            course_deg: course,
            ..s[2]
        };
        assert!(
            matches!(
                check_pair(&s[1], &moving, &exact()),
                Err(ConsistencyError::StationaryWithSpeed { .. })
            ),
            "{speed}"
        );
    }
    // Even observation noise does not license it.
    let loose = ConsistencyTolerance {
        speed_noise_reach_mps: 10.0,
        heading_noise_reach_deg: 180.0,
        accuracy: None,
    };
    let moving = SyntheticLocation {
        speed_mps: Some(0.5),
        ..s[2]
    };
    assert!(check_pair(&s[1], &moving, &loose).is_err());
}

#[test]
fn observation_noise_widens_the_contract_by_exactly_its_reach() {
    let s = steady();
    let (prev, good) = (s[1], s[2]);
    let (speed, course) = (good.speed_mps.unwrap(), good.course_deg.unwrap());
    let tolerance = ConsistencyTolerance {
        speed_noise_reach_mps: 0.3,
        heading_noise_reach_deg: 6.0,
        accuracy: None,
    };
    let altered = |dv: f64, dc: f64| SyntheticLocation {
        speed_mps: Some(speed + dv),
        course_deg: Some(course + dc),
        ..good
    };
    assert_eq!(check_pair(&prev, &altered(0.299, 5.99), &tolerance), Ok(()));
    assert_eq!(
        check_pair(&prev, &altered(-0.299, -5.99), &tolerance),
        Ok(())
    );
    assert!(matches!(
        check_pair(&prev, &altered(0.301, 0.0), &tolerance),
        Err(ConsistencyError::SpeedMismatch { .. })
    ));
    assert!(matches!(
        check_pair(&prev, &altered(0.0, 6.01), &tolerance),
        Err(ConsistencyError::CourseMismatch { .. })
    ));
}

#[test]
fn accuracy_must_stay_within_its_configured_band() {
    let s = steady();
    let tolerance = ConsistencyTolerance {
        accuracy: Some(AccuracyBand {
            horizontal_m: 5.0,
            vertical_m: 8.0,
            reach_m: 1.5,
        }),
        ..exact()
    };
    assert_eq!(check_first(&s[0], &tolerance), Ok(()));
    assert_eq!(check_pair(&s[1], &s[2], &tolerance), Ok(()));
    let ok = SyntheticLocation {
        horizontal_accuracy_m: 6.5,
        vertical_accuracy_m: 6.5,
        ..s[2]
    };
    assert_eq!(check_pair(&s[1], &ok, &tolerance), Ok(()));
    // Accuracy inflated to "explain" anything is refused.
    let inflated = SyntheticLocation {
        horizontal_accuracy_m: 500.0,
        ..s[2]
    };
    assert_eq!(
        check_pair(&s[1], &inflated, &tolerance),
        Err(ConsistencyError::AccuracyOutOfBand {
            reported_m: 500.0,
            nominal_m: 5.0,
            reach_m: 1.5
        })
    );
    let vertical = SyntheticLocation {
        vertical_accuracy_m: 9.6,
        ..s[0]
    };
    assert!(check_first(&vertical, &tolerance).is_err());
}

#[test]
fn works_across_the_antimeridian_and_beside_a_pole() {
    // Eastwards over the date line.
    let a = Coordinate::new(5.0, 179.99995).unwrap();
    let b = destination(a, 90.0, 20.0).unwrap();
    assert!(b.longitude() < 0.0);
    let s = stream(&[(0, a), (2 * SEC, b)]);
    assert!((s[1].speed_mps.unwrap() - 10.0).abs() < 1e-7);
    assert!((s[1].course_deg.unwrap() - 90.0).abs() < 1e-3);
    assert_eq!(check_pair(&s[0], &s[1], &exact()), Ok(()));

    // 60 m steps along one geodesic 500 m from the pole: the arrival bearing
    // differs from the departure bearing by the convergence, and it is the
    // arrival bearing that is reported.
    let mut position = Coordinate::new(89.9955, 0.0).unwrap();
    let mut heading = 80.0;
    let mut fixes = vec![(0, position)];
    let mut headings = vec![];
    for k in 1..=10 {
        let (next, arrival) = crate::geographic::direct(position, heading, 60.0).unwrap();
        fixes.push((k * SEC, next));
        headings.push(arrival);
        position = next;
        heading = arrival;
    }
    let s = stream(&fixes);
    for (k, arrival) in headings.iter().enumerate() {
        let reported = s[k + 1].course_deg.unwrap();
        assert!(
            bearing_difference(*arrival, reported).abs() < 1e-6,
            "step {k}"
        );
        assert!((s[k + 1].speed_mps.unwrap() - 60.0).abs() < 1e-6);
        assert_eq!(check_pair(&s[k], &s[k + 1], &exact()), Ok(()));
    }
    // The bearing swung a long way although the path never turned.
    assert!(bearing_difference(headings[0], headings[9]).abs() > 30.0);
}
