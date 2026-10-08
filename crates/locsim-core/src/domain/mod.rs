//! Canonical, Core Location–independent domain model.

mod error;
mod location;
mod params;
mod scenario;
mod state;
mod time;

pub use crate::geographic::Coordinate;
pub use error::{ConfigError, InvalidTransition, LocationError};
pub use location::{LocationSource, SyntheticLocation};
pub use params::{MovementMode, MovementParameters, NoiseParameters, RotationDirection};
pub use scenario::{
    PlaybackParameters, Route, RoutePoint, Scenario, SpeedRange, CURRENT_SCHEMA_VERSION,
};
pub use state::{HealthState, SimulationState};
pub use time::Timestamp;
