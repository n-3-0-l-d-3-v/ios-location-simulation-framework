//! WGS84 geographic engine: coordinate validation, geodesics on the
//! ellipsoid, and local tangent-plane (ENU) conversion.
//!
//! All angles at the public API are in degrees, all lengths in metres.

mod coordinate;
mod enu;
mod geodesic;
pub mod wgs84;

pub use coordinate::{
    bearing_difference, normalize_bearing, normalize_longitude, Coordinate, CoordinateError,
};
pub use enu::{Enu, EnuFrame};
pub use geodesic::{
    bearing, destination, direct, distance, interpolate, inverse, velocity_between, within_radius,
    Geodesic, MAX_GEODESIC_DISTANCE_M,
};

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum GeoError {
    Coordinate(CoordinateError),
    /// An argument was non-finite or outside its documented domain.
    InvalidInput(&'static str),
    /// The geodesic iteration did not converge (nearly antipodal points).
    NotConverged,
}

impl fmt::Display for GeoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeoError::Coordinate(e) => write!(f, "invalid coordinate: {e}"),
            GeoError::InvalidInput(what) => write!(f, "invalid input: {what}"),
            GeoError::NotConverged => {
                write!(f, "geodesic did not converge (nearly antipodal points)")
            }
        }
    }
}

impl std::error::Error for GeoError {}

impl From<CoordinateError> for GeoError {
    fn from(e: CoordinateError) -> Self {
        GeoError::Coordinate(e)
    }
}
