//! T09: storing the last-known record, and what that record is not.

mod support;

use locsim_core::domain::{Scenario, SimulationState, SyntheticLocation, Timestamp};
use locsim_core::provider::{LocationProvider, SimulationProvider};
use locsim_scenario::{import_scenario, scenario_fingerprint, LastKnown};
use locsim_store::{Corruption, EnvelopeError, Record, Store, StoreError};
use std::time::Instant;
use support::scenario::common::START;
use support::scenario::{example, fingerprint as bits, EXAMPLES};
use support::Scratch;

fn scenario(name: &str) -> Scenario {
    import_scenario(example(name)).unwrap()
}

fn at(scenario: &Scenario, tick: i64) -> Timestamp {
    Timestamp::from_nanos(START + tick * (scenario.update_interval_s * 1e9) as i64)
}

/// A provider polled punctually for ticks `0..ticks`, and what it emitted.
fn started(scenario: &Scenario, ticks: i64) -> (SimulationProvider, Vec<SyntheticLocation>) {
    let mut provider = SimulationProvider::new(scenario.clone());
    provider.start(at(scenario, 0)).unwrap();
    let stream = (0..ticks)
        .map(|tick| provider.poll(at(scenario, tick)).unwrap().expect("due"))
        .collect();
    (provider, stream)
}

fn record_of(scenario: &Scenario, provider: &SimulationProvider) -> LastKnown {
    LastKnown::new(
        scenario_fingerprint(scenario).unwrap(),
        provider.current_location().expect("a sample was emitted"),
        &provider.status(),
    )
    .unwrap()
}

#[test]
fn nothing_stored_is_none_whether_or_not_a_scenario_is_stored() {
    let scratch = Scratch::new("lk-missing");
    let store = Store::open(scratch.path()).unwrap();
    assert_eq!(store.load_last_known(), Ok(None));
    store.save_scenario(&scenario("walking")).unwrap();
    assert_eq!(store.load_last_known(), Ok(None));
    assert_eq!(scratch.names(), ["scenario.locsim"]);
}

#[test]
fn records_from_running_providers_are_stored_and_loaded_bit_for_bit() {
    let scratch = Scratch::new("lk-round-trip");
    let store = Store::open(scratch.path()).unwrap();
    for (name, _) in EXAMPLES {
        let scenario = scenario(name);
        let mut provider = SimulationProvider::new(scenario.clone());
        provider.start(at(&scenario, 0)).unwrap();
        for tick in 0..40 {
            provider.poll(at(&scenario, tick)).unwrap().expect("due");
            let record = record_of(&scenario, &provider);
            store.save_last_known(&record).unwrap();
            let loaded = store.load_last_known().unwrap().expect("stored");
            assert_eq!(bits(&loaded), bits(&record), "{name} tick {tick}");
        }
    }
    assert_eq!(scratch.names(), ["last_known.locsim"]);
}

#[test]
fn the_two_records_are_separate_files_and_saving_one_leaves_the_other() {
    let scratch = Scratch::new("lk-separate");
    let store = Store::open(scratch.path()).unwrap();
    let walking = scenario("walking");
    let (provider, _) = started(&walking, 10);
    store.save_scenario(&walking).unwrap();
    store
        .save_last_known(&record_of(&walking, &provider))
        .unwrap();
    assert_eq!(scratch.names(), ["last_known.locsim", "scenario.locsim"]);

    let record_bytes = std::fs::read(scratch.file("last_known.locsim")).unwrap();
    store.save_scenario(&scenario("driving")).unwrap();
    assert_eq!(
        std::fs::read(scratch.file("last_known.locsim")).unwrap(),
        record_bytes
    );
    let scenario_bytes = std::fs::read(scratch.file("scenario.locsim")).unwrap();
    store
        .save_last_known(&record_of(&walking, &provider))
        .unwrap();
    assert_eq!(
        std::fs::read(scratch.file("scenario.locsim")).unwrap(),
        scenario_bytes
    );
}

#[test]
fn a_record_from_another_scenario_loads_and_its_fingerprint_does_not_match() {
    let scratch = Scratch::new("lk-stale");
    let store = Store::open(scratch.path()).unwrap();
    let walking = scenario("walking");
    let (provider, _) = started(&walking, 25);
    store.save_scenario(&walking).unwrap();
    store
        .save_last_known(&record_of(&walking, &provider))
        .unwrap();

    let belongs = |store: &Store| {
        let scenario = store.load_scenario().unwrap().unwrap();
        let record = store.load_last_known().unwrap().unwrap();
        record.scenario_fingerprint == scenario_fingerprint(&scenario).unwrap()
    };
    assert!(belongs(&store));

    // The scenario is replaced; the record on disk is now from another one.
    // That is not corruption: it loads, and the fingerprint says so.
    let mut edited = walking.clone();
    edited.seed += 1;
    store.save_scenario(&edited).unwrap();
    assert!(!belongs(&store));
    store.save_scenario(&scenario("driving")).unwrap();
    assert!(!belongs(&store));
    // Back to the very same scenario: it matches again.
    store.save_scenario(&walking).unwrap();
    assert!(belongs(&store));
}

/// The record is the last output, not a checkpoint. A simulation
/// "restarted" from what is stored is a new run: it starts where every run
/// of that scenario starts, with no speed, and does not continue the
/// stream the interrupted run would have produced.
#[test]
fn the_stored_record_cannot_continue_a_run() {
    for name in [
        "driving",
        "random_walk",
        "walking",
        "circular",
        "route_replay",
    ] {
        let scratch = Scratch::new("lk-not-checkpoint");
        let original = scenario(name);

        // The run that is never interrupted: ticks 0..200.
        let (_, uninterrupted) = started(&original, 200);

        // The run that is interrupted after tick 99, with everything this
        // crate can store saved first.
        let (provider, before) = started(&original, 100);
        let store = Store::open(scratch.path()).unwrap();
        store.save_scenario(&original).unwrap();
        store
            .save_last_known(&record_of(&original, &provider))
            .unwrap();
        drop(provider);

        // After the "restart": everything that was stored is loaded.
        let store = Store::open(scratch.path()).unwrap();
        let loaded = store.load_scenario().unwrap().unwrap();
        let record = store.load_last_known().unwrap().unwrap();
        // The record is faithful to the last output...
        assert_eq!(bits(&record.sample), bits(&before[99]), "{name}");
        assert_eq!(bits(&record.sample), bits(&uninterrupted[99]), "{name}");
        assert_eq!(record.sample_count, 100);

        // ...but the only thing that can be started is the scenario, and
        // that is a new run. There is no interface that takes the record.
        let mut restarted = SimulationProvider::new(loaded);
        restarted.start(at(&original, 100)).unwrap();
        let first = restarted.poll(at(&original, 100)).unwrap().unwrap();
        assert_eq!(restarted.status().sample_count, 1, "{name}");
        // A first sample: no speed and no course, unlike tick 100 of the
        // uninterrupted run.
        assert_eq!((first.speed_mps, first.course_deg), (None, None), "{name}");
        assert!(uninterrupted[100].speed_mps.is_some(), "{name}");
        // It is where the run began, not where the interrupted run was.
        assert_eq!(
            bits(&first.coordinate),
            bits(&uninterrupted[0].coordinate),
            "{name}"
        );
        assert_ne!(first.coordinate, uninterrupted[100].coordinate, "{name}");
        assert_ne!(first.coordinate, record.sample.coordinate, "{name}");
        // And it replays the beginning, shifted in time, not the continuation.
        let second = restarted.poll(at(&original, 101)).unwrap().unwrap();
        assert_eq!(
            bits(&second.coordinate),
            bits(&uninterrupted[1].coordinate),
            "{name}"
        );
        assert_ne!(second.coordinate, uninterrupted[101].coordinate, "{name}");
    }
}

#[test]
fn an_invalid_record_is_refused_and_the_stored_one_is_kept() {
    let scratch = Scratch::new("lk-invalid");
    let store = Store::open(scratch.path()).unwrap();
    let walking = scenario("walking");
    let (provider, _) = started(&walking, 5);
    let good = record_of(&walking, &provider);

    let mut bad = good;
    bad.sample.altitude_m = f64::NAN;
    assert!(matches!(
        store.save_last_known(&bad),
        Err(StoreError::Invalid {
            record: Record::LastKnown,
            ..
        })
    ));
    assert!(scratch.names().is_empty());

    store.save_last_known(&good).unwrap();
    bad.sample.altitude_m = 0.0;
    bad.sample_count = 0;
    assert!(store.save_last_known(&bad).is_err());
    assert_eq!(
        bits(&store.load_last_known().unwrap().unwrap()),
        bits(&good)
    );
}

#[test]
fn a_damaged_or_foreign_record_is_corrupt_and_left_alone() {
    let scratch = Scratch::new("lk-corrupt");
    let store = Store::open(scratch.path()).unwrap();
    let walking = scenario("walking");
    let (provider, _) = started(&walking, 5);
    store
        .save_last_known(&record_of(&walking, &provider))
        .unwrap();
    let file = std::fs::read_to_string(scratch.file("last_known.locsim")).unwrap();
    let cause = |store: &Store| match store.load_last_known() {
        Err(StoreError::Corrupt {
            record: Record::LastKnown,
            cause,
            ..
        }) => cause,
        other => panic!("{other:?}"),
    };

    // One digit of the sample count: a valid record, were it not for the digest.
    let changed = file.replace("\"sample_count\": \"5\"", "\"sample_count\": \"6\"");
    assert_ne!(changed, file);
    std::fs::write(scratch.file("last_known.locsim"), &changed).unwrap();
    assert!(matches!(
        cause(&store),
        Corruption::Envelope(EnvelopeError::DigestMismatch { .. })
    ));
    assert_eq!(
        std::fs::read_to_string(scratch.file("last_known.locsim")).unwrap(),
        changed
    );

    for (bytes, expected) in [
        (&b""[..], EnvelopeError::Empty),
        (b"{}", EnvelopeError::NotAnEnvelope),
    ] {
        std::fs::write(scratch.file("last_known.locsim"), bytes).unwrap();
        assert_eq!(cause(&store), Corruption::Envelope(expected));
    }
    for end in (0..file.len()).step_by(11) {
        std::fs::write(scratch.file("last_known.locsim"), &file.as_bytes()[..end]).unwrap();
        assert!(
            matches!(cause(&store), Corruption::Envelope(_)),
            "cut at {end}"
        );
    }
    // A scenario is unaffected by any of it.
    assert_eq!(store.load_scenario(), Ok(None));
}

#[test]
fn a_paused_providers_record_keeps_the_state_and_the_sample_apart() {
    let scratch = Scratch::new("lk-paused");
    let store = Store::open(scratch.path()).unwrap();
    let walking = scenario("walking");
    let (mut provider, stream) = started(&walking, 12);
    provider.pause(at(&walking, 12)).unwrap();
    store
        .save_last_known(&record_of(&walking, &provider))
        .unwrap();
    let loaded = store.load_last_known().unwrap().unwrap();
    assert_eq!(loaded.state, SimulationState::Paused);
    assert_eq!(loaded.sample.simulation_state, SimulationState::Running);
    assert_eq!(bits(&loaded.sample), bits(&stream[11]));
}

#[test]
fn temporaries_of_both_records_are_listed() {
    let scratch = Scratch::new("lk-temporaries");
    let store = Store::open(scratch.path()).unwrap();
    for name in [".last_known.locsim.tmp-9-0", ".scenario.locsim.tmp-9-1"] {
        std::fs::write(scratch.file(name), b"x").unwrap();
    }
    assert_eq!(
        store.stale_temporaries(),
        Ok(vec![
            scratch.file(".last_known.locsim.tmp-9-0"),
            scratch.file(".scenario.locsim.tmp-9-1"),
        ])
    );
    assert_eq!(store.remove_stale_temporaries(), Ok(2));
    assert!(scratch.names().is_empty());
}

/// Not an assertion about speed: a measurement, printed, of what one
/// synced save costs here, so that a caller can choose how often to save.
#[test]
fn the_cost_of_a_save_is_measured() {
    let scratch = Scratch::new("lk-cost");
    let store = Store::open(scratch.path()).unwrap();
    let walking = scenario("walking");
    let (provider, _) = started(&walking, 3);
    let record = record_of(&walking, &provider);
    let saves = 200;
    let begun = Instant::now();
    for _ in 0..saves {
        store.save_last_known(&record).unwrap();
    }
    let each = begun.elapsed().as_secs_f64() * 1e3 / f64::from(saves);
    println!("last-known save: {each:.2} ms each over {saves} synced saves");
    assert_eq!(scratch.names(), ["last_known.locsim"]);
}
