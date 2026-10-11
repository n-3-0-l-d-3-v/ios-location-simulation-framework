//! The header against the Rust it describes, as text.
//!
//! This catches a constant whose number was mistyped and a function or
//! constant that exists on one side only. It is a supplement: it proves
//! nothing about layouts, argument types or calling convention. Those are
//! checked by compiling `include/locsim.h` and running `harness/harness.c`
//! against the built library (`harness/run.sh`), which `cargo test` does
//! not do.

use locsim_ffi::*;
use std::collections::BTreeMap;

const HEADER: &str = include_str!("../include/locsim.h");

/// Every `#define LOCSIM_NAME <integer>` of the header.
fn defined() -> BTreeMap<String, u64> {
    let mut found = BTreeMap::new();
    for line in HEADER.lines() {
        let mut words = line.split_whitespace();
        if words.next() != Some("#define") {
            continue;
        }
        let (Some(name), Some(value)) = (words.next(), words.next()) else {
            continue;
        };
        if let Ok(value) = value.trim_end_matches('u').parse::<u64>() {
            let previous = found.insert(name.to_string(), value);
            assert!(previous.is_none(), "{name} is defined twice");
        }
    }
    found
}

macro_rules! constants {
    ($($name:ident),* $(,)?) => {
        [$((stringify!($name), $name as u64)),*]
    };
}

#[test]
fn every_constant_has_the_same_number_on_both_sides_and_none_is_missing() {
    let rust = constants![
        LOCSIM_ABI_VERSION,
        LOCSIM_SCENARIO_MAX_BYTES,
        LOCSIM_CREATE_ERROR_TEXT_MAX,
        LOCSIM_EVENT_CAPACITY_MAX,
        LOCSIM_SESSION_LIMIT,
        LOCSIM_OK,
        LOCSIM_ERR_NULL_POINTER,
        LOCSIM_ERR_INVALID_HANDLE,
        LOCSIM_ERR_INVALID_UTF8,
        LOCSIM_ERR_INVALID_ARGUMENT,
        LOCSIM_ERR_STRUCT_SIZE,
        LOCSIM_ERR_BUSY,
        LOCSIM_ERR_LIMIT,
        LOCSIM_ERR_SCENARIO,
        LOCSIM_ERR_POLICY,
        LOCSIM_ERR_PROVIDER_INVALID_SCENARIO,
        LOCSIM_ERR_PROVIDER_TRANSITION,
        LOCSIM_ERR_PROVIDER_MOVEMENT,
        LOCSIM_ERR_PROVIDER_NOISE,
        LOCSIM_ERR_PROVIDER_DERIVATION,
        LOCSIM_ERR_PROVIDER_VALIDATION,
        LOCSIM_ERR_SCHEDULE_INVALID_INTERVAL,
        LOCSIM_ERR_SCHEDULE_OVERFLOW,
        LOCSIM_ERR_SCHEDULE_ALREADY_PAUSED,
        LOCSIM_ERR_SCHEDULE_NOT_PAUSED,
        LOCSIM_ERR_SCHEDULE_CLOCK_WENT_BACKWARDS,
        LOCSIM_ERR_PANIC,
        LOCSIM_ERR_POISONED,
        LOCSIM_STATE_IDLE,
        LOCSIM_STATE_STARTING,
        LOCSIM_STATE_RUNNING,
        LOCSIM_STATE_PAUSED,
        LOCSIM_STATE_STOPPING,
        LOCSIM_STATE_ERROR,
        LOCSIM_STATE_RECOVERING,
        LOCSIM_SOURCE_SIMULATION,
        LOCSIM_SOURCE_REPLAY,
        LOCSIM_SOURCE_TEST,
        LOCSIM_HEALTH_HEALTHY,
        LOCSIM_HEALTH_DEGRADED,
        LOCSIM_HEALTH_RECOVERING,
        LOCSIM_HEALTH_FAILED,
        LOCSIM_HEALTH_STOPPED,
        LOCSIM_EMISSION_NOT_EXPECTED,
        LOCSIM_EMISSION_FLOWING,
        LOCSIM_EMISSION_STALLED,
        LOCSIM_CAUSE_STALLED,
        LOCSIM_CAUSE_FALLING_BEHIND,
        LOCSIM_CAUSE_PROBATION,
        LOCSIM_CAUSE_EXTERNAL_FAULT,
        LOCSIM_COMPONENT_STORE,
        LOCSIM_COMPONENT_ADAPTER,
        LOCSIM_COMPONENT_DELIVERY,
        LOCSIM_LEVEL_ERROR,
        LOCSIM_LEVEL_WARN,
        LOCSIM_LEVEL_INFO,
        LOCSIM_LEVEL_DEBUG,
        LOCSIM_EVENT_RUN_STARTED,
        LOCSIM_EVENT_RUN_ENDED,
        LOCSIM_EVENT_STOPPED,
        LOCSIM_EVENT_PAUSED,
        LOCSIM_EVENT_RESUMED,
        LOCSIM_EVENT_OPERATION_REJECTED,
        LOCSIM_EVENT_RUN_FAILED,
        LOCSIM_EVENT_RECOVERY_SCHEDULED,
        LOCSIM_EVENT_RECOVERY_ATTEMPTED,
        LOCSIM_EVENT_RECOVERY_START_FAILED,
        LOCSIM_EVENT_RECOVERED,
        LOCSIM_EVENT_FAILED,
        LOCSIM_EVENT_STALLED,
        LOCSIM_EVENT_STALL_CLEARED,
        LOCSIM_EVENT_FALLING_BEHIND,
        LOCSIM_EVENT_CAUGHT_UP,
        LOCSIM_EVENT_FAULT_REPORTED,
        LOCSIM_EVENT_FAULT_CLEARED,
        LOCSIM_EVENT_HEALTH_CHANGED,
        LOCSIM_EVENT_CLOCK_WENT_BACKWARDS,
    ];
    let header = defined();
    for (name, value) in rust {
        assert_eq!(header.get(name), Some(&value), "{name}");
    }
    // Nothing in the header that the list above does not name.
    let listed: Vec<&str> = rust.iter().map(|(name, _)| *name).collect();
    for name in header.keys() {
        assert!(
            listed.contains(&name.as_str()),
            "{name} is only in the header"
        );
    }
    assert_eq!(header.len(), rust.len());
}

#[test]
fn the_header_declares_exactly_the_exported_functions() {
    // Each of these is a function of this crate, or this does not compile.
    let exported: [(&str, *const ()); 15] = [
        ("locsim_abi_version", locsim_abi_version as *const ()),
        ("locsim_session_check", locsim_session_check as *const ()),
        (
            "locsim_session_clear_fault",
            locsim_session_clear_fault as *const (),
        ),
        ("locsim_session_create", locsim_session_create as *const ()),
        (
            "locsim_session_destroy",
            locsim_session_destroy as *const (),
        ),
        (
            "locsim_session_events_dropped",
            locsim_session_events_dropped as *const (),
        ),
        ("locsim_session_health", locsim_session_health as *const ()),
        (
            "locsim_session_next_deadline",
            locsim_session_next_deadline as *const (),
        ),
        (
            "locsim_session_next_event",
            locsim_session_next_event as *const (),
        ),
        ("locsim_session_pause", locsim_session_pause as *const ()),
        ("locsim_session_poll", locsim_session_poll as *const ()),
        (
            "locsim_session_report_fault",
            locsim_session_report_fault as *const (),
        ),
        ("locsim_session_resume", locsim_session_resume as *const ()),
        ("locsim_session_start", locsim_session_start as *const ()),
        ("locsim_session_stop", locsim_session_stop as *const ()),
    ];
    // A declaration is `LOCSIM_API <type> <name>(`.
    let mut declared: Vec<&str> = HEADER
        .lines()
        .filter(|line| line.starts_with("LOCSIM_API "))
        .map(|line| {
            let before = line.split('(').next().unwrap();
            before.split_whitespace().last().unwrap()
        })
        .collect();
    declared.sort_unstable();
    let names: Vec<&str> = exported.iter().map(|(name, _)| *name).collect();
    assert_eq!(declared, names);
}

#[test]
fn the_sizes_the_header_asserts_are_the_sizes_of_the_rust_structs() {
    for (name, size) in [
        ("locsim_policy_t", std::mem::size_of::<LocsimPolicy>()),
        ("locsim_sample_t", std::mem::size_of::<LocsimSample>()),
        ("locsim_health_t", std::mem::size_of::<LocsimHealth>()),
        ("locsim_event_t", std::mem::size_of::<LocsimEvent>()),
    ] {
        let asserted = format!("LOCSIM_ASSERT(sizeof({name}) == {size});");
        assert!(HEADER.contains(&asserted), "{asserted}");
    }
}
