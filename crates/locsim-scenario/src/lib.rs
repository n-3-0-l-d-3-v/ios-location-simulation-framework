//! Scenario documents: strict JSON import and export of
//! [`locsim_core::domain::Scenario`], with schema versioning.
//!
//! This crate turns text into a validated `Scenario` and back. It reads and
//! writes strings only; files and persistence are a separate concern.
//!
//! # Import
//!
//! [`import_scenario`] runs these stages in order and stops at the first
//! that reports anything, returning everything that stage found:
//!
//! 1. **Parse** the text as exactly one JSON document; duplicate member
//!    names are errors.
//! 2. **Version**: `schema_version` must be one this build can read.
//! 3. **Migrate** an older document to the current schema.
//! 4. **Decode** strictly: every member required, none unknown, exact
//!    types, coordinates and routes built by their validating constructors.
//! 5. **Validate** with `Scenario::validate`.
//!
//! Nothing is defaulted, repaired or normalised at any stage. A scenario
//! that comes back is exactly what the document says, and is valid.
//!
//! # Export
//!
//! [`export_scenario`] validates first and writes only a valid scenario.
//! The text is deterministic (fixed member order, shortest floats that read
//! back to the same bits), so importing it yields a bit-identical scenario
//! and exporting that yields the identical text.
//!
//! Importing a scenario does not admit its route: whether a route can be
//! replayed within the movement limits is decided when a run starts.

#![forbid(unsafe_code)]

mod error;
mod json;
mod migrate;
mod schema;

pub use error::ScenarioError;
pub use locsim_core::domain::CURRENT_SCHEMA_VERSION;

use locsim_core::domain::Scenario;
use migrate::MigrationChain;

fn invalid(scenario: &Scenario) -> Result<(), Vec<ScenarioError>> {
    scenario.validate().map_err(|errors| {
        errors
            .into_iter()
            .map(ScenarioError::InvalidScenario)
            .collect()
    })
}

fn import_with(text: &str, chain: &MigrationChain) -> Result<Scenario, Vec<ScenarioError>> {
    let document = json::parse(text)?;
    let document = chain.migrate(document).map_err(|e| vec![e])?;
    let scenario = schema::decode(&document)?;
    invalid(&scenario)?;
    Ok(scenario)
}

/// Reads a scenario document. See the crate documentation for the stages
/// and for what is rejected.
pub fn import_scenario(text: &str) -> Result<Scenario, Vec<ScenarioError>> {
    import_with(text, &migrate::PRODUCTION)
}

/// Writes a valid scenario as a schema version
/// [`CURRENT_SCHEMA_VERSION`] document. An invalid scenario is not written.
pub fn export_scenario(scenario: &Scenario) -> Result<String, Vec<ScenarioError>> {
    invalid(scenario)?;
    let document = schema::encode(scenario)?;
    json::write(&document).map_err(|e| vec![e])
}

#[cfg(test)]
mod tests {
    //! The whole import pipeline with a migration in it. The old layout
    //! here ("version 0") exists only in this test: no such schema was ever
    //! released, and `import_scenario` refuses it.

    use super::*;
    use crate::json::{Json, Number};
    use locsim_core::domain::{
        Coordinate, MovementMode, MovementParameters, NoiseParameters, PlaybackParameters,
    };

    fn members(document: &mut Json) -> Result<&mut Vec<(String, Json)>, String> {
        match document {
            Json::Object(members) => Ok(members),
            _ => Err("not an object".into()),
        }
    }

    /// Test-only v0 → v1: the seed was a number called `random_seed`, and
    /// the origin a `[latitude, longitude]` pair.
    fn from_v0(document: &mut Json) -> Result<(), String> {
        let members = members(document)?;
        for (key, value) in members.iter_mut() {
            match (key.as_str(), &value) {
                ("random_seed", Json::Number(Number::PosInt(seed))) => {
                    *value = Json::String(seed.to_string());
                    *key = "seed".into();
                }
                ("random_seed", _) => return Err("random_seed is not an integer".into()),
                ("origin", Json::Array(pair)) if pair.len() == 2 => {
                    *value = Json::Object(vec![
                        ("latitude".into(), pair[0].clone()),
                        ("longitude".into(), pair[1].clone()),
                    ]);
                }
                ("origin", _) => return Err("origin is not a pair".into()),
                _ => {}
            }
        }
        Ok(())
    }

    const WITH_V0: MigrationChain = MigrationChain {
        oldest: 0,
        steps: &[from_v0],
    };

    fn walking() -> Scenario {
        Scenario {
            schema_version: CURRENT_SCHEMA_VERSION,
            name: "walk".into(),
            origin: Coordinate::new(12.9352, 77.6245).unwrap(),
            altitude_m: 920.0,
            mode: MovementMode::Walking,
            movement: MovementParameters::walking_preset(),
            noise: NoiseParameters::NONE,
            horizontal_accuracy_m: 5.0,
            vertical_accuracy_m: 8.0,
            update_interval_s: 1.0,
            seed: 9_007_199_254_740_993,
            route: None,
            playback: PlaybackParameters::REAL_TIME,
        }
    }

    /// The v0 text of `walking()`, made by undoing the migration by hand.
    fn v0_text(extra: &str) -> String {
        let text = export_scenario(&walking()).unwrap();
        let old = text
            .replace("\"schema_version\": 1", "\"schema_version\": 0")
            .replace(
                "\"origin\": {\n    \"latitude\": 12.9352,\n    \"longitude\": 77.6245\n  }",
                "\"origin\": [12.9352, 77.6245]",
            )
            .replace(
                "\"seed\": \"9007199254740993\"",
                &format!("\"random_seed\": 9007199254740993{extra}"),
            );
        assert_eq!(old.matches("random_seed").count(), 1);
        assert!(old.contains("\"origin\": ["));
        old
    }

    #[test]
    fn an_old_document_is_migrated_then_strictly_decoded_and_validated() {
        let migrated = import_with(&v0_text(""), &WITH_V0).unwrap();
        assert_eq!(format!("{migrated:?}"), format!("{:?}", walking()));
        // A current document goes through the same chain untouched.
        let current = export_scenario(&walking()).unwrap();
        assert_eq!(import_with(&current, &WITH_V0), Ok(walking()));
    }

    #[test]
    fn production_refuses_the_test_only_version() {
        assert_eq!(
            import_scenario(&v0_text("")),
            Err(vec![ScenarioError::UnsupportedVersion {
                found: 0,
                supported: 1
            }])
        );
    }

    #[test]
    fn what_a_migration_leaves_behind_is_caught_by_the_strict_decoder() {
        // An old member the step does not know about is not carried along.
        assert_eq!(
            import_with(&v0_text(",\n  \"legacy_flag\": true"), &WITH_V0),
            Err(vec![ScenarioError::UnknownField {
                path: "legacy_flag".into()
            }])
        );
        // A migrated value is still subject to scenario validation.
        let text = v0_text("").replace("\"update_interval_s\": 1.0", "\"update_interval_s\": 0.0");
        assert!(matches!(
            import_with(&text, &WITH_V0).unwrap_err().as_slice(),
            [ScenarioError::InvalidScenario(e)] if e.field == "update_interval_s"
        ));
    }

    #[test]
    fn a_migration_failure_is_reported_as_such() {
        let text = v0_text("").replace("\"random_seed\": 9007199254740993", "\"random_seed\": 1.5");
        assert_eq!(
            import_with(&text, &WITH_V0),
            Err(vec![ScenarioError::Migration {
                from: 0,
                reason: "random_seed is not an integer".into()
            }])
        );
    }
}
