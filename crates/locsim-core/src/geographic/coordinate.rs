use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CoordinateError {
    NonFinite,
    LatitudeOutOfRange(f64),
    LongitudeOutOfRange(f64),
}

impl fmt::Display for CoordinateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoordinateError::NonFinite => write!(f, "latitude/longitude must be finite"),
            CoordinateError::LatitudeOutOfRange(v) => {
                write!(f, "latitude {v} outside [-90, 90]")
            }
            CoordinateError::LongitudeOutOfRange(v) => {
                write!(f, "longitude {v} outside [-180, 180]")
            }
        }
    }
}

impl std::error::Error for CoordinateError {}

/// A validated WGS84 geodetic position in degrees.
///
/// Fields are private: a `Coordinate` that exists is always finite with
/// latitude in `[-90, 90]` and longitude in `[-180, 180]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinate {
    latitude: f64,
    longitude: f64,
}

impl Coordinate {
    /// Strict constructor: rejects anything out of range, never adjusts.
    pub fn new(latitude: f64, longitude: f64) -> Result<Self, CoordinateError> {
        if !latitude.is_finite() || !longitude.is_finite() {
            return Err(CoordinateError::NonFinite);
        }
        if !(-90.0..=90.0).contains(&latitude) {
            return Err(CoordinateError::LatitudeOutOfRange(latitude));
        }
        if !(-180.0..=180.0).contains(&longitude) {
            return Err(CoordinateError::LongitudeOutOfRange(longitude));
        }
        Ok(Self {
            latitude,
            longitude,
        })
    }

    /// Like [`Coordinate::new`] but wraps longitude into `[-180, 180]`.
    /// Latitude is still validated strictly (it cannot be wrapped without
    /// also changing longitude, which callers must do explicitly).
    pub fn new_wrapping_longitude(latitude: f64, longitude: f64) -> Result<Self, CoordinateError> {
        if !longitude.is_finite() {
            return Err(CoordinateError::NonFinite);
        }
        Self::new(latitude, normalize_longitude(longitude))
    }

    pub fn latitude(&self) -> f64 {
        self.latitude
    }

    pub fn longitude(&self) -> f64 {
        self.longitude
    }
}

/// Wraps a finite longitude in degrees into `[-180, 180]`.
pub fn normalize_longitude(longitude: f64) -> f64 {
    if (-180.0..=180.0).contains(&longitude) {
        return longitude;
    }
    (longitude + 180.0).rem_euclid(360.0) - 180.0
}

/// Wraps a finite bearing in degrees into `[0, 360)`.
pub fn normalize_bearing(bearing: f64) -> f64 {
    let r = bearing.rem_euclid(360.0);
    // rem_euclid can round up to exactly 360.0 for tiny negative inputs.
    if r >= 360.0 {
        0.0
    } else {
        r
    }
}

/// Signed smallest rotation from `from` to `to`, in `(-180, 180]` degrees.
/// Positive is clockwise.
pub fn bearing_difference(from: f64, to: f64) -> f64 {
    let d = normalize_bearing(to - from);
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_boundaries() {
        for (lat, lon) in [
            (90.0, 180.0),
            (-90.0, -180.0),
            (0.0, 0.0),
            (90.0, 0.0),
            (-0.0, -0.0),
        ] {
            let c = Coordinate::new(lat, lon).unwrap();
            assert_eq!(c.latitude(), lat);
            assert_eq!(c.longitude(), lon);
        }
    }

    #[test]
    fn rejects_out_of_range_and_non_finite() {
        assert_eq!(
            Coordinate::new(90.000001, 0.0),
            Err(CoordinateError::LatitudeOutOfRange(90.000001))
        );
        assert_eq!(
            Coordinate::new(0.0, -180.5),
            Err(CoordinateError::LongitudeOutOfRange(-180.5))
        );
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(Coordinate::new(bad, 0.0), Err(CoordinateError::NonFinite));
            assert_eq!(Coordinate::new(0.0, bad), Err(CoordinateError::NonFinite));
            assert_eq!(
                Coordinate::new_wrapping_longitude(0.0, bad),
                Err(CoordinateError::NonFinite)
            );
        }
    }

    #[test]
    fn longitude_wrapping() {
        assert_eq!(normalize_longitude(190.0), -170.0);
        assert_eq!(normalize_longitude(-190.0), 170.0);
        assert_eq!(normalize_longitude(540.0), -180.0);
        assert_eq!(normalize_longitude(360.0), 0.0);
        assert_eq!(normalize_longitude(179.5), 179.5);
        let c = Coordinate::new_wrapping_longitude(10.0, 365.0).unwrap();
        assert!((c.longitude() - 5.0).abs() < 1e-12);
        assert!(Coordinate::new_wrapping_longitude(91.0, 0.0).is_err());
    }

    #[test]
    fn bearing_normalization() {
        assert_eq!(normalize_bearing(360.0), 0.0);
        assert_eq!(normalize_bearing(-90.0), 270.0);
        assert_eq!(normalize_bearing(725.0), 5.0);
        let tiny = normalize_bearing(-1e-17);
        assert!((0.0..360.0).contains(&tiny));
    }

    #[test]
    fn bearing_difference_is_shortest_signed_rotation() {
        assert_eq!(bearing_difference(350.0, 10.0), 20.0);
        assert_eq!(bearing_difference(10.0, 350.0), -20.0);
        assert_eq!(bearing_difference(0.0, 180.0), 180.0);
        assert_eq!(bearing_difference(90.0, 90.0), 0.0);
    }
}
