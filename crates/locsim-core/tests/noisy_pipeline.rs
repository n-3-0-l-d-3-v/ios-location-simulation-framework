//! End-to-end checks that noise cannot make the *final output* of the
//! pipeline (scenario → movement → noise → validation → provider) violate
//! coordinate bounds, the boundary, displacement, speed or timestamp order.
//!
//! Movement is the fixed model, the only one that exists so far.

use locsim_core::domain::{
    Coordinate, MovementMode, MovementParameters, NoiseParameters, PlaybackParameters,
    RotationDirection, Scenario, SimulationState, SyntheticLocation, Timestamp,
    CURRENT_SCHEMA_VERSION, NOISE_CLIP_SIGMA,
};
use locsim_core::geographic::distance;
use locsim_core::noise::NoiseError;
use locsim_core::provider::{LocationProvider, ProviderError, SimulationProvider};
use locsim_core::rng::Rng;

fn scenario(
    origin: Coordinate,
    noise: NoiseParameters,
    radius_m: Option<f64>,
    seed: u64,
) -> Scenario {
    Scenario {
        schema_version: CURRENT_SCHEMA_VERSION,
        name: "noisy fixed".into(),
        origin,
        altitude_m: 30.0,
        mode: MovementMode::Fixed,
        movement: MovementParameters {
            min_speed_mps: 0.0,
            max_speed_mps: 0.0,
            max_acceleration_mps2: 0.0,
            max_deceleration_mps2: 0.0,
            max_heading_rate_dps: 0.0,
            radius_m,
            step_distance_m: None,
            heading_persistence: 0.0,
            pause_probability: 0.0,
            max_pause_s: 0.0,
            angular_velocity_dps: None,
            direction: RotationDirection::Clockwise,
            start_phase_deg: 0.0,
            speed_change_interval_s: 0.0,
            max_displacement_per_sample_m: None,
        },
        noise,
        horizontal_accuracy_m: 6.0,
        vertical_accuracy_m: 9.0,
        update_interval_s: 1.0,
        seed,
        route: None,
        playback: PlaybackParameters::REAL_TIME,
    }
}

fn typical_noise() -> NoiseParameters {
    NoiseParameters {
        position_noise_m: 2.0,
        max_position_offset_m: 12.0,
        speed_noise_mps: 0.5,
        heading_noise_deg: 10.0,
        accuracy_noise_m: 0.8,
        drift_rate_mps: 0.05,
        position_correlation_time_s: 6.0,
        max_offset_rate_mps: 1.5,
    }
}

/// Polls irregularly (gaps up to `max_gap` intervals) and returns the stream.
fn run(sc: &Scenario, polls: usize, poll_seed: u64, max_gap: f64) -> Vec<SyntheticLocation> {
    let start = 1_700_000_000_000_000_000i64;
    let interval = sc.update_interval_s * 1e9;
    let mut rng = Rng::from_seed(poll_seed);
    let mut p = SimulationProvider::new(sc.clone());
    p.start(Timestamp::from_nanos(start)).unwrap();
    let mut now = start;
    let mut out = Vec::new();
    for _ in 0..polls {
        match p.poll(Timestamp::from_nanos(now)) {
            Ok(Some(s)) => out.push(s),
            Ok(None) => {}
            Err(e) => panic!("pipeline rejected a sample: {e}"),
        }
        now += (rng.uniform(0.0, max_gap) * interval) as i64;
    }
    let st = p.status();
    assert_eq!(st.state, SimulationState::Running);
    assert_eq!(st.failed_count, 0);
    assert_eq!(st.sample_count, out.len() as u64);
    out
}

#[test]
fn final_output_respects_every_limit_for_random_noisy_scenarios() {
    let mut rng = Rng::from_seed(0x0404);
    let mut noisy_streams = 0;
    for case in 0..300u64 {
        let origin = match case % 10 {
            0 => Coordinate::new(90.0, 0.0).unwrap(),
            1 => Coordinate::new(-90.0, 0.0).unwrap(),
            2 => Coordinate::new(rng.uniform(-80.0, 80.0), 180.0).unwrap(),
            _ => Coordinate::new(rng.uniform(-90.0, 90.0), rng.uniform(-180.0, 180.0)).unwrap(),
        };
        let sigma = 10f64.powf(rng.uniform(-1.5, 1.3));
        let rate = 10f64.powf(rng.uniform(-2.0, 1.3));
        let noise = NoiseParameters {
            position_noise_m: sigma,
            max_position_offset_m: NOISE_CLIP_SIGMA * sigma * rng.uniform(1.05, 3.0),
            speed_noise_mps: rng.uniform(0.0, 2.0),
            heading_noise_deg: rng.uniform(0.0, 30.0),
            accuracy_noise_m: rng.uniform(0.0, 1.9),
            drift_rate_mps: if case % 2 == 0 {
                rng.uniform(0.0, rate)
            } else {
                0.0
            },
            position_correlation_time_s: if case % 3 == 0 {
                0.0
            } else {
                rng.uniform(0.5, 60.0)
            },
            max_offset_rate_mps: rate,
        };
        // Some boundaries are much tighter than the noise.
        let radius = (case % 2 == 1).then(|| sigma * rng.uniform(0.05, 10.0));
        let mut sc = scenario(origin, noise, radius, case);
        sc.update_interval_s = 10f64.powf(rng.uniform(-1.5, 1.0));
        assert_eq!(sc.validate(), Ok(()), "case {case}");

        let stream = run(&sc, 400, case ^ 0x77, 2.5);
        assert!(stream.len() > 50, "case {case}");
        let interval_ns = (sc.update_interval_s * 1e9).round() as i64;

        let mut previous: Option<SyntheticLocation> = None;
        let mut moved = false;
        for s in &stream {
            // Field validity implies finite values and coordinate bounds.
            assert_eq!(s.validate(), Ok(()), "case {case}");
            let (lat, lon) = (s.coordinate.latitude(), s.coordinate.longitude());
            assert!((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon));

            // A fixed scenario stays stationary in its metadata.
            assert_eq!(
                (s.speed_mps, s.course_deg),
                (Some(0.0), None),
                "case {case}"
            );
            assert_eq!(s.altitude_m, 30.0);

            // Offset bound and boundary, measured from the scenario origin.
            let from_origin = distance(origin, s.coordinate).unwrap();
            assert!(from_origin <= noise.max_position_offset_m, "case {case}");
            if let Some(r) = radius {
                assert!(from_origin <= r, "case {case}: {from_origin} > {r}");
            }

            // Accuracy positive and within the clipped reach.
            let reach = NOISE_CLIP_SIGMA * noise.accuracy_noise_m;
            assert!(s.horizontal_accuracy_m > 0.0 && s.vertical_accuracy_m > 0.0);
            assert!((s.horizontal_accuracy_m - 6.0).abs() <= reach + 1e-12);
            assert!((s.vertical_accuracy_m - 9.0).abs() <= reach + 1e-12);

            if let Some(prev) = previous {
                // Timestamps strictly increase and stay on the tick grid.
                assert!(s.timestamp > prev.timestamp, "case {case}");
                let dt_ns = s.timestamp.as_nanos() - prev.timestamp.as_nanos();
                assert_eq!(dt_ns % interval_ns, 0, "case {case}");
                // Maximum displacement: the true point does not move, so the
                // output may move at most max_offset_rate × elapsed time,
                // including across skipped ticks.
                let step = distance(prev.coordinate, s.coordinate).unwrap();
                let limit = noise.max_offset_rate_mps * s.timestamp.seconds_since(prev.timestamp);
                assert!(step <= limit, "case {case}: step {step} > {limit}");
                moved |= step > 0.0;
            }
            previous = Some(*s);
        }
        noisy_streams += moved as u32;
    }
    assert!(
        noisy_streams > 290,
        "noise was active in only {noisy_streams} streams"
    );
}

#[test]
fn noisy_stream_is_reproducible_and_seed_dependent() {
    let origin = Coordinate::new(12.9352, 77.6245).unwrap();
    let a = run(
        &scenario(origin, typical_noise(), None, 12345),
        1_000,
        9,
        2.0,
    );
    let b = run(
        &scenario(origin, typical_noise(), None, 12345),
        1_000,
        9,
        2.0,
    );
    let c = run(
        &scenario(origin, typical_noise(), None, 12346),
        1_000,
        9,
        2.0,
    );
    assert_eq!(a, b);
    assert_eq!(a.len(), c.len());
    assert_ne!(a, c);
    // Same timestamps either way: the seed changes noise, not timing.
    assert!(a.iter().zip(&c).all(|(x, y)| x.timestamp == y.timestamp));
}

#[test]
fn restart_replays_the_same_noise() {
    let origin = Coordinate::new(51.5074, -0.1278).unwrap();
    let mut p = SimulationProvider::new(scenario(origin, typical_noise(), Some(8.0), 5));
    let start = Timestamp::from_nanos(1_700_000_000_000_000_000);
    let collect = |p: &mut SimulationProvider| -> Vec<SyntheticLocation> {
        p.start(start).unwrap();
        let v = (0..300)
            .map(|n| {
                p.poll(Timestamp::from_nanos(start.as_nanos() + n * 1_000_000_000))
                    .unwrap()
                    .unwrap()
            })
            .collect();
        p.stop().unwrap();
        v
    };
    let first = collect(&mut p);
    let second = collect(&mut p);
    assert_eq!(first, second);
}

#[test]
fn disabled_noise_leaves_the_fixed_stream_exact() {
    let origin = Coordinate::new(-33.8688, 151.2093).unwrap();
    let stream = run(
        &scenario(origin, NoiseParameters::NONE, Some(1.0), 1),
        500,
        3,
        1.5,
    );
    assert!(stream.iter().all(|s| s.coordinate == origin
        && s.horizontal_accuracy_m == 6.0
        && s.vertical_accuracy_m == 9.0));
}

#[test]
fn invalid_noise_configuration_is_rejected_before_starting() {
    let origin = Coordinate::new(0.0, 0.0).unwrap();
    let bad = NoiseParameters {
        position_noise_m: 2.0, // no offset bound, no rate limit
        ..NoiseParameters::NONE
    };
    let mut p = SimulationProvider::new(scenario(origin, bad, None, 1));
    match p.start(Timestamp::from_nanos(0)) {
        Err(ProviderError::InvalidScenario(errors)) => {
            let fields: Vec<_> = errors.iter().map(|e| e.field).collect();
            assert_eq!(
                fields,
                ["noise.max_position_offset_m", "noise.max_offset_rate_mps"]
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(p.status().state, SimulationState::Idle);
    // NoiseError is part of the provider's error surface.
    let _: fn(NoiseError) -> ProviderError = ProviderError::from;
}
