//! T07: the emitted speed and course describe the emitted trajectory.
//!
//! Every movement source is run through the real pipeline, with and without
//! noise and at sampling intervals from 1 ms to several seconds, and each
//! consecutive pair of emitted samples is re-checked independently
//! (`common::recheck_stream`). Then emitted metadata is corrupted on purpose
//! and the validation gate has to notice.

mod common;

use common::*;
use locsim_core::consistency::{
    check_pair, ConsistencyError, ConsistencyTolerance, KinematicsDeriver, ObservationNoise,
};
use locsim_core::domain::{
    Coordinate, LocationError, MovementMode, MovementParameters, NoiseParameters,
    PlaybackParameters, Route, RoutePoint, Scenario, SimulationState, SyntheticLocation, Timestamp,
    CURRENT_SCHEMA_VERSION,
};
use locsim_core::geographic::{bearing_difference, destination, distance, inverse};
use locsim_core::movement::model_for;
use locsim_core::provider::{LocationProvider, SimulationProvider};
use locsim_core::rng::Rng;
use locsim_core::route::RoutePlan;
use locsim_core::validation::{SampleLimits, SampleValidator, ValidationError};
use std::time::Instant;

fn bengaluru() -> Coordinate {
    Coordinate::new(12.9352, 77.6245).unwrap()
}

fn scenario(mode: MovementMode, origin: Coordinate, movement: MovementParameters) -> Scenario {
    Scenario {
        schema_version: CURRENT_SCHEMA_VERSION,
        name: format!("{mode:?}"),
        origin,
        altitude_m: 20.0,
        mode,
        movement,
        noise: NoiseParameters::NONE,
        horizontal_accuracy_m: 6.0,
        vertical_accuracy_m: 9.0,
        update_interval_s: 1.0,
        seed: 7,
        route: None,
        playback: PlaybackParameters::REAL_TIME,
    }
}

fn typical_noise() -> NoiseParameters {
    NoiseParameters {
        position_noise_m: 1.5,
        max_position_offset_m: 8.0,
        speed_noise_mps: 0.1,
        heading_noise_deg: 4.0,
        accuracy_noise_m: 0.8,
        drift_rate_mps: 0.05,
        position_correlation_time_s: 6.0,
        max_offset_rate_mps: 1.0,
    }
}

fn walking(origin: Coordinate) -> Scenario {
    scenario(
        MovementMode::Walking,
        origin,
        MovementParameters::walking_preset(),
    )
}

fn driving(origin: Coordinate) -> Scenario {
    scenario(
        MovementMode::Driving,
        origin,
        MovementParameters::driving_preset(),
    )
}

fn fixed(origin: Coordinate) -> Scenario {
    let mut m = MovementParameters::walking_preset();
    m.min_speed_mps = 0.0;
    m.max_speed_mps = 0.0;
    scenario(MovementMode::Fixed, origin, m)
}

fn circular(origin: Coordinate) -> Scenario {
    let mut m = MovementParameters::walking_preset();
    m.min_speed_mps = 0.0;
    m.max_speed_mps = 3.0;
    m.radius_m = Some(60.0);
    m.angular_velocity_dps = Some(1.5); // 1.57 m/s
    scenario(MovementMode::Circular, origin, m)
}

fn random_walk(origin: Coordinate) -> Scenario {
    let mut m = MovementParameters::walking_preset();
    m.radius_m = Some(40.0);
    m.step_distance_m = Some(1.2);
    scenario(MovementMode::RandomWalk, origin, m)
}

/// Replay of a 30-leg walk, admitted for sampling at `interval_s`.
fn route_replay(origin: Coordinate, interval_s: f64) -> Scenario {
    let route = smooth_route(&mut Rng::from_seed(0x0720), origin, 30, 1.4, true);
    let plan = RoutePlan::build(&route, &PlaybackParameters::REAL_TIME, 20.0).unwrap();
    let limits = declared_limits(&plan, origin, 1.05, interval_s).unwrap();
    route_scenario(route, PlaybackParameters::REAL_TIME, &limits, interval_s)
}

/// All six movement sources at 1 Hz.
fn sources(origin: Coordinate) -> Vec<(&'static str, Scenario)> {
    vec![
        ("fixed", fixed(origin)),
        ("circular", circular(origin)),
        ("random walk", random_walk(origin)),
        ("walking", walking(origin)),
        ("driving", driving(origin)),
        ("route replay", route_replay(origin, 1.0)),
    ]
}

fn at_interval(mut sc: Scenario, interval_s: f64) -> Scenario {
    if sc.mode == MovementMode::RouteReplay {
        // A route is admitted for a particular shortest interval.
        let mut again = route_replay(sc.origin, interval_s);
        again.noise = sc.noise;
        return again;
    }
    if sc.mode == MovementMode::RandomWalk {
        // Keep the walk's speed: the step is defined per update interval.
        sc.movement.step_distance_m = Some(1.2 * interval_s);
    }
    sc.update_interval_s = interval_s;
    sc
}

/// Polls `polls` times. With `jitter`, gaps vary between 0.3 and 2.5
/// intervals, so ticks are skipped and timestamps are irregular.
fn run(sc: &Scenario, polls: usize, jitter: Option<u64>) -> Vec<SyntheticLocation> {
    assert_eq!(sc.validate(), Ok(()), "{}", sc.name);
    let interval = sc.update_interval_s * 1e9;
    let mut rng = Rng::from_seed(jitter.unwrap_or(0));
    let mut p = SimulationProvider::new(sc.clone());
    p.start(Timestamp::from_nanos(START)).unwrap();
    let mut now = START;
    let mut out = Vec::new();
    for _ in 0..polls {
        match p.poll(Timestamp::from_nanos(now)) {
            Ok(Some(s)) => out.push(s),
            Ok(None) => {}
            Err(e) => panic!(
                "{} at {} s: sample {} rejected: {e}",
                sc.name,
                sc.update_interval_s,
                out.len()
            ),
        }
        let gap = if jitter.is_some() {
            rng.uniform(0.3, 2.5)
        } else {
            1.0
        };
        now += (gap * interval) as i64;
    }
    let st = p.status();
    assert_eq!((st.state, st.failed_count), (SimulationState::Running, 0));
    out
}

// --- Every source, with and without noise -------------------------------------

#[test]
fn every_movement_source_emits_consistent_kinematics() {
    let mut total = Recheck::default();
    for (label, clean) in sources(bengaluru()) {
        let mut noisy = clean.clone();
        noisy.noise = typical_noise();
        for (kind, sc) in [("no noise", &clean), ("noise", &noisy)] {
            let mut found = Recheck::default();
            found.absorb(recheck_stream(sc, &run(sc, 600, None), label));
            found.absorb(recheck_stream(sc, &run(sc, 400, Some(11)), label));
            println!(
                "{label:<13} {kind:<8} pairs {:>4}, with course {:>4}, stationary {:>4}, \
                 max |speed - d/dt| {:.3e} m/s, max course error {:.3e} deg",
                found.pairs,
                found.with_course,
                found.stationary,
                found.max_speed_error_mps,
                found.max_course_error_deg
            );
            assert!(found.pairs > 700, "{label} {kind}");
            if kind == "no noise" {
                // No observation noise: the metadata *is* the finite
                // difference, to the last bit of the division.
                assert!(found.max_speed_error_mps <= 1e-12, "{label}: {found:?}");
                assert!(found.max_course_error_deg <= 1e-9, "{label}: {found:?}");
            } else {
                // Observation noise of 3 sigma = 0.3 m/s and 12 degrees.
                assert!(
                    found.max_speed_error_mps <= 0.3 + 1e-9,
                    "{label}: {found:?}"
                );
                assert!(
                    found.max_course_error_deg <= 12.0 + 1e-9,
                    "{label}: {found:?}"
                );
            }
            // Sources that move have courses; the noiseless fixed one never does.
            match (label, kind) {
                ("fixed", "no noise") => {
                    assert_eq!((found.with_course, found.stationary), (0, found.pairs))
                }
                ("fixed", _) => assert!(found.with_course > 0, "noise moves the fix"),
                // (A finished route holds its last point, so not every pair moves.)
                _ => assert!(found.with_course > 50, "{label} {kind}: {found:?}"),
            }
            total.absorb(found);
        }
    }
    println!("all sources: {} consecutive pairs re-checked", total.pairs);
}

// --- Sampling rates -------------------------------------------------------------

#[test]
fn consistency_holds_from_one_millisecond_to_several_seconds() {
    let mut pairs_by_interval = Vec::new();
    for interval in [0.001, 0.01, 0.1, 1.0, 5.0] {
        let mut found = Recheck::default();
        for (label, clean) in sources(bengaluru()) {
            let sc = at_interval(clean, interval);
            let mut noisy = sc.clone();
            noisy.noise = typical_noise();
            for (kind, sc) in [("no noise", &sc), ("noise", &noisy)] {
                let tag = format!("{label} {kind} @ {interval} s");
                // Regular, then jittered with skipped ticks.
                found.absorb(recheck_stream(sc, &run(sc, 400, None), &tag));
                found.absorb(recheck_stream(sc, &run(sc, 300, Some(3)), &tag));
            }
        }
        println!(
            "interval {interval:>5} s: {} pairs re-checked, {} with a course",
            found.pairs, found.with_course
        );
        assert!(found.pairs > 5_000);
        pairs_by_interval.push(found.pairs);
    }
    println!(
        "sampling rates: {} pairs in total",
        pairs_by_interval.iter().sum::<usize>()
    );
}

#[test]
fn the_same_trajectory_reports_interval_means_at_every_rate() {
    // An orbit is the same curve whatever the sampling: its chord speed at
    // interval dt is 2 r sin(w dt / 2) / dt, approaching r w from below.
    let (radius, omega) = (60.0f64, 1.5f64.to_radians());
    for interval in [0.001, 0.01, 0.1, 1.0, 5.0, 20.0] {
        let sc = at_interval(circular(bengaluru()), interval);
        let stream = run(&sc, 200, None);
        let expected = 2.0 * radius * (omega * interval / 2.0).sin() / interval;
        for s in &stream[1..] {
            let speed = s.speed_mps.unwrap();
            // Position rounding limits the speed to 4 nm per interval.
            assert!(
                (speed - expected).abs() <= 1e-6 * expected + 4e-9 / interval,
                "{interval} s: {speed} vs {expected}"
            );
            // Never above the arc speed, beyond that same rounding.
            assert!(speed <= radius * omega + 4e-9 / interval);
        }
        // The course advances by w dt per sample.
        for pair in stream[1..].windows(2) {
            let turn = bearing_difference(pair[0].course_deg.unwrap(), pair[1].course_deg.unwrap());
            let per_sample = (omega * interval).to_degrees();
            // 4 nm across a chord, twice, plus the tiny convergence over it.
            let resolution = 2.0 * (4e-9 / (expected * interval)).to_degrees();
            assert!(
                (turn - per_sample).abs() < 1e-3 * per_sample + resolution + 1e-6,
                "{interval} s: {turn}"
            );
        }
    }
}

// --- Property: speed is distance over time, course is the direction moved ------------

#[test]
fn derived_kinematics_match_the_emitted_positions_for_random_scenarios() {
    let mut rng = Rng::from_seed(0x0721);
    let mut total = Recheck::default();
    let mut noise_free = Recheck::default();
    let cases = 180u64;
    for case in 0..cases {
        let origin = special_origin(&mut rng, case);
        let interval = log_uniform(&mut rng, -2.0, 1.0); // 10 ms … 10 s
        let mut sc = match case % 6 {
            0 => fixed(origin),
            1 => circular(origin),
            2 => random_walk(origin),
            3 => walking(origin),
            4 => driving(origin),
            _ => route_replay(origin, interval),
        };
        sc = at_interval(sc, interval);
        sc.seed = case;
        let with_noise = case % 2 == 1;
        if with_noise {
            let sigma = log_uniform(&mut rng, -1.0, 0.5);
            let rate = log_uniform(&mut rng, -1.5, 0.5);
            sc.noise = NoiseParameters {
                position_noise_m: sigma,
                max_position_offset_m: 3.0 * sigma * rng.uniform(1.05, 3.0),
                speed_noise_mps: if case % 4 == 1 {
                    rng.uniform(0.0, 0.5)
                } else {
                    0.0
                },
                heading_noise_deg: if case % 4 == 1 {
                    rng.uniform(0.0, 10.0)
                } else {
                    0.0
                },
                accuracy_noise_m: rng.uniform(0.0, 1.9),
                drift_rate_mps: 0.0,
                position_correlation_time_s: rng.uniform(0.0, 20.0),
                max_offset_rate_mps: rate,
            };
        }
        let stream = run(&sc, 300, (case % 3 == 0).then_some(case));
        let found = recheck_stream(&sc, &stream, &format!("case {case}"));
        if !with_noise {
            noise_free.absorb(found);
        }
        total.absorb(found);
    }
    println!(
        "random scenarios: {cases} streams, {} pairs re-checked ({} with a course, {} stationary); \
         without noise: max |speed - d/dt| {:.3e} m/s, max course error {:.3e} deg",
        total.pairs, total.with_course, total.stationary,
        noise_free.max_speed_error_mps, noise_free.max_course_error_deg
    );
    assert!(total.pairs > 40_000 && total.with_course > 15_000 && total.stationary > 1_000);
    // The documented numerical bound: exact for speed, 1e-9 degrees for course.
    assert!(noise_free.max_speed_error_mps <= 1e-12);
    assert!(noise_free.max_course_error_deg <= 1e-9);
}

#[test]
fn identical_inputs_produce_identical_metadata() {
    for (label, clean) in sources(bengaluru()) {
        let mut sc = clean;
        sc.noise = typical_noise();
        let a = run(&sc, 300, Some(5));
        let b = run(&sc, 300, Some(5));
        assert_eq!(a, b, "{label}");
        // A different seed changes the noise and therefore the metadata,
        // which stays consistent with its own positions (re-checked above).
        sc.seed += 1;
        assert_ne!(a, run(&sc, 300, Some(5)), "{label}");
    }
}

// --- Particular geometries ---------------------------------------------------------

#[test]
fn straight_line_acceleration_and_braking_report_interval_means() {
    // 60 m due east in 60 s from rest to rest: distance covered after a
    // fraction x of the time is 60 (3x^2 - 2x^3).
    let end = destination(bengaluru(), 90.0, 60.0).unwrap();
    let route = Route::new(vec![
        RoutePoint::new(0, bengaluru()),
        RoutePoint::new(60 * SEC, end),
    ])
    .unwrap();
    let plan = RoutePlan::build(&route, &PlaybackParameters::REAL_TIME, 0.0).unwrap();
    let limits = declared_limits(&plan, bengaluru(), 1.05, 1.0).unwrap();
    let sc = route_scenario(route, PlaybackParameters::REAL_TIME, &limits, 1.0);
    let stream = run(&sc, 70, None);
    recheck_stream(&sc, &stream, "straight line");
    let covered = |seconds: f64| {
        let x = (seconds / 60.0).min(1.0);
        60.0 * (3.0 * x * x - 2.0 * x * x * x)
    };
    for (k, s) in stream.iter().enumerate().skip(1) {
        let expected = covered(k as f64) - covered(k as f64 - 1.0);
        assert!((s.speed_mps.unwrap() - expected).abs() < 1e-6, "second {k}");
        if k <= 60 {
            assert!(bearing_difference(s.course_deg.unwrap(), 90.0).abs() < 1e-3);
        } else {
            // Arrived: the position no longer changes.
            assert_eq!((s.speed_mps, s.course_deg), (Some(0.0), None));
        }
    }
    // Rising for the first half, falling for the second.
    assert!(stream[10].speed_mps < stream[20].speed_mps);
    assert!(stream[50].speed_mps < stream[40].speed_mps);
    assert!((stream[30].speed_mps.unwrap() - 1.5).abs() < 0.01);
}

#[test]
fn stationary_periods_report_zero_speed_and_no_course() {
    let mut sc = walking(bengaluru());
    sc.movement.pause_probability = 0.05;
    sc.movement.max_pause_s = 20.0;
    let stream = run(&sc, 2_000, None);
    let found = recheck_stream(&sc, &stream, "pauses");
    assert!(found.stationary > 100, "{found:?}");
    let mut resumed = 0;
    for pair in stream.windows(2) {
        if pair[0].coordinate == pair[1].coordinate {
            assert_eq!((pair[1].speed_mps, pair[1].course_deg), (Some(0.0), None));
        } else if pair[0].speed_mps == Some(0.0) {
            // Leaving a pause: the course is the direction now taken, not
            // one remembered from before the stop.
            let line = inverse(pair[0].coordinate, pair[1].coordinate).unwrap();
            if let Some(course) = pair[1].course_deg {
                assert!(bearing_difference(line.final_bearing_deg, course).abs() < 1e-9);
                resumed += 1;
            }
        }
    }
    assert!(resumed > 5, "{resumed}");
}

#[test]
fn movement_too_short_for_a_direction_has_a_speed_but_no_course() {
    // 5 mm/s sampled at 100 Hz: 50 um per sample, below the 0.1 mm at which
    // a direction means anything.
    let mut m = MovementParameters::walking_preset();
    m.min_speed_mps = 0.004;
    m.max_speed_mps = 0.005;
    m.max_acceleration_mps2 = 0.01;
    m.max_deceleration_mps2 = 0.01;
    m.pause_probability = 0.0;
    let mut sc = scenario(MovementMode::Walking, bengaluru(), m);
    sc.update_interval_s = 0.01;
    let stream = run(&sc, 3_000, None);
    let found = recheck_stream(&sc, &stream, "creeping");
    assert_eq!(found.with_course, 0);
    let late = &stream[2_000..];
    assert!(late.iter().all(|s| s.course_deg.is_none()));
    // 4 nm of rounding per 10 ms is 4e-7 m/s of speed resolution.
    assert!(late
        .iter()
        .all(|s| (0.0039..=0.00501).contains(&s.speed_mps.unwrap())));
    // The same creep sampled once a second moves 5 mm per sample: a course exists.
    sc.update_interval_s = 1.0;
    let slow = run(&sc, 200, None);
    assert!(recheck_stream(&sc, &slow, "creeping at 1 Hz").with_course > 150);
}

#[test]
fn antimeridian_and_high_latitude() {
    // Straight east across the date line.
    let mut m = MovementParameters::driving_preset();
    m.heading_persistence = 1.0;
    m.pause_probability = 0.0;
    let mut crossed = false;
    for seed in 0..40 {
        let mut sc = scenario(
            MovementMode::Driving,
            Coordinate::new(-12.0, 179.9990).unwrap(),
            m,
        );
        sc.seed = seed;
        let stream = run(&sc, 120, None);
        recheck_stream(&sc, &stream, "date line");
        let signs: Vec<bool> = stream
            .iter()
            .map(|s| s.coordinate.longitude() < 0.0)
            .collect();
        if signs.contains(&true) && signs.contains(&false) {
            crossed = true;
            // Across the flip in longitude the speed and course carry on.
            for pair in stream[20..].windows(2) {
                let dv = pair[1].speed_mps.unwrap() - pair[0].speed_mps.unwrap();
                assert!(dv.abs() <= 4.5 + 1e-6);
                let line = inverse(pair[0].coordinate, pair[1].coordinate).unwrap();
                let turn = bearing_difference(
                    pair[0].course_deg.unwrap() + line.convergence_deg,
                    pair[1].course_deg.unwrap(),
                );
                assert!(turn.abs() < 1e-6, "{turn}");
            }
        }
    }
    assert!(crossed);

    // Around a pole: 1.1 km away, with and without noise, fenced and free.
    for origin in [
        Coordinate::new(89.99, 40.0).unwrap(),
        Coordinate::new(-89.99, -100.0).unwrap(),
    ] {
        let mut total = Recheck::default();
        for (label, clean) in sources(origin) {
            let mut noisy = clean.clone();
            noisy.noise = typical_noise();
            for sc in [&clean, &noisy] {
                total.absorb(recheck_stream(sc, &run(sc, 300, Some(9)), label));
            }
        }
        assert!(
            total.pairs > 2_000 && total.with_course > 1_000,
            "{total:?}"
        );
    }
}

#[test]
fn a_long_pause_gives_a_long_interval_not_a_wrong_speed() {
    let mut sc = walking(bengaluru());
    sc.movement.pause_probability = 0.0;
    let mut p = SimulationProvider::new(sc.clone());
    p.start(Timestamp::from_nanos(START)).unwrap();
    let mut stream = Vec::new();
    for n in 0..30 {
        stream.push(
            p.poll(Timestamp::from_nanos(START + n * SEC))
                .unwrap()
                .unwrap(),
        );
    }
    p.pause(Timestamp::from_nanos(START + 29 * SEC)).unwrap();
    let resumed = START + (29 + 3_600) * SEC;
    p.resume(Timestamp::from_nanos(resumed)).unwrap();
    for n in 1..30 {
        stream.push(
            p.poll(Timestamp::from_nanos(resumed + n * SEC))
                .unwrap()
                .unwrap(),
        );
    }
    recheck_stream(&sc, &stream, "paused");
    // The first fix after the pause is a second's walk away, an hour later:
    // its speed is that distance over the real elapsed time.
    let (before, after) = (&stream[29], &stream[30]);
    let elapsed = after.timestamp.seconds_since(before.timestamp);
    assert_eq!(elapsed, 3_601.0);
    let moved = distance(before.coordinate, after.coordinate).unwrap();
    assert!(moved > 0.5 && moved < 1.8);
    assert_eq!(after.speed_mps, Some(moved / elapsed));
    assert!(after.speed_mps.unwrap() < 0.001);
}

#[test]
fn deriving_metadata_never_changes_a_position() {
    // Without noise the emitted positions are the model positions, exactly.
    for (label, sc) in sources(bengaluru()) {
        let stream = run(&sc, 300, None);
        let mut model = model_for(&sc).unwrap();
        for (n, s) in stream.iter().enumerate() {
            let t = Timestamp::from_nanos(START + n as i64 * SEC);
            assert_eq!(
                s.coordinate,
                model.sample_at(t).unwrap().coordinate,
                "{label} sample {n}"
            );
        }
    }
}

// --- Mutations: corrupted metadata must be caught --------------------------------------

/// Validates `stream[..k]`, then the mutated sample `k`, with the gate the
/// provider would use for `sc`. Returns the rejection.
fn rejection(
    sc: &Scenario,
    stream: &[SyntheticLocation],
    k: usize,
    mutated: SyntheticLocation,
) -> ValidationError {
    let mut gate = SampleValidator::with_limits(SampleLimits::for_scenario(sc));
    for (n, s) in stream[..k].iter().enumerate() {
        gate.validate(s)
            .unwrap_or_else(|e| panic!("clean sample {n}: {e}"));
    }
    let error = gate
        .validate(&mutated)
        .expect_err("mutated sample accepted");
    // The untouched sample is still fine: the mutation alone was the cause.
    assert_eq!(gate.validate(&stream[k]), Ok(()));
    error
}

/// An index at which the walker is under way and has just changed course.
fn turning_index(stream: &[SyntheticLocation]) -> usize {
    (10..stream.len())
        .find(|&k| {
            let (a, b) = (&stream[k - 1], &stream[k]);
            match (a.course_deg, b.course_deg, b.speed_mps) {
                (Some(c0), Some(c1), Some(v)) => v > 0.8 && bearing_difference(c0, c1).abs() > 2.0,
                _ => false,
            }
        })
        .expect("the walker turns somewhere")
}

#[test]
fn corrupted_metadata_is_detected_by_the_gate() {
    use ConsistencyError as C;
    use ValidationError as V;
    for (label, noise) in [
        ("no noise", NoiseParameters::NONE),
        ("noise", typical_noise()),
    ] {
        let mut sc = walking(bengaluru());
        sc.movement.pause_probability = 0.0;
        sc.noise = noise;
        let stream = run(&sc, 400, None);
        let k = turning_index(&stream);
        let good = stream[k];
        let (speed, course) = (good.speed_mps.unwrap(), good.course_deg.unwrap());
        let check = |name: &str, mutated: SyntheticLocation, expected: fn(&V) -> bool| {
            let error = rejection(&sc, &stream, k, mutated);
            assert!(expected(&error), "{label}, {name}: reported as {error:?}");
        };

        check(
            "speed doubled",
            SyntheticLocation {
                speed_mps: Some(2.0 * speed),
                ..good
            },
            |e| matches!(e, V::Inconsistent(C::SpeedMismatch { .. })),
        );
        check(
            "speed zero while moving",
            SyntheticLocation {
                speed_mps: Some(0.0),
                course_deg: None,
                ..good
            },
            |e| matches!(e, V::Inconsistent(C::SpeedMismatch { .. })),
        );
        check(
            "course reversed",
            SyntheticLocation {
                course_deg: Some((course + 180.0) % 360.0),
                ..good
            },
            |e| matches!(e, V::Inconsistent(C::CourseMismatch { .. })),
        );
        check(
            "course missing",
            SyntheticLocation {
                course_deg: None,
                ..good
            },
            |e| matches!(e, V::Inconsistent(C::MissingCourse { .. })),
        );
        check(
            "speed missing",
            SyntheticLocation {
                speed_mps: None,
                course_deg: None,
                ..good
            },
            |e| matches!(e, V::Inconsistent(C::MissingSpeed)),
        );
        // Stamped two seconds late: the positions now imply a third of the
        // reported speed.
        check(
            "timestamp shifted",
            SyntheticLocation {
                timestamp: Timestamp::from_nanos(good.timestamp.as_nanos() + 2 * SEC),
                ..good
            },
            |e| matches!(e, V::Inconsistent(C::SpeedMismatch { .. })),
        );
        check(
            "speed NaN",
            SyntheticLocation {
                speed_mps: Some(f64::NAN),
                ..good
            },
            |e| matches!(e, V::Location(LocationError::NonFinite("speed"))),
        );
        check(
            "speed infinite",
            SyntheticLocation {
                speed_mps: Some(f64::INFINITY),
                ..good
            },
            |e| matches!(e, V::Location(LocationError::NonFinite("speed"))),
        );
        check(
            "course NaN",
            SyntheticLocation {
                course_deg: Some(f64::NAN),
                ..good
            },
            |e| matches!(e, V::Location(LocationError::NonFinite("course"))),
        );
        check(
            "accuracy inflated",
            SyntheticLocation {
                horizontal_accuracy_m: 60.0,
                ..good
            },
            |e| matches!(e, V::Inconsistent(C::AccuracyOutOfBand { .. })),
        );
        if label == "no noise" {
            // A course left over from the previous sample. (With 12 degrees of
            // heading noise allowed, a 2 degree staleness is within contract.)
            check(
                "course stale",
                SyntheticLocation {
                    course_deg: stream[k - 1].course_deg,
                    ..good
                },
                |e| matches!(e, V::Inconsistent(C::CourseMismatch { .. })),
            );
            // Even a 0.1 % error in speed.
            check(
                "speed off by 0.1 %",
                SyntheticLocation {
                    speed_mps: Some(speed * 1.001),
                    ..good
                },
                |e| matches!(e, V::Inconsistent(C::SpeedMismatch { .. })),
            );
        }
        // The same corruptions, seen by the consistency check on its own.
        let tolerance = ConsistencyTolerance::for_scenario(&sc);
        assert_eq!(check_pair(&stream[k - 1], &good, &tolerance), Ok(()));
        let doubled = SyntheticLocation {
            speed_mps: Some(2.0 * speed),
            ..good
        };
        assert!(check_pair(&stream[k - 1], &doubled, &tolerance).is_err());

        // A first sample claiming kinematics.
        let first = SyntheticLocation {
            speed_mps: Some(0.0),
            ..stream[0]
        };
        assert!(matches!(
            rejection(&sc, &stream, 0, first),
            V::Inconsistent(C::FirstSampleHasKinematics)
        ));
    }
}

#[test]
fn a_stationary_fix_claiming_speed_is_detected() {
    let sc = fixed(bengaluru());
    let stream = run(&sc, 20, None);
    for (speed, course) in [(0.5, None), (0.5, Some(90.0)), (1e-9, None)] {
        let mutated = SyntheticLocation {
            speed_mps: Some(speed),
            course_deg: course,
            ..stream[10]
        };
        assert!(
            matches!(
                rejection(&sc, &stream, 10, mutated),
                ValidationError::Inconsistent(ConsistencyError::StationaryWithSpeed { .. })
            ),
            "{speed} {course:?}"
        );
    }
}

// --- Cost -------------------------------------------------------------------------------

#[test]
fn cost_of_deriving_kinematics() {
    // Not an assertion on speed, only a measurement: run with
    // `cargo test --release --test consistency_pipeline cost -- --nocapture`.
    let sc = walking(bengaluru());
    let samples = 50_000usize;
    let mut p = SimulationProvider::new(sc.clone());
    p.start(Timestamp::from_nanos(START)).unwrap();
    let started = Instant::now();
    let mut stream = Vec::with_capacity(samples);
    for n in 0..samples {
        stream.push(
            p.poll(Timestamp::from_nanos(START + n as i64 * SEC))
                .unwrap()
                .unwrap(),
        );
    }
    let pipeline = started.elapsed();

    let started = Instant::now();
    let mut deriver = KinematicsDeriver::new();
    let mut checksum = 0.0;
    for s in &stream {
        let k = deriver
            .derive(s.timestamp, s.coordinate, ObservationNoise::NONE)
            .unwrap();
        deriver.accept(s.timestamp, s.coordinate);
        checksum += k.speed_mps.unwrap_or(0.0);
    }
    let derive = started.elapsed();

    let started = Instant::now();
    let mut gate = SampleValidator::with_limits(SampleLimits::for_scenario(&sc));
    for s in &stream {
        gate.validate(s).unwrap();
    }
    let validate = started.elapsed();

    let per = |d: std::time::Duration| d.as_nanos() as f64 / samples as f64;
    println!(
        "cost per sample over {samples} samples: whole pipeline {:.0} ns, deriving kinematics {:.0} ns \
         ({:.0} % of the pipeline), final validation {:.0} ns; checksum {checksum:.3}",
        per(pipeline),
        per(derive),
        100.0 * per(derive) / per(pipeline),
        per(validate)
    );
    assert!(checksum > 0.0);
}
