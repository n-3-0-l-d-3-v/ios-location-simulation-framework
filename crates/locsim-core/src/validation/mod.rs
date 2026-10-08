//! Per-sample validation gate. A sample that fails here is never emitted,
//! and nothing may modify a sample after it has passed.
//!
//! Stages, in order:
//! 1. field validity (finite, in range, course only while moving);
//! 2. timestamp strictly after the previous accepted sample;
//! 3. with [`SampleLimits`]: reported speed ≤ maximum, position inside the
//!    scenario boundary, and — relative to the previous accepted sample —
//!    displacement, change of speed (acceleration and deceleration
//!    separately) and change of course (heading rate) all physically
//!    possible in the elapsed time.
//!
//! The gate is independent of the engines upstream: it re-derives every
//! quantity from the emitted samples with exact geodesics and shares no
//! state with them, so a bug in a movement model or in the noise engine
//! cannot leak out.
//!
//! # Tolerances
//!
//! Comparisons are strict; there is no numerical slack. The only allowances
//! are physical and derived from configured noise, never from rounding:
//! measurement noise on speed and heading is clipped at ±3 σ, so two
//! consecutive readings can differ by up to 6 σ beyond the true change, and
//! position noise of reach `m` can change the meridian convergence between
//! two fixes by up to `2·m·tan(lat)/R`. Upstream engines are expected to stay
//! [`KINEMATIC_MARGIN`](crate::domain::KINEMATIC_MARGIN) below their limits.
//!
//! # Turning is measured on the surface
//!
//! A straight (geodesic) path changes its bearing as it goes — by tens of
//! degrees per kilometre near a pole. Comparing raw course values would
//! reject straight travel there, so the previous course is first carried
//! along the geodesic to the new position (adding the meridian convergence)
//! and only the remaining difference counts as turning.

use crate::domain::{
    Boundary, LocationError, Scenario, SyntheticLocation, Timestamp, NOISE_CLIP_SIGMA,
};
use crate::geographic::{self, bearing_difference, wgs84, GeoError};
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
    /// Speed rose faster than the acceleration limit allows.
    AccelerationExceeded {
        change_mps: f64,
        limit_mps: f64,
    },
    /// Speed fell faster than the deceleration limit allows.
    DecelerationExceeded {
        change_mps: f64,
        limit_mps: f64,
    },
    /// Course turned further than the heading-rate limit allows.
    HeadingRateExceeded {
        turn_deg: f64,
        limit_deg: f64,
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
            ValidationError::AccelerationExceeded {
                change_mps,
                limit_mps,
            } => write!(
                f,
                "speed rose by {change_mps} m/s since the previous sample, limit is {limit_mps} m/s"
            ),
            ValidationError::DecelerationExceeded {
                change_mps,
                limit_mps,
            } => write!(
                f,
                "speed fell by {change_mps} m/s since the previous sample, limit is {limit_mps} m/s"
            ),
            ValidationError::HeadingRateExceeded {
                turn_deg,
                limit_deg,
            } => write!(
                f,
                "course turned {turn_deg} deg since the previous sample, limit is {limit_deg} deg"
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
    pub max_acceleration_mps2: f64,
    pub max_deceleration_mps2: f64,
    pub max_heading_rate_dps: f64,
    /// Extra change between two speed readings attributable to noise.
    pub speed_noise_allowance_mps: f64,
    /// Extra change between two course readings attributable to noise.
    pub heading_noise_allowance_deg: f64,
    /// Largest distance position noise can move a fix (0 without noise).
    pub position_noise_reach_m: f64,
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
        let position_noise_reach_m = if scenario.noise.has_position_noise() {
            scenario.noise.max_position_offset_m
        } else {
            0.0
        };
        Self {
            max_speed_mps: scenario.movement.max_speed_mps,
            max_displacement_rate_mps: scenario.effective_max_speed_mps() + noise_rate,
            boundary: scenario.boundary(),
            max_acceleration_mps2: scenario.movement.max_acceleration_mps2,
            max_deceleration_mps2: scenario.movement.max_deceleration_mps2,
            max_heading_rate_dps: scenario.movement.max_heading_rate_dps,
            // Each reading is within ±3 σ of the truth, so two differ by ≤ 6 σ.
            speed_noise_allowance_mps: 2.0 * NOISE_CLIP_SIGMA * scenario.noise.speed_noise_mps,
            heading_noise_allowance_deg: 2.0 * NOISE_CLIP_SIGMA * scenario.noise.heading_noise_deg,
            position_noise_reach_m,
        }
    }

    /// How much position noise can alter the meridian convergence between
    /// two fixes at up to `latitude_deg`: each fix may be displaced by the
    /// noise reach, shifting the longitude difference by up to
    /// `2·reach / (R·cos φ)`, and convergence is that times `sin φ`. The
    /// latitude is first pushed poleward by the reach itself. Unbounded at
    /// the poles, where the check is then skipped.
    fn convergence_allowance_deg(&self, latitude_deg: f64) -> f64 {
        if self.position_noise_reach_m <= 0.0 {
            return 0.0;
        }
        let angle = self.position_noise_reach_m / wgs84::MIN_CURVATURE_RADIUS;
        let latitude = (latitude_deg.abs().to_radians() + angle).min(std::f64::consts::FRAC_PI_2);
        (2.0 * angle * latitude.tan()).to_degrees()
    }
}

/// Stateful validator for one sample stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SampleValidator {
    limits: Option<SampleLimits>,
    last_accepted: Option<SyntheticLocation>,
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
        if let Some(previous) = self.last_accepted {
            if sample.timestamp <= previous.timestamp {
                return Err(ValidationError::NonMonotonicTimestamp {
                    previous: previous.timestamp,
                    current: sample.timestamp,
                });
            }
        }
        if let Some(limits) = self.limits {
            self.check_limits(&limits, sample)?;
        }
        self.last_accepted = Some(*sample);
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
        let Some(previous) = self.last_accepted else {
            return Ok(());
        };
        let dt = sample.timestamp.seconds_since(previous.timestamp);
        let line = geographic::inverse(previous.coordinate, sample.coordinate)?;

        // Teleportation.
        let limit_m = limits.max_displacement_rate_mps * dt;
        if line.distance_m > limit_m {
            return Err(ValidationError::ImpossibleDisplacement {
                distance_m: line.distance_m,
                limit_m,
            });
        }

        // Instantaneous acceleration or braking.
        if let (Some(before), Some(after)) = (previous.speed_mps, sample.speed_mps) {
            let change_mps = after - before;
            let limit_mps = limits.max_acceleration_mps2 * dt + limits.speed_noise_allowance_mps;
            if change_mps > limit_mps {
                return Err(ValidationError::AccelerationExceeded {
                    change_mps,
                    limit_mps,
                });
            }
            let limit_mps = limits.max_deceleration_mps2 * dt + limits.speed_noise_allowance_mps;
            if -change_mps > limit_mps {
                return Err(ValidationError::DecelerationExceeded {
                    change_mps: -change_mps,
                    limit_mps,
                });
            }
        }

        // Instantaneous heading change. Only defined while moving at both
        // samples; a limit of half a turn or more cannot be violated.
        if let (Some(before), Some(after)) = (previous.course_deg, sample.course_deg) {
            let latitude = previous
                .coordinate
                .latitude()
                .abs()
                .max(sample.coordinate.latitude().abs());
            let limit_deg = limits.max_heading_rate_dps * dt
                + limits.heading_noise_allowance_deg
                + limits.convergence_allowance_deg(latitude);
            if limit_deg < 180.0 {
                let carried = before + line.convergence_deg;
                let turn_deg = bearing_difference(carried, after).abs();
                if turn_deg > limit_deg {
                    return Err(ValidationError::HeadingRateExceeded {
                        turn_deg,
                        limit_deg,
                    });
                }
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
    use crate::domain::{Coordinate, LocationSource, SimulationState};

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

    /// Generous kinematic limits, so each test tightens only what it probes.
    const LOOSE: SampleLimits = SampleLimits {
        max_speed_mps: 2.0,
        max_displacement_rate_mps: 3.0,
        boundary: None,
        max_acceleration_mps2: 1e9,
        max_deceleration_mps2: 1e9,
        max_heading_rate_dps: 1e9,
        speed_noise_allowance_mps: 0.0,
        heading_noise_allowance_deg: 0.0,
        position_noise_reach_m: 0.0,
    };

    fn limited() -> SampleValidator {
        SampleValidator::with_limits(SampleLimits {
            boundary: Some(Boundary {
                center: origin(),
                radius_m: 100.0,
            }),
            ..LOOSE
        })
    }

    /// A moving sample `metres` along `bearing` from the origin.
    fn moving(
        seconds: i64,
        bearing: f64,
        metres: f64,
        speed: f64,
        course: f64,
    ) -> SyntheticLocation {
        SyntheticLocation {
            speed_mps: Some(speed),
            course_deg: Some(course),
            ..at(seconds, bearing, metres)
        }
    }

    #[test]
    fn acceleration_and_deceleration_limits_are_separate() {
        let limits = SampleLimits {
            max_speed_mps: 50.0,
            max_displacement_rate_mps: 50.0,
            max_acceleration_mps2: 1.0,
            max_deceleration_mps2: 3.0,
            ..LOOSE
        };
        let mut v = SampleValidator::with_limits(limits);
        v.validate(&moving(0, 0.0, 0.0, 10.0, 0.0)).unwrap();
        // +2 m/s over 2 s is exactly the limit: accepted.
        assert_eq!(v.validate(&moving(2, 0.0, 20.0, 12.0, 0.0)), Ok(()));
        // +1.5 m/s in 1 s is not.
        assert_eq!(
            v.validate(&moving(3, 0.0, 30.0, 13.5, 0.0)),
            Err(ValidationError::AccelerationExceeded {
                change_mps: 1.5,
                limit_mps: 1.0
            })
        );
        // Braking may be harder: −3 m/s in 1 s passes, −3.5 does not.
        assert_eq!(v.validate(&moving(3, 0.0, 30.0, 9.0, 0.0)), Ok(()));
        assert_eq!(
            v.validate(&moving(4, 0.0, 40.0, 5.5, 0.0)),
            Err(ValidationError::DecelerationExceeded {
                change_mps: 3.5,
                limit_mps: 3.0
            })
        );
        // A standing start to full speed in one sample is caught too.
        let mut v = SampleValidator::with_limits(limits);
        v.validate(&at(0, 0.0, 0.0)).unwrap();
        assert!(matches!(
            v.validate(&moving(1, 0.0, 1.0, 30.0, 0.0)),
            Err(ValidationError::AccelerationExceeded { .. })
        ));
    }

    #[test]
    fn unknown_speed_or_course_skips_the_rate_checks() {
        let limits = SampleLimits {
            max_acceleration_mps2: 0.0,
            max_deceleration_mps2: 0.0,
            max_heading_rate_dps: 0.0,
            ..LOOSE
        };
        let mut v = SampleValidator::with_limits(limits);
        v.validate(&moving(0, 0.0, 0.0, 1.0, 10.0)).unwrap();
        let unknown = SyntheticLocation {
            speed_mps: None,
            course_deg: None,
            ..at(1, 0.0, 1.0)
        };
        assert_eq!(v.validate(&unknown), Ok(()));
        assert_eq!(v.validate(&moving(2, 0.0, 2.0, 2.0, 200.0)), Ok(()));
    }

    #[test]
    fn heading_rate_limit_is_enforced() {
        let limits = SampleLimits {
            max_heading_rate_dps: 10.0,
            ..LOOSE
        };
        let mut v = SampleValidator::with_limits(limits);
        v.validate(&moving(0, 0.0, 0.0, 1.0, 350.0)).unwrap();
        // 350° → 10° is a 20° turn through north: allowed in 2 s.
        assert_eq!(v.validate(&moving(2, 0.0, 2.0, 1.0, 9.99)), Ok(()));
        match v.validate(&moving(3, 0.0, 3.0, 1.0, 25.0)) {
            Err(ValidationError::HeadingRateExceeded {
                turn_deg,
                limit_deg,
            }) => {
                assert!((turn_deg - 15.01).abs() < 1e-6, "{turn_deg}");
                assert_eq!(limit_deg, 10.0);
            }
            other => panic!("unexpected {other:?}"),
        }
        // An about-turn in one sample is the classic discontinuity.
        assert!(matches!(
            v.validate(&moving(3, 0.0, 3.0, 1.0, 189.0)),
            Err(ValidationError::HeadingRateExceeded { .. })
        ));
        // With a limit of ≥ 180° per step nothing can be a violation.
        assert_eq!(v.validate(&moving(30, 0.0, 3.0, 1.0, 189.0)), Ok(()));
    }

    #[test]
    fn straight_travel_near_a_pole_is_not_mistaken_for_turning() {
        // Drive 600 m along one geodesic 1 km from the north pole, in 100 m
        // steps. The bearing swings by tens of degrees, yet nothing turns.
        let limits = SampleLimits {
            max_speed_mps: 200.0,
            max_displacement_rate_mps: 200.0,
            max_heading_rate_dps: 0.001,
            ..LOOSE
        };
        let mut v = SampleValidator::with_limits(limits);
        let mut position = Coordinate::new(89.991, 10.0).unwrap();
        let mut course = 90.0;
        let mut swing = 0.0f64;
        for step in 0..7 {
            let sample = SyntheticLocation {
                coordinate: position,
                speed_mps: Some(100.0),
                course_deg: Some(course),
                ..sample(step * SEC)
            };
            assert_eq!(v.validate(&sample), Ok(()), "step {step}");
            let (next, arrival) = geographic::direct(position, course, 100.0).unwrap();
            swing += bearing_difference(course, arrival).abs();
            position = next;
            course = arrival;
        }
        assert!(swing > 30.0, "bearing only swung {swing} degrees");

        // A real 5° turn on top of the same geometry is still caught.
        let (next, arrival) = geographic::direct(position, course, 100.0).unwrap();
        let turned = SyntheticLocation {
            coordinate: next,
            speed_mps: Some(100.0),
            course_deg: Some(geographic::normalize_bearing(arrival + 5.0)),
            ..sample(8 * SEC)
        };
        match v.validate(&turned) {
            Err(ValidationError::HeadingRateExceeded { turn_deg, .. }) => {
                assert!((turn_deg - 5.0).abs() < 1e-6, "{turn_deg}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn noise_allowances_widen_the_rate_limits_by_exactly_their_amount() {
        let limits = SampleLimits {
            max_speed_mps: 50.0,
            max_displacement_rate_mps: 50.0,
            max_acceleration_mps2: 1.0,
            max_deceleration_mps2: 1.0,
            max_heading_rate_dps: 10.0,
            speed_noise_allowance_mps: 0.5,
            heading_noise_allowance_deg: 4.0,
            ..LOOSE
        };
        let mut v = SampleValidator::with_limits(limits);
        v.validate(&moving(0, 0.0, 0.0, 10.0, 100.0)).unwrap();
        assert_eq!(v.validate(&moving(1, 0.0, 10.0, 11.5, 114.0)), Ok(()));
        assert!(matches!(
            v.validate(&moving(2, 0.0, 20.0, 13.1, 114.0)),
            Err(ValidationError::AccelerationExceeded { .. })
        ));
        assert!(matches!(
            v.validate(&moving(2, 0.0, 20.0, 11.5, 128.1)),
            Err(ValidationError::HeadingRateExceeded { .. })
        ));
    }

    #[test]
    fn position_noise_allowance_grows_towards_the_poles() {
        let limits = SampleLimits {
            position_noise_reach_m: 10.0,
            ..LOOSE
        };
        assert_eq!(LOOSE.convergence_allowance_deg(80.0), 0.0);
        let equator = limits.convergence_allowance_deg(0.0);
        let mid = limits.convergence_allowance_deg(45.0);
        let high = limits.convergence_allowance_deg(89.9);
        assert!(equator < 1e-9, "{equator}");
        // 2 · 10 m / 6 335 km · tan 45° ≈ 1.8e-4°.
        assert!((mid - 1.809e-4).abs() < 1e-6, "{mid}");
        assert!(high > 0.1 && high < 0.11, "{high}");
        assert!(limits.convergence_allowance_deg(90.0) > 180.0);
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
            ..LOOSE
        });
        v.validate(&at(0, 0.0, 0.0)).unwrap();
        assert_eq!(v.validate(&at(1, 0.0, 0.0)), Ok(()));
        assert!(matches!(
            v.validate(&at(2, 0.0, 0.001)),
            Err(ValidationError::ImpossibleDisplacement { .. })
        ));
    }
}
