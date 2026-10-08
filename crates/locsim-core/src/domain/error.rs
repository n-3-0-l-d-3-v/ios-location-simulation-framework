use super::state::SimulationState;
use std::fmt;

/// A rejected configuration value. `field` is a dotted path into the scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub field: &'static str,
    pub reason: String,
}

impl ConfigError {
    pub(crate) fn new(field: &'static str, reason: impl Into<String>) -> Self {
        Self {
            field,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

impl std::error::Error for ConfigError {}

/// Why a [`super::SyntheticLocation`] is not fit to emit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LocationError {
    NonFinite(&'static str),
    NegativeAccuracy(&'static str),
    NegativeSpeed(f64),
    CourseOutOfRange(f64),
    /// A course was supplied without a speed, or with zero speed.
    CourseWithoutMotion,
}

impl fmt::Display for LocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocationError::NonFinite(field) => write!(f, "{field} is not finite"),
            LocationError::NegativeAccuracy(field) => write!(f, "{field} is negative"),
            LocationError::NegativeSpeed(v) => write!(f, "speed {v} is negative"),
            LocationError::CourseOutOfRange(v) => write!(f, "course {v} outside [0, 360)"),
            LocationError::CourseWithoutMotion => {
                write!(f, "course present but speed is absent or zero")
            }
        }
    }
}

impl std::error::Error for LocationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransition {
    pub from: SimulationState,
    pub to: SimulationState,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "illegal state transition {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl std::error::Error for InvalidTransition {}
