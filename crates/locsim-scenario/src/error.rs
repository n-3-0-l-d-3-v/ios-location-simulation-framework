use locsim_core::domain::{ConfigError, LocationError, RouteError};
use locsim_core::geographic::CoordinateError;
use std::fmt;

/// Why a scenario document could not be imported or a scenario exported.
///
/// `path` is a dotted path from the document root, with list positions in
/// brackets (`route.points[3].coordinate.latitude`); the root itself is `$`.
/// Paths use the same field names as [`ConfigError::field`].
#[derive(Debug, Clone, PartialEq)]
pub enum ScenarioError {
    /// The text is not a JSON document. Line and column are 1-based.
    Syntax {
        line: usize,
        column: usize,
        message: String,
    },
    /// An object has two members with the same name. Neither is used.
    DuplicateKey {
        path: String,
    },
    /// The document's version is not one this build can read.
    UnsupportedVersion {
        found: u64,
        supported: u32,
    },
    /// A migration step from schema version `from` failed.
    Migration {
        from: u32,
        reason: String,
    },
    MissingField {
        path: String,
    },
    UnknownField {
        path: String,
    },
    WrongType {
        path: String,
        expected: &'static str,
        found: &'static str,
    },
    /// The value has the right JSON type but is not acceptable.
    InvalidValue {
        path: String,
        reason: String,
    },
    InvalidCoordinate {
        path: String,
        error: CoordinateError,
    },
    InvalidRoute {
        path: String,
        error: RouteError,
    },
    /// A last-known record holds a sample that is not fit to emit.
    InvalidSample {
        path: String,
        error: LocationError,
    },
    /// The document is well formed but describes an invalid scenario
    /// (reported by `Scenario::validate`).
    InvalidScenario(ConfigError),
    /// Export only: JSON has no representation for NaN or infinity.
    NonFiniteNumber {
        path: String,
    },
    /// Export only: the JSON writer failed. Not expected to occur.
    Serialization {
        message: String,
    },
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScenarioError::Syntax {
                line,
                column,
                message,
            } => write!(f, "invalid JSON at line {line}, column {column}: {message}"),
            ScenarioError::DuplicateKey { path } => write!(f, "{path}: duplicate key"),
            ScenarioError::UnsupportedVersion { found, supported } => write!(
                f,
                "unsupported document version {found} (this build supports {supported})"
            ),
            ScenarioError::Migration { from, reason } => {
                write!(f, "migration from schema version {from} failed: {reason}")
            }
            ScenarioError::MissingField { path } => write!(f, "{path}: missing"),
            ScenarioError::UnknownField { path } => write!(f, "{path}: unknown field"),
            ScenarioError::WrongType {
                path,
                expected,
                found,
            } => write!(f, "{path}: expected {expected}, found {found}"),
            ScenarioError::InvalidValue { path, reason } => write!(f, "{path}: {reason}"),
            ScenarioError::InvalidCoordinate { path, error } => write!(f, "{path}: {error}"),
            ScenarioError::InvalidRoute { path, error } => write!(f, "{path}: {error}"),
            ScenarioError::InvalidSample { path, error } => write!(f, "{path}: {error}"),
            ScenarioError::InvalidScenario(e) => write!(f, "{e}"),
            ScenarioError::NonFiniteNumber { path } => {
                write!(f, "{path}: not finite, cannot be written as JSON")
            }
            ScenarioError::Serialization { message } => write!(f, "JSON writer: {message}"),
        }
    }
}

impl std::error::Error for ScenarioError {}
