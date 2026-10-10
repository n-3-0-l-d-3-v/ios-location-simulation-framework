//! T09: the last-known record and the scenario fingerprint, through the
//! public interface and against a real provider.

mod support;

use locsim_core::domain::{SimulationState, Timestamp};
use locsim_core::provider::{LocationProvider, SimulationProvider};
use locsim_scenario::{
    export_last_known, export_scenario, import_last_known, import_scenario, scenario_fingerprint,
    LastKnown,
};
use support::common::START;
use support::{example, fingerprint as bits, EXAMPLES};

/// FNV-1a 64 of each example file, computed outside this code base (a
/// separate script over the bytes of the files).
const FINGERPRINTS: [(&str, u64); 8] = [
    ("antimeridian_loop", 0xeb27_9a69_ae4f_6b13),
    ("circular", 0x783e_e89c_888a_666c),
    ("driving", 0x7230_3e34_30dd_d844),
    ("fixed", 0x9170_10fa_72b5_30a9),
    ("high_latitude", 0xf905_e815_82d4_62cc),
    ("random_walk", 0x6df1_7e15_f039_2ac9),
    ("route_replay", 0xd2b7_2637_57df_8024),
    ("walking", 0x3f29_4995_c8f3_1115),
];

#[test]
fn fingerprints_are_those_of_the_exported_text_and_tell_the_examples_apart() {
    assert_eq!(FINGERPRINTS.len(), EXAMPLES.len());
    for (name, expected) in FINGERPRINTS {
        let scenario = import_scenario(example(name)).unwrap();
        assert_eq!(scenario_fingerprint(&scenario), Ok(expected), "{name}");
        // Stable across a round trip through text.
        let again = import_scenario(&export_scenario(&scenario).unwrap()).unwrap();
        assert_eq!(scenario_fingerprint(&again), Ok(expected), "{name}");
    }
}

#[test]
fn any_change_to_a_scenario_changes_its_fingerprint() {
    let original = import_scenario(example("route_replay")).unwrap();
    let reference = scenario_fingerprint(&original).unwrap();
    let edits = [
        ("\"seed\": \"12345\"", "\"seed\": \"12346\""),
        (
            "\"name\": \"Replay of a recorded walk\"",
            "\"name\": \"Replay of a recorded walk.\"",
        ),
        (
            "\"altitude_m\": 920.0,\n  \"mode\"",
            "\"altitude_m\": 920.0000000000001,\n  \"mode\"",
        ),
        ("\"max_speed_mps\": 1.9", "\"max_speed_mps\": 1.91"),
        ("\"elapsed_ns\": 40000000000", "\"elapsed_ns\": 40000000001"),
        ("\"position_noise_m\": 1.5", "\"position_noise_m\": 1.4"),
    ];
    for (from, to) in edits {
        assert_eq!(example("route_replay").matches(from).count(), 1, "{from}");
        let edited = import_scenario(&example("route_replay").replace(from, to)).unwrap();
        assert_ne!(scenario_fingerprint(&edited).unwrap(), reference, "{to}");
    }
}

#[test]
fn an_invalid_scenario_has_no_fingerprint() {
    let mut scenario = import_scenario(example("walking")).unwrap();
    scenario.update_interval_s = 0.0;
    assert!(scenario_fingerprint(&scenario).is_err());
}

#[test]
fn a_record_taken_from_a_running_provider_round_trips_exactly() {
    for (name, _) in EXAMPLES {
        let scenario = import_scenario(example(name)).unwrap();
        let fingerprint = scenario_fingerprint(&scenario).unwrap();
        let interval = (scenario.update_interval_s * 1e9) as i64;
        let mut provider = SimulationProvider::new(scenario);
        provider.start(Timestamp::from_nanos(START)).unwrap();
        // Before anything is emitted there is nothing to record.
        assert_eq!(provider.current_location(), None);
        for tick in 0..50 {
            let now = Timestamp::from_nanos(START + tick * interval);
            let sample = provider.poll(now).unwrap().expect("a sample is due");
            let record = LastKnown::new(fingerprint, sample, &provider.status()).unwrap();
            assert_eq!(record.sample_count, tick as u64 + 1);
            assert_eq!(record.state, SimulationState::Running);
            // The first sample has no speed; the record must say so.
            assert_eq!(record.sample.speed_mps.is_none(), tick == 0, "{name}");
            let text = export_last_known(&record).unwrap();
            let back = import_last_known(&text).unwrap();
            assert_eq!(bits(&back), bits(&record), "{name} tick {tick}");
            assert_eq!(export_last_known(&back).unwrap(), text);
        }
        // A paused provider still has a last sample; the record carries the
        // provider's state beside the sample's own.
        let later = Timestamp::from_nanos(START + 50 * interval);
        provider.pause(later).unwrap();
        let sample = provider.current_location().unwrap();
        let record = LastKnown::new(fingerprint, sample, &provider.status()).unwrap();
        assert_eq!(record.state, SimulationState::Paused);
        assert_eq!(record.sample.simulation_state, SimulationState::Running);
        let back = import_last_known(&export_last_known(&record).unwrap()).unwrap();
        assert_eq!(bits(&back), bits(&record));
    }
}

#[test]
fn a_record_taken_with_a_stale_status_is_refused() {
    let scenario = import_scenario(example("walking")).unwrap();
    let fingerprint = scenario_fingerprint(&scenario).unwrap();
    let mut provider = SimulationProvider::new(scenario);
    provider.start(Timestamp::from_nanos(START)).unwrap();
    let first = provider
        .poll(Timestamp::from_nanos(START))
        .unwrap()
        .unwrap();
    let second_time = Timestamp::from_nanos(START + 1_000_000_000);
    provider.poll(second_time).unwrap().unwrap();
    // The status now names the second sample.
    assert!(LastKnown::new(fingerprint, first, &provider.status()).is_err());
}
