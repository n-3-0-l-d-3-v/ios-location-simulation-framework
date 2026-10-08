//! Noise / realism engine.
//!
//! Sits between the movement model and the final validation gate:
//! base movement → **noise** → metadata → validation → emit. It knows nothing
//! about scheduling or delivery; it is driven only by the timestamps and base
//! samples it is handed.
//!
//! # Model
//!
//! Noise is treated as *measurement error on top of the true motion*:
//!
//! * **Jitter** — per-axis first-order Gauss–Markov process (σ =
//!   `position_noise_m`, correlation time `position_correlation_time_s`),
//!   clipped at [`NOISE_CLIP_SIGMA`]. With a correlation time of zero it
//!   degenerates to independent high-frequency noise.
//! * **Drift** — a point wandering at exactly `drift_rate_mps` between
//!   uniformly drawn waypoints inside a disc, i.e. slow and continuous.
//! * **Speed / heading / accuracy** — scalar Gauss–Markov channels, clipped.
//!
//! # Guarantees
//!
//! Every returned position is checked against exact geodesic distances
//! before it leaves [`NoiseEngine::apply`]:
//!
//! 1. offset from the true position ≤ `max_position_offset_m`;
//! 2. displacement from the previous output ≤ true displacement +
//!    `max_offset_rate_mps × dt`;
//! 3. inside the scenario boundary, if there is one.
//!
//! The candidate is first shaped by projections onto those three sets. If the
//! result still fails the exact check (curvature, rounding, or limits below
//! the numerical resolution), fallbacks are tried in order: a point on the
//! geodesic from the previous output towards the true position (which
//! satisfies all three by the triangle inequality), then holding the previous
//! output, then the true position itself. If none passes the exact check the
//! engine returns an error instead of a position. With all components
//! disabled the engine is an exact identity.

use crate::domain::{Boundary, ConfigError, Coordinate, NoiseParameters, Scenario, Timestamp};
use crate::domain::{SyntheticLocation, NOISE_CLIP_SIGMA};
use crate::geographic::{self, normalize_bearing, EnuFrame, GeoError};
use crate::movement::MovementSample;
use crate::rng::Rng;
use std::fmt;

/// Relative safety margin applied to limits while shaping a candidate, so
/// that the exact geodesic check normally passes on the first attempt.
const MARGIN: f64 = 1e-6;
/// Absolute counterpart of [`MARGIN`], well above the ~1e-9 m resolution of
/// the geodesic routines, for limits so small that a relative margin alone
/// would vanish.
const ABSOLUTE_MARGIN_M: f64 = 1e-7;
/// The working tangent plane is re-anchored when the true position moves
/// this far from its origin, keeping planar and geodesic distances in
/// agreement to ~1e-8 relative.
const REANCHOR_DISTANCE_M: f64 = 1_000.0;
const MAX_PROJECTION_ROUNDS: usize = 4;
/// Upper bound on waypoints consumed in one step (bounds work for huge `dt`).
const MAX_DRIFT_LEGS: usize = 64;

// Independent random streams, so enabling one component never changes the
// sequence of another.
const STREAM_ROOT: u64 = 0x004E_4F49_5345; // "NOISE"
const STREAM_JITTER_E: u64 = 1;
const STREAM_JITTER_N: u64 = 2;
const STREAM_DRIFT: u64 = 3;
const STREAM_SPEED: u64 = 4;
const STREAM_HEADING: u64 = 5;
const STREAM_H_ACCURACY: u64 = 6;
const STREAM_V_ACCURACY: u64 = 7;

#[derive(Debug, Clone, PartialEq)]
pub enum NoiseError {
    InvalidParameters(Vec<ConfigError>),
    /// `apply` was called with a time not after the previous call's.
    NonIncreasingTime {
        previous: Timestamp,
        current: Timestamp,
    },
    /// The movement model handed over a position outside the boundary;
    /// noise refuses to mask that by pulling it back in.
    BaseOutsideBoundary {
        distance_m: f64,
        radius_m: f64,
    },
    Geo(GeoError),
    /// No position satisfying all limits could be constructed.
    ConstraintUnsatisfiable,
}

impl fmt::Display for NoiseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoiseError::InvalidParameters(errors) => {
                write!(f, "invalid noise configuration:")?;
                for e in errors {
                    write!(f, " [{e}]")?;
                }
                Ok(())
            }
            NoiseError::NonIncreasingTime { previous, current } => write!(
                f,
                "noise applied at {} ns, not after previous {} ns",
                current.as_nanos(),
                previous.as_nanos()
            ),
            NoiseError::BaseOutsideBoundary {
                distance_m,
                radius_m,
            } => write!(
                f,
                "base position is {distance_m} m from the boundary centre, radius is {radius_m} m"
            ),
            NoiseError::Geo(e) => write!(f, "noise geometry failed: {e}"),
            NoiseError::ConstraintUnsatisfiable => {
                write!(f, "no noisy position satisfies the configured limits")
            }
        }
    }
}

impl std::error::Error for NoiseError {}

impl From<GeoError> for NoiseError {
    fn from(e: GeoError) -> Self {
        NoiseError::Geo(e)
    }
}

/// Limits from the rest of the scenario that noise must respect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseConstraints {
    /// Reported speed is never pushed above this.
    pub max_speed_mps: f64,
    pub boundary: Option<Boundary>,
}

/// Base sample with noise applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoisySample {
    pub coordinate: Coordinate,
    pub altitude_m: f64,
    pub speed_mps: Option<f64>,
    pub course_deg: Option<f64>,
    pub horizontal_accuracy_m: f64,
    pub vertical_accuracy_m: f64,
    /// Geodesic distance between the true and the reported position.
    pub position_offset_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct V2 {
    e: f64,
    n: f64,
}

/// Nearest point to `p` inside the disc. Non-expansive, which is what lets
/// several limits be enforced one after another.
fn project_into_disc(p: V2, center: V2, radius: f64) -> V2 {
    let (de, dn) = (p.e - center.e, p.n - center.n);
    let d = de.hypot(dn);
    if d <= radius {
        return p;
    }
    let k = radius / d;
    V2 {
        e: center.e + de * k,
        n: center.n + dn * k,
    }
}

fn in_disc(p: V2, center: V2, radius: f64) -> bool {
    (p.e - center.e).hypot(p.n - center.n) <= radius
}

/// Nearest point to `p` inside the intersection of two discs, or `None` if
/// the discs do not intersect.
///
/// Projecting onto one disc and then the other is *not* enough: the second
/// projection can leave the first disc, and alternating converges slowly
/// when the intersection is a thin lens. The exact answer is either the
/// projection onto one disc (when that already lies in the other) or one of
/// the two corners of the lens.
fn project_into_lens(p: V2, c1: V2, r1: f64, c2: V2, r2: f64) -> Option<V2> {
    let a = project_into_disc(p, c1, r1);
    if in_disc(a, c2, r2) {
        return Some(a);
    }
    let b = project_into_disc(p, c2, r2);
    if in_disc(b, c1, r1) {
        return Some(b);
    }
    let (de, dn) = (c2.e - c1.e, c2.n - c1.n);
    let d = de.hypot(dn);
    if d == 0.0 {
        return None;
    }
    // Distance from c1 along the centre line to the chord joining the corners.
    let along = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let half_chord_sq = r1 * r1 - along * along;
    if half_chord_sq < 0.0 {
        return None;
    }
    let half_chord = half_chord_sq.sqrt();
    let (ue, un) = (de / d, dn / d);
    let corner = |sign: f64| V2 {
        e: c1.e + along * ue - sign * half_chord * un,
        n: c1.n + along * un + sign * half_chord * ue,
    };
    let (k1, k2) = (corner(1.0), corner(-1.0));
    let dist = |k: V2| (k.e - p.e).hypot(k.n - p.n);
    Some(if dist(k1) <= dist(k2) { k1 } else { k2 })
}

/// A limit pulled in by the safety margins (never below zero).
fn tighten(limit_m: f64) -> f64 {
    (limit_m * (1.0 - MARGIN) - ABSOLUTE_MARGIN_M).max(0.0)
}

/// Clipped first-order Gauss–Markov scalar process.
#[derive(Debug, Clone)]
struct Channel {
    sigma: f64,
    state: f64,
    rng: Rng,
}

impl Channel {
    fn new(sigma: f64, rng: Rng) -> Self {
        Self {
            sigma,
            state: 0.0,
            rng,
        }
    }

    /// `rho` is the correlation with the previous value; 0 on the first call.
    fn next(&mut self, rho: f64) -> f64 {
        if self.sigma == 0.0 {
            return 0.0;
        }
        // Stationary update: the variance stays σ² for every rho in [0, 1].
        let innovation = (1.0 - rho * rho).sqrt() * self.sigma * self.rng.standard_normal();
        self.state = rho * self.state + innovation;
        let limit = NOISE_CLIP_SIGMA * self.sigma;
        self.state.clamp(-limit, limit)
    }
}

/// Low-frequency drift: constant-speed travel between random waypoints
/// inside a disc.
#[derive(Debug, Clone)]
struct Drift {
    rate_mps: f64,
    radius_m: f64,
    position: V2,
    target: Option<V2>,
    rng: Rng,
}

impl Drift {
    fn draw_target(&mut self) -> V2 {
        // sqrt gives a uniform density over the disc's area.
        let r = self.radius_m * self.rng.next_f64().sqrt();
        let theta = std::f64::consts::TAU * self.rng.next_f64();
        V2 {
            e: r * theta.cos(),
            n: r * theta.sin(),
        }
    }

    fn advance(&mut self, dt_s: f64) {
        let mut remaining = self.rate_mps * dt_s;
        for _ in 0..MAX_DRIFT_LEGS {
            if remaining <= 0.0 {
                break;
            }
            let target = match self.target {
                Some(t) => t,
                None => {
                    let t = self.draw_target();
                    self.target = Some(t);
                    t
                }
            };
            let (de, dn) = (target.e - self.position.e, target.n - self.position.n);
            let d = de.hypot(dn);
            if d <= remaining {
                self.position = target;
                self.target = None;
                remaining -= d;
            } else {
                let k = remaining / d;
                self.position.e += de * k;
                self.position.n += dn * k;
                remaining = 0.0;
            }
        }
    }
}

#[derive(Debug, Clone)]
struct PositionNoise {
    jitter_e: Channel,
    jitter_n: Channel,
    drift: Drift,
}

#[derive(Debug, Clone, Copy)]
struct Previous {
    t: Timestamp,
    base: Coordinate,
    out: Coordinate,
}

/// Limit on the distance from the previous output, for one step.
#[derive(Debug, Clone, Copy)]
struct StepLimit {
    from: Coordinate,
    max_m: f64,
}

#[derive(Debug, Clone)]
pub struct NoiseEngine {
    params: NoiseParameters,
    constraints: NoiseConstraints,
    seed: u64,
    position: Option<PositionNoise>,
    speed: Channel,
    heading: Channel,
    horizontal_accuracy: Channel,
    vertical_accuracy: Channel,
    frame: Option<EnuFrame>,
    previous: Option<Previous>,
    fallback_count: u64,
}

impl NoiseEngine {
    pub fn new(
        params: NoiseParameters,
        constraints: NoiseConstraints,
        seed: u64,
    ) -> Result<Self, NoiseError> {
        let mut errors = Vec::new();
        params.validate(&mut errors);
        if !constraints.max_speed_mps.is_finite() || constraints.max_speed_mps < 0.0 {
            errors.push(ConfigError {
                field: "movement.max_speed_mps",
                reason: "must be finite and >= 0".into(),
            });
        }
        if let Some(b) = constraints.boundary {
            if !b.radius_m.is_finite() || b.radius_m <= 0.0 {
                errors.push(ConfigError {
                    field: "movement.radius_m",
                    reason: "must be finite and > 0".into(),
                });
            }
        }
        if !errors.is_empty() {
            return Err(NoiseError::InvalidParameters(errors));
        }

        let mut root = Rng::from_seed(seed).fork(STREAM_ROOT);
        // Fork every stream unconditionally and in a fixed order, so a
        // stream's content never depends on which components are enabled.
        let mut stream = |id| root.fork(id);
        let jitter_e = Channel::new(params.position_noise_m, stream(STREAM_JITTER_E));
        let jitter_n = Channel::new(params.position_noise_m, stream(STREAM_JITTER_N));
        let drift_rng = stream(STREAM_DRIFT);
        let speed = Channel::new(params.speed_noise_mps, stream(STREAM_SPEED));
        let heading = Channel::new(params.heading_noise_deg, stream(STREAM_HEADING));
        let horizontal_accuracy = Channel::new(params.accuracy_noise_m, stream(STREAM_H_ACCURACY));
        let vertical_accuracy = Channel::new(params.accuracy_noise_m, stream(STREAM_V_ACCURACY));

        let position = params.has_position_noise().then(|| PositionNoise {
            jitter_e,
            jitter_n,
            drift: Drift {
                rate_mps: params.drift_rate_mps,
                // Leave room for the clipped jitter on top of the drift.
                radius_m: (params.max_position_offset_m
                    - NOISE_CLIP_SIGMA * params.position_noise_m)
                    .max(0.0),
                position: V2 { e: 0.0, n: 0.0 },
                target: None,
                rng: drift_rng,
            },
        });

        Ok(Self {
            params,
            constraints,
            seed,
            position,
            speed,
            heading,
            horizontal_accuracy,
            vertical_accuracy,
            frame: None,
            previous: None,
            fallback_count: 0,
        })
    }

    /// Engine configured from a scenario's noise settings, limits and seed.
    pub fn for_scenario(scenario: &Scenario) -> Result<Self, NoiseError> {
        Self::new(
            scenario.noise,
            NoiseConstraints {
                max_speed_mps: scenario.movement.max_speed_mps,
                boundary: scenario.boundary(),
            },
            scenario.seed,
        )
    }

    pub fn parameters(&self) -> &NoiseParameters {
        &self.params
    }

    /// How many samples needed the geodesic fallback instead of the shaped
    /// candidate. Expected to stay at or near zero; exposed for monitoring.
    pub fn fallback_count(&self) -> u64 {
        self.fallback_count
    }

    /// Returns the engine to its initial state: the same calls then produce
    /// the same output again.
    pub fn reset(&mut self) -> Result<(), NoiseError> {
        *self = Self::new(self.params, self.constraints, self.seed)?;
        Ok(())
    }

    /// Applies noise to one base sample taken at time `t`. Times must
    /// strictly increase from call to call.
    pub fn apply(
        &mut self,
        t: Timestamp,
        base: &MovementSample,
        horizontal_accuracy_m: f64,
        vertical_accuracy_m: f64,
    ) -> Result<NoisySample, NoiseError> {
        let dt = match self.previous {
            None => None,
            Some(prev) => {
                let dt = t.seconds_since(prev.t);
                if dt <= 0.0 {
                    return Err(NoiseError::NonIncreasingTime {
                        previous: prev.t,
                        current: t,
                    });
                }
                Some(dt)
            }
        };
        if let Some(b) = self.constraints.boundary {
            let distance_m = geographic::distance(b.center, base.coordinate)?;
            if distance_m > b.radius_m {
                return Err(NoiseError::BaseOutsideBoundary {
                    distance_m,
                    radius_m: b.radius_m,
                });
            }
        }
        let tau = self.params.position_correlation_time_s;
        let rho = match dt {
            Some(dt) if tau > 0.0 => (-dt / tau).exp(),
            _ => 0.0,
        };

        let (coordinate, position_offset_m) = if self.position.is_some() {
            let c = self.noisy_position(base.coordinate, dt, rho)?;
            (c, geographic::distance(base.coordinate, c)?)
        } else {
            (base.coordinate, 0.0)
        };

        // Channels advance on every sample, moving or not, so their
        // sequences depend only on the sample index.
        let speed_noise = self.speed.next(rho);
        let heading_noise = self.heading.next(rho);
        let max_speed = self.constraints.max_speed_mps;
        let (speed_mps, course_deg) = match base.speed_mps {
            // Noise applies only to a moving, in-limit base. A stationary
            // sample stays exactly stationary; an out-of-limit one is passed
            // through untouched for the validation gate to reject.
            Some(v) if v > 0.0 && v <= max_speed => {
                let speed = (v + speed_noise).clamp(0.0, max_speed);
                // A course is only meaningful while moving.
                let course = base
                    .course_deg
                    .filter(|_| speed > 0.0)
                    .map(|c| normalize_bearing(c + heading_noise));
                (Some(speed), course)
            }
            other => (other, base.course_deg),
        };

        let sample = NoisySample {
            coordinate,
            altitude_m: base.altitude_m,
            speed_mps,
            course_deg,
            horizontal_accuracy_m: horizontal_accuracy_m + self.horizontal_accuracy.next(rho),
            vertical_accuracy_m: vertical_accuracy_m + self.vertical_accuracy.next(rho),
            position_offset_m,
        };
        self.previous = Some(Previous {
            t,
            base: base.coordinate,
            out: coordinate,
        });
        Ok(sample)
    }

    fn noisy_position(
        &mut self,
        base: Coordinate,
        dt: Option<f64>,
        rho: f64,
    ) -> Result<Coordinate, NoiseError> {
        let frame = match self.frame {
            Some(f) if f.to_enu(base, 0.0)?.horizontal_distance() <= REANCHOR_DISTANCE_M => f,
            _ => {
                // Altitude 0: the plane then touches the ellipsoid, so planar
                // distances are not scaled relative to surface distances.
                let f = EnuFrame::new(base, 0.0)?;
                self.frame = Some(f);
                f
            }
        };
        let to_plane = |c: Coordinate| -> Result<V2, NoiseError> {
            let enu = frame.to_enu(c, 0.0)?;
            Ok(V2 {
                e: enu.east,
                n: enu.north,
            })
        };
        // Inverse of `to_plane`. Dropping a plane point onto the ellipsoid
        // follows the *local* normal, which is tilted relative to the plane's
        // own, so the landing point maps back slightly closer to the anchor
        // (by ρ³/2R², ~12 µm at 1 km). One correction step removes that;
        // without it the planar step limit and the geodesic check disagree
        // whenever the limit is small.
        let from_plane = |q: V2| -> Result<Coordinate, NoiseError> {
            let landed = to_plane(frame.horizontal_to_coordinate(q.e, q.n)?)?;
            Ok(frame.horizontal_to_coordinate(q.e + (q.e - landed.e), q.n + (q.n - landed.n))?)
        };

        let position = self.position.as_mut().expect("checked by caller");
        position.drift.advance(dt.unwrap_or(0.0));
        let offset = V2 {
            e: position.drift.position.e + position.jitter_e.next(rho),
            n: position.drift.position.n + position.jitter_n.next(rho),
        };

        let max_offset_m = self.params.max_position_offset_m;
        let step = match (self.previous, dt) {
            (Some(prev), Some(dt)) => Some(StepLimit {
                from: prev.out,
                max_m: geographic::distance(prev.base, base)?
                    + self.params.max_offset_rate_mps * dt,
            }),
            _ => None,
        };
        let boundary = self.constraints.boundary;
        let satisfies = |c: Coordinate| -> Result<bool, NoiseError> {
            if geographic::distance(base, c)? > max_offset_m {
                return Ok(false);
            }
            if let Some(s) = step {
                if geographic::distance(s.from, c)? > s.max_m {
                    return Ok(false);
                }
            }
            if let Some(b) = boundary {
                if geographic::distance(b.center, c)? > b.radius_m {
                    return Ok(false);
                }
            }
            Ok(true)
        };

        let p = to_plane(base)?;
        let step_plane = match step {
            Some(s) => Some((to_plane(s.from)?, tighten(s.max_m))),
            None => None,
        };
        let mut q = V2 {
            e: p.e + offset.e,
            n: p.n + offset.n,
        };
        for _ in 0..MAX_PROJECTION_ROUNDS {
            q = match step_plane {
                None => project_into_disc(q, p, tighten(max_offset_m)),
                Some((center, radius)) => {
                    match project_into_lens(q, p, tighten(max_offset_m), center, radius) {
                        Some(q) => q,
                        // Limits too tight to resolve in the plane.
                        None => break,
                    }
                }
            };
            let mut c = from_plane(q)?;
            if let Some(b) = boundary {
                // The boundary centre may be far from the working plane, so
                // this limit is enforced with geodesics, not in the plane.
                let g = geographic::inverse(b.center, c)?;
                let radius = tighten(b.radius_m);
                if g.distance_m > radius {
                    c = geographic::destination(b.center, g.initial_bearing_deg, radius)?;
                    q = to_plane(c)?;
                }
            }
            if satisfies(c)? {
                return Ok(c);
            }
        }

        // Fallbacks, used when the shaped candidate fails the exact check
        // (curvature, rounding, or limits below the resolution of the
        // geodesic computation). Each is verified like any other candidate.
        self.fallback_count += 1;
        let Some(s) = step else {
            // First sample: zero offset satisfies the offset bound, and the
            // base was already checked against the boundary.
            return Ok(base);
        };
        // 1. A point on the geodesic from the previous output to the true
        //    position. A fraction f of the way is within the step limit when
        //    f·d ≤ max step, and within the offset bound when (1 − f)·d ≤
        //    max offset; that interval is never empty (triangle inequality),
        //    and its midpoint has the most slack against rounding. Both ends
        //    lie inside the boundary, hence so does the segment.
        let d = geographic::distance(s.from, base)?;
        if d > 0.0 {
            let lowest = (1.0 - max_offset_m / d).max(0.0);
            let highest = (s.max_m / d).min(1.0);
            let c = geographic::interpolate(s.from, base, 0.5 * (lowest + highest))?;
            if satisfies(c)? {
                return Ok(c);
            }
        }
        // 2. Hold the previous output (zero step), or 3. report the true
        //    position (zero offset). One of them is exact whenever the
        //    remaining slack is too small to resolve numerically.
        for c in [s.from, base] {
            if satisfies(c)? {
                return Ok(c);
            }
        }
        Err(NoiseError::ConstraintUnsatisfiable)
    }
}

impl NoisySample {
    /// Assembles the emit-ready sample. Final validation happens after this.
    pub fn into_location(
        self,
        timestamp: Timestamp,
        source: crate::domain::LocationSource,
        simulation_state: crate::domain::SimulationState,
    ) -> SyntheticLocation {
        SyntheticLocation {
            timestamp,
            coordinate: self.coordinate,
            altitude_m: self.altitude_m,
            horizontal_accuracy_m: self.horizontal_accuracy_m,
            vertical_accuracy_m: self.vertical_accuracy_m,
            speed_mps: self.speed_mps,
            course_deg: self.course_deg,
            source,
            simulation_state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: i64 = 1_000_000_000;

    fn t(n: i64) -> Timestamp {
        Timestamp::from_nanos(n * SEC)
    }

    fn origin() -> Coordinate {
        Coordinate::new(12.9352, 77.6245).unwrap()
    }

    fn still(c: Coordinate) -> MovementSample {
        MovementSample {
            coordinate: c,
            altitude_m: 920.0,
            speed_mps: Some(0.0),
            course_deg: None,
        }
    }

    fn moving(c: Coordinate, speed: f64, course: f64) -> MovementSample {
        MovementSample {
            coordinate: c,
            altitude_m: 920.0,
            speed_mps: Some(speed),
            course_deg: Some(course),
        }
    }

    fn free(max_speed_mps: f64) -> NoiseConstraints {
        NoiseConstraints {
            max_speed_mps,
            boundary: None,
        }
    }

    fn jitter() -> NoiseParameters {
        NoiseParameters {
            position_noise_m: 2.0,
            max_position_offset_m: 8.0,
            position_correlation_time_s: 5.0,
            max_offset_rate_mps: 3.0,
            ..NoiseParameters::NONE
        }
    }

    #[test]
    fn disabled_noise_is_an_exact_identity() {
        let mut e = NoiseEngine::new(NoiseParameters::NONE, free(10.0), 1).unwrap();
        for n in 0..100 {
            let base = moving(origin(), 3.5, 123.25);
            let s = e.apply(t(n), &base, 5.0, 8.0).unwrap();
            assert_eq!(s.coordinate, base.coordinate);
            assert_eq!(s.altitude_m, base.altitude_m);
            assert_eq!((s.speed_mps, s.course_deg), (Some(3.5), Some(123.25)));
            assert_eq!((s.horizontal_accuracy_m, s.vertical_accuracy_m), (5.0, 8.0));
            assert_eq!(s.position_offset_m, 0.0);
        }
        assert_eq!(e.fallback_count(), 0);
    }

    #[test]
    fn same_seed_reproduces_and_reset_replays() {
        let run = |e: &mut NoiseEngine| -> Vec<NoisySample> {
            (0..200)
                .map(|n| e.apply(t(n), &still(origin()), 5.0, 8.0).unwrap())
                .collect()
        };
        let mut a = NoiseEngine::new(jitter(), free(0.0), 42).unwrap();
        let mut b = NoiseEngine::new(jitter(), free(0.0), 42).unwrap();
        let first = run(&mut a);
        assert_eq!(first, run(&mut b));
        a.reset().unwrap();
        assert_eq!(first, run(&mut a));

        let mut c = NoiseEngine::new(jitter(), free(0.0), 43).unwrap();
        assert_ne!(first, run(&mut c));
    }

    #[test]
    fn components_use_independent_streams() {
        let positions = |p: NoiseParameters| -> Vec<Coordinate> {
            let mut e = NoiseEngine::new(p, free(10.0), 7).unwrap();
            (0..100)
                .map(|n| {
                    e.apply(t(n), &moving(origin(), 2.0, 90.0), 5.0, 8.0)
                        .unwrap()
                        .coordinate
                })
                .collect()
        };
        let with_extras = NoiseParameters {
            speed_noise_mps: 0.3,
            heading_noise_deg: 4.0,
            accuracy_noise_m: 0.5,
            ..jitter()
        };
        assert_eq!(positions(jitter()), positions(with_extras));
    }

    #[test]
    fn position_stays_within_offset_and_step_limits() {
        let p = jitter();
        let mut e = NoiseEngine::new(p, free(0.0), 3).unwrap();
        let mut prev: Option<Coordinate> = None;
        let mut moved = false;
        for n in 0..2_000 {
            let s = e.apply(t(n), &still(origin()), 5.0, 8.0).unwrap();
            assert!(s.position_offset_m <= p.max_position_offset_m);
            if let Some(prev) = prev {
                let step = geographic::distance(prev, s.coordinate).unwrap();
                assert!(step <= p.max_offset_rate_mps, "step {step}");
                moved |= step > 0.0;
            }
            prev = Some(s.coordinate);
        }
        assert!(moved);
    }

    #[test]
    fn stationary_metadata_is_untouched() {
        let p = NoiseParameters {
            speed_noise_mps: 1.0,
            heading_noise_deg: 10.0,
            ..NoiseParameters::NONE
        };
        let mut e = NoiseEngine::new(p, free(5.0), 1).unwrap();
        for n in 0..50 {
            let s = e.apply(t(n), &still(origin()), 5.0, 8.0).unwrap();
            assert_eq!((s.speed_mps, s.course_deg), (Some(0.0), None));
        }
        let unknown = MovementSample {
            speed_mps: None,
            ..still(origin())
        };
        let s = e.apply(t(50), &unknown, 5.0, 8.0).unwrap();
        assert_eq!((s.speed_mps, s.course_deg), (None, None));
    }

    #[test]
    fn speed_and_course_noise_respect_limits_and_consistency() {
        let p = NoiseParameters {
            speed_noise_mps: 2.0,
            heading_noise_deg: 30.0,
            ..NoiseParameters::NONE
        };
        let max = 3.0;
        let mut e = NoiseEngine::new(p, free(max), 9).unwrap();
        let (mut varied, mut hit_zero, mut hit_max) = (false, false, false);
        for n in 0..5_000 {
            let s = e
                .apply(t(n), &moving(origin(), 2.5, 350.0), 5.0, 8.0)
                .unwrap();
            let speed = s.speed_mps.unwrap();
            assert!((0.0..=max).contains(&speed));
            varied |= speed != 2.5;
            hit_zero |= speed == 0.0;
            hit_max |= speed == max;
            match s.course_deg {
                Some(c) => {
                    assert!(speed > 0.0);
                    assert!((0.0..360.0).contains(&c));
                    // Clipped at 3 σ = 90° around 350°.
                    assert!(geographic::bearing_difference(350.0, c).abs() <= 90.0 + 1e-9);
                }
                None => assert_eq!(speed, 0.0),
            }
        }
        assert!(varied && hit_zero && hit_max);
    }

    #[test]
    fn out_of_limit_base_speed_is_not_masked() {
        let p = NoiseParameters {
            speed_noise_mps: 2.0,
            ..NoiseParameters::NONE
        };
        let mut e = NoiseEngine::new(p, free(3.0), 9).unwrap();
        for n in 0..200 {
            let s = e
                .apply(t(n), &moving(origin(), 9.0, 10.0), 5.0, 8.0)
                .unwrap();
            assert_eq!(s.speed_mps, Some(9.0));
        }
    }

    #[test]
    fn accuracy_noise_is_clipped_around_the_base() {
        let p = NoiseParameters {
            accuracy_noise_m: 1.0,
            ..NoiseParameters::NONE
        };
        let mut e = NoiseEngine::new(p, free(0.0), 5).unwrap();
        let mut varied = false;
        for n in 0..5_000 {
            let s = e.apply(t(n), &still(origin()), 5.0, 8.0).unwrap();
            assert!((2.0..=8.0).contains(&s.horizontal_accuracy_m));
            assert!((5.0..=11.0).contains(&s.vertical_accuracy_m));
            varied |= s.horizontal_accuracy_m != 5.0;
        }
        assert!(varied);
    }

    #[test]
    fn boundary_is_enforced_and_base_violations_are_reported() {
        let boundary = Boundary {
            center: origin(),
            radius_m: 1.0,
        };
        let p = NoiseParameters {
            position_noise_m: 5.0,
            max_position_offset_m: 20.0,
            max_offset_rate_mps: 50.0,
            ..NoiseParameters::NONE
        };
        let constraints = NoiseConstraints {
            max_speed_mps: 0.0,
            boundary: Some(boundary),
        };
        let mut e = NoiseEngine::new(p, constraints, 11).unwrap();
        for n in 0..1_000 {
            let s = e.apply(t(n), &still(origin()), 5.0, 8.0).unwrap();
            assert!(geographic::distance(origin(), s.coordinate).unwrap() <= 1.0);
        }

        let outside = geographic::destination(origin(), 45.0, 1.5).unwrap();
        match e.apply(t(1_000), &still(outside), 5.0, 8.0) {
            Err(NoiseError::BaseOutsideBoundary {
                distance_m,
                radius_m,
            }) => {
                assert!((distance_m - 1.5).abs() < 1e-6);
                assert_eq!(radius_m, 1.0);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn time_must_strictly_increase() {
        let mut e = NoiseEngine::new(jitter(), free(0.0), 1).unwrap();
        e.apply(t(10), &still(origin()), 5.0, 8.0).unwrap();
        for bad in [10, 9] {
            assert_eq!(
                e.apply(t(bad), &still(origin()), 5.0, 8.0),
                Err(NoiseError::NonIncreasingTime {
                    previous: t(10),
                    current: t(bad)
                })
            );
        }
        e.apply(t(11), &still(origin()), 5.0, 8.0).unwrap();
    }

    #[test]
    fn invalid_configuration_is_rejected() {
        let bad = NoiseParameters {
            position_noise_m: 2.0,
            ..NoiseParameters::NONE
        };
        match NoiseEngine::new(bad, free(f64::NAN), 1) {
            Err(NoiseError::InvalidParameters(errors)) => {
                let fields: Vec<_> = errors.iter().map(|e| e.field).collect();
                assert_eq!(
                    fields,
                    [
                        "noise.max_position_offset_m",
                        "noise.max_offset_rate_mps",
                        "movement.max_speed_mps"
                    ]
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        let zero_radius = NoiseConstraints {
            max_speed_mps: 1.0,
            boundary: Some(Boundary {
                center: origin(),
                radius_m: 0.0,
            }),
        };
        assert!(NoiseEngine::new(NoiseParameters::NONE, zero_radius, 1).is_err());
    }

    #[test]
    fn projection_into_disc() {
        let c = V2 { e: 1.0, n: 1.0 };
        let inside = V2 { e: 1.5, n: 1.0 };
        assert_eq!(project_into_disc(inside, c, 1.0), inside);
        let p = project_into_disc(V2 { e: 4.0, n: 5.0 }, c, 2.5);
        assert!(((p.e - 1.0).hypot(p.n - 1.0) - 2.5).abs() < 1e-12);
        assert!((p.e - 2.5).abs() < 1e-12 && (p.n - 3.0).abs() < 1e-12);
        assert_eq!(project_into_disc(c, c, 0.0), c);
    }

    #[test]
    fn projection_into_lens_is_the_nearest_point_of_the_intersection() {
        let (c1, r1) = (V2 { e: 0.0, n: 0.0 }, 5.0);
        let (c2, r2) = (V2 { e: 6.0, n: 0.0 }, 5.0);
        let project = |e, n| project_into_lens(V2 { e, n }, c1, r1, c2, r2).unwrap();
        // Already inside both.
        assert_eq!(project(3.0, 1.0), V2 { e: 3.0, n: 1.0 });
        // Nearest point is on one arc.
        let p = project(-4.0, 0.0);
        assert!((p.e - 1.0).abs() < 1e-12 && p.n.abs() < 1e-12);
        // Nearest point is a corner of the lens: (3, ±4).
        let p = project(3.0, 40.0);
        assert!((p.e - 3.0).abs() < 1e-12 && (p.n - 4.0).abs() < 1e-12);
        let p = project(20.0, -30.0);
        assert!((p.e - 3.0).abs() < 1e-12 && (p.n + 4.0).abs() < 1e-12);

        // Brute force: no point of the intersection is closer than the result.
        let mut rng = Rng::from_seed(4);
        for _ in 0..200 {
            let q = V2 {
                e: rng.uniform(-15.0, 20.0),
                n: rng.uniform(-15.0, 15.0),
            };
            let best = project_into_lens(q, c1, r1, c2, r2).unwrap();
            assert!(in_disc(best, c1, r1 + 1e-9) && in_disc(best, c2, r2 + 1e-9));
            let best_d = (best.e - q.e).hypot(best.n - q.n);
            for _ in 0..500 {
                let x = V2 {
                    e: rng.uniform(1.0, 5.0),
                    n: rng.uniform(-4.0, 4.0),
                };
                if in_disc(x, c1, r1) && in_disc(x, c2, r2) {
                    assert!((x.e - q.e).hypot(x.n - q.n) >= best_d - 1e-9);
                }
            }
        }

        // Disjoint or concentric discs have no answer.
        let far = V2 { e: 100.0, n: 0.0 };
        assert_eq!(project_into_lens(c1, c1, 1.0, far, 1.0), None);
        assert_eq!(
            project_into_lens(far, c1, 1.0, c1, 2.0),
            Some(V2 { e: 1.0, n: 0.0 })
        );
    }

    #[test]
    fn drift_moves_at_its_rate_and_stays_in_its_disc() {
        let mut d = Drift {
            rate_mps: 0.5,
            radius_m: 10.0,
            position: V2 { e: 0.0, n: 0.0 },
            target: None,
            rng: Rng::from_seed(1),
        };
        let mut travelled = 0.0;
        for _ in 0..10_000 {
            let before = d.position;
            d.advance(1.0);
            let step = (d.position.e - before.e).hypot(d.position.n - before.n);
            // Straight legs cover exactly rate·dt; a turn at a waypoint less.
            assert!(step <= 0.5 + 1e-12);
            assert!(d.position.e.hypot(d.position.n) <= 10.0 + 1e-9);
            travelled += step;
        }
        assert!(travelled > 0.9 * 5_000.0, "travelled {travelled}");
        // A huge step is bounded work and still ends inside the disc.
        d.advance(1e12);
        assert!(d.position.e.hypot(d.position.n) <= 10.0 + 1e-9);
    }
}
