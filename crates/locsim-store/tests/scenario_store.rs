//! T09: storing and loading a scenario, through the public interface and
//! the real file system.

mod support;

use locsim_core::domain::Scenario;
use locsim_scenario::{export_scenario, import_scenario};
use locsim_store::{Corruption, EnvelopeError, Record, Store, StoreError};
use support::scenario::{example, fingerprint, run, EXAMPLES};
use support::{payload_of, Scratch};

fn scenario(name: &str) -> Scenario {
    import_scenario(example(name)).unwrap()
}

fn cause(store: &Store) -> Corruption {
    match store.load_scenario() {
        Err(StoreError::Corrupt {
            record: Record::Scenario,
            cause,
            ..
        }) => cause,
        other => panic!("{other:?}"),
    }
}

// --- Opening ------------------------------------------------------------------------

#[test]
fn a_store_needs_an_existing_directory_and_creates_nothing() {
    let scratch = Scratch::new("open");
    let missing = scratch.file("not-here");
    assert_eq!(
        Store::open(&missing).err(),
        Some(StoreError::NotADirectory {
            path: missing.clone()
        })
    );
    assert!(!missing.exists());

    let a_file = scratch.file("a-file");
    std::fs::write(&a_file, b"x").unwrap();
    assert_eq!(
        Store::open(&a_file).err(),
        Some(StoreError::NotADirectory { path: a_file })
    );

    let store = Store::open(scratch.path()).unwrap();
    assert_eq!(store.directory(), scratch.path());
    assert_eq!(scratch.names(), ["a-file"]);
}

// --- Missing is not corrupt -------------------------------------------------------

#[test]
fn nothing_stored_is_none_and_loading_writes_nothing() {
    let scratch = Scratch::new("missing");
    let store = Store::open(scratch.path()).unwrap();
    assert_eq!(store.load_scenario(), Ok(None));
    assert_eq!(store.stale_temporaries(), Ok(vec![]));
    assert!(scratch.names().is_empty());
}

#[test]
fn an_empty_file_is_corrupt_not_missing() {
    let scratch = Scratch::new("empty");
    std::fs::write(scratch.file("scenario.locsim"), b"").unwrap();
    let store = Store::open(scratch.path()).unwrap();
    assert_eq!(cause(&store), Corruption::Envelope(EnvelopeError::Empty));
    // Still there, still empty: nothing was "repaired".
    assert_eq!(std::fs::read(scratch.file("scenario.locsim")).unwrap(), b"");
}

// --- Round trips ----------------------------------------------------------------------

#[test]
fn every_example_is_stored_and_loaded_bit_for_bit() {
    let scratch = Scratch::new("examples");
    let store = Store::open(scratch.path()).unwrap();
    for (name, text) in EXAMPLES {
        let original = scenario(name);
        store.save_scenario(&original).unwrap();
        let loaded = store.load_scenario().unwrap().expect("stored");
        assert_eq!(fingerprint(&loaded), fingerprint(&original), "{name}");

        // The payload is the exported document, unchanged: the example file.
        let file = std::fs::read(store.path_of(Record::Scenario)).unwrap();
        assert_eq!(payload_of(&file), text.as_bytes(), "{name}");
        assert_eq!(
            payload_of(&file),
            export_scenario(&original).unwrap().as_bytes()
        );
        assert_eq!(scratch.names(), ["scenario.locsim"], "{name}");
    }
}

#[test]
fn the_stored_file_is_the_documented_envelope() {
    let scratch = Scratch::new("format");
    let store = Store::open(scratch.path()).unwrap();
    store.save_scenario(&scenario("walking")).unwrap();
    let file = std::fs::read_to_string(scratch.file("scenario.locsim")).unwrap();
    let payload = example("walking");
    // The digest is the one `sha256sum Examples/Scenarios/walking.json` prints.
    let header = format!(
        "locsim-store 1\n\
         payload-sha256 ffa44c8d998b3245b5a4101fa397a33cc6813ffefc2441852e984141f7d7ed96\n\
         payload-bytes {}\n\n",
        payload.len()
    );
    assert_eq!(file, format!("{header}{payload}"));
}

#[test]
fn a_scenario_loaded_in_a_new_store_drives_the_provider_to_the_same_stream() {
    for (name, _) in EXAMPLES {
        let scratch = Scratch::new("stream");
        let original = scenario(name);
        Store::open(scratch.path())
            .unwrap()
            .save_scenario(&original)
            .unwrap();
        // As after a restart: a new store on the same directory.
        let loaded = Store::open(scratch.path())
            .unwrap()
            .load_scenario()
            .unwrap()
            .unwrap();
        let (a, _) = run(&original, 300, Some(5));
        let (b, _) = run(&loaded, 300, Some(5));
        assert_eq!(fingerprint(&a), fingerprint(&b), "{name}");
    }
}

#[test]
fn saving_replaces_the_stored_scenario_whatever_its_size() {
    let scratch = Scratch::new("replace");
    let store = Store::open(scratch.path()).unwrap();
    for name in ["antimeridian_loop", "fixed", "route_replay", "circular"] {
        store.save_scenario(&scenario(name)).unwrap();
        assert_eq!(store.load_scenario(), Ok(Some(scenario(name))));
    }
    assert_eq!(scratch.names(), ["scenario.locsim"]);
}

// --- Invalid values are not written ------------------------------------------------

#[test]
fn an_invalid_scenario_is_refused_and_the_stored_one_is_kept() {
    let scratch = Scratch::new("invalid");
    let store = Store::open(scratch.path()).unwrap();

    let mut invalid = scenario("walking");
    invalid.update_interval_s = 0.0;
    invalid.name.clear();
    let expected = invalid.validate().unwrap_err();

    // Into an empty directory: nothing appears.
    match store.save_scenario(&invalid) {
        Err(StoreError::Invalid {
            record: Record::Scenario,
            errors,
        }) => assert_eq!(errors.len(), expected.len()),
        other => panic!("{other:?}"),
    }
    assert!(scratch.names().is_empty());

    // Over a stored scenario: it stays.
    store.save_scenario(&scenario("driving")).unwrap();
    let before = std::fs::read(scratch.file("scenario.locsim")).unwrap();
    assert!(store.save_scenario(&invalid).is_err());
    assert_eq!(
        std::fs::read(scratch.file("scenario.locsim")).unwrap(),
        before
    );
    assert_eq!(store.load_scenario(), Ok(Some(scenario("driving"))));
}

// --- Corruption -----------------------------------------------------------------------

#[test]
fn a_bare_document_is_not_a_stored_record() {
    // The exported JSON on its own is a valid scenario document, but it
    // carries no digest. It is refused; importing it is the caller's job.
    let scratch = Scratch::new("bare");
    std::fs::write(scratch.file("scenario.locsim"), example("walking")).unwrap();
    let store = Store::open(scratch.path()).unwrap();
    assert_eq!(
        cause(&store),
        Corruption::Envelope(EnvelopeError::NotAnEnvelope)
    );
}

#[test]
fn every_truncation_of_a_stored_file_is_corrupt_never_missing_never_loaded() {
    let scratch = Scratch::new("truncated");
    let store = Store::open(scratch.path()).unwrap();
    store.save_scenario(&scenario("walking")).unwrap();
    let whole = std::fs::read(scratch.file("scenario.locsim")).unwrap();
    // Every seventh cut through real files; every cut is tested in memory
    // by the unit tests.
    for end in (0..whole.len()).step_by(7) {
        std::fs::write(scratch.file("scenario.locsim"), &whole[..end]).unwrap();
        assert!(
            matches!(cause(&store), Corruption::Envelope(_)),
            "cut at {end}"
        );
    }
    std::fs::write(scratch.file("scenario.locsim"), &whole).unwrap();
    assert_eq!(store.load_scenario(), Ok(Some(scenario("walking"))));
}

#[test]
fn a_changed_digit_in_a_stored_file_is_corrupt_not_another_scenario() {
    // The change a document codec cannot see: one valid number for another.
    let scratch = Scratch::new("digit");
    let store = Store::open(scratch.path()).unwrap();
    store.save_scenario(&scenario("walking")).unwrap();
    let file = std::fs::read_to_string(scratch.file("scenario.locsim")).unwrap();
    for (from, to) in [
        ("\"max_speed_mps\": 1.8", "\"max_speed_mps\": 1.3"),
        ("\"latitude\": 12.9352", "\"latitude\": 12.9852"),
        ("\"seed\": \"12345\"", "\"seed\": \"12845\""),
        ("\"looping\": false", "\"looping\": true "),
    ] {
        assert_eq!(file.matches(from).count(), 1, "{from}");
        // The bare document with this change is a valid, different scenario.
        if !to.ends_with(' ') {
            assert!(import_scenario(&example("walking").replace(from, to)).is_ok());
        }
        std::fs::write(scratch.file("scenario.locsim"), file.replace(from, to)).unwrap();
        assert!(
            matches!(
                cause(&store),
                Corruption::Envelope(EnvelopeError::DigestMismatch { .. })
            ),
            "{to}"
        );
    }
}

#[test]
fn a_corrupt_record_is_left_alone_and_can_be_replaced_by_a_save() {
    let scratch = Scratch::new("left-alone");
    let store = Store::open(scratch.path()).unwrap();
    store.save_scenario(&scenario("walking")).unwrap();
    let mut bytes = std::fs::read(scratch.file("scenario.locsim")).unwrap();
    let last = bytes.len() - 20;
    bytes[last] ^= 0x04;
    std::fs::write(scratch.file("scenario.locsim"), &bytes).unwrap();

    for _ in 0..3 {
        assert!(matches!(
            cause(&store),
            Corruption::Envelope(EnvelopeError::DigestMismatch { .. })
        ));
    }
    assert_eq!(
        std::fs::read(scratch.file("scenario.locsim")).unwrap(),
        bytes
    );
    assert_eq!(scratch.names(), ["scenario.locsim"]);

    // Replacing it is a decision the caller makes by saving.
    store.save_scenario(&scenario("driving")).unwrap();
    assert_eq!(store.load_scenario(), Ok(Some(scenario("driving"))));
}

#[test]
fn an_envelope_from_a_newer_build_is_refused_and_kept() {
    let scratch = Scratch::new("newer");
    let store = Store::open(scratch.path()).unwrap();
    store.save_scenario(&scenario("walking")).unwrap();
    let file = std::fs::read_to_string(scratch.file("scenario.locsim")).unwrap();
    let newer = file.replacen("locsim-store 1\n", "locsim-store 2\n", 1);
    std::fs::write(scratch.file("scenario.locsim"), &newer).unwrap();
    assert_eq!(
        cause(&store),
        Corruption::Envelope(EnvelopeError::UnsupportedVersion {
            found: 2,
            supported: 1
        })
    );
    assert_eq!(
        std::fs::read_to_string(scratch.file("scenario.locsim")).unwrap(),
        newer
    );
}

// --- Temporaries and other files ------------------------------------------------------

#[test]
fn only_this_stores_temporaries_are_listed_and_removed() {
    let scratch = Scratch::new("temporaries");
    let store = Store::open(scratch.path()).unwrap();
    store.save_scenario(&scenario("walking")).unwrap();
    for name in [
        ".scenario.locsim.tmp-4242-0",
        ".scenario.locsim.tmp-4242-17",
        "notes.txt",
        ".scenario.locsim.bak",
        "scenario.locsim.tmp-1-1",
        ".other.tmp-1-1",
    ] {
        std::fs::write(scratch.file(name), b"x").unwrap();
    }
    assert_eq!(
        store.stale_temporaries(),
        Ok(vec![
            scratch.file(".scenario.locsim.tmp-4242-0"),
            scratch.file(".scenario.locsim.tmp-4242-17"),
        ])
    );
    // Listing and loading change nothing.
    assert_eq!(store.load_scenario(), Ok(Some(scenario("walking"))));
    assert_eq!(scratch.names().len(), 7);

    assert_eq!(store.remove_stale_temporaries(), Ok(2));
    assert_eq!(
        scratch.names(),
        [
            ".other.tmp-1-1",
            ".scenario.locsim.bak",
            "notes.txt",
            "scenario.locsim",
            "scenario.locsim.tmp-1-1",
        ]
    );
    assert_eq!(store.load_scenario(), Ok(Some(scenario("walking"))));
}

// --- One store, several threads -----------------------------------------------------

#[test]
fn saves_through_one_store_are_serialised_and_readers_see_whole_records() {
    let scratch = Scratch::new("threads");
    let store = Store::open(scratch.path()).unwrap();
    let scenarios: Vec<Scenario> = EXAMPLES.iter().map(|(name, _)| scenario(name)).collect();
    store.save_scenario(&scenarios[0]).unwrap();

    let (mut loads, mut retried) = (0u32, 0u32);
    std::thread::scope(|scope| {
        let writers: Vec<_> = (0..4)
            .map(|w| {
                let (store, scenarios) = (&store, &scenarios);
                scope.spawn(move || {
                    for i in 0..60 {
                        store
                            .save_scenario(&scenarios[(w + i) % scenarios.len()])
                            .unwrap();
                    }
                })
            })
            .collect();
        // A reader with its own store, as another part of a program would.
        let reader = Store::open(scratch.path()).unwrap();
        while writers.iter().any(|w| !w.is_finished()) {
            match reader.load_scenario() {
                Ok(Some(found)) => {
                    assert!(scenarios.contains(&found));
                    loads += 1;
                }
                // Never absent once stored, never a damaged record.
                Ok(None) => panic!("the record disappeared during a replacement"),
                Err(StoreError::Corrupt { cause, .. }) => panic!("partial record: {cause}"),
                // Windows may refuse a read that coincides with a rename.
                Err(StoreError::Io { .. }) => retried += 1,
                Err(other) => panic!("{other}"),
            }
        }
        for writer in writers {
            writer.join().unwrap();
        }
    });
    println!("threads: 240 saves, {loads} whole records read, {retried} reads refused by the OS");
    assert!(loads > 0);
    assert!(scenarios.contains(&store.load_scenario().unwrap().unwrap()));
    assert_eq!(scratch.names(), ["scenario.locsim"]);
}
