//! Movement engine: pluggable models that turn time into a kinematic state.
//!
//! A model knows nothing about scheduling, noise, validation or delivery.
//! Only the fixed model exists so far; the other modes arrive with T05/T06
//! and are reported as unsupported until then rather than approximated.

mod fixed;

pub use fixed::FixedModel;

use crate::domain::{Coordinate, MovementMode, Scenario, Timestamp};
use crate::geographic::GeoError;
use std::fmt;

/// Noise-free kinematic state at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementSample {
    pub coordinate: Coordinate,
    pub altitude_m: f64,
    pub speed_mps: Option<f64>,
    pub course_deg: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MovementError {
    Geo(GeoError),
    /// The mode is valid but this build has no model for it yet.
    UnsupportedMode(MovementMode),
}

impl fmt::Display for MovementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MovementError::Geo(e) => write!(f, "movement geometry failed: {e}"),
            MovementError::UnsupportedMode(m) => {
                write!(f, "movement mode {m:?} is not implemented yet")
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
    /// State at time `t`. Called with strictly increasing times within a run.
    fn sample_at(&mut self, t: Timestamp) -> Result<MovementSample, MovementError>;
}

/// Builds the model for an already validated scenario.
pub fn model_for(scenario: &Scenario) -> Result<Box<dyn MovementModel + Send>, MovementError> {
    match scenario.mode {
        MovementMode::Fixed => Ok(Box::new(FixedModel::new(
            scenario.origin,
            scenario.altitude_m,
        ))),
        other => Err(MovementError::UnsupportedMode(other)),
    }
}
