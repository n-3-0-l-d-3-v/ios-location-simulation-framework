//! T08: the documents in `Examples/Scenarios` are real, current and usable.
//!
//! Each one is imported, compared with what it claims to be, written back
//! and compared byte for byte, and then run through the provider and the
//! final validation gate, with the emitted stream re-checked independently.

mod support;

use locsim_core::domain::{
    Coordinate, MovementMode, MovementParameters, NoiseParameters, PlaybackParameters,
    RotationDirection, Scenario, SimulationState, Timestamp, CURRENT_SCHEMA_VERSION,
};
use locsim_core::geographic::distance;
use locsim_core::provider::{LocationProvider, ProviderError, SimulationProvider};
use locsim_core::route::admit;
use locsim_scenario::{export_scenario, import_scenario, ScenarioError};
use support::common::{recheck_stream, Recheck, START};
use support::{example, fingerprint, run, EXAMPLES};

fn import(name: &str) -> Scenario {
    import_scenario(example(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

#[test]
fn the_examples_directory_holds_exactly_the_tested_files() {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Examples/Scenarios");
    let mut on_disk: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != ".gitkeep")
        .collect();
    let mut tested: Vec<String> = EXAMPLES.iter().map(|(n, _)| format!("{n}.json")).collect();
    on_disk.sort();
    tested.sort();
    assert_eq!(on_disk, tested);
}

#[test]
fn every_example_imports_and_is_already_in_exported_form() {
    for (name, text) in EXAMPLES {
        let scenario = import(name);
        assert_eq!(scenario.validate(), Ok(()), "{name}");
        assert_eq!(scenario.schema_version, CURRENT_SCHEMA_VERSION);
        // Byte for byte: a file edited by hand into another spelling of the
        // same numbers would still import, but would show up here.
        assert_eq!(export_scenario(&scenario).unwrap(), text, "{name}");
    }
}

#[test]
fn the_examples_cover_every_mode_and_the_awkward_places() {
    let mut modes: Vec<String> = EXAMPLES
        .iter()
        .map(|(name, _)| format!("{:?}", import(name).mode))
        .collect();
    modes.sort();
    modes.dedup();
    assert_eq!(
        modes,
        [
            "Circular",
            "Driving",
            "Fixed",
            "RandomWalk",
            "RouteReplay",
            "Walking"
        ]
    );
    assert_eq!(import("antimeridian_loop").origin.longitude(), 180.0);
    assert!(import("high_latitude").origin.latitude() > 89.9);
    // Seeds beyond what a JSON double holds exactly.
    assert_eq!(import("driving").seed, u64::MAX);
    assert_eq!(import("high_latitude").seed, (1 << 53) + 1);
}

/// What `walking.json` says, written independently as Rust. This ties the
/// member names of a real file to the fields they are meant to fill.
#[test]
fn the_walking_example_is_the_scenario_it_describes() {
    let expected = Scenario {
        schema_version: 1,
        name: "Walking Test".into(),
        origin: Coordinate::new(12.9352, 77.6245).unwrap(),
        altitude_m: 920.0,
        mode: MovementMode::Walking,
        movement: MovementParameters {
            min_speed_mps: 0.8,
            max_speed_mps: 1.8,
            max_acceleration_mps2: 0.8,
            max_deceleration_mps2: 1.2,
            max_heading_rate_dps: 60.0,
            radius_m: Some(500.0),
            step_distance_m: None,
            heading_persistence: 0.9,
            pause_probability: 0.01,
            max_pause_s: 20.0,
            angular_velocity_dps: None,
            direction: RotationDirection::Clockwise,
            start_phase_deg: 0.0,
            speed_change_interval_s: 30.0,
            max_displacement_per_sample_m: None,
        },
        noise: NoiseParameters {
            position_noise_m: 1.5,
            max_position_offset_m: 8.0,
            speed_noise_mps: 0.1,
            heading_noise_deg: 4.0,
            accuracy_noise_m: 0.8,
            drift_rate_mps: 0.05,
            position_correlation_time_s: 6.0,
            max_offset_rate_mps: 1.0,
        },
        horizontal_accuracy_m: 5.0,
        vertical_accuracy_m: 8.0,
        update_interval_s: 1.0,
        seed: 12345,
        route: None,
        playback: PlaybackParameters::REAL_TIME,
    };
    assert_eq!(fingerprint(&import("walking")), fingerprint(&expected));
}

#[test]
fn the_route_examples_carry_their_recordings_exactly() {
    let open = import("route_replay");
    let route = open.route.as_ref().unwrap();
    assert_eq!(route.name(), Some("Recorded walk, 40 s"));
    assert_eq!(route.points().len(), 11);
    assert_eq!(route.duration_ns(), 40_000_000_000);
    assert!(route.has_altitude() && !route.is_closed());
    assert_eq!(route.points()[0].coordinate, open.origin);
    assert_eq!(route.points()[10].altitude_m, Some(925.0));

    let circuit = import("antimeridian_loop");
    let route = circuit.route.as_ref().unwrap();
    assert_eq!(route.points().len(), 13);
    assert!(route.is_closed() && !route.has_altitude());
    assert!(circuit.playback.looping);
    // Twelve distinct points every 30° of bearing from a centre on the
    // 180th meridian: two on it, five on each side.
    let longitudes: Vec<f64> = route.points()[..12]
        .iter()
        .map(|p| p.coordinate.longitude())
        .collect();
    let on = longitudes.iter().filter(|l| l.abs() == 180.0).count();
    let east = longitudes.iter().filter(|l| (-180.0..0.0).contains(*l));
    let west = longitudes.iter().filter(|l| (0.0..180.0).contains(*l));
    assert_eq!((on, east.count(), west.count()), (2, 5, 5));
}

// --- Through the provider and the final gate ------------------------------------

#[test]
fn every_example_runs_through_the_provider_and_the_final_gate() {
    let mut total = Recheck::default();
    for (name, _) in EXAMPLES {
        let scenario = import(name);
        for jitter in [None, Some(8)] {
            let (stream, status) = run(&scenario, 900, jitter);
            assert_eq!(status.state, SimulationState::Running, "{name}");
            assert!(stream.len() >= 600, "{name}: {} samples", stream.len());
            assert_eq!(status.sample_count as usize, stream.len());
            total.absorb(recheck_stream(&scenario, &stream, name));
        }
    }
    println!(
        "examples: {} pairs re-checked ({} with a course, {} stationary)",
        total.pairs, total.with_course, total.stationary
    );
    assert!(total.pairs > 9_600 && total.with_course > 4_000 && total.stationary > 0);
}

#[test]
fn an_imported_scenario_produces_the_stream_of_the_scenario_it_was_exported_from() {
    // Export → import must not move a single bit of the output (C1).
    for (name, text) in EXAMPLES {
        let first = import(name);
        let second = import_scenario(&export_scenario(&first).unwrap()).unwrap();
        let (a, _) = run(&first, 400, Some(3));
        let (b, _) = run(&second, 400, Some(3));
        assert_eq!(fingerprint(&a), fingerprint(&b), "{name}");
        // Insignificant formatting does not matter either.
        let compact: String = text.split('\n').map(str::trim_start).collect();
        let third = import_scenario(&compact).unwrap();
        assert_eq!(fingerprint(&third), fingerprint(&first), "{name}");
    }
}

#[test]
fn the_antimeridian_circuit_crosses_the_date_line_on_every_lap() {
    let scenario = import("antimeridian_loop");
    let (stream, status) = run(&scenario, 720, None); // three laps of 240 s
    assert!(!status.trajectory_complete);
    let sides: Vec<bool> = stream
        .iter()
        .filter(|s| s.coordinate.longitude().abs() < 180.0)
        .map(|s| s.coordinate.longitude() > 0.0)
        .collect();
    let crossings = sides.windows(2).filter(|w| w[0] != w[1]).count();
    // The circuit starts on the meridian and meets it every 120 s: inside
    // 720 samples that is at 120, 240, 360, 480 and 600 s.
    assert_eq!(crossings, 5);
    for s in &stream {
        assert!(s.coordinate.longitude().abs() > 179.99, "{s:?}");
        let from_centre = distance(scenario.origin, s.coordinate).unwrap();
        assert!((70.0..90.0).contains(&from_centre), "{from_centre}");
    }
    // The same point of the circuit, one and two laps later.
    assert_eq!(stream[5].coordinate, stream[245].coordinate);
    assert_eq!(stream[5].coordinate, stream[485].coordinate);
}

#[test]
fn the_high_latitude_walk_stays_by_the_pole_and_moves() {
    let scenario = import("high_latitude");
    let (stream, _) = run(&scenario, 900, None);
    let mut farthest: f64 = 0.0;
    for s in &stream {
        assert!(s.coordinate.latitude() > 89.94, "{s:?}");
        farthest = farthest.max(distance(scenario.origin, s.coordinate).unwrap());
    }
    assert!(farthest > 50.0 && farthest <= 300.0, "{farthest}");
}

#[test]
fn the_open_route_is_replayed_to_its_end_and_held_there() {
    let mut scenario = import("route_replay");
    scenario.noise = NoiseParameters::NONE;
    let (stream, status) = run(&scenario, 60, None);
    assert!(status.trajectory_complete);
    let route = scenario.route.as_ref().unwrap();
    // Sampled every second; recorded every four.
    for (j, point) in route.points().iter().enumerate() {
        let d = distance(stream[4 * j].coordinate, point.coordinate).unwrap();
        assert!(d < 1e-6, "point {j}: {d} m");
    }
    assert_eq!(stream[59].coordinate, stream[40].coordinate);
    assert_eq!(stream[59].speed_mps, Some(0.0));
}

#[test]
fn import_validates_a_route_but_admission_happens_when_a_run_starts() {
    let original = example("route_replay");
    // A speed limit below the recording is a scenario error, found on import.
    let slow = original.replace("\"max_speed_mps\": 1.9", "\"max_speed_mps\": 1.0");
    assert_ne!(slow, original);
    match import_scenario(&slow).unwrap_err().as_slice() {
        [ScenarioError::InvalidScenario(e)] => assert_eq!(e.field, "route.points"),
        other => panic!("{other:?}"),
    }
    // An acceleration limit below what the interpolated trajectory needs is
    // not: the document imports, and the provider refuses to start it.
    let gentle = original.replace(
        "\"max_acceleration_mps2\": 0.48",
        "\"max_acceleration_mps2\": 0.1",
    );
    assert_ne!(gentle, original);
    let scenario = import_scenario(&gentle).unwrap();
    assert!(admit(&scenario).is_err());
    let mut provider = SimulationProvider::new(scenario);
    let refused = provider.start(Timestamp::from_nanos(START));
    assert!(
        matches!(refused, Err(ProviderError::Movement(_))),
        "{refused:?}"
    );
    assert_eq!(provider.status().state, SimulationState::Idle);
}
