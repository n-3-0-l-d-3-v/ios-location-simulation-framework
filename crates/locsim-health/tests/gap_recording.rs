//! The long-gap recording in `Docs/Diagnostics/long_gap` is what the
//! diagnostic writes today.
//!
//! This pins a recording, not a requirement: several of the recorded
//! outcomes are the open pause/resume defect (`PROGRESS.md`, "Known
//! Issues"). A change to the engine that changes any outcome fails here
//! and must regenerate the files, so that the change is seen in review:
//!
//!     cargo run -p locsim-health --example gap_diagnostic -- \
//!         Examples/Scenarios Docs/Diagnostics/long_gap
//!
//! Two things are compared, and they are not the same claim:
//!
//! - `summary.md`, the outcome of every case, on every host. It was the
//!   same on Windows x86_64 and on macOS arm64.
//! - `results.jsonl`, every number of every case, only on the kind of host
//!   it was recorded on (Windows x86_64). On macOS arm64 the outcomes are
//!   the same and numbers differ in their last digits (workflow run
//!   38238724688), so the raw file is the recording of one platform.

#[path = "../examples/gap_diagnostic.rs"]
#[allow(dead_code)]
mod diagnostic;

/// Whether this host is the kind the raw recording was made on.
const RECORDED_HERE: bool = cfg!(all(target_os = "windows", target_arch = "x86_64"));

fn root() -> String {
    format!("{}/../..", env!("CARGO_MANIFEST_DIR"))
}

/// A checked-in file as text, whatever line endings the checkout gave it.
fn recorded(name: &str) -> String {
    let path = format!("{}/Docs/Diagnostics/long_gap/{name}", root());
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .replace("\r\n", "\n")
}

#[test]
fn the_recording_is_what_a_fresh_run_writes() {
    let fresh = diagnostic::record(&format!("{}/Examples/Scenarios", root()));
    assert_eq!(
        fresh.cases,
        diagnostic::KINDS.len()
            * diagnostic::PIPELINES.len()
            * diagnostic::EXAMPLES.len()
            * diagnostic::PLACEMENTS.len()
            * diagnostic::GAPS_S.len()
    );
    assert_eq!(
        fresh.repeat_mismatches, 0,
        "a case differed between two runs"
    );

    // The outcome of every case, on every host. Compared line by line so
    // that a failure names the scenario and placement.
    let summary = recorded("summary.md");
    for (old, new) in summary.lines().zip(fresh.summary.lines()) {
        assert_eq!(old, new, "summary.md");
    }
    assert_eq!(summary, fresh.summary);

    let results = recorded("results.jsonl");
    assert_eq!(results.lines().count(), fresh.results.lines().count());
    if RECORDED_HERE {
        for (number, (old, new)) in results.lines().zip(fresh.results.lines()).enumerate() {
            assert_eq!(old, new, "results.jsonl, line {}", number + 1);
        }
    }
}
