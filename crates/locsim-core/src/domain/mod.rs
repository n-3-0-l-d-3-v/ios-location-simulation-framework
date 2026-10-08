//! Canonical, Core Location–independent domain model.

mod error;
mod location;
mod params;
mod route;
mod scenario;
mod state;
mod time;

pub use crate::geographic::Coordinate;
pub use error::{ConfigError, InvalidTransition, LocationError};
pub use location::{LocationSource, SyntheticLocation};
pub use params::{
    MovementMode, MovementParameters, NoiseParameters, RotationDirection, KINEMATIC_MARGIN,
    NOISE_CLIP_SIGMA,
};
pub use route::{Route, RouteError, RouteLeg, RoutePoint};
pub use scenario::{Boundary, PlaybackParameters, Scenario, SpeedRange, CURRENT_SCHEMA_VERSION};
pub use state::{HealthState, SimulationState};
pub use time::Timestamp;
