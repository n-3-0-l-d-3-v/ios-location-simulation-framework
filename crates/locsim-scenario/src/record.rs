//! The last-known record: the most recent sample a provider emitted, with
//! the provider's counters at that moment and the fingerprint of the
//! scenario that produced it.
//!
//! # What the record is, and is not
//!
//! It is a faithful copy of one emitted sample, for inspection, diagnostics
//! and for showing where a simulation was. It is **not a checkpoint**: a
//! run cannot be continued from it. Continuing would need the state of the
//! movement model, the random streams, the noise engine and the validation
//! gate, none of which is here. A provider started later begins a new run.
//!
//! # Document (record version 1)
//!
//! Strict in the same way as a scenario document: every member required,
//! unknown and duplicate members refused, exact types.
//!
//! - `record_version`: integer.
//! - `scenario_fingerprint`: 16 lower-case hexadecimal digits; see
//!   [`scenario_fingerprint`].
//! - `sample`: the sample as emitted. `timestamp_ns` is an integer, the
//!   sample's own timestamp (the ideal time of its tick, in whatever epoch
//!   the caller's clock uses; nothing is converted). `speed_mps` and
//!   `course_deg` are a number or `null`; `null` means unknown, never zero.
//! - `provider`: `state`, the three counters as canonical decimal strings
//!   (they are `u64`), and `trajectory_complete`.
//!
//! A record must hold a sample that passes `SyntheticLocation::validate`
//! and a `sample_count` of at least one, on export and on import alike.

use crate::error::ScenarioError;
use crate::json::{self, Json, Number, ROOT};
use crate::migrate::MigrationChain;
use crate::schema::{
    coordinate, decimal_u64, integer_i64, invalid, object, string, Errors, Fields,
};
use locsim_core::domain::{
    LocationSource, Scenario, SimulationState, SyntheticLocation, Timestamp,
};
use locsim_core::provider::ProviderStatus;

/// Version of the last-known record this build reads and writes.
pub const CURRENT_RECORD_VERSION: u32 = 1;

const RECORD_VERSION_KEY: &str = "record_version";

/// Only record version 1 exists; no older layout has been invented.
const RECORDS: MigrationChain = MigrationChain {
    version_key: RECORD_VERSION_KEY,
    oldest: 1,
    steps: &[],
};

const _: () = assert!(RECORDS.newest() == CURRENT_RECORD_VERSION);

/// The most recent emitted sample and the provider counters that went with
/// it. See the module documentation for what this does not allow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LastKnown {
    /// [`scenario_fingerprint`] of the scenario that produced the sample.
    pub scenario_fingerprint: u64,
    /// The sample exactly as emitted.
    pub sample: SyntheticLocation,
    /// The provider's state when the record was taken (the sample itself
    /// always says `Running`: only a running provider emits).
    pub state: SimulationState,
    pub sample_count: u64,
    pub failed_count: u64,
    pub missed_ticks: u64,
    pub trajectory_complete: bool,
}

impl LastKnown {
    /// Combines a provider's latest sample with its status. `sample` and
    /// `status` must have been read together: a status that does not name
    /// this sample as the latest is refused rather than recorded.
    pub fn new(
        scenario_fingerprint: u64,
        sample: SyntheticLocation,
        status: &ProviderStatus,
    ) -> Result<Self, ScenarioError> {
        if status.last_sample_time != Some(sample.timestamp) {
            return Err(ScenarioError::InvalidValue {
                path: "sample.timestamp_ns".into(),
                reason: format!(
                    "the status names {:?} as its latest sample time, not this sample's {} ns",
                    status.last_sample_time.map(|t| t.as_nanos()),
                    sample.timestamp.as_nanos()
                ),
            });
        }
        Ok(Self {
            scenario_fingerprint,
            sample,
            state: status.state,
            sample_count: status.sample_count,
            failed_count: status.failed_count,
            missed_ticks: status.missed_ticks,
            trajectory_complete: status.trajectory_complete,
        })
    }

    fn check(&self) -> Result<(), Errors> {
        let mut errors = Vec::new();
        if let Err(error) = self.sample.validate() {
            errors.push(ScenarioError::InvalidSample {
                path: "sample".into(),
                error,
            });
        }
        if self.sample_count == 0 {
            errors.push(ScenarioError::InvalidValue {
                path: "provider.sample_count".into(),
                reason: "a record holds an emitted sample, so at least one was emitted".into(),
            });
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// FNV-1a, 64 bits.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Identifies a scenario by the FNV-1a 64 hash of its exported text.
///
/// Two uses only: telling whether a last-known record was produced by a
/// given scenario, and noticing that it was not. Equal scenarios have equal
/// fingerprints, on every machine and in every build that writes the same
/// schema version. It is **not** cryptographic (different scenarios with
/// the same fingerprint can be constructed, and about one unrelated pair in
/// 2^64 collides by chance) and it is **not** a check that a file is intact.
/// An invalid scenario has no exported text and so no fingerprint.
pub fn scenario_fingerprint(scenario: &Scenario) -> Result<u64, Vec<ScenarioError>> {
    Ok(fnv1a_64(crate::export_scenario(scenario)?.as_bytes()))
}

const STATES: [SimulationState; 7] = SimulationState::ALL;

fn state_name(state: SimulationState) -> &'static str {
    match state {
        SimulationState::Idle => "idle",
        SimulationState::Starting => "starting",
        SimulationState::Running => "running",
        SimulationState::Paused => "paused",
        SimulationState::Stopping => "stopping",
        SimulationState::Error => "error",
        SimulationState::Recovering => "recovering",
    }
}

const SOURCES: [LocationSource; 3] = [
    LocationSource::Simulation,
    LocationSource::Replay,
    LocationSource::Test,
];

fn source_name(source: LocationSource) -> &'static str {
    match source {
        LocationSource::Simulation => "simulation",
        LocationSource::Replay => "replay",
        LocationSource::Test => "test",
    }
}

fn text(value: impl Into<String>) -> Json {
    Json::String(value.into())
}

fn integer(value: i64) -> Json {
    Json::Number(match u64::try_from(value) {
        Ok(v) => Number::PosInt(v),
        Err(_) => Number::NegInt(value),
    })
}

fn optional(value: Option<f64>) -> Json {
    value.map_or(Json::Null, Json::float)
}

fn encode(record: &LastKnown) -> Json {
    let s = &record.sample;
    object(vec![
        (
            RECORD_VERSION_KEY,
            Json::Number(Number::PosInt(CURRENT_RECORD_VERSION.into())),
        ),
        (
            "scenario_fingerprint",
            text(format!("{:016x}", record.scenario_fingerprint)),
        ),
        (
            "sample",
            object(vec![
                ("timestamp_ns", integer(s.timestamp.as_nanos())),
                (
                    "coordinate",
                    object(vec![
                        ("latitude", Json::float(s.coordinate.latitude())),
                        ("longitude", Json::float(s.coordinate.longitude())),
                    ]),
                ),
                ("altitude_m", Json::float(s.altitude_m)),
                (
                    "horizontal_accuracy_m",
                    Json::float(s.horizontal_accuracy_m),
                ),
                ("vertical_accuracy_m", Json::float(s.vertical_accuracy_m)),
                ("speed_mps", optional(s.speed_mps)),
                ("course_deg", optional(s.course_deg)),
                ("source", text(source_name(s.source))),
                ("simulation_state", text(state_name(s.simulation_state))),
            ]),
        ),
        (
            "provider",
            object(vec![
                ("state", text(state_name(record.state))),
                ("sample_count", text(record.sample_count.to_string())),
                ("failed_count", text(record.failed_count.to_string())),
                ("missed_ticks", text(record.missed_ticks.to_string())),
                (
                    "trajectory_complete",
                    Json::Bool(record.trajectory_complete),
                ),
            ]),
        ),
    ])
}

/// Exactly 16 lower-case hexadecimal digits.
fn fingerprint(value: &Json, path: &str, errors: &mut Errors) -> Option<u64> {
    let written = string(value, path, errors)?;
    let canonical = written.len() == 16
        && written
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    match u64::from_str_radix(written, 16) {
        Ok(v) if canonical => Some(v),
        _ => invalid(
            path,
            format!("{written:?} is not 16 lower-case hexadecimal digits"),
            errors,
        ),
    }
}

fn sample(value: &Json, path: &str, errors: &mut Errors) -> Option<SyntheticLocation> {
    let mut f = Fields::open(value, path, errors)?;
    let timestamp = f
        .take("timestamp_ns", errors)
        .and_then(|(v, p)| integer_i64(v, &p, errors));
    let position = f
        .take("coordinate", errors)
        .and_then(|(v, p)| coordinate(v, &p, errors));
    let altitude_m = f.float("altitude_m", errors);
    let horizontal_accuracy_m = f.float("horizontal_accuracy_m", errors);
    let vertical_accuracy_m = f.float("vertical_accuracy_m", errors);
    let speed_mps = f.optional_float("speed_mps", errors);
    let course_deg = f.optional_float("course_deg", errors);
    let source = f.named("source", &SOURCES, source_name, errors);
    let simulation_state = f.named("simulation_state", &STATES, state_name, errors);
    f.close(errors);
    Some(SyntheticLocation {
        timestamp: Timestamp::from_nanos(timestamp?),
        coordinate: position?,
        altitude_m: altitude_m?,
        horizontal_accuracy_m: horizontal_accuracy_m?,
        vertical_accuracy_m: vertical_accuracy_m?,
        speed_mps: speed_mps?,
        course_deg: course_deg?,
        source: source?,
        simulation_state: simulation_state?,
    })
}

struct Counters {
    state: SimulationState,
    sample_count: u64,
    failed_count: u64,
    missed_ticks: u64,
    trajectory_complete: bool,
}

fn provider(value: &Json, path: &str, errors: &mut Errors) -> Option<Counters> {
    let mut f = Fields::open(value, path, errors)?;
    let state = f.named("state", &STATES, state_name, errors);
    let counter = |f: &mut Fields<'_>, key: &str, errors: &mut Errors| {
        f.take(key, errors)
            .and_then(|(v, p)| decimal_u64(v, &p, errors))
    };
    let sample_count = counter(&mut f, "sample_count", errors);
    let failed_count = counter(&mut f, "failed_count", errors);
    let missed_ticks = counter(&mut f, "missed_ticks", errors);
    let trajectory_complete = f.boolean("trajectory_complete", errors);
    f.close(errors);
    Some(Counters {
        state: state?,
        sample_count: sample_count?,
        failed_count: failed_count?,
        missed_ticks: missed_ticks?,
        trajectory_complete: trajectory_complete?,
    })
}

fn decode(document: &Json) -> Result<LastKnown, Errors> {
    let mut errors = Vec::new();
    let record = (|| {
        let mut f = Fields::open(document, ROOT, &mut errors)?;
        // Checked by the version stage; read here so it is not "unknown".
        let _version = f.take(RECORD_VERSION_KEY, &mut errors);
        let scenario_fingerprint = f
            .take("scenario_fingerprint", &mut errors)
            .and_then(|(v, p)| fingerprint(v, &p, &mut errors));
        let emitted = f
            .take("sample", &mut errors)
            .and_then(|(v, p)| sample(v, &p, &mut errors));
        let counters = f
            .take("provider", &mut errors)
            .and_then(|(v, p)| provider(v, &p, &mut errors));
        f.close(&mut errors);
        let counters = counters?;
        Some(LastKnown {
            scenario_fingerprint: scenario_fingerprint?,
            sample: emitted?,
            state: counters.state,
            sample_count: counters.sample_count,
            failed_count: counters.failed_count,
            missed_ticks: counters.missed_ticks,
            trajectory_complete: counters.trajectory_complete,
        })
    })();
    match record {
        Some(record) if errors.is_empty() => Ok(record),
        _ => Err(errors),
    }
}

/// Writes a last-known record as a record version
/// [`CURRENT_RECORD_VERSION`] document. A record whose sample is not fit to
/// emit, or that claims no sample was emitted, is not written.
pub fn export_last_known(record: &LastKnown) -> Result<String, Vec<ScenarioError>> {
    record.check()?;
    json::write(&encode(record)).map_err(|e| vec![e])
}

/// Reads a last-known record: parse, version, strict decode, then the same
/// checks as on export. Each stage stops the import and reports everything
/// it found, as for a scenario document.
pub fn import_last_known(text: &str) -> Result<LastKnown, Vec<ScenarioError>> {
    let document = json::parse(text)?;
    let document = RECORDS.migrate(document).map_err(|e| vec![e])?;
    let record = decode(&document)?;
    record.check()?;
    Ok(record)
}

#[cfg(test)]
mod tests;
