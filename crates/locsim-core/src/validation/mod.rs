//! Per-sample validation gate. A sample that fails here is never emitted.
//!
//! Stages implemented so far: field validity and timestamp monotonicity.
//! Movement-constraint, metadata-consistency and scenario-boundary stages
//! join this pipeline with the movement (T05) and consistency (T07) tickets.

use crate::domain::{LocationError, SyntheticLocation, Timestamp};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValidationError {
    Location(LocationError),
    /// Timestamp did not strictly increase over the last accepted sample.
    NonMonotonicTimestamp {
        previous: Timestamp,
        current: Timestamp,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::Location(e) => write!(f, "invalid sample: {e}"),
            ValidationError::NonMonotonicTimestamp { previous, current } => write!(
                f,
                "timestamp {} ns does not follow previous {} ns",
                current.as_nanos(),
                previous.as_nanos()
            ),
        }
    }
}

impl std::error::Error for ValidationError {}

impl From<LocationError> for ValidationError {
    fn from(e: LocationError) -> Self {
        ValidationError::Location(e)
    }
}

/// Stateful validator for one sample stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SampleValidator {
    last_accepted: Option<Timestamp>,
}

impl SampleValidator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accepts or rejects `sample`. Only accepted samples advance the
    /// validator's state, so a rejected sample cannot poison later checks.
    pub fn validate(&mut self, sample: &SyntheticLocation) -> Result<(), ValidationError> {
        sample.validate()?;
        if let Some(previous) = self.last_accepted {
            if sample.timestamp <= previous {
                return Err(ValidationError::NonMonotonicTimestamp {
                    previous,
                    current: sample.timestamp,
                });
            }
        }
        self.last_accepted = Some(sample.timestamp);
        Ok(())
    }

    /// Forgets stream history; call when a new simulation run starts.
    pub fn reset(&mut self) {
        self.last_accepted = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Coordinate, LocationSource, SimulationState};

    fn sample(nanos: i64) -> SyntheticLocation {
        SyntheticLocation {
            timestamp: Timestamp::from_nanos(nanos),
            coordinate: Coordinate::new(12.9352, 77.6245).unwrap(),
            altitude_m: 920.0,
            horizontal_accuracy_m: 5.0,
            vertical_accuracy_m: 8.0,
            speed_mps: Some(0.0),
            course_deg: None,
            source: LocationSource::Simulation,
            simulation_state: SimulationState::Running,
        }
    }

    #[test]
    fn accepts_increasing_timestamps() {
        let mut v = SampleValidator::new();
        for n in [0, 1, 2, 1_000_000_000] {
            assert_eq!(v.validate(&sample(n)), Ok(()));
        }
    }

    #[test]
    fn rejects_equal_or_earlier_timestamps() {
        let mut v = SampleValidator::new();
        v.validate(&sample(100)).unwrap();
        for n in [100, 99] {
            assert_eq!(
                v.validate(&sample(n)),
                Err(ValidationError::NonMonotonicTimestamp {
                    previous: Timestamp::from_nanos(100),
                    current: Timestamp::from_nanos(n),
                })
            );
        }
    }

    #[test]
    fn rejects_invalid_fields_without_advancing_state() {
        let mut v = SampleValidator::new();
        v.validate(&sample(100)).unwrap();
        let bad = SyntheticLocation {
            altitude_m: f64::NAN,
            ..sample(500)
        };
        assert_eq!(
            v.validate(&bad),
            Err(ValidationError::Location(LocationError::NonFinite(
                "altitude"
            )))
        );
        // 200 would be rejected had the bad sample at 500 been recorded.
        assert_eq!(v.validate(&sample(200)), Ok(()));
    }

    #[test]
    fn reset_starts_a_new_stream() {
        let mut v = SampleValidator::new();
        v.validate(&sample(100)).unwrap();
        v.reset();
        assert_eq!(v.validate(&sample(50)), Ok(()));
    }
}
