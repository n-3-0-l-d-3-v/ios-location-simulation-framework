//! WGS84 ellipsoid defining constants and derived quantities.

/// Semi-major axis (m).
pub const A: f64 = 6_378_137.0;
/// Flattening.
pub const F: f64 = 1.0 / 298.257_223_563;
/// Semi-minor axis (m).
pub const B: f64 = A * (1.0 - F);
/// First eccentricity squared.
pub const E2: f64 = F * (2.0 - F);
/// Smallest radius of curvature anywhere on the ellipsoid (meridional, at
/// the equator), in metres. Dividing a length by it bounds the angle it
/// subtends.
pub const MIN_CURVATURE_RADIUS: f64 = B * B / A;
