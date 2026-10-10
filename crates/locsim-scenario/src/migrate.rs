//! Schema versions and migration between them.
//!
//! A document states its `schema_version`. A [`MigrationChain`] knows the
//! oldest version it can read and holds one step per version change, in
//! order: step `i` turns version `oldest + i` into `oldest + i + 1`. A
//! document is brought to the chain's newest version by running, in order,
//! every step from its own version onwards; the strict decoder then reads
//! the result, so a step that leaves anything behind is caught there.
//!
//! Steps work on the document tree, before any typing, because an old
//! layout by definition does not fit the current types.
//!
//! **There is only schema version 1.** The production chain therefore has
//! no steps, and no historical layout has been invented to give it one. The
//! mechanism is exercised by the tests below with chains that exist only in
//! the tests.

use crate::error::ScenarioError;
use crate::json::{Json, Number, ROOT};

const VERSION_KEY: &str = "schema_version";

/// Rewrites a document from one schema version to the next. It must not
/// touch `schema_version`: the chain advances it. An `Err` carries the
/// reason the document cannot be migrated.
pub(crate) type MigrationStep = fn(&mut Json) -> Result<(), String>;

pub(crate) struct MigrationChain {
    /// The oldest schema version that can still be read.
    pub(crate) oldest: u32,
    /// `steps[i]` migrates version `oldest + i` to `oldest + i + 1`.
    pub(crate) steps: &'static [MigrationStep],
}

/// The chain used by `import_scenario`.
pub(crate) const PRODUCTION: MigrationChain = MigrationChain {
    oldest: 1,
    steps: &[],
};

// The chain must end at the version the core's `Scenario` is defined for.
const _: () = assert!(PRODUCTION.newest() == locsim_core::domain::CURRENT_SCHEMA_VERSION);

impl MigrationChain {
    /// The version every document is migrated to.
    pub(crate) const fn newest(&self) -> u32 {
        self.oldest + self.steps.len() as u32
    }

    /// The document's stated version, if this chain can read it.
    pub(crate) fn version_of(&self, document: &Json) -> Result<u32, ScenarioError> {
        if document.members().is_none() {
            return Err(ScenarioError::WrongType {
                path: ROOT.into(),
                expected: "object",
                found: document.kind(),
            });
        }
        let Some(value) = document.get(VERSION_KEY) else {
            return Err(ScenarioError::MissingField {
                path: VERSION_KEY.into(),
            });
        };
        match value {
            Json::Number(Number::PosInt(found)) => match u32::try_from(*found) {
                Ok(v) if (self.oldest..=self.newest()).contains(&v) => Ok(v),
                _ => Err(ScenarioError::UnsupportedVersion {
                    found: *found,
                    supported: self.newest(),
                }),
            },
            Json::Number(Number::NegInt(found)) => Err(ScenarioError::InvalidValue {
                path: VERSION_KEY.into(),
                reason: format!("{found} is not a schema version"),
            }),
            other => Err(ScenarioError::WrongType {
                path: VERSION_KEY.into(),
                expected: "integer",
                found: other.kind(),
            }),
        }
    }

    /// Checks the document's version and brings it to [`Self::newest`].
    /// A document already at the newest version is returned untouched.
    pub(crate) fn migrate(&self, mut document: Json) -> Result<Json, ScenarioError> {
        let found = self.version_of(&document)?;
        for from in found..self.newest() {
            let step = self.steps[(from - self.oldest) as usize];
            let failed = |reason: String| ScenarioError::Migration { from, reason };
            step(&mut document).map_err(failed)?;
            let version = match &mut document {
                Json::Object(members) => members.iter_mut().find(|(k, _)| k == VERSION_KEY),
                _ => None,
            };
            match version {
                Some((_, value)) => *value = Json::Number(Number::PosInt(u64::from(from) + 1)),
                None => return Err(failed(format!("the step removed {VERSION_KEY}"))),
            }
        }
        Ok(document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    // Test-only steps over toy documents. Each one appends its name to the
    // `log` member, so the order in which steps ran is visible.

    fn log(document: &mut Json, entry: &str) -> Result<(), String> {
        match document {
            Json::Object(members) => match members.iter_mut().find(|(k, _)| k == "log") {
                Some((_, Json::Array(entries))) => {
                    entries.push(Json::String(entry.into()));
                    Ok(())
                }
                _ => Err("no log".into()),
            },
            _ => Err("not an object".into()),
        }
    }

    /// v1 → v2: `metres` is renamed `distance_m`.
    fn rename(document: &mut Json) -> Result<(), String> {
        log(document, "1->2")?;
        let Json::Object(members) = document else {
            return Err("not an object".into());
        };
        match members.iter_mut().find(|(k, _)| k == "metres") {
            Some(member) => {
                member.0 = "distance_m".into();
                Ok(())
            }
            None => Err("no member `metres`".into()),
        }
    }

    /// v2 → v3: `distance_m` moves into a new `step` object.
    fn nest(document: &mut Json) -> Result<(), String> {
        log(document, "2->3")?;
        let Json::Object(members) = document else {
            return Err("not an object".into());
        };
        let index = members
            .iter()
            .position(|(k, _)| k == "distance_m")
            .ok_or("no member `distance_m`")?;
        let (key, value) = members.remove(index);
        members.push(("step".into(), Json::Object(vec![(key, value)])));
        Ok(())
    }

    /// v3 → v4: a step that deletes the version member.
    fn drop_version(document: &mut Json) -> Result<(), String> {
        log(document, "3->4")?;
        if let Json::Object(members) = document {
            members.retain(|(k, _)| k != VERSION_KEY);
        }
        Ok(())
    }

    const CHAIN: MigrationChain = MigrationChain {
        oldest: 1,
        steps: &[rename, nest],
    };

    fn doc(text: &str) -> Json {
        parse(text).unwrap()
    }

    #[test]
    fn the_production_chain_reads_exactly_the_current_version_and_has_no_steps() {
        assert_eq!(PRODUCTION.oldest, 1);
        assert!(PRODUCTION.steps.is_empty());
        assert_eq!(
            PRODUCTION.newest(),
            locsim_core::domain::CURRENT_SCHEMA_VERSION
        );
    }

    #[test]
    fn steps_run_in_order_from_the_documents_version_to_the_newest() {
        assert_eq!(CHAIN.newest(), 3);
        assert_eq!(
            CHAIN.migrate(doc(r#"{"schema_version": 1, "metres": 2.5, "log": []}"#)),
            Ok(doc(
                r#"{"schema_version": 3, "log": ["1->2", "2->3"], "step": {"distance_m": 2.5}}"#
            ))
        );
        // A newer document skips the steps it does not need.
        assert_eq!(
            CHAIN.migrate(doc(
                r#"{"schema_version": 2, "distance_m": 2.5, "log": []}"#
            )),
            Ok(doc(
                r#"{"schema_version": 3, "log": ["2->3"], "step": {"distance_m": 2.5}}"#
            ))
        );
    }

    #[test]
    fn a_document_at_the_newest_version_is_not_touched() {
        let current = doc(r#"{"log": [], "anything": [1, {"x": null}], "schema_version": 3}"#);
        assert_eq!(CHAIN.migrate(current.clone()), Ok(current));
    }

    #[test]
    fn versions_outside_the_chain_are_refused_without_running_anything() {
        for found in [
            0,
            4,
            5,
            u64::from(u32::MAX),
            u64::from(u32::MAX) + 1,
            u64::MAX,
        ] {
            let text = format!(r#"{{"schema_version": {found}, "metres": 1.0, "log": []}}"#);
            assert_eq!(
                CHAIN.migrate(doc(&text)),
                Err(ScenarioError::UnsupportedVersion {
                    found,
                    supported: 3
                }),
            );
        }
        for found in [0, 2, 3] {
            let text = format!(r#"{{"schema_version": {found}}}"#);
            assert_eq!(
                PRODUCTION.migrate(doc(&text)),
                Err(ScenarioError::UnsupportedVersion {
                    found,
                    supported: 1
                }),
            );
        }
    }

    #[test]
    fn the_version_must_be_present_and_an_integer_literal() {
        assert_eq!(
            CHAIN.migrate(doc(r#"{"metres": 1.0}"#)),
            Err(ScenarioError::MissingField {
                path: "schema_version".into()
            })
        );
        for (literal, found) in [
            ("1.0", "non-integer number"),
            ("1e0", "non-integer number"),
            ("\"1\"", "string"),
            ("null", "null"),
            ("true", "boolean"),
            ("[1]", "array"),
            ("{}", "object"),
            ("-0", "non-integer number"),
        ] {
            let text = format!(r#"{{"schema_version": {literal}}}"#);
            assert_eq!(
                CHAIN.migrate(doc(&text)),
                Err(ScenarioError::WrongType {
                    path: "schema_version".into(),
                    expected: "integer",
                    found
                }),
                "{literal}"
            );
        }
        assert!(matches!(
            CHAIN.migrate(doc(r#"{"schema_version": -1}"#)),
            Err(ScenarioError::InvalidValue { path, .. }) if path == "schema_version"
        ));
        for root in ["[]", "null", "1", "\"x\""] {
            assert!(matches!(
                CHAIN.migrate(doc(root)),
                Err(ScenarioError::WrongType { path, expected: "object", .. }) if path == ROOT
            ));
        }
    }

    #[test]
    fn a_failing_step_stops_the_chain_and_names_its_version() {
        // The first step succeeds, the second finds nothing to move.
        const BROKEN: MigrationChain = MigrationChain {
            oldest: 1,
            steps: &[nest, nest],
        };
        assert_eq!(
            BROKEN.migrate(doc(
                r#"{"schema_version": 1, "distance_m": 1.0, "log": []}"#
            )),
            Err(ScenarioError::Migration {
                from: 2,
                reason: "no member `distance_m`".into()
            })
        );
        assert_eq!(
            CHAIN.migrate(doc(r#"{"schema_version": 1, "log": []}"#)),
            Err(ScenarioError::Migration {
                from: 1,
                reason: "no member `metres`".into()
            })
        );
    }

    #[test]
    fn a_step_cannot_lose_the_version() {
        const CARELESS: MigrationChain = MigrationChain {
            oldest: 3,
            steps: &[drop_version],
        };
        assert_eq!(
            CARELESS.migrate(doc(r#"{"schema_version": 3, "log": []}"#)),
            Err(ScenarioError::Migration {
                from: 3,
                reason: "the step removed schema_version".into()
            })
        );
    }

    #[test]
    fn the_chain_not_the_step_advances_the_version() {
        // A step that writes a wrong version is overruled.
        fn liar(document: &mut Json) -> Result<(), String> {
            if let Json::Object(members) = document {
                for (key, value) in members.iter_mut() {
                    if key == VERSION_KEY {
                        *value = Json::Number(Number::PosInt(99));
                    }
                }
            }
            Ok(())
        }
        const CHAIN: MigrationChain = MigrationChain {
            oldest: 7,
            steps: &[liar, liar],
        };
        assert_eq!(
            CHAIN.migrate(doc(r#"{"schema_version": 7}"#)),
            Ok(doc(r#"{"schema_version": 9}"#))
        );
    }
}
