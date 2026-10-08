//! Per-sample validation gate. A sample that fails here is never emitted,
//! and nothing may modify a sample after it has passed.
//!
//! Stages, in order:
//! 1. field validity (finite, in range, course only while moving);
//! 2. timestamp strictly after the previous accepted sample;
//! 3. with [`SampleLimits`]: reported speed ≤ maximum, position inside the
//!    scenario boundary, displacement since the previous accepted sample
//!    physically possible.
//!
//! The gate is independent of the engines upstream: it re-derives every
//! quantity from the sample itself with exact geodesics, so a bug in a
//! movement model or in the noise engine cannot leak out. Heading-rate and
//! acceleration limits join with the movement (T05) and consistency (T07)
//! tickets.

use crate::domain::{Boundary, Coordinate, LocationError, Scenario, SyntheticLocation, Timestamp};
use crate::geographic::{self, GeoError};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValidationError {
    Location(LocationError),
    /// Timestamp did not strictly increase over the last accepted sample.
    NonMonotonicTimestamp {
        previous: Timestamp,
        current: Timestamp,
    },
    SpeedAboveMaximum {
        speed_mps: f64,
        max_mps: f64,
    },
    OutsideBoundary {
        distance_m: f64,
        radius_m: f64,
    },
    /// The position moved further since the previous sample than the
    /// configured limits allow in the elapsed time.
    ImpossibleDisplacement {
        distance_m: f64,
        limit_m: f64,
    },
    /// A distance needed for a check could not be computed.
    Geo(GeoError),
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
            ValidationError::SpeedAboveMaximum { speed_mps, max_mps } => {
                write!(f, "speed {speed_mps} m/s exceeds maximum {max_mps} m/s")
            }
            ValidationError::OutsideBoundary {
                distance_m,
                radius_m,
            } => write!(
                f,
                "position is {distance_m} m from the boundary centre, radius is {radius_m} m"
            ),
            ValidationError::ImpossibleDisplacement {
                distance_m,
                limit_m,
            } => write!(
                f,
                "moved {distance_m} m since the previous sample, limit is {limit_m} m"
            ),
            ValidationError::Geo(e) => write!(f, "validation geometry failed: {e}"),
        }
    }
}

impl std::error::Error for ValidationError {}

impl From<LocationError> for ValidationError {
    fn from(e: LocationError) -> Self {
        ValidationError::Location(e)
    }
}

impl From<GeoError> for ValidationError {
    fn from(e: GeoError) -> Self {
        ValidationError::Geo(e)
    }
}

/// Scenario-derived limits enforced on the final output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SampleLimits {
    /// Upper bound on the reported speed field.
    pub max_speed_mps: f64,
    /// Upper bound on distance between consecutive outputs, per second.
    pub max_displacement_rate_mps: f64,
    pub boundary: Option<Boundary>,
}

impl SampleLimits {
    /// Output may move at most as fast as the true motion plus the rate at
    /// which position noise is allowed to change.
    pub fn for_scenario(scenario: &Scenario) -> Self {
        let noise_rate = if scenario.noise.has_position_noise() {
            scenario.noise.max_offset_rate_mps
        } else {
            0.0
        };
        Self {
            max_speed_mps: scenario.movement.max_speed_mps,
            max_displacement_rate_mps: scenario.movement.max_speed_mps + noise_rate,
            boundary: scenario.boundary(),
        }
    }
}

/// Stateful validator for one sample stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SampleValidator {
    limits: Option<SampleLimits>,
    last_accepted: Option<(Timestamp, Coordinate)>,
}

impl SampleValidator {
    /// Validator with stages 1–2 only.
    pub fn new() -> Self {
        Self::default()
    }

    /// Validator that additionally enforces scenario limits (stage 3).
    pub fn with_limits(limits: SampleLimits) -> Self {
        Self {
            limits: Some(limits),
            last_accepted: None,
        }
    }

    /// Accepts or rejects `sample`. Only accepted samples advance the
    /// validator's state, so a rejected sample cannot poison later checks.
    pub fn validate(&mut self, sample: &SyntheticLocation) -> Result<(), ValidationError> {
        sample.validate()?;
        if let Some((previous, _)) = self.last_accepted {
            if sample.timestamp <= previous {
                return Err(ValidationError::NonMonotonicTimestamp {
                    previous,
                    current: sample.timestamp,
                });
            }
        }
        if let Some(limits) = self.limits {
            self.check_limits(&limits, sample)?;
        }
        self.last_accepted = Some((sample.timestamp, sample.coordinate));
        Ok(())
    }

    fn check_limits(
        &self,
        limits: &SampleLimits,
        sample: &SyntheticLocation,
    ) -> Result<(), ValidationError> {
        if let Some(speed_mps) = sample.speed_mps {
            if speed_mps > limits.max_speed_mps {
                return Err(ValidationError::SpeedAboveMaximum {
                    speed_mps,
                    max_mps: limits.max_speed_mps,
                });
            }
        }
        if let Some(b) = limits.boundary {
            let distance_m = geographic::distance(b.center, sample.coordinate)?;
            if distance_m > b.radius_m {
                return Err(ValidationError::OutsideBoundary {
                    distance_m,
                    radius_m: b.radius_m,
                });
            }
        }
        if let Some((previous_time, previous_coordinate)) = self.last_accepted {
            let dt = sample.timestamp.seconds_since(previous_time);
            let limit_m = limits.max_displacement_rate_mps * dt;
            let distance_m = geographic::distance(previous_coordinate, sample.coordinate)?;
            if distance_m > limit_m {
                return Err(ValidationError::ImpossibleDisplacement {
                    distance_m,
                    limit_m,
                });
            }
        }
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
    use crate::domain::{LocationSource, SimulationState};

    fn origin() -> Coordinate {
        Coordinate::new(12.9352, 77.6245).unwrap()
    }

    fn sample(nanos: i64) -> SyntheticLocation {
        SyntheticLocation {
            timestamp: Timestamp::from_nanos(nanos),
            coordinate: origin(),
            altitude_m: 920.0,
            horizontal_accuracy_m: 5.0,
            vertical_accuracy_m: 8.0,
            speed_mps: Some(0.0),
            course_deg: None,
            source: LocationSource::Simulation,
            simulation_state: SimulationState::Running,
        }
    }

    const SEC: i64 = 1_000_000_000;

    fn at(seconds: i64, bearing: f64, metres: f64) -> SyntheticLocation {
        SyntheticLocation {
            coordinate: geographic::destination(origin(), bearing, metres).unwrap(),
            ..sample(seconds * SEC)
        }
    }

    fn limited() -> SampleValidator {
        SampleValidator::with_limits(SampleLimits {
            max_speed_mps: 2.0,
            max_displacement_rate_mps: 3.0,
            boundary: Some(Boundary {
                center: origin(),
                radius_m: 100.0,
            }),
        })
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

    #[test]
    fn without_limits_any_displacement_passes() {
        let mut v = SampleValidator::new();
        v.validate(&at(0, 0.0, 0.0)).unwrap();
        assert_eq!(v.validate(&at(1, 0.0, 50_000.0)), Ok(()));
    }

    #[test]
    fn speed_limit_is_inclusive() {
        let mut v = limited();
        let ok = SyntheticLocation {
            speed_mps: Some(2.0),
            course_deg: Some(10.0),
            ..sample(0)
        };
        assert_eq!(v.validate(&ok), Ok(()));
        let fast = SyntheticLocation {
            speed_mps: Some(2.000001),
            course_deg: Some(10.0),
            ..sample(SEC)
        };
        assert_eq!(
            v.validate(&fast),
            Err(ValidationError::SpeedAboveMaximum {
                speed_mps: 2.000001,
                max_mps: 2.0
            })
        );
        // Unknown speed is not a violation.
        let unknown = SyntheticLocation {
            speed_mps: None,
            ..sample(SEC)
        };
        assert_eq!(v.validate(&unknown), Ok(()));
    }

    #[test]
    fn boundary_is_enforced() {
        let mut v = limited();
        assert_eq!(v.validate(&at(0, 90.0, 99.999)), Ok(()));
        match v.validate(&at(1, 90.0, 100.001)) {
            Err(ValidationError::OutsideBoundary {
                distance_m,
                radius_m,
            }) => {
                assert!((distance_m - 100.001).abs() < 1e-6);
                assert_eq!(radius_m, 100.0);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn displacement_limit_scales_with_elapsed_time() {
        let mut v = limited();
        v.validate(&at(0, 0.0, 0.0)).unwrap();
        // 3 m/s for 2 s allows 6 m.
        assert_eq!(v.validate(&at(2, 0.0, 5.999)), Ok(()));
        // From 5.999 m, one more second allows up to 8.999 m.
        match v.validate(&at(3, 0.0, 9.1)) {
            Err(ValidationError::ImpossibleDisplacement {
                distance_m,
                limit_m,
            }) => {
                assert!((distance_m - 3.101).abs() < 1e-6);
                assert_eq!(limit_m, 3.0);
            }
            other => panic!("unexpected {other:?}"),
        }
        // The rejected sample did not become the reference point.
        assert_eq!(v.validate(&at(3, 0.0, 8.9)), Ok(()));
    }

    #[test]
    fn zero_rate_requires_an_exactly_stationary_stream() {
        let mut v = SampleValidator::with_limits(SampleLimits {
            max_speed_mps: 0.0,
            max_displacement_rate_mps: 0.0,
            boundary: None,
        });
        v.validate(&at(0, 0.0, 0.0)).unwrap();
        assert_eq!(v.validate(&at(1, 0.0, 0.0)), Ok(()));
        assert!(matches!(
            v.validate(&at(2, 0.0, 0.001)),
            Err(ValidationError::ImpossibleDisplacement { .. })
        ));
    }
}
