//! Every movement model through the real pipeline:
//! scenario → movement → noise → final validation → provider.
//!
//! Two questions are asked here. Do correct models pass the independent gate
//! for any valid scenario, with and without noise? And does the gate reject
//! a model that misbehaves?

use locsim_core::domain::{
    Coordinate, MovementMode, MovementParameters, NoiseParameters, PlaybackParameters,
    RotationDirection, Scenario, SimulationState, SyntheticLocation, Timestamp,
    CURRENT_SCHEMA_VERSION, NOISE_CLIP_SIGMA,
};
use locsim_core::geographic::{bearing_difference, destination, distance, inverse};
use locsim_core::movement::{MovementError, MovementModel, MovementSample};
use locsim_core::noise::NoiseError;
use locsim_core::provider::{LocationProvider, ModelFactory, ProviderError, SimulationProvider};
use locsim_core::rng::Rng;
use locsim_core::validation::ValidationError;
use std::sync::Mutex;

const START: i64 = 1_700_000_000_000_000_000;
const SEC: i64 = 1_000_000_000;

fn scenario(mode: MovementMode, origin: Coordinate, movement: MovementParameters) -> Scenario {
    Scenario {
        schema_version: CURRENT_SCHEMA_VERSION,
        name: format!("{mode:?}"),
        origin,
        altitude_m: 15.0,
        mode,
        movement,
        noise: NoiseParameters::NONE,
        horizontal_accuracy_m: 6.0,
        vertical_accuracy_m: 9.0,
        update_interval_s: 1.0,
        seed: 1,
        route: None,
        playback: PlaybackParameters::REAL_TIME,
    }
}

fn bengaluru() -> Coordinate {
    Coordinate::new(12.9352, 77.6245).unwrap()
}

fn log_uniform(rng: &mut Rng, low_exp: f64, high_exp: f64) -> f64 {
    10f64.powf(rng.uniform(low_exp, high_exp))
}

fn random_origin(rng: &mut Rng, case: u64) -> Coordinate {
    let (lat, lon) = match case % 8 {
        0 => (rng.uniform(89.9, 89.99), rng.uniform(-180.0, 180.0)),
        1 => (rng.uniform(-89.99, -89.9), rng.uniform(-180.0, 180.0)),
        2 => (rng.uniform(-70.0, 70.0), 180.0),
        3 => (rng.uniform(-70.0, 70.0), -179.99999),
        _ => (rng.uniform(-85.0, 85.0), rng.uniform(-180.0, 180.0)),
    };
    Coordinate::new(lat, lon).unwrap()
}

/// A random valid scenario of any implemented moving mode.
fn random_scenario(rng: &mut Rng, case: u64) -> Scenario {
    let mode = [
        MovementMode::RandomWalk,
        MovementMode::Walking,
        MovementMode::Driving,
        MovementMode::Circular,
    ][(case % 4) as usize];
    let interval = log_uniform(rng, -2.0, 1.3); // 10 ms … 20 s
    let max_speed = log_uniform(rng, -0.5, 1.7); // 0.3 … 50 m/s
    let min_speed = max_speed * rng.uniform(0.1, 0.9);
    let mut m = MovementParameters {
        min_speed_mps: min_speed,
        max_speed_mps: max_speed,
        max_acceleration_mps2: log_uniform(rng, -1.0, 1.0),
        max_deceleration_mps2: log_uniform(rng, -1.0, 1.0),
        max_heading_rate_dps: log_uniform(rng, 0.0, 2.3),
        radius_m: None,
        step_distance_m: None,
        heading_persistence: rng.uniform(0.0, 1.0),
        pause_probability: if case % 3 == 0 {
            rng.uniform(0.0, 0.1)
        } else {
            0.0
        },
        max_pause_s: rng.uniform(0.0, 30.0),
        angular_velocity_dps: None,
        direction: if case % 8 < 4 {
            RotationDirection::Clockwise
        } else {
            RotationDirection::CounterClockwise
        },
        start_phase_deg: rng.uniform(0.0, 360.0),
        speed_change_interval_s: rng.uniform(0.0, 60.0),
        max_displacement_per_sample_m: None,
    };
    if case % 5 == 0 {
        // A cap somewhere between "just above min speed" and "not binding".
        let capped_speed = rng.uniform(min_speed * 1.01, max_speed * 1.2);
        m.max_displacement_per_sample_m = Some(capped_speed * interval);
    }
    let effective = m.effective_max_speed_mps(interval);
    match mode {
        MovementMode::RandomWalk => {
            m.radius_m = Some(log_uniform(rng, 0.7, 3.3));
            m.step_distance_m = Some(interval * rng.uniform(min_speed, 0.999 * effective));
        }
        MovementMode::Walking | MovementMode::Driving => {
            if case % 8 >= 4 {
                m.radius_m = Some(log_uniform(rng, 0.7, 3.3));
            }
        }
        _ => {
            m.min_speed_mps = 0.0;
            let radius = log_uniform(rng, 0.5, 3.5);
            let fastest = (0.99 * effective / radius)
                .to_degrees()
                .min(0.99 * m.max_heading_rate_dps);
            m.radius_m = Some(radius);
            m.angular_velocity_dps = Some(fastest * rng.uniform(0.1, 1.0));
        }
    }
    let mut s = scenario(mode, random_origin(rng, case), m);
    s.update_interval_s = interval;
    s.seed = case;
    if case % 2 == 1 {
        let sigma = log_uniform(rng, -1.0, 1.0);
        let rate = log_uniform(rng, -1.5, 1.0);
        s.noise = NoiseParameters {
            position_noise_m: sigma,
            max_position_offset_m: NOISE_CLIP_SIGMA * sigma * rng.uniform(1.05, 3.0),
            speed_noise_mps: rng.uniform(0.0, 1.5),
            heading_noise_deg: rng.uniform(0.0, 15.0),
            accuracy_noise_m: rng.uniform(0.0, 1.9),
            drift_rate_mps: if case % 4 == 1 {
                rng.uniform(0.0, rate)
            } else {
                0.0
            },
            position_correlation_time_s: rng.uniform(0.0, 30.0),
            max_offset_rate_mps: rate,
        };
    }
    assert_eq!(
        s.validate(),
        Ok(()),
        "case {case}: generator made an invalid scenario"
    );
    s
}

/// Polls with gaps of up to `max_gap` intervals; panics if any sample is rejected.
fn run(sc: &Scenario, polls: usize, poll_seed: u64, max_gap: f64) -> Vec<SyntheticLocation> {
    let interval = sc.update_interval_s * 1e9;
    let mut rng = Rng::from_seed(poll_seed);
    let mut p = SimulationProvider::new(sc.clone());
    p.start(Timestamp::from_nanos(START)).unwrap();
    let mut now = START;
    let mut out = Vec::new();
    for _ in 0..polls {
        match p.poll(Timestamp::from_nanos(now)) {
            Ok(Some(s)) => out.push(s),
            Ok(None) => {}
            Err(e) => panic!("{}: sample {} rejected: {e}\n{sc:#?}", sc.name, out.len()),
        }
        now += (rng.uniform(0.3, max_gap) * interval) as i64;
    }
    let st = p.status();
    assert_eq!((st.state, st.failed_count), (SimulationState::Running, 0));
    out
}

/// Independent re-check of a finished stream against the scenario's limits.
/// Returns how many consecutive pairs were examined.
fn check_stream(sc: &Scenario, stream: &[SyntheticLocation], case: u64) -> usize {
    let m = &sc.movement;
    let noisy_position = sc.noise.has_position_noise();
    let offset_rate = if noisy_position {
        sc.noise.max_offset_rate_mps
    } else {
        0.0
    };
    let reach = if noisy_position {
        sc.noise.max_position_offset_m
    } else {
        0.0
    };
    let speed_slack = 2.0 * NOISE_CLIP_SIGMA * sc.noise.speed_noise_mps;
    let heading_slack = 2.0 * NOISE_CLIP_SIGMA * sc.noise.heading_noise_deg;
    let interval_ns = (sc.update_interval_s * 1e9).round() as i64;
    let boundary = sc.boundary();

    for s in stream {
        assert_eq!(s.validate(), Ok(()), "case {case}");
        if let Some(v) = s.speed_mps {
            assert!(v <= m.max_speed_mps, "case {case}: speed {v}");
        }
        if let Some(b) = boundary {
            let d = distance(b.center, s.coordinate).unwrap();
            assert!(d <= b.radius_m, "case {case}: {d} outside {}", b.radius_m);
        }
    }
    for pair in stream.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(b.timestamp > a.timestamp, "case {case}");
        assert_eq!(
            (b.timestamp.as_nanos() - START) % interval_ns,
            0,
            "case {case}"
        );
        let dt = b.timestamp.seconds_since(a.timestamp);
        let line = inverse(a.coordinate, b.coordinate).unwrap();

        let limit = (sc.effective_max_speed_mps() + offset_rate) * dt;
        assert!(
            line.distance_m <= limit,
            "case {case}: moved {} > {limit}",
            line.distance_m
        );

        if let (Some(v0), Some(v1)) = (a.speed_mps, b.speed_mps) {
            let dv = v1 - v0;
            assert!(
                dv <= m.max_acceleration_mps2 * dt + speed_slack,
                "case {case}: dv {dv}"
            );
            assert!(
                -dv <= m.max_deceleration_mps2 * dt + speed_slack,
                "case {case}: dv {dv}"
            );
        }
        if let (Some(c0), Some(c1)) = (a.course_deg, b.course_deg) {
            // Slightly more generous than the gate's own allowance (smaller
            // radius), so this never fails where the gate passed.
            let lat = a
                .coordinate
                .latitude()
                .abs()
                .max(b.coordinate.latitude().abs());
            let pole = (lat.to_radians() + reach / 6.3e6).min(std::f64::consts::FRAC_PI_2);
            let convergence_slack = (2.0 * reach / 6.3e6 * pole.tan()).to_degrees();
            let limit = m.max_heading_rate_dps * dt + heading_slack + convergence_slack;
            if limit < 180.0 {
                let turn = bearing_difference(c0 + line.convergence_deg, c1).abs();
                assert!(turn <= limit, "case {case}: turned {turn} > {limit}");
            }
        }
    }
    stream.len().saturating_sub(1)
}

#[test]
fn every_model_passes_the_gate_for_random_valid_scenarios() {
    let mut rng = Rng::from_seed(0x0505);
    let mut per_mode = [0usize; 4];
    let (mut pairs, mut noisy, mut moving_streams) = (0usize, 0usize, 0usize);
    let cases = 600u64;
    for case in 0..cases {
        let sc = random_scenario(&mut rng, case);
        // A third polled punctually, the rest with gaps that skip ticks.
        let max_gap = if case % 3 == 0 { 0.9 } else { 2.5 };
        let stream = run(&sc, 300, case ^ 0xA5, max_gap);
        assert!(stream.len() >= 100, "case {case}");
        pairs += check_stream(&sc, &stream, case);
        per_mode[(case % 4) as usize] += 1;
        noisy += (case % 2 == 1) as usize;
        let travelled: f64 = stream
            .windows(2)
            .map(|w| distance(w[0].coordinate, w[1].coordinate).unwrap())
            .sum();
        moving_streams += (travelled > 0.0) as usize;
    }
    println!(
        "pipeline: {cases} scenarios (random walk/walking/driving/circular = {per_mode:?}), \
         {noisy} with noise, {pairs} consecutive pairs re-checked, {moving_streams} streams moved"
    );
    assert_eq!(per_mode, [150; 4]);
    assert!(moving_streams as u64 > cases * 95 / 100);
}

#[test]
fn presets_run_cleanly_for_an_hour_each() {
    for (mode, movement) in [
        (MovementMode::Walking, MovementParameters::walking_preset()),
        (MovementMode::Driving, MovementParameters::driving_preset()),
    ] {
        let mut sc = scenario(mode, bengaluru(), movement);
        sc.seed = 2026;
        let stream = run(&sc, 3_600, 1, 0.9);
        assert!(stream.len() > 2_000);
        check_stream(&sc, &stream, 0);
        let fastest = stream
            .iter()
            .filter_map(|s| s.speed_mps)
            .fold(0.0f64, f64::max);
        let stopped = stream.iter().filter(|s| s.speed_mps == Some(0.0)).count();
        let far = distance(bengaluru(), stream.last().unwrap().coordinate).unwrap();
        println!("{mode:?} preset: fastest {fastest:.2} m/s, {stopped} stopped samples, ended {far:.0} m away");
        assert!(fastest > 0.8 * movement.min_speed_mps);
        assert!(fastest <= movement.max_speed_mps);
        assert!(stopped >= 1, "starts at rest");
    }
}

#[test]
fn identical_inputs_reproduce_and_seeds_diverge() {
    let mut rng = Rng::from_seed(0x0506);
    for case in 0..24u64 {
        let sc = random_scenario(&mut rng, case);
        let a = run(&sc, 250, 7, 2.0);
        let b = run(&sc, 250, 7, 2.0);
        assert_eq!(a, b, "case {case}");
        if sc.mode == MovementMode::Circular {
            continue; // deterministic by construction, seed-independent
        }
        let mut other = sc.clone();
        other.seed = sc.seed + 1_000;
        let c = run(&other, 250, 7, 2.0);
        assert_eq!(a.len(), c.len());
        assert!(a.iter().zip(&c).all(|(x, y)| x.timestamp == y.timestamp));
        assert_ne!(a, c, "case {case}: the seed changed nothing");
    }
}

#[test]
fn six_simulated_hours_of_orbit_do_not_drift() {
    let mut m = MovementParameters::walking_preset();
    m.min_speed_mps = 0.0;
    m.max_speed_mps = 3.0;
    m.radius_m = Some(75.0);
    m.angular_velocity_dps = Some(1.5); // 1.96 m/s, 4-minute lap
    let mut sc = scenario(MovementMode::Circular, bengaluru(), m);
    sc.update_interval_s = 0.1;
    let mut p = SimulationProvider::new(sc);
    p.start(Timestamp::from_nanos(START)).unwrap();
    let ticks = 6 * 3_600 * 10i64;
    let first = p.poll(Timestamp::from_nanos(START)).unwrap().unwrap();
    let mut worst_radius_error = 0.0f64;
    let mut last = first;
    for n in 1..ticks {
        last = p
            .poll(Timestamp::from_nanos(START + n * SEC / 10))
            .unwrap()
            .unwrap();
        if n % 97 == 0 {
            let r = distance(bengaluru(), last.coordinate).unwrap();
            worst_radius_error = worst_radius_error.max((r - 75.0).abs());
        }
    }
    assert!(
        worst_radius_error < 1e-6,
        "radius error {worst_radius_error}"
    );
    // 21 599.9 s at 1.5°/s is 32 399.85°: 0.15° short of 90 whole laps.
    let bearing = inverse(bengaluru(), last.coordinate)
        .unwrap()
        .initial_bearing_deg;
    assert!(
        bearing_difference(bearing, 359.85).abs() < 1e-6,
        "{bearing}"
    );
    assert_eq!(p.status().sample_count, ticks as u64);
}

#[test]
fn fenced_noisy_walk_stays_inside_for_two_simulated_hours() {
    let mut m = MovementParameters::walking_preset();
    m.radius_m = Some(60.0);
    let mut sc = scenario(MovementMode::Walking, bengaluru(), m);
    sc.noise = NoiseParameters {
        position_noise_m: 2.0,
        max_position_offset_m: 10.0,
        speed_noise_mps: 0.15,
        heading_noise_deg: 5.0,
        accuracy_noise_m: 0.8,
        drift_rate_mps: 0.05,
        position_correlation_time_s: 6.0,
        max_offset_rate_mps: 1.0,
    };
    let stream = run(&sc, 7_200, 3, 0.9);
    check_stream(&sc, &stream, 0);
    let furthest = stream
        .iter()
        .map(|s| distance(bengaluru(), s.coordinate).unwrap())
        .fold(0.0f64, f64::max);
    assert!(furthest <= 60.0 && furthest > 40.0, "furthest {furthest}");
}

#[test]
fn a_long_pause_does_not_make_a_walker_leap() {
    let mut sc = scenario(
        MovementMode::Walking,
        bengaluru(),
        MovementParameters::walking_preset(),
    );
    sc.movement.pause_probability = 0.0;
    let mut p = SimulationProvider::new(sc);
    p.start(Timestamp::from_nanos(START)).unwrap();
    let mut before = None;
    for n in 0..60 {
        before = p.poll(Timestamp::from_nanos(START + n * SEC)).unwrap();
    }
    let before = before.unwrap();
    assert!(before.speed_mps.unwrap() > 0.5);
    p.pause(Timestamp::from_nanos(START + 59 * SEC)).unwrap();
    let resumed = START + (59 + 86_400) * SEC; // a day later
    p.resume(Timestamp::from_nanos(resumed)).unwrap();
    let after = p
        .poll(Timestamp::from_nanos(resumed + SEC))
        .unwrap()
        .unwrap();
    assert_eq!(after.timestamp.as_nanos(), resumed + SEC);
    // One second of walking, not a day's worth.
    let moved = distance(before.coordinate, after.coordinate).unwrap();
    assert!(moved <= 1.8 && moved > 0.5, "moved {moved}");
}

// --- The gate must catch models that misbehave. ---------------------------

/// Replays a fixed list of samples regardless of time.
struct Scripted(std::vec::IntoIter<MovementSample>);

impl MovementModel for Scripted {
    fn sample_at(&mut self, _t: Timestamp) -> Result<MovementSample, MovementError> {
        Ok(self.0.next().expect("script long enough"))
    }
}

/// A model factory that hands out the script once.
fn scripted_factory(script: Vec<MovementSample>) -> ModelFactory {
    let script = Mutex::new(Some(script));
    Box::new(move |_| {
        let samples = script.lock().unwrap().take().expect("one run per script");
        Ok(Box::new(Scripted(samples.into_iter())))
    })
}

fn at(north_m: f64, speed: f64, course: Option<f64>) -> MovementSample {
    MovementSample {
        coordinate: destination(bengaluru(), 0.0, north_m).unwrap(),
        altitude_m: 15.0,
        speed_mps: Some(speed),
        course_deg: course,
    }
}

/// Runs a script through the provider (walking limits: 1.8 m/s, 0.8 m/s²
/// up, 1.2 m/s² down, 60°/s, 1 Hz) and returns the first rejection and how
/// many samples were emitted before it.
fn first_rejection(script: Vec<MovementSample>, radius_m: Option<f64>) -> (usize, ProviderError) {
    let mut m = MovementParameters::walking_preset();
    m.radius_m = radius_m;
    let sc = scenario(MovementMode::Walking, bengaluru(), m);
    let length = script.len();
    let mut p = SimulationProvider::with_model_factory(sc, scripted_factory(script));
    p.start(Timestamp::from_nanos(START)).unwrap();
    for n in 0..length {
        if let Err(e) = p.poll(Timestamp::from_nanos(START + n as i64 * SEC)) {
            assert_eq!(p.status().state, SimulationState::Error);
            assert_eq!(p.status().sample_count, n as u64);
            return (n, e);
        }
    }
    panic!("the gate accepted the whole script");
}

#[test]
fn gate_rejects_teleportation() {
    let script = vec![
        at(0.0, 0.0, None),
        at(0.0, 0.0, None),
        at(500.0, 0.0, None), // half a kilometre in one second
    ];
    match first_rejection(script, None) {
        (
            2,
            ProviderError::Validation(ValidationError::ImpossibleDisplacement {
                distance_m,
                limit_m,
            }),
        ) => {
            assert!((distance_m - 500.0).abs() < 1e-6);
            assert_eq!(limit_m, 1.8);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn gate_rejects_instantaneous_acceleration() {
    // Positions are plausible; only the speed jumps from rest to full.
    let script = vec![at(0.0, 0.0, None), at(0.9, 1.8, Some(0.0))];
    match first_rejection(script, None) {
        (
            1,
            ProviderError::Validation(ValidationError::AccelerationExceeded {
                change_mps,
                limit_mps,
            }),
        ) => {
            assert_eq!((change_mps, limit_mps), (1.8, 0.8));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn gate_rejects_instantaneous_stops() {
    let script = vec![
        at(0.0, 0.0, None),
        at(0.4, 0.8, Some(0.0)),
        at(1.6, 1.6, Some(0.0)),
        at(3.2, 1.6, Some(0.0)),
        at(4.0, 0.0, None), // 1.6 m/s to nothing in a second
    ];
    match first_rejection(script, None) {
        (
            4,
            ProviderError::Validation(ValidationError::DecelerationExceeded {
                change_mps,
                limit_mps,
            }),
        ) => {
            assert_eq!((change_mps, limit_mps), (1.6, 1.2));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn gate_rejects_instantaneous_heading_changes() {
    let script = vec![
        at(0.0, 0.0, None),
        at(0.4, 0.8, Some(0.0)),
        at(1.4, 1.0, Some(0.0)),
        at(2.4, 1.0, Some(0.0)),
        at(1.4, 1.0, Some(180.0)), // about-turn at full stride
    ];
    match first_rejection(script, None) {
        (
            4,
            ProviderError::Validation(ValidationError::HeadingRateExceeded {
                turn_deg,
                limit_deg,
            }),
        ) => {
            assert!((turn_deg - 180.0).abs() < 1e-9);
            assert_eq!(limit_deg, 60.0);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_model_leaving_the_boundary_is_stopped_before_emission() {
    // Kinematically impeccable, but it walks out of a 3 m fence.
    let script = vec![
        at(0.0, 0.0, None),
        at(0.4, 0.8, Some(0.0)),
        at(1.4, 1.0, Some(0.0)),
        at(2.4, 1.0, Some(0.0)),
        at(3.4, 1.0, Some(0.0)),
    ];
    match first_rejection(script, Some(3.0)) {
        // The noise stage sits upstream and refuses an out-of-bounds base
        // first; the gate's own boundary check is covered by its unit tests.
        (
            4,
            ProviderError::Noise(NoiseError::BaseOutsideBoundary {
                distance_m,
                radius_m,
            }),
        ) => {
            assert!((distance_m - 3.4).abs() < 1e-6);
            assert_eq!(radius_m, 3.0);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_well_behaved_script_is_accepted() {
    // The same harness must not reject legitimate motion.
    let script = vec![
        at(0.0, 0.0, None),
        at(0.4, 0.8, Some(0.0)),
        at(1.6, 1.6, Some(0.0)),
        at(3.3, 1.8, Some(0.0)),
        at(4.8, 1.2, Some(0.0)),
        at(5.4, 0.0, None),
    ];
    let mut m = MovementParameters::walking_preset();
    m.radius_m = Some(10.0);
    let sc = scenario(MovementMode::Walking, bengaluru(), m);
    let mut p = SimulationProvider::with_model_factory(sc, scripted_factory(script));
    p.start(Timestamp::from_nanos(START)).unwrap();
    for n in 0..6 {
        p.poll(Timestamp::from_nanos(START + n * SEC))
            .unwrap()
            .unwrap();
    }
    assert_eq!(p.status().sample_count, 6);
}
