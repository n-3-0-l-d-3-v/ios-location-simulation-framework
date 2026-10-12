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

#[path = "../examples/gap_diagnostic.rs"]
#[allow(dead_code)]
mod diagnostic;

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

    let results = recorded("results.jsonl");
    for (number, (old, new)) in results.lines().zip(fresh.results.lines()).enumerate() {
        assert_eq!(old, new, "results.jsonl, line {}", number + 1);
    }
    assert_eq!(results.lines().count(), fresh.results.lines().count());
    assert_eq!(recorded("summary.md"), fresh.summary);
}
