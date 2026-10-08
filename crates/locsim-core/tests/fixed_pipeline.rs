//! End-to-end properties of the fixed-location pipeline:
//! scenario → movement → validation → scheduler → provider.

use locsim_core::domain::{
    Coordinate, MovementMode, MovementParameters, NoiseParameters, PlaybackParameters,
    RotationDirection, Scenario, SimulationState, SyntheticLocation, Timestamp,
    CURRENT_SCHEMA_VERSION,
};
use locsim_core::provider::{LocationProvider, SimulationProvider};
use locsim_core::rng::Rng;

fn scenario(origin: Coordinate, interval_s: f64, seed: u64) -> Scenario {
    Scenario {
        schema_version: CURRENT_SCHEMA_VERSION,
        name: "fixed".into(),
        origin,
        altitude_m: 12.0,
        mode: MovementMode::Fixed,
        movement: MovementParameters {
            min_speed_mps: 0.0,
            max_speed_mps: 0.0,
            max_acceleration_mps2: 0.0,
            max_deceleration_mps2: 0.0,
            max_heading_rate_dps: 0.0,
            radius_m: None,
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
        noise: NoiseParameters::NONE,
        horizontal_accuracy_m: 5.0,
        vertical_accuracy_m: 8.0,
        update_interval_s: interval_s,
        seed,
        route: None,
        playback: PlaybackParameters::REAL_TIME,
    }
}

/// Drives a provider with an irregular poller and returns what it emitted.
/// `poll_seed` controls only *when* polls happen.
fn run(
    sc: &Scenario,
    start: Timestamp,
    polls: usize,
    poll_seed: u64,
    max_gap_intervals: f64,
) -> (Vec<SyntheticLocation>, SimulationProvider) {
    let interval = sc.update_interval_s * 1e9;
    let mut rng = Rng::from_seed(poll_seed);
    let mut p = SimulationProvider::new(sc.clone());
    p.start(start).unwrap();
    let mut now = start.as_nanos();
    let mut out = Vec::new();
    for _ in 0..polls {
        if let Some(s) = p.poll(Timestamp::from_nanos(now)).unwrap() {
            out.push(s);
        }
        now += (rng.uniform(0.0, max_gap_intervals) * interval) as i64;
    }
    (out, p)
}

#[test]
fn samples_are_valid_stationary_and_on_the_grid_for_random_scenarios() {
    let mut rng = Rng::from_seed(0x5EED);
    for case in 0..500 {
        let origin = Coordinate::new(rng.uniform(-90.0, 90.0), rng.uniform(-180.0, 180.0)).unwrap();
        let interval_s = 10f64.powf(rng.uniform(-2.0, 1.0)); // 10 ms … 10 s
        let sc = scenario(origin, interval_s, case);
        let start = Timestamp::from_nanos(rng.uniform(0.0, 2e18) as i64);
        // Gaps up to 3 intervals: some ticks are punctual, some are missed.
        let (samples, p) = run(&sc, start, 200, case ^ 0xABCD, 3.0);

        assert!(
            samples.len() > 20,
            "case {case}: only {} samples",
            samples.len()
        );
        let interval_ns = (interval_s * 1e9).round() as i64;
        let mut previous: Option<Timestamp> = None;
        for s in &samples {
            assert_eq!(s.validate(), Ok(()), "case {case}");
            assert_eq!(s.coordinate, origin, "case {case}");
            // Speed is derived from the emitted positions: unknown for the
            // first fix, exactly zero for every later one. Never a course.
            assert_eq!(s.speed_mps, previous.map(|_| 0.0), "case {case}");
            assert_eq!(s.course_deg, None, "case {case}");
            assert_eq!(s.simulation_state, SimulationState::Running);
            // Exactly on the grid: no drift regardless of poll timing.
            let offset = s.timestamp.as_nanos() - start.as_nanos();
            assert_eq!(offset % interval_ns, 0, "case {case}: off-grid by {offset}");
            if let Some(prev) = previous {
                assert!(s.timestamp > prev, "case {case}: timestamps not increasing");
            }
            previous = Some(s.timestamp);
        }

        // Every slot up to the last emitted one is either emitted or counted missed.
        let st = p.status();
        let last_slot = (previous.unwrap().as_nanos() - start.as_nanos()) / interval_ns;
        assert_eq!(st.sample_count, samples.len() as u64, "case {case}");
        assert_eq!(
            st.sample_count + st.missed_ticks,
            last_slot as u64 + 1,
            "case {case}"
        );
        assert_eq!(st.failed_count, 0);
    }
}

#[test]
fn identical_inputs_reproduce_the_identical_stream() {
    let sc = scenario(Coordinate::new(12.9352, 77.6245).unwrap(), 0.5, 12345);
    let start = Timestamp::from_nanos(1_700_000_000_000_000_000);
    let (a, _) = run(&sc, start, 2_000, 99, 2.5);
    let (b, _) = run(&sc, start, 2_000, 99, 2.5);
    assert!(!a.is_empty());
    assert_eq!(a, b);
}

#[test]
fn punctual_polling_makes_content_independent_of_poll_jitter() {
    // With gaps under one interval no slot is ever missed, so two different
    // poll patterns must yield exactly the same samples.
    let sc = scenario(Coordinate::new(-33.8688, 151.2093).unwrap(), 1.0, 7);
    let start = Timestamp::from_nanos(1_700_000_000_000_000_000);
    let (a, pa) = run(&sc, start, 5_000, 1, 0.9);
    let (b, pb) = run(&sc, start, 5_000, 2, 0.9);
    assert_eq!(pa.status().missed_ticks, 0);
    assert_eq!(pb.status().missed_ticks, 0);
    let n = a.len().min(b.len());
    assert!(n > 1_000);
    assert_eq!(a[..n], b[..n]);
}

#[test]
fn six_simulated_hours_at_10_hz_stay_exact() {
    let sc = scenario(Coordinate::new(51.5074, -0.1278).unwrap(), 0.1, 1);
    let start = Timestamp::from_nanos(1_700_000_000_000_000_000);
    let mut p = SimulationProvider::new(sc);
    p.start(start).unwrap();
    let step = 100_000_000i64;
    let ticks = 6 * 3600 * 10;
    let mut last = None;
    for n in 0..ticks {
        // Always 7 ms late: lateness must not leak into timestamps.
        let now = Timestamp::from_nanos(start.as_nanos() + n * step + 7_000_000);
        last = Some(p.poll(now).unwrap().expect("a sample every tick"));
    }
    let st = p.status();
    assert_eq!(
        (st.sample_count, st.missed_ticks, st.failed_count),
        (ticks as u64, 0, 0)
    );
    assert_eq!(
        last.unwrap().timestamp.as_nanos(),
        start.as_nanos() + (ticks - 1) * step
    );
}
