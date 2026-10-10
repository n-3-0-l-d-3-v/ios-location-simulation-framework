//! T08: a valid scenario survives export and import exactly, and so does
//! the stream it produces.
//!
//! Random valid scenarios of every mode are placed at the poles, on the
//! date line and elsewhere, sampled from 1 ms to 10 s, with and without
//! noise. Each is exported and imported; the scenario must come back bit
//! for bit, the text must be canonical, and the imported scenario must
//! drive the provider to the identical stream, which is re-checked
//! independently.

mod support;

use locsim_core::domain::{
    Coordinate, MovementMode, MovementParameters, NoiseParameters, PlaybackParameters, Scenario,
    CURRENT_SCHEMA_VERSION,
};
use locsim_core::rng::Rng;
use locsim_core::route::RoutePlan;
use locsim_scenario::{export_scenario, import_scenario};
use support::common::{
    closed_route, declared_limits, log_uniform, recheck_stream, route_scenario, smooth_route,
    special_origin, Recheck,
};
use support::{fingerprint, run};

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

/// A valid scenario of the mode chosen by `case`, sampled every `interval`.
fn valid_scenario(rng: &mut Rng, case: u64, origin: Coordinate, interval: f64) -> Scenario {
    let walking = MovementParameters::walking_preset;
    let mut sc = match case % 7 {
        0 => {
            let mut m = walking();
            m.min_speed_mps = 0.0;
            m.max_speed_mps = 0.0;
            scenario(MovementMode::Fixed, origin, m)
        }
        1 => {
            let mut m = walking();
            m.min_speed_mps = 0.0;
            m.max_speed_mps = 3.0;
            m.radius_m = Some(rng.uniform(40.0, 80.0));
            m.angular_velocity_dps = Some(rng.uniform(0.5, 1.5));
            m.start_phase_deg = rng.uniform(0.0, 360.0);
            scenario(MovementMode::Circular, origin, m)
        }
        2 => {
            let mut m = walking();
            m.radius_m = Some(rng.uniform(30.0, 60.0));
            m.step_distance_m = Some(1.2 * interval);
            scenario(MovementMode::RandomWalk, origin, m)
        }
        3 => {
            let mut m = walking();
            m.radius_m = (case % 2 == 0).then(|| rng.uniform(100.0, 400.0));
            scenario(MovementMode::Walking, origin, m)
        }
        4 => scenario(
            MovementMode::Driving,
            origin,
            MovementParameters::driving_preset(),
        ),
        5 => {
            let route = smooth_route(rng, origin, 20, 1.4, true);
            let plan = RoutePlan::build(&route, &PlaybackParameters::REAL_TIME, interval).unwrap();
            let limits = declared_limits(&plan, origin, 1.05, interval).unwrap();
            route_scenario(route, PlaybackParameters::REAL_TIME, &limits, interval)
        }
        _ => {
            let route = closed_route(rng, origin, 12, 60.0, 240.0);
            let playback = PlaybackParameters {
                speed: 1.0,
                looping: true,
                reverse: false,
            };
            let plan = RoutePlan::build(&route, &playback, interval).unwrap();
            let limits = declared_limits(&plan, origin, 1.05, interval).unwrap();
            route_scenario(route, playback, &limits, interval)
        }
    };
    sc.update_interval_s = interval;
    sc.seed = match case % 5 {
        0 => 0,
        1 => u64::MAX,
        _ => rng.next_u64(),
    };
    if case % 2 == 1 {
        let sigma = log_uniform(rng, -1.0, 0.5);
        sc.noise = NoiseParameters {
            position_noise_m: sigma,
            max_position_offset_m: 3.0 * sigma * rng.uniform(1.05, 3.0),
            speed_noise_mps: rng.uniform(0.0, 0.5),
            heading_noise_deg: rng.uniform(0.0, 10.0),
            accuracy_noise_m: rng.uniform(0.0, 1.9),
            drift_rate_mps: 0.0,
            position_correlation_time_s: rng.uniform(0.0, 20.0),
            max_offset_rate_mps: log_uniform(rng, -1.5, 0.5),
        };
    }
    sc
}

#[test]
fn valid_scenarios_and_their_streams_survive_export_and_import_exactly() {
    let mut rng = Rng::from_seed(0x0808);
    let mut total = Recheck::default();
    let cases = 210u64;
    let mut bytes = 0;
    for case in 0..cases {
        let origin = special_origin(&mut rng, case);
        // 1 ms … 10 s, with the two ends hit exactly now and then.
        let interval = match case % 11 {
            0 => 0.001,
            1 => 10.0,
            _ => log_uniform(&mut rng, -3.0, 1.0),
        };
        let sc = valid_scenario(&mut rng, case, origin, interval);
        assert_eq!(sc.validate(), Ok(()), "case {case}");

        let text = export_scenario(&sc).unwrap_or_else(|e| panic!("case {case}: {e:?}"));
        let back = import_scenario(&text).unwrap_or_else(|e| panic!("case {case}: {e:?}\n{text}"));
        assert_eq!(fingerprint(&back), fingerprint(&sc), "case {case}");
        assert_eq!(export_scenario(&back).unwrap(), text, "case {case}");
        bytes += text.len();

        let jitter = (case % 3 == 0).then_some(case);
        let (from_original, _) = run(&sc, 200, jitter);
        let (from_imported, _) = run(&back, 200, jitter);
        assert_eq!(
            fingerprint(&from_imported),
            fingerprint(&from_original),
            "case {case}"
        );
        total.absorb(recheck_stream(
            &back,
            &from_imported,
            &format!("case {case}"),
        ));
    }
    println!(
        "round trip: {cases} scenarios, {bytes} bytes of JSON; {} pairs re-checked \
         ({} with a course, {} stationary)",
        total.pairs, total.with_course, total.stationary
    );
    assert!(total.pairs > 25_000 && total.with_course > 15_000);
}

#[test]
fn the_sign_of_zero_and_the_last_bit_survive() {
    let mut sc = scenario(
        MovementMode::Walking,
        Coordinate::new(-0.0, -0.0).unwrap(),
        MovementParameters::walking_preset(),
    );
    sc.altitude_m = -0.0;
    sc.movement.start_phase_deg = -0.0;
    sc.movement.heading_persistence = 0.1 + 0.2; // 0.30000000000000004
    sc.movement.max_speed_mps = f64::from_bits(1.8f64.to_bits() + 1);
    sc.horizontal_accuracy_m = f64::MIN_POSITIVE;
    sc.vertical_accuracy_m = f64::MAX;
    sc.update_interval_s = 1.0 / 3.0;
    let text = export_scenario(&sc).unwrap();
    let back = import_scenario(&text).unwrap();
    assert_eq!(fingerprint(&back), fingerprint(&sc));
    assert_eq!(back.origin.latitude().to_bits(), (-0.0f64).to_bits());
    assert_eq!(back.origin.longitude().to_bits(), (-0.0f64).to_bits());
    assert_eq!(back.altitude_m.to_bits(), (-0.0f64).to_bits());
    assert_eq!(back.movement.max_speed_mps.to_bits(), 1.8f64.to_bits() + 1);
    assert!(text.contains("\"heading_persistence\": 0.30000000000000004"));
    assert!(text.contains("\"latitude\": -0.0"));
}
