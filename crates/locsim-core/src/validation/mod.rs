//! Per-sample validation gate. A sample that fails here is never emitted,
//! and nothing may modify a sample after it has passed.
//!
//! Stages, in order:
//! 1. **field validity** — finite, in range, course only while moving;
//! 2. **timestamp** strictly after the previous accepted sample;
//! 3. with [`SampleLimits`]:
//!    * **boundary** — inside the scenario fence;
//!    * **displacement** — no further from the previous fix than speed and
//!      position noise allow in the elapsed time (teleportation);
//!    * **consistency** — speed and course are what the positions and
//!      timestamps say (see [`crate::consistency`]);
//!    * **speed** — the reported speed is physically possible;
//!    * **acceleration / deceleration** and **heading rate** — the change of
//!      speed and course since the previous sample is physically possible.
//!
//! The gate is independent of the engines upstream: it re-derives every
//! quantity from the emitted samples with exact geodesics and shares no
//! state with them, so a bug in a movement model, the noise engine or the
//! consistency engine cannot leak out.
//!
//! # What speed and course mean here
//!
//! Since the consistency engine, a sample's speed is the **mean ground speed
//! over the interval that ends at it** and its course the **direction of the
//! geodesic chord over that interval** (backward differences of the emitted
//! positions). The kinematic stages are stated for exactly those quantities.
//! With `dt` the interval ending at the sample and `dt′` the one before it:
//!
//! * Two consecutive interval means of a speed whose rate of change is at
//!   most `a` differ by at most `a · (dt + dt′)/2` — the time between the
//!   interval midpoints. That is the acceleration (and, with the
//!   deceleration limit, the braking) bound. For evenly spaced samples it is
//!   `a · dt`, as before; for uneven ones `a · dt` would be wrong in both
//!   directions.
//! * A chord's direction lies within the range of directions travelled
//!   during its interval, so two consecutive chords differ by at most the
//!   turning possible over both intervals, `ω · (dt + dt′)`.
//!
//! # Allowances
//!
//! Comparisons are strict. Every amount added to a limit is physical or a
//! stated resolution, derived from configuration, never a free tolerance:
//!
//! * **Chords are shorter than paths.** A path that turns by `θ` within an
//!   interval has a chord no shorter than `cos(θ/2)` of its length, so the
//!   chord speed can drop by up to `v_max · (1 − cos(ω·dt/2))` without any
//!   braking. Added to the acceleration and deceleration bounds.
//! * **Position noise** moving at up to `r` adds up to `r` to each derived
//!   speed (so `2r` to their difference), and can deflect a chord of length
//!   `d` by up to `asin(r·dt / (d − r·dt))` — any direction at all once the
//!   noise step is half the chord.
//!   The noise engine limits how fast its offset changes except where the
//!   fence forces more: an output that would leave the fence is pulled back
//!   onto it. A fix *held at the fence* therefore has a noise step that is
//!   not bounded by `r·dt`, and the speed-change and heading stages are
//!   skipped for the two comparisons it takes part in. (Only with position
//!   noise and a fence; position, displacement and consistency still apply.)
//! * **Observation noise** on speed and course, clipped at ±3 σ: 6 σ between
//!   two readings.
//! * **Coordinate resolution.** Fixes are stored to ~2 nm, so a derived
//!   speed is known to `4 nm / dt` and a derived course to `4 nm / d` rad
//!   ([`crate::consistency::speed_resolution_mps`],
//!   [`crate::consistency::course_resolution_deg`]). Negligible at ordinary
//!   rates (4e-9 m/s at 1 Hz), dominant at kilohertz sampling.
//!
//! # Turning is measured on the surface
//!
//! A straight (geodesic) path changes its bearing as it goes — by tens of
//! degrees per kilometre near a pole. The previous course is therefore
//! carried along the geodesic to the new position (adding the meridian
//! convergence) before it is compared; only the remainder counts as turning.

use crate::consistency::{
    self, course_resolution_deg, speed_resolution_mps, ConsistencyError, ConsistencyTolerance,
};
use crate::domain::{
    Boundary, LocationError, Scenario, SyntheticLocation, Timestamp, KINEMATIC_MARGIN,
    NOISE_CLIP_SIGMA,
};
use crate::geographic::{self, bearing_difference, GeoError};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValidationError {
    Location(LocationError),
    /// Timestamp did not strictly increase over the last accepted sample.
    NonMonotonicTimestamp {
        previous: Timestamp,
        current: Timestamp,
    },
    /// The reported speed is higher than motion plus noise can produce.
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
    /// Speed or course contradict the positions and timestamps.
    Inconsistent(ConsistencyError),
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
            ValidationError::Inconsistent(e) => write!(f, "inconsistent sample: {e}"),
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

impl From<ConsistencyError> for ValidationError {
    fn from(e: ConsistencyError) -> Self {
        ValidationError::Inconsistent(e)
    }
}

/// Scenario-derived limits enforced on the final output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SampleLimits {
    /// Fastest the underlying motion may go (after any displacement cap).
    pub max_speed_mps: f64,
    /// Fastest position noise may move a fix; 0 without position noise.
    pub position_noise_rate_mps: f64,
    pub boundary: Option<Boundary>,
    pub max_acceleration_mps2: f64,
    pub max_deceleration_mps2: f64,
    pub max_heading_rate_dps: f64,
    /// 3 σ of observation noise on the reported speed.
    pub speed_noise_reach_mps: f64,
    /// 3 σ of observation noise on the reported course.
    pub heading_noise_reach_deg: f64,
    /// Metadata consistency contract; `None` skips that stage.
    pub consistency: Option<ConsistencyTolerance>,
}

impl SampleLimits {
    pub fn for_scenario(scenario: &Scenario) -> Self {
        let noise = &scenario.noise;
        Self {
            max_speed_mps: scenario.effective_max_speed_mps(),
            position_noise_rate_mps: if noise.has_position_noise() {
                noise.max_offset_rate_mps
            } else {
                0.0
            },
            boundary: scenario.boundary(),
            max_acceleration_mps2: scenario.movement.max_acceleration_mps2,
            max_deceleration_mps2: scenario.movement.max_deceleration_mps2,
            max_heading_rate_dps: scenario.movement.max_heading_rate_dps,
            speed_noise_reach_mps: NOISE_CLIP_SIGMA * noise.speed_noise_mps,
            heading_noise_reach_deg: NOISE_CLIP_SIGMA * noise.heading_noise_deg,
            consistency: Some(ConsistencyTolerance::for_scenario(scenario)),
        }
    }

    /// Fastest an emitted position can move: the motion plus position noise.
    pub fn max_displacement_rate_mps(&self) -> f64 {
        self.max_speed_mps + self.position_noise_rate_mps
    }

    /// How far a path's chord speed can fall below its path speed because
    /// the path turned within an interval of `dt` seconds: a path turning by
    /// `θ ≤ π` has a chord at least `cos(θ/2)` of its length.
    fn chord_shortfall_mps(&self, dt: f64) -> f64 {
        let half_turn = 0.5 * self.max_heading_rate_dps * dt;
        if half_turn >= 90.0 {
            // Half a turn or more: the path can come back on itself.
            return self.max_speed_mps;
        }
        self.max_speed_mps * (1.0 - half_turn.to_radians().cos())
    }

    /// Largest angle by which position noise can deflect a chord of length
    /// `distance_m` covered in `dt` seconds: each end moves by at most the
    /// noise step, so the true chord is at least `d − step` long and the
    /// noise can swing it by `asin(step / (d − step))`. Once the step is
    /// half the chord the direction is unconstrained.
    fn noise_deflection_deg(&self, distance_m: f64, dt: f64) -> f64 {
        let step = self.position_noise_rate_mps * dt;
        if step <= 0.0 {
            0.0
        } else if distance_m <= 2.0 * step {
            180.0
        } else {
            (step / (distance_m - step)).asin().to_degrees()
        }
    }
}

/// The interval that ended at an accepted sample.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Interval {
    elapsed_s: f64,
    distance_m: f64,
}

/// Whether position noise had to hold this fix at the fence (see the module
/// documentation). The noise engine places such a fix one safety margin
/// inside the radius; twice that margin is the test.
fn held_at_fence(limits: &SampleLimits, distance_from_centre_m: Option<f64>) -> bool {
    match (limits.boundary, distance_from_centre_m) {
        (Some(b), Some(d)) if limits.position_noise_rate_mps > 0.0 => {
            b.radius_m - d <= 2.0 * (KINEMATIC_MARGIN * b.radius_m + 1e-7)
        }
        _ => false,
    }
}

/// Stateful validator for one sample stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SampleValidator {
    limits: Option<SampleLimits>,
    last_accepted: Option<SyntheticLocation>,
    last_interval: Option<Interval>,
    last_held_at_fence: bool,
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
            last_interval: None,
            last_held_at_fence: false,
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
        let (interval, held) = match self.limits {
            Some(limits) => self.check_limits(&limits, sample)?,
            None => (None, false),
        };
        self.last_accepted = Some(*sample);
        self.last_interval = interval;
        self.last_held_at_fence = held;
        Ok(())
    }

    fn check_limits(
        &self,
        limits: &SampleLimits,
        sample: &SyntheticLocation,
    ) -> Result<(Option<Interval>, bool), ValidationError> {
        let mut from_centre_m = None;
        if let Some(b) = limits.boundary {
            let distance_m = geographic::distance(b.center, sample.coordinate)?;
            if distance_m > b.radius_m {
                return Err(ValidationError::OutsideBoundary {
                    distance_m,
                    radius_m: b.radius_m,
                });
            }
            from_centre_m = Some(distance_m);
        }
        let held = held_at_fence(limits, from_centre_m);
        let rate = limits.max_displacement_rate_mps();
        // The division `distance / dt` may round one unit in the last place
        // above `rate` for a fix that is exactly `rate × dt` away.
        let max_reported_mps = rate * (1.0 + 4.0 * f64::EPSILON) + limits.speed_noise_reach_mps;
        let check_speed = |speed: Option<f64>| match speed {
            Some(speed_mps) if speed_mps > max_reported_mps => {
                Err(ValidationError::SpeedAboveMaximum {
                    speed_mps,
                    max_mps: max_reported_mps,
                })
            }
            _ => Ok(()),
        };

        let Some(previous) = self.last_accepted else {
            if let Some(tolerance) = limits.consistency {
                consistency::check_first(sample, &tolerance)?;
            }
            check_speed(sample.speed_mps)?;
            return Ok((None, held));
        };
        let dt = sample.timestamp.seconds_since(previous.timestamp);
        let line = geographic::inverse(previous.coordinate, sample.coordinate)?;
        let interval = Interval {
            elapsed_s: dt,
            distance_m: line.distance_m,
        };
        // Without a recorded earlier interval (the previous sample was the
        // first), judge against this one.
        let before = self.last_interval.unwrap_or(interval);

        // Teleportation.
        let limit_m = rate * dt;
        if line.distance_m > limit_m {
            return Err(ValidationError::ImpossibleDisplacement {
                distance_m: line.distance_m,
                limit_m,
            });
        }

        // Metadata must describe these positions and timestamps.
        if let Some(tolerance) = limits.consistency {
            consistency::check_pair_along(&previous, sample, &line, &tolerance)?;
        }
        check_speed(sample.speed_mps)?;

        if held || self.last_held_at_fence {
            // The noise step of this chord or the previous one is unbounded.
            return Ok((Some(interval), held));
        }

        // Instantaneous acceleration or braking.
        if let (Some(earlier), Some(later)) = (previous.speed_mps, sample.speed_mps) {
            let change_mps = later - earlier;
            let between_midpoints = 0.5 * (dt + before.elapsed_s);
            let allowance = limits.chord_shortfall_mps(dt.max(before.elapsed_s))
                + 2.0 * limits.position_noise_rate_mps
                + 2.0 * limits.speed_noise_reach_mps
                + speed_resolution_mps(dt)
                + speed_resolution_mps(before.elapsed_s);
            let limit_mps = limits.max_acceleration_mps2 * between_midpoints + allowance;
            if change_mps > limit_mps {
                return Err(ValidationError::AccelerationExceeded {
                    change_mps,
                    limit_mps,
                });
            }
            let limit_mps = limits.max_deceleration_mps2 * between_midpoints + allowance;
            if -change_mps > limit_mps {
                return Err(ValidationError::DecelerationExceeded {
                    change_mps: -change_mps,
                    limit_mps,
                });
            }
        }

        // Instantaneous heading change. Only defined when both samples have
        // a course; a limit of half a turn or more cannot be violated.
        if let (Some(earlier), Some(later)) = (previous.course_deg, sample.course_deg) {
            let limit_deg = limits.max_heading_rate_dps * (dt + before.elapsed_s)
                + limits.noise_deflection_deg(line.distance_m, dt)
                + limits.noise_deflection_deg(before.distance_m, before.elapsed_s)
                + 2.0 * limits.heading_noise_reach_deg
                + course_resolution_deg(line.distance_m)
                + course_resolution_deg(before.distance_m);
            if limit_deg < 180.0 {
                let carried = earlier + line.convergence_deg;
                let turn_deg = bearing_difference(carried, later).abs();
                if turn_deg > limit_deg {
                    return Err(ValidationError::HeadingRateExceeded {
                        turn_deg,
                        limit_deg,
                    });
                }
            }
        }
        Ok((Some(interval), held))
    }

    /// Forgets stream history; call when a new simulation run starts.
    pub fn reset(&mut self) {
        self.last_accepted = None;
        self.last_interval = None;
        self.last_held_at_fence = false;
    }
}

#[cfg(test)]
mod tests;
