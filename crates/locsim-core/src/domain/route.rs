//! Recorded routes: validated input data for route replay.
//!
//! A route is *data, not authority*. Being a well-formed [`Route`] only means
//! the recording is structurally sound; whether it may be replayed under a
//! scenario's movement limits is decided separately by route admission
//! (`crate::route`), and every replayed sample still passes the final
//! validation gate.
//!
//! # Timing
//!
//! Points carry **elapsed time from the start of the route**, in integer
//! nanoseconds; the first point is at zero. A route therefore has no absolute
//! date: replay places it wherever the simulation starts, which is what makes
//! a replay reproducible. Recordings with absolute timestamps are converted
//! explicitly with [`Route::from_absolute`].

use super::error::ConfigError;
use super::time::Timestamp;
use crate::geographic::{self, Coordinate, GeoError};
use std::fmt;

/// One recorded fix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutePoint {
    /// Nanoseconds since the first point of the route.
    pub elapsed_ns: i64,
    pub coordinate: Coordinate,
    /// Altitude in metres. Either every point of a route has one or none does.
    pub altitude_m: Option<f64>,
}

impl RoutePoint {
    pub fn new(elapsed_ns: i64, coordinate: Coordinate) -> Self {
        Self {
            elapsed_ns,
            coordinate,
            altitude_m: None,
        }
    }

    pub fn with_altitude(self, altitude_m: f64) -> Self {
        Self {
            altitude_m: Some(altitude_m),
            ..self
        }
    }
}

/// Why a list of points is not a route. Indices refer to the input list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RouteError {
    TooFewPoints {
        count: usize,
    },
    /// The first point must be at elapsed time zero.
    FirstPointNotAtZero {
        elapsed_ns: i64,
    },
    /// Point `index` is not strictly later than the point before it. This
    /// covers out-of-order points and zero-duration segments alike.
    NonIncreasingTime {
        index: usize,
        previous_ns: i64,
        elapsed_ns: i64,
    },
    NonFiniteAltitude {
        index: usize,
    },
    /// Point `index` has an altitude where the first point has none, or
    /// the reverse.
    InconsistentAltitude {
        index: usize,
    },
    /// An absolute timestamp could not be expressed relative to the first.
    TimeOutOfRange {
        index: usize,
    },
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouteError::TooFewPoints { count } => {
                write!(f, "a route needs at least 2 points, got {count}")
            }
            RouteError::FirstPointNotAtZero { elapsed_ns } => {
                write!(f, "first point must be at elapsed time 0, got {elapsed_ns} ns")
            }
            RouteError::NonIncreasingTime {
                index,
                previous_ns,
                elapsed_ns,
            } => write!(
                f,
                "point {index} at {elapsed_ns} ns is not after the previous point at {previous_ns} ns"
            ),
            RouteError::NonFiniteAltitude { index } => {
                write!(f, "point {index} has a non-finite altitude")
            }
            RouteError::InconsistentAltitude { index } => write!(
                f,
                "point {index} disagrees with the first point about having an altitude"
            ),
            RouteError::TimeOutOfRange { index } => {
                write!(f, "timestamp of point {index} is out of range")
            }
        }
    }
}

impl std::error::Error for RouteError {}

/// Geometry and timing of the straight (geodesic) leg between two
/// consecutive recorded points, from the geographic engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteLeg {
    /// Index of the leg's first point.
    pub index: usize,
    pub distance_m: f64,
    pub duration_s: f64,
    /// `distance / duration`: the average speed the recording implies.
    pub mean_speed_mps: f64,
    /// Bearing on leaving the first point; `None` for a leg that does not move.
    pub initial_bearing_deg: Option<f64>,
    /// Bearing on reaching the second point.
    pub final_bearing_deg: Option<f64>,
}

/// A structurally valid recording: at least two points, first at time zero,
/// strictly increasing times, finite and consistently present altitudes.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    name: Option<String>,
    points: Vec<RoutePoint>,
}

impl Route {
    /// Validates `points` as given. Nothing is repaired, reordered or dropped.
    pub fn new(points: Vec<RoutePoint>) -> Result<Self, RouteError> {
        if points.len() < 2 {
            return Err(RouteError::TooFewPoints {
                count: points.len(),
            });
        }
        if points[0].elapsed_ns != 0 {
            return Err(RouteError::FirstPointNotAtZero {
                elapsed_ns: points[0].elapsed_ns,
            });
        }
        let with_altitude = points[0].altitude_m.is_some();
        for (index, p) in points.iter().enumerate() {
            match p.altitude_m {
                Some(a) if !a.is_finite() => return Err(RouteError::NonFiniteAltitude { index }),
                a if a.is_some() != with_altitude => {
                    return Err(RouteError::InconsistentAltitude { index })
                }
                _ => {}
            }
            if index > 0 && p.elapsed_ns <= points[index - 1].elapsed_ns {
                return Err(RouteError::NonIncreasingTime {
                    index,
                    previous_ns: points[index - 1].elapsed_ns,
                    elapsed_ns: p.elapsed_ns,
                });
            }
        }
        Ok(Self { name: None, points })
    }

    /// Builds a route from fixes with absolute timestamps. The documented
    /// normalisation is exactly one thing: the first timestamp is subtracted
    /// from all of them. Order and spacing are validated, not corrected.
    pub fn from_absolute(
        fixes: &[(Timestamp, Coordinate, Option<f64>)],
    ) -> Result<Self, RouteError> {
        let Some(first) = fixes.first() else {
            return Err(RouteError::TooFewPoints { count: 0 });
        };
        let mut points = Vec::with_capacity(fixes.len());
        for (index, (t, coordinate, altitude_m)) in fixes.iter().enumerate() {
            let elapsed_ns = t
                .as_nanos()
                .checked_sub(first.0.as_nanos())
                .ok_or(RouteError::TimeOutOfRange { index })?;
            points.push(RoutePoint {
                elapsed_ns,
                coordinate: *coordinate,
                altitude_m: *altitude_m,
            });
        }
        Self::new(points)
    }

    /// Attaches an identifier. Purely descriptive; it has no effect on replay.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn points(&self) -> &[RoutePoint] {
        &self.points
    }

    pub fn has_altitude(&self) -> bool {
        self.points[0].altitude_m.is_some()
    }

    pub fn duration_ns(&self) -> i64 {
        self.points[self.points.len() - 1].elapsed_ns
    }

    pub fn duration_s(&self) -> f64 {
        self.duration_ns() as f64 / 1e9
    }

    /// A route is closed when its last point has exactly the coordinate of
    /// its first. "Nearly closed" is open: the gap would be a jump on every
    /// lap, and closing it silently would be repairing the input.
    pub fn is_closed(&self) -> bool {
        self.points[0].coordinate == self.points[self.points.len() - 1].coordinate
    }

    /// Distance, duration, mean speed and bearings of every leg.
    pub fn legs(&self) -> Result<Vec<RouteLeg>, GeoError> {
        self.points
            .windows(2)
            .enumerate()
            .map(|(index, w)| {
                let line = geographic::inverse(w[0].coordinate, w[1].coordinate)?;
                let duration_s = (w[1].elapsed_ns - w[0].elapsed_ns) as f64 / 1e9;
                let moves = line.distance_m > 0.0;
                Ok(RouteLeg {
                    index,
                    distance_m: line.distance_m,
                    duration_s,
                    mean_speed_mps: line.distance_m / duration_s,
                    initial_bearing_deg: moves.then_some(line.initial_bearing_deg),
                    final_bearing_deg: moves.then_some(line.final_bearing_deg),
                })
            })
            .collect()
    }

    /// Highest leg mean speed implied by the recording, in m/s.
    /// Fails if a leg's geodesic cannot be computed (antipodal jump).
    pub fn max_segment_speed_mps(&self) -> Result<f64, ConfigError> {
        let legs = self.legs().map_err(|e| {
            ConfigError::new("route.points", format!("a leg is not computable: {e}"))
        })?;
        Ok(legs.iter().map(|l| l.mean_speed_mps).fold(0.0, f64::max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: i64 = 1_000_000_000;

    fn c(lat: f64, lon: f64) -> Coordinate {
        Coordinate::new(lat, lon).unwrap()
    }

    fn p(seconds: i64, lat: f64, lon: f64) -> RoutePoint {
        RoutePoint::new(seconds * SEC, c(lat, lon))
    }

    #[test]
    fn accepts_a_minimal_route() {
        let r = Route::new(vec![p(0, 0.0, 0.0), p(10, 0.0, 0.0001)]).unwrap();
        assert_eq!(r.points().len(), 2);
        assert_eq!((r.duration_ns(), r.duration_s()), (10 * SEC, 10.0));
        assert!(!r.is_closed() && !r.has_altitude());
        assert_eq!(r.name(), None);
        assert_eq!(r.with_name("commute v2").name(), Some("commute v2"));
    }

    #[test]
    fn rejects_too_few_points() {
        assert_eq!(
            Route::new(vec![]),
            Err(RouteError::TooFewPoints { count: 0 })
        );
        assert_eq!(
            Route::new(vec![p(0, 0.0, 0.0)]),
            Err(RouteError::TooFewPoints { count: 1 })
        );
    }

    #[test]
    fn rejects_a_first_point_that_is_not_at_zero() {
        assert_eq!(
            Route::new(vec![p(5, 0.0, 0.0), p(6, 0.0, 0.1)]),
            Err(RouteError::FirstPointNotAtZero {
                elapsed_ns: 5 * SEC
            })
        );
        assert!(Route::new(vec![p(-1, 0.0, 0.0), p(6, 0.0, 0.1)]).is_err());
    }

    #[test]
    fn rejects_out_of_order_and_zero_duration_segments_with_the_index() {
        assert_eq!(
            Route::new(vec![p(0, 0.0, 0.0), p(4, 0.0, 0.1), p(4, 0.0, 0.2)]),
            Err(RouteError::NonIncreasingTime {
                index: 2,
                previous_ns: 4 * SEC,
                elapsed_ns: 4 * SEC
            })
        );
        assert_eq!(
            Route::new(vec![
                p(0, 0.0, 0.0),
                p(9, 0.0, 0.1),
                p(3, 0.0, 0.2),
                p(12, 0.0, 0.3)
            ]),
            Err(RouteError::NonIncreasingTime {
                index: 2,
                previous_ns: 9 * SEC,
                elapsed_ns: 3 * SEC
            })
        );
    }

    #[test]
    fn altitude_must_be_finite_and_all_or_nothing() {
        let ok = Route::new(vec![
            p(0, 0.0, 0.0).with_altitude(10.0),
            p(1, 0.0, 0.00001).with_altitude(12.0),
        ])
        .unwrap();
        assert!(ok.has_altitude());
        assert_eq!(
            Route::new(vec![
                p(0, 0.0, 0.0).with_altitude(10.0),
                p(1, 0.0, 0.1).with_altitude(f64::NAN)
            ]),
            Err(RouteError::NonFiniteAltitude { index: 1 })
        );
        assert_eq!(
            Route::new(vec![p(0, 0.0, 0.0).with_altitude(10.0), p(1, 0.0, 0.1)]),
            Err(RouteError::InconsistentAltitude { index: 1 })
        );
        assert_eq!(
            Route::new(vec![
                p(0, 0.0, 0.0),
                p(1, 0.0, 0.1),
                p(2, 0.0, 0.2).with_altitude(1.0)
            ]),
            Err(RouteError::InconsistentAltitude { index: 2 })
        );
        assert_eq!(
            Route::new(vec![
                p(0, 0.0, 0.0).with_altitude(f64::INFINITY),
                p(1, 0.0, 0.1)
            ]),
            Err(RouteError::NonFiniteAltitude { index: 0 })
        );
    }

    #[test]
    fn invalid_coordinates_cannot_even_be_expressed() {
        // The coordinate type is the guard: there is no way to build a
        // RoutePoint from an out-of-range or NaN position.
        assert!(Coordinate::new(91.0, 0.0).is_err());
        assert!(Coordinate::new(0.0, f64::NAN).is_err());
    }

    #[test]
    fn absolute_timestamps_are_shifted_to_the_first_and_nothing_else() {
        let t = |s: i64| Timestamp::from_nanos(1_700_000_000 * SEC + s * SEC);
        let r = Route::from_absolute(&[
            (t(0), c(0.0, 0.0), None),
            (t(2), c(0.0, 0.00001), None),
            (t(7), c(0.0, 0.00002), None),
        ])
        .unwrap();
        let elapsed: Vec<i64> = r.points().iter().map(|p| p.elapsed_ns).collect();
        assert_eq!(elapsed, [0, 2 * SEC, 7 * SEC]);

        // Out-of-order input is rejected, not sorted.
        assert_eq!(
            Route::from_absolute(&[
                (t(0), c(0.0, 0.0), None),
                (t(5), c(0.0, 0.1), None),
                (t(3), c(0.0, 0.2), None),
            ]),
            Err(RouteError::NonIncreasingTime {
                index: 2,
                previous_ns: 5 * SEC,
                elapsed_ns: 3 * SEC
            })
        );
        assert_eq!(
            Route::from_absolute(&[]),
            Err(RouteError::TooFewPoints { count: 0 })
        );
        let far = [
            (Timestamp::from_nanos(i64::MIN), c(0.0, 0.0), None),
            (Timestamp::from_nanos(i64::MAX), c(0.0, 0.1), None),
        ];
        assert_eq!(
            Route::from_absolute(&far),
            Err(RouteError::TimeOutOfRange { index: 1 })
        );
    }

    #[test]
    fn closed_means_exactly_the_same_coordinate() {
        let closed = Route::new(vec![p(0, 1.0, 2.0), p(5, 1.0, 2.0001), p(9, 1.0, 2.0)]).unwrap();
        assert!(closed.is_closed());
        let nearly = Route::new(vec![
            p(0, 1.0, 2.0),
            p(5, 1.0, 2.0001),
            p(9, 1.0, 2.000000001),
        ])
        .unwrap();
        assert!(!nearly.is_closed());
    }

    #[test]
    fn legs_come_from_the_geographic_engine() {
        // 0.0001° of longitude on the equator ≈ 11.132 m.
        let r = Route::new(vec![
            p(0, 0.0, 0.0),
            p(10, 0.0, 0.0001),
            p(14, 0.0, 0.0001), // a 4 s wait
            p(16, 0.0001, 0.0001),
        ])
        .unwrap();
        let legs = r.legs().unwrap();
        assert_eq!(legs.len(), 3);
        assert_eq!(legs[0].index, 0);
        assert!((legs[0].distance_m - 11.1319).abs() < 1e-3);
        assert_eq!(legs[0].duration_s, 10.0);
        assert!((legs[0].mean_speed_mps - 1.11319).abs() < 1e-4);
        assert!((legs[0].initial_bearing_deg.unwrap() - 90.0).abs() < 1e-6);
        assert_eq!(
            (
                legs[1].distance_m,
                legs[1].mean_speed_mps,
                legs[1].initial_bearing_deg
            ),
            (0.0, 0.0, None)
        );
        assert!(legs[2].initial_bearing_deg.unwrap().abs() < 1e-6);
        assert!((r.max_segment_speed_mps().unwrap() - legs[2].mean_speed_mps).abs() < 1e-12);

        let antipodal = Route::new(vec![p(0, 0.0, 0.0), p(10, 0.0, 180.0)]).unwrap();
        assert!(antipodal.legs().is_err());
        assert_eq!(
            antipodal.max_segment_speed_mps().unwrap_err().field,
            "route.points"
        );
    }
}
