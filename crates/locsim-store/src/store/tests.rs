//! Tests that need the crate's insides: a file system that fails on
//! request, and envelopes sealed around payloads the public interface would
//! never write.

use super::*;
use crate::atomic::testing::{Fault, Faulty, Scratch, FAULTS_BEFORE_REPLACEMENT};
use crate::envelope::EnvelopeError;
use locsim_scenario::ScenarioError;

pub(crate) const WALKING: &str = include_str!("../../../../Examples/Scenarios/walking.json");
pub(crate) const DRIVING: &str = include_str!("../../../../Examples/Scenarios/driving.json");

pub(crate) fn scenario(text: &str) -> Scenario {
    import_scenario(text).unwrap()
}

pub(crate) fn faulty(scratch: &Scratch, fault: Fault) -> Store {
    Store::with_files(scratch.path().to_path_buf(), Box::new(Faulty::new(fault))).unwrap()
}

/// Puts an intact envelope around `payload` at the scenario's place.
fn plant(scratch: &Scratch, payload: &[u8]) {
    std::fs::write(
        scratch.path().join(Record::Scenario.file_name()),
        envelope::seal(payload),
    )
    .unwrap();
}

fn corruption(store: &Store) -> Corruption {
    match store.load_scenario() {
        Err(StoreError::Corrupt {
            record: Record::Scenario,
            path,
            cause,
        }) => {
            assert_eq!(path, store.path_of(Record::Scenario));
            cause
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_save_that_fails_at_any_step_leaves_the_stored_scenario_loadable() {
    for fault in FAULTS_BEFORE_REPLACEMENT {
        let scratch = Scratch::new("store-fault");
        Store::open(scratch.path())
            .unwrap()
            .save_scenario(&scenario(WALKING))
            .unwrap();
        let before = std::fs::read(scratch.path().join("scenario.locsim")).unwrap();

        let store = faulty(&scratch, fault);
        let error = store.save_scenario(&scenario(DRIVING)).unwrap_err();
        assert!(!error.record_was_replaced(), "{fault:?}: {error}");

        // Byte for byte what it was, and still the walking scenario.
        assert_eq!(
            std::fs::read(scratch.path().join("scenario.locsim")).unwrap(),
            before,
            "{fault:?}"
        );
        let loaded = Store::open(scratch.path()).unwrap().load_scenario();
        assert_eq!(loaded, Ok(Some(scenario(WALKING))), "{fault:?}");
        assert_eq!(scratch.names(), ["scenario.locsim"], "{fault:?}");
        assert_eq!(store.stale_temporaries(), Ok(vec![]), "{fault:?}");
    }
}

#[test]
fn a_first_save_that_fails_leaves_no_record_at_all() {
    for fault in FAULTS_BEFORE_REPLACEMENT {
        let scratch = Scratch::new("store-first");
        let store = faulty(&scratch, fault);
        store.save_scenario(&scenario(WALKING)).unwrap_err();
        // Checked through the real file system, not the failing one.
        let real = Store::open(scratch.path()).unwrap();
        assert_eq!(real.load_scenario(), Ok(None), "{fault:?}");
        assert!(scratch.names().is_empty(), "{fault:?}");
    }
}

#[test]
fn a_directory_sync_failure_reports_that_the_new_scenario_is_in_place() {
    let scratch = Scratch::new("store-dirsync");
    Store::open(scratch.path())
        .unwrap()
        .save_scenario(&scenario(WALKING))
        .unwrap();
    let store = faulty(&scratch, Fault::SyncDirectory);
    let error = store.save_scenario(&scenario(DRIVING)).unwrap_err();
    assert!(error.record_was_replaced());
    assert_eq!(store.load_scenario(), Ok(Some(scenario(DRIVING))));
}

#[test]
fn a_read_error_is_an_io_error_not_a_missing_or_corrupt_record() {
    let scratch = Scratch::new("store-read");
    Store::open(scratch.path())
        .unwrap()
        .save_scenario(&scenario(WALKING))
        .unwrap();
    match faulty(&scratch, Fault::ReadBack).load_scenario() {
        Err(StoreError::Io {
            operation: Operation::Read,
            path,
            ..
        }) => assert!(path.ends_with("scenario.locsim")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_leftover_temporary_is_listed_and_removed_only_on_request() {
    let scratch = Scratch::new("store-leftover");
    Store::open(scratch.path())
        .unwrap()
        .save_scenario(&scenario(WALKING))
        .unwrap();
    // A save fails and cannot clean up after itself.
    let files = Faulty::new(Fault::Rename);
    files
        .remove_fails
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let store = Store::with_files(scratch.path().to_path_buf(), Box::new(files)).unwrap();
    match store.save_scenario(&scenario(DRIVING)).unwrap_err() {
        StoreError::Io {
            operation: Operation::Replace,
            cleanup: Some(_),
            ..
        } => {}
        other => panic!("{other:?}"),
    }
    let leftover = scratch.path().join(".scenario.locsim.tmp-t0");

    // A later store sees it, is not disturbed by it, and removes it when told.
    let later = Store::open(scratch.path()).unwrap();
    assert_eq!(later.stale_temporaries(), Ok(vec![leftover.clone()]));
    assert_eq!(later.load_scenario(), Ok(Some(scenario(WALKING))));
    later.save_scenario(&scenario(DRIVING)).unwrap();
    assert_eq!(later.stale_temporaries(), Ok(vec![leftover]));
    assert_eq!(later.remove_stale_temporaries(), Ok(1));
    assert_eq!(later.remove_stale_temporaries(), Ok(0));
    assert_eq!(scratch.names(), ["scenario.locsim"]);
    assert_eq!(later.load_scenario(), Ok(Some(scenario(DRIVING))));
}

#[test]
fn a_temporary_that_cannot_be_removed_is_reported() {
    let scratch = Scratch::new("store-stuck");
    std::fs::write(scratch.path().join(".scenario.locsim.tmp-1-1"), b"x").unwrap();
    let files = Faulty::new(Fault::None);
    files
        .remove_fails
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let store = Store::with_files(scratch.path().to_path_buf(), Box::new(files)).unwrap();
    match store.remove_stale_temporaries() {
        Err(StoreError::Io {
            operation: Operation::RemoveTemporary,
            path,
            ..
        }) => assert!(path.ends_with(".scenario.locsim.tmp-1-1")),
        other => panic!("{other:?}"),
    }
}

// --- Intact envelopes around payloads that are not a loadable scenario --------------

#[test]
fn an_intact_file_with_a_refused_document_is_corrupt_with_the_codecs_reasons() {
    let scratch = Scratch::new("store-document");
    let store = Store::open(scratch.path()).unwrap();

    // A schema version from the future.
    plant(
        &scratch,
        WALKING
            .replace("\"schema_version\": 1", "\"schema_version\": 2")
            .as_bytes(),
    );
    assert_eq!(
        corruption(&store),
        Corruption::Document(vec![ScenarioError::UnsupportedVersion {
            found: 2,
            supported: 1
        }])
    );

    // Well formed, but not a valid scenario.
    plant(
        &scratch,
        WALKING
            .replace("\"update_interval_s\": 1.0", "\"update_interval_s\": 0.0")
            .as_bytes(),
    );
    match corruption(&store) {
        Corruption::Document(errors) => match errors.as_slice() {
            [ScenarioError::InvalidScenario(e)] => assert_eq!(e.field, "update_interval_s"),
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }

    // Not a scenario document at all.
    for payload in [&b""[..], b"{}", b"[]", b"null", b"not json"] {
        plant(&scratch, payload);
        assert!(
            matches!(corruption(&store), Corruption::Document(_)),
            "{payload:?}"
        );
    }
}

#[test]
fn an_intact_file_whose_payload_is_not_text_is_corrupt() {
    let scratch = Scratch::new("store-utf8");
    let store = Store::open(scratch.path()).unwrap();
    plant(&scratch, &[0xff, 0xfe, 0x00, 0x7b]);
    assert_eq!(corruption(&store), Corruption::NotUtf8);
}

#[test]
fn the_digest_is_checked_before_the_document() {
    // A payload that is both damaged and no longer a scenario: the report
    // is the damage, and the codec never sees it.
    let scratch = Scratch::new("store-order");
    let store = Store::open(scratch.path()).unwrap();
    let sealed = String::from_utf8(envelope::seal(WALKING.as_bytes())).unwrap();
    let damaged = sealed.replace("\"mode\": \"walking\"", "\"mode\": \"flying!\"");
    assert_eq!(damaged.len(), sealed.len());
    std::fs::write(scratch.path().join("scenario.locsim"), damaged).unwrap();
    assert!(matches!(
        corruption(&store),
        Corruption::Envelope(EnvelopeError::DigestMismatch { .. })
    ));
}

// --- Damage, in memory ----------------------------------------------------------------

const EXAMPLES: [&str; 8] = [
    include_str!("../../../../Examples/Scenarios/fixed.json"),
    include_str!("../../../../Examples/Scenarios/random_walk.json"),
    WALKING,
    DRIVING,
    include_str!("../../../../Examples/Scenarios/circular.json"),
    include_str!("../../../../Examples/Scenarios/route_replay.json"),
    include_str!("../../../../Examples/Scenarios/antimeridian_loop.json"),
    include_str!("../../../../Examples/Scenarios/high_latitude.json"),
];

fn interpreted(bytes: &[u8]) -> Result<Scenario, StoreError> {
    interpret(
        Record::Scenario,
        Path::new("scenario.locsim"),
        bytes,
        import_scenario,
    )
}

#[test]
fn every_truncation_of_a_stored_scenario_is_corrupt() {
    let whole = envelope::seal(WALKING.as_bytes());
    for end in 0..whole.len() {
        assert!(
            matches!(
                interpreted(&whole[..end]),
                Err(StoreError::Corrupt {
                    cause: Corruption::Envelope(_),
                    ..
                })
            ),
            "cut at {end}"
        );
    }
    assert_eq!(interpreted(&whole), Ok(scenario(WALKING)));
}

/// The damage T08 measured on bare documents: one character changed,
/// removed or inserted. A bare document accepts about one such change in
/// ten as a valid scenario. Inside the envelope none gets through.
#[test]
fn damage_that_the_document_codec_accepts_is_caught_by_the_digest() {
    let mut rng = locsim_core::rng::Rng::from_seed(0xD0C);
    let (mut tried, mut bare_accepted, mut different) = (0u32, 0u32, 0u32);
    for text in EXAMPLES {
        let whole = envelope::seal(text.as_bytes());
        let header = whole.len() - text.len();
        let original = format!("{:?}", scenario(text));
        for _ in 0..4_000 {
            let offset = (rng.next_u64() % text.len() as u64) as usize;
            let random = b' ' + (rng.next_u64() % 95) as u8;
            let kind = rng.next_u64() % 4;
            let damage = |bytes: &mut Vec<u8>, position: usize| match kind {
                0 => {
                    bytes.remove(position);
                }
                1 => bytes.insert(position, random),
                2 => bytes[position] = random,
                _ => bytes[position] = b"0123456789-.eE\"{}[],:"[(random % 21) as usize],
            };
            let mut bare = text.as_bytes().to_vec();
            damage(&mut bare, offset);
            if bare == text.as_bytes() {
                continue; // a character replaced by itself
            }
            tried += 1;
            if let Ok(accepted) = import_scenario(&String::from_utf8(bare).unwrap()) {
                bare_accepted += 1;
                different += u32::from(format!("{accepted:?}") != original);
            }
            let mut stored = whole.clone();
            damage(&mut stored, header + offset);
            match interpreted(&stored) {
                Err(StoreError::Corrupt {
                    cause: Corruption::Envelope(_),
                    ..
                }) => {}
                other => panic!("damage got through or was misreported: {other:?}"),
            }
        }
    }
    println!(
        "damage: {tried} changes; as bare documents {bare_accepted} are accepted ({different} as a different scenario); stored, none"
    );
    assert!(tried > 30_000 && different > 1_000);
}
