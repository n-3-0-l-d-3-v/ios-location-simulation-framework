//! Consistency engine: reported kinematics that describe the trajectory that
//! is actually emitted.
//!
//! # The authoritative observation
//!
//! The **emitted position and its timestamp** are the source of truth. Speed
//! and course are not produced by the movement model or the noise engine;
//! they are derived here, from the positions that will be emitted, after
//! noise. Nothing in this module can change a position.
//!
//! ```text
//! movement → noise → final emitted position ─┐
//!                                            ├→ derive speed, course → final validation → emit
//!                      previous emitted fix ─┘
//! ```
//!
//! # Finite-difference scheme
//!
//! One scheme, used everywhere: the **backward difference** over the interval
//! that ends at the sample.
//!
//! * `speed_k  = geodesic distance(p_{k−1}, p_k) / (t_k − t_{k−1})` — the
//!   mean ground speed over the interval, not an instantaneous value.
//! * `course_k =` the bearing, at `p_k`, of the geodesic from `p_{k−1}` to
//!   `p_k` — the direction of travel on arrival.
//!
//! It is causal (a central difference would need the next fix and delay
//! every sample by one interval), it uses only the geographic engine's
//! geodesic inverse, and it has no free parameter. `t_k − t_{k−1}` is the
//! difference of two integer-nanosecond timestamps that the gate requires to
//! be strictly increasing, so it is never zero.
//!
//! The first sample of a run has no predecessor: its speed and course are
//! unknown (`None`), not zero.
//!
//! # Stationary semantics
//!
//! Two thresholds on the displacement `d` between consecutive emitted fixes,
//! both fixed constants:
//!
//! * `d <` [`STATIONARY_DISPLACEMENT_M`] (4 nm, twice the resolution of a
//!   stored coordinate): the fixes are indistinguishable. Speed is exactly 0
//!   and there is no course.
//! * `d <` [`MIN_COURSE_DISPLACEMENT_M`] (0.1 mm): the fix moved, so speed is
//!   `d / dt`, but the direction of so short a line is dominated by
//!   coordinate rounding, so there is **no course**. At 0.1 mm the rounding
//!   contributes at most 0.0023°; below it the error grows as `1/d`.
//!
//! A course is never carried over from an earlier sample.
//!
//! # Resolution
//!
//! A coordinate is stored to about 2 nm, so a distance between two fixes is
//! uncertain by up to [`DISTANCE_RESOLUTION_M`] (4 nm). Derived speed
//! therefore has a resolution of `4 nm / dt` — 4e-9 m/s at 1 Hz, 4e-6 m/s at
//! 1 kHz, 4 mm/s at 1 MHz — and derived course of `4 nm / d` radians. These
//! are properties of the observation, not tolerances chosen to make tests
//! pass; the validation gate adds exactly these amounts where it compares
//! derived quantities with physical limits.
//!
//! # Observation noise
//!
//! `noise.speed_noise_mps` and `noise.heading_noise_deg` are an observation
//! model on top of the derived values: `reported = derived + n`, with `n`
//! clipped at ±3 σ by the noise engine. The consistency contract is then
//! `|reported − derived| ≤ 3 σ`. A stationary sample gets no speed noise (it
//! must not claim movement) and a sample without a course gets none either.
//!
//! # Accuracy
//!
//! Accuracy is not kinematic and is not derived. Reported horizontal and
//! vertical accuracy are the scenario's configured values plus the noise
//! engine's clipped accuracy noise, nothing else. In particular accuracy is
//! never widened to cover a disagreement between position and speed.

use crate::domain::{NoiseParameters, Scenario, SyntheticLocation, Timestamp, NOISE_CLIP_SIGMA};
use crate::geographic::{
    self, bearing_difference, normalize_bearing, Coordinate, GeoError, Geodesic,
    COORDINATE_RESOLUTION_M,
};
use std::fmt;

/// Largest error of a distance computed between two stored fixes: each end
/// is rounded to [`COORDINATE_RESOLUTION_M`].
pub const DISTANCE_RESOLUTION_M: f64 = 2.0 * COORDINATE_RESOLUTION_M;

/// Below this displacement two consecutive fixes are the same point as far
/// as stored coordinates can tell: speed is exactly zero.
pub const STATIONARY_DISPLACEMENT_M: f64 = DISTANCE_RESOLUTION_M;

/// Below this displacement no course is reported: the direction of the line
/// between the fixes would be mostly rounding (0.0023° at the threshold,
/// growing as `1/d` below it).
pub const MIN_COURSE_DISPLACEMENT_M: f64 = 1e-4;

/// Resolution of a speed derived over `elapsed_s`, in m/s.
pub fn speed_resolution_mps(elapsed_s: f64) -> f64 {
    DISTANCE_RESOLUTION_M / elapsed_s
}

/// Resolution of a course derived over `displacement_m`, in degrees.
pub fn course_resolution_deg(displacement_m: f64) -> f64 {
    (DISTANCE_RESOLUTION_M / displacement_m).to_degrees()
}

/// Observation noise to add to the derived values (already clipped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObservationNoise {
    pub speed_mps: f64,
    pub heading_deg: f64,
}

impl ObservationNoise {
    pub const NONE: ObservationNoise = ObservationNoise {
        speed_mps: 0.0,
        heading_deg: 0.0,
    };
}

/// Kinematics derived for one fix from it and its predecessor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DerivedKinematics {
    /// `None` only for the first fix of a run.
    pub speed_mps: Option<f64>,
    pub course_deg: Option<f64>,
    /// Geodesic distance from the previous fix; `None` for the first.
    pub displacement_m: Option<f64>,
    /// Time since the previous fix; `None` for the first.
    pub elapsed_s: Option<f64>,
}

impl DerivedKinematics {
    const UNKNOWN: DerivedKinematics = DerivedKinematics {
        speed_mps: None,
        course_deg: None,
        displacement_m: None,
        elapsed_s: None,
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeriveError {
    /// The fix is not later than the one it would be differenced against.
    NonIncreasingTime {
        previous: Timestamp,
        current: Timestamp,
    },
    /// The geodesic between the two fixes could not be computed.
    Geo(GeoError),
}

impl fmt::Display for DeriveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeriveError::NonIncreasingTime { previous, current } => write!(
                f,
                "cannot derive kinematics: fix at {} ns is not after the previous one at {} ns",
                current.as_nanos(),
                previous.as_nanos()
            ),
            DeriveError::Geo(e) => write!(f, "cannot derive kinematics: {e}"),
        }
    }
}

impl std::error::Error for DeriveError {}

impl From<GeoError> for DeriveError {
    fn from(e: GeoError) -> Self {
        DeriveError::Geo(e)
    }
}

/// Derives speed and course for each fix of a stream from the fix itself and
/// the last *accepted* one.
///
/// The only state is that previous fix — one timestamp and one coordinate.
/// Nothing is cached beyond it, so nothing can go stale: `derive` is a pure
/// function of its arguments and that fix, and the fix only changes through
/// [`KinematicsDeriver::accept`], which the caller invokes once a sample has
/// actually been emitted. A rejected sample therefore never becomes the
/// reference for the next one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KinematicsDeriver {
    previous: Option<(Timestamp, Coordinate)>,
}

impl KinematicsDeriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Kinematics of a fix at `t`, `position`. One geodesic inverse; no
    /// allocation. Does not change the deriver.
    pub fn derive(
        &self,
        t: Timestamp,
        position: Coordinate,
        noise: ObservationNoise,
    ) -> Result<DerivedKinematics, DeriveError> {
        let Some((previous_t, previous_position)) = self.previous else {
            return Ok(DerivedKinematics::UNKNOWN);
        };
        if t <= previous_t {
            return Err(DeriveError::NonIncreasingTime {
                previous: previous_t,
                current: t,
            });
        }
        let elapsed_s = t.seconds_since(previous_t);
        let line = geographic::inverse(previous_position, position)?;
        let displacement_m = line.distance_m;
        let (speed, course) = if displacement_m < STATIONARY_DISPLACEMENT_M {
            (0.0, None)
        } else {
            // Observation noise cannot make a speed negative.
            let speed = (displacement_m / elapsed_s + noise.speed_mps).max(0.0);
            let course = (displacement_m >= MIN_COURSE_DISPLACEMENT_M && speed > 0.0)
                .then(|| normalize_bearing(line.final_bearing_deg + noise.heading_deg));
            (speed, course)
        };
        Ok(DerivedKinematics {
            speed_mps: Some(speed),
            course_deg: course,
            displacement_m: Some(displacement_m),
            elapsed_s: Some(elapsed_s),
        })
    }

    /// Records that the fix at `t`, `position` was emitted; the next
    /// `derive` is relative to it.
    pub fn accept(&mut self, t: Timestamp, position: Coordinate) {
        self.previous = Some((t, position));
    }

    /// Forgets the previous fix; the next one is a first fix again.
    pub fn reset(&mut self) {
        self.previous = None;
    }
}

/// What the reported metadata is allowed to differ from the derived values
/// by: only configured observation noise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConsistencyTolerance {
    /// 3 σ of speed observation noise.
    pub speed_noise_reach_mps: f64,
    /// 3 σ of heading observation noise.
    pub heading_noise_reach_deg: f64,
    /// If set, reported accuracies must lie within the band.
    pub accuracy: Option<AccuracyBand>,
}

/// Nominal accuracies and how far clipped accuracy noise may move them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccuracyBand {
    pub horizontal_m: f64,
    pub vertical_m: f64,
    pub reach_m: f64,
}

impl ConsistencyTolerance {
    /// No observation noise: metadata must equal the derived values.
    pub const EXACT: ConsistencyTolerance = ConsistencyTolerance {
        speed_noise_reach_mps: 0.0,
        heading_noise_reach_deg: 0.0,
        accuracy: None,
    };

    pub fn for_noise(noise: &NoiseParameters) -> Self {
        Self {
            speed_noise_reach_mps: NOISE_CLIP_SIGMA * noise.speed_noise_mps,
            heading_noise_reach_deg: NOISE_CLIP_SIGMA * noise.heading_noise_deg,
            accuracy: None,
        }
    }

    pub fn for_scenario(scenario: &Scenario) -> Self {
        Self {
            accuracy: Some(AccuracyBand {
                horizontal_m: scenario.horizontal_accuracy_m,
                vertical_m: scenario.vertical_accuracy_m,
                reach_m: NOISE_CLIP_SIGMA * scenario.noise.accuracy_noise_m,
            }),
            ..Self::for_noise(&scenario.noise)
        }
    }
}

/// How a sample's metadata contradicts the positions and timestamps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConsistencyError {
    /// A first sample has no predecessor to derive speed or course from.
    FirstSampleHasKinematics,
    NonIncreasingTime {
        previous: Timestamp,
        current: Timestamp,
    },
    /// Every sample after the first has a derivable speed.
    MissingSpeed,
    SpeedMismatch {
        reported_mps: f64,
        derived_mps: f64,
        tolerance_mps: f64,
    },
    /// The position did not move, yet a speed is claimed.
    StationaryWithSpeed {
        reported_mps: f64,
        displacement_m: f64,
    },
    /// The position moved far enough to have a direction, yet none is given.
    MissingCourse {
        displacement_m: f64,
    },
    /// A course is given although the position did not move far enough for
    /// one to exist.
    CourseWithoutMovement {
        displacement_m: f64,
    },
    CourseMismatch {
        reported_deg: f64,
        derived_deg: f64,
        tolerance_deg: f64,
    },
    AccuracyOutOfBand {
        reported_m: f64,
        nominal_m: f64,
        reach_m: f64,
    },
    Geo(GeoError),
}

impl fmt::Display for ConsistencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConsistencyError::FirstSampleHasKinematics => {
                write!(f, "first sample reports a speed or course it cannot know")
            }
            ConsistencyError::NonIncreasingTime { previous, current } => write!(
                f,
                "sample at {} ns is not after the previous one at {} ns",
                current.as_nanos(),
                previous.as_nanos()
            ),
            ConsistencyError::MissingSpeed => write!(f, "sample has no speed"),
            ConsistencyError::SpeedMismatch {
                reported_mps,
                derived_mps,
                tolerance_mps,
            } => write!(
                f,
                "reported speed {reported_mps} m/s, positions and timestamps give \
                 {derived_mps} m/s (tolerance {tolerance_mps})"
            ),
            ConsistencyError::StationaryWithSpeed {
                reported_mps,
                displacement_m,
            } => write!(
                f,
                "reported speed {reported_mps} m/s although the position moved only \
                 {displacement_m} m"
            ),
            ConsistencyError::MissingCourse { displacement_m } => write!(
                f,
                "no course although the position moved {displacement_m} m"
            ),
            ConsistencyError::CourseWithoutMovement { displacement_m } => write!(
                f,
                "course reported although the position moved only {displacement_m} m"
            ),
            ConsistencyError::CourseMismatch {
                reported_deg,
                derived_deg,
                tolerance_deg,
            } => write!(
                f,
                "reported course {reported_deg} deg, positions give {derived_deg} deg \
                 (tolerance {tolerance_deg})"
            ),
            ConsistencyError::AccuracyOutOfBand {
                reported_m,
                nominal_m,
                reach_m,
            } => write!(
                f,
                "reported accuracy {reported_m} m is not within {reach_m} m of the configured \
                 {nominal_m} m"
            ),
            ConsistencyError::Geo(e) => write!(f, "consistency geometry failed: {e}"),
        }
    }
}

impl std::error::Error for ConsistencyError {}

impl From<GeoError> for ConsistencyError {
    fn from(e: GeoError) -> Self {
        ConsistencyError::Geo(e)
    }
}

/// Rounding of one division and one addition; not a modelling tolerance.
const ARITHMETIC_SLACK: f64 = 8.0 * f64::EPSILON;
/// Rounding of a bearing normalised into `[0, 360)`.
const BEARING_SLACK_DEG: f64 = 1e-9;

fn check_accuracy(
    sample: &SyntheticLocation,
    tolerance: &ConsistencyTolerance,
) -> Result<(), ConsistencyError> {
    let Some(band) = tolerance.accuracy else {
        return Ok(());
    };
    for (reported_m, nominal_m) in [
        (sample.horizontal_accuracy_m, band.horizontal_m),
        (sample.vertical_accuracy_m, band.vertical_m),
    ] {
        let off = (reported_m - nominal_m).abs();
        // `reported − nominal` is rounded at the magnitude of the accuracy
        // itself, not of the (smaller) difference.
        let slack = ARITHMETIC_SLACK * (nominal_m.abs() + band.reach_m);
        if off.is_nan() || off > band.reach_m + slack {
            return Err(ConsistencyError::AccuracyOutOfBand {
                reported_m,
                nominal_m,
                reach_m: band.reach_m,
            });
        }
    }
    Ok(())
}

/// Checks a first sample: it can know neither speed nor course.
pub fn check_first(
    sample: &SyntheticLocation,
    tolerance: &ConsistencyTolerance,
) -> Result<(), ConsistencyError> {
    if sample.speed_mps.is_some() || sample.course_deg.is_some() {
        return Err(ConsistencyError::FirstSampleHasKinematics);
    }
    check_accuracy(sample, tolerance)
}

/// Checks that `current`'s speed and course are what its position and
/// timestamp, together with `previous`'s, say they are.
///
/// Independent of [`KinematicsDeriver`]: it trusts no stored result and
/// recomputes the expected values from the two samples with the geodesic
/// inverse. Metadata that came from the deriver and was then altered —
/// doubled, zeroed, reversed, left stale, or computed for a different
/// timestamp — fails here.
pub fn check_pair(
    previous: &SyntheticLocation,
    current: &SyntheticLocation,
    tolerance: &ConsistencyTolerance,
) -> Result<(), ConsistencyError> {
    let line = geographic::inverse(previous.coordinate, current.coordinate)?;
    check_pair_along(previous, current, &line, tolerance)
}

/// [`check_pair`] for a caller that already has the geodesic between the
/// two positions.
pub fn check_pair_along(
    previous: &SyntheticLocation,
    current: &SyntheticLocation,
    line: &Geodesic,
    tolerance: &ConsistencyTolerance,
) -> Result<(), ConsistencyError> {
    if current.timestamp <= previous.timestamp {
        return Err(ConsistencyError::NonIncreasingTime {
            previous: previous.timestamp,
            current: current.timestamp,
        });
    }
    check_accuracy(current, tolerance)?;
    let elapsed_s = current.timestamp.seconds_since(previous.timestamp);
    let displacement_m = line.distance_m;
    let Some(reported_mps) = current.speed_mps else {
        return Err(ConsistencyError::MissingSpeed);
    };

    if displacement_m < STATIONARY_DISPLACEMENT_M {
        if reported_mps != 0.0 {
            return Err(ConsistencyError::StationaryWithSpeed {
                reported_mps,
                displacement_m,
            });
        }
    } else {
        let derived_mps = displacement_m / elapsed_s;
        // `derived + noise` is rounded at the magnitude of the sum.
        let tolerance_mps = tolerance.speed_noise_reach_mps
            + (derived_mps + tolerance.speed_noise_reach_mps) * ARITHMETIC_SLACK;
        let off = (reported_mps - derived_mps).abs();
        if off.is_nan() || off > tolerance_mps {
            return Err(ConsistencyError::SpeedMismatch {
                reported_mps,
                derived_mps,
                tolerance_mps,
            });
        }
    }

    // A course exists exactly when the fix moved far enough for a direction
    // to be meaningful and the reported speed is not zero.
    let expects_course = displacement_m >= MIN_COURSE_DISPLACEMENT_M && reported_mps > 0.0;
    match (expects_course, current.course_deg) {
        (false, None) => Ok(()),
        (false, Some(_)) => Err(ConsistencyError::CourseWithoutMovement { displacement_m }),
        (true, None) => Err(ConsistencyError::MissingCourse { displacement_m }),
        (true, Some(reported_deg)) => {
            let derived_deg = line.final_bearing_deg;
            let tolerance_deg = tolerance.heading_noise_reach_deg + BEARING_SLACK_DEG;
            let off = bearing_difference(derived_deg, reported_deg).abs();
            if off.is_nan() || off > tolerance_deg {
                return Err(ConsistencyError::CourseMismatch {
                    reported_deg,
                    derived_deg,
                    tolerance_deg,
                });
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
