//! Movement engine: pluggable models that turn simulated time into a
//! coherent kinematic state.
//!
//! A model knows nothing about scheduling, noise, validation or delivery, and
//! it is not trusted: everything it emits passes the independent validation
//! gate. Models keep [`KINEMATIC_MARGIN`](crate::domain::KINEMATIC_MARGIN)
//! below their limits so that the gate's strict comparisons, made with
//! different arithmetic, are never tripped by rounding.
//!
//! | Mode | Model | Nature |
//! |---|---|---|
//! | Fixed | [`FixedModel`] | stationary |
//! | Circular | [`CircularModel`] | closed form in time, no accumulated state |
//! | RouteReplay | — | not implemented (T06), reported as unsupported |
//!
//! All geometry uses the `geographic` primitives (geodesic direct/inverse);
//! no model does arithmetic on latitude/longitude.

mod circular;
mod fixed;

pub use circular::CircularModel;
pub use fixed::FixedModel;

use crate::domain::{Coordinate, MovementMode, Scenario, Timestamp};
use crate::geographic::GeoError;
use std::fmt;

/// Noise-free kinematic state at one instant, as handed to the noise engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementSample {
    pub coordinate: Coordinate,
    pub altitude_m: f64,
    pub speed_mps: Option<f64>,
    pub course_deg: Option<f64>,
}

/// Full trajectory state of a model at its latest sample. Unlike
/// [`MovementSample`] it always carries a heading (a stopped mover still
/// faces somewhere) and the rates of change that produced the sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kinematics {
    pub position: Coordinate,
    pub speed_mps: f64,
    /// Direction the mover faces, degrees clockwise from north, `[0, 360)`.
    pub heading_deg: f64,
    /// Mean rate of change of speed over the last step (negative = braking).
    pub acceleration_mps2: f64,
    /// Mean turn rate over the last step, positive clockwise. Turning
    /// relative to the path, i.e. excluding meridian convergence.
    pub heading_rate_dps: f64,
}

impl Kinematics {
    /// East and north components of the velocity, m/s.
    pub fn velocity_en_mps(&self) -> (f64, f64) {
        let (sin, cos) = self.heading_deg.to_radians().sin_cos();
        (self.speed_mps * sin, self.speed_mps * cos)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MovementError {
    Geo(GeoError),
    /// The mode is valid but this build has no model for it yet.
    UnsupportedMode(MovementMode),
    /// A model was asked for a time not after its previous sample.
    NonIncreasingTime {
        previous: Timestamp,
        current: Timestamp,
    },
    /// The scenario lacks something the model needs (it was not validated).
    InvalidConfiguration(&'static str),
}

impl fmt::Display for MovementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MovementError::Geo(e) => write!(f, "movement geometry failed: {e}"),
            MovementError::UnsupportedMode(m) => {
                write!(f, "movement mode {m:?} is not implemented yet")
            }
            MovementError::NonIncreasingTime { previous, current } => write!(
                f,
                "movement sampled at {} ns, not after previous {} ns",
                current.as_nanos(),
                previous.as_nanos()
            ),
            MovementError::InvalidConfiguration(what) => {
                write!(f, "movement configuration is incomplete: {what}")
            }
        }
    }
}

impl std::error::Error for MovementError {}

impl From<GeoError> for MovementError {
    fn from(e: GeoError) -> Self {
        MovementError::Geo(e)
    }
}

pub trait MovementModel {
    /// State at simulated time `t`. Called with strictly increasing times
    /// within a run; the first call defines the start of the trajectory.
    fn sample_at(&mut self, t: Timestamp) -> Result<MovementSample, MovementError>;

    /// Trajectory state behind the latest sample, where the model has one.
    fn kinematics(&self) -> Option<Kinematics> {
        None
    }
}

/// Builds the model for an already validated scenario.
pub fn model_for(scenario: &Scenario) -> Result<Box<dyn MovementModel + Send>, MovementError> {
    match scenario.mode {
        MovementMode::Fixed => Ok(Box::new(FixedModel::new(
            scenario.origin,
            scenario.altitude_m,
        ))),
        MovementMode::Circular => Ok(Box::new(CircularModel::for_scenario(scenario)?)),
        other => Err(MovementError::UnsupportedMode(other)),
    }
}
