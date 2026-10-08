//! The continuous trajectory a route is replayed as, and its kinematics.

use super::poly::{add, cross, dot, dot_poly, norm, scale, sub, supremum, Poly, Vec3};
use crate::domain::{Coordinate, PlaybackParameters, Route};
use crate::geographic::{
    ecef_to_geodetic, geodetic_to_ecef, local_axes, normalize_bearing, wgs84, GeoError,
};
use std::fmt;

/// Longest chord a single segment may span. The interpolant is a curve in
/// space pulled down onto the ellipsoid; beyond this it sags too far below
/// the surface for the speed bounds to stay meaningful.
pub const MAX_SEGMENT_CHORD_M: f64 = 100_000.0;

/// Why a route cannot be turned into a trajectory at all, independent of
/// any movement limits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanError {
    /// Looping was requested for a route whose last point is not its first.
    OpenRouteCannotLoop,
    /// A leg is longer than [`MAX_SEGMENT_CHORD_M`]. `segment` indexes the
    /// route's legs.
    SegmentTooLong {
        segment: usize,
        length_m: f64,
        limit_m: f64,
    },
    /// Playback speed compresses a leg to less than a nanosecond.
    SegmentCollapsed {
        segment: usize,
    },
    InvalidPlaybackSpeed(f64),
    NonFiniteAltitude,
    /// The scenario has no route.
    MissingRoute,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::OpenRouteCannotLoop => write!(
                f,
                "an open route cannot loop: its last point is not its first"
            ),
            PlanError::SegmentTooLong {
                segment,
                length_m,
                limit_m,
            } => write!(
                f,
                "route segment {segment} spans {length_m} m, more than the {limit_m} m supported"
            ),
            PlanError::SegmentCollapsed { segment } => write!(
                f,
                "route segment {segment} lasts less than 1 ns at this playback speed"
            ),
            PlanError::InvalidPlaybackSpeed(v) => {
                write!(f, "playback speed {v} is not finite and positive")
            }
            PlanError::NonFiniteAltitude => write!(f, "default altitude is not finite"),
            PlanError::MissingRoute => write!(f, "the scenario has no route"),
        }
    }
}

impl std::error::Error for PlanError {}

/// A peak value on a segment and when (played time) it occurs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peak {
    pub value: f64,
    pub at_elapsed_s: f64,
}

/// Kinematics of one segment of the replayed trajectory.
///
/// Peaks are suprema over the whole segment, not samples: each is an upper
/// bound within a relative 1e-7 of the true maximum (or infinite if the
/// quantity is unbounded, e.g. the heading rate where the trajectory reverses
/// through zero speed between two recorded points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentKinematics {
    /// Index of the route leg this segment replays (in the route's own
    /// order, also under reverse playback).
    pub segment: usize,
    /// Played time at which the segment starts.
    pub start_elapsed_s: f64,
    pub duration_s: f64,
    /// Straight-line distance between the two recorded points.
    pub chord_m: f64,
    /// `chord / duration`.
    pub mean_speed_mps: f64,
    pub stationary: bool,
    pub peak_speed_mps: Peak,
    /// Largest rate of increase of speed.
    pub peak_acceleration_mps2: Peak,
    /// Largest rate of decrease of speed (as a positive number).
    pub peak_deceleration_mps2: Peak,
    pub peak_heading_rate_dps: Peak,
    /// Direction of travel on leaving the first point; `None` if stationary.
    pub entry_bearing_deg: Option<f64>,
    /// Direction of travel on reaching the second point.
    pub exit_bearing_deg: Option<f64>,
    /// Factor by which ground speed may exceed the interpolant's speed
    /// because the curve runs slightly below the surface: `1 + (L/R)²` for a
    /// segment of extent `L`. 1 + 2.5e-8 for a 1 km segment.
    pub surface_scale: f64,
}

#[derive(Debug, Clone, Copy)]
struct Knot {
    t_ns: i64,
    coordinate: Coordinate,
    x: Vec3,
    /// Velocity, tangent to the ellipsoid, in ECEF m/s.
    v: Vec3,
    altitude_m: f64,
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    /// Duration in seconds.
    h: f64,
    /// Position = c0 + c1 τ + c2 τ² + c3 τ³ for τ ∈ [0, 1], in ECEF metres.
    c: [Vec3; 4],
    /// The same cubic read backwards, `p(1 − σ)`, with its (vanishing)
    /// linear term exactly zero. Only meaningful when `rest_at_end`.
    mirror: [Vec3; 4],
    stationary: bool,
    rest_at_start: bool,
    rest_at_end: bool,
}

/// State of the replayed trajectory at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteState {
    pub coordinate: Coordinate,
    pub altitude_m: f64,
    pub speed_mps: f64,
    /// Direction of travel; `None` while at rest.
    pub course_deg: Option<f64>,
    /// Rate of change of speed.
    pub acceleration_mps2: f64,
    /// Turn rate, positive clockwise.
    pub heading_rate_dps: f64,
    /// True once an open route has reached its final point.
    pub complete: bool,
}

/// A route turned into a continuous trajectory: position, speed and course
/// as closed-form functions of played time. See the module documentation
/// for the interpolation contract.
#[derive(Debug, Clone)]
pub struct RoutePlan {
    knots: Vec<Knot>,
    segments: Vec<Segment>,
    kinematics: Vec<SegmentKinematics>,
    looping: bool,
    total_ns: i64,
}

const ZERO: Vec3 = [0.0; 3];

impl RoutePlan {
    /// Builds the trajectory for `route` as played back with `playback`.
    /// `default_altitude_m` is used when the route carries no altitudes.
    ///
    /// This checks only that a trajectory exists; it does not look at any
    /// movement limit. Use [`super::admit`] to decide whether the route may
    /// be replayed.
    pub fn build(
        route: &Route,
        playback: &PlaybackParameters,
        default_altitude_m: f64,
    ) -> Result<Self, PlanError> {
        if !playback.speed.is_finite() || playback.speed <= 0.0 {
            return Err(PlanError::InvalidPlaybackSpeed(playback.speed));
        }
        if !default_altitude_m.is_finite() {
            return Err(PlanError::NonFiniteAltitude);
        }
        if playback.looping && !route.is_closed() {
            return Err(PlanError::OpenRouteCannotLoop);
        }
        let points = route.points();
        let count = points.len();
        let legs = count - 1;
        let total_recorded = route.duration_ns();
        // Segment `j` of the played trajectory replays this leg of the route.
        let source = |j: usize| if playback.reverse { legs - 1 - j } else { j };

        // Played order and played time. Reverse playback is the same route
        // read backwards; playback speed compresses time. Knot times are
        // rounded to whole nanoseconds, which is then the definition of the
        // played route (not an error that accumulates).
        let mut knots: Vec<Knot> = Vec::with_capacity(count);
        for i in 0..count {
            let p = if playback.reverse {
                points[count - 1 - i]
            } else {
                points[i]
            };
            let recorded_ns = if playback.reverse {
                total_recorded - p.elapsed_ns
            } else {
                p.elapsed_ns
            };
            let t_ns = (recorded_ns as f64 / playback.speed).round() as i64;
            if let Some(previous) = knots.last() {
                if t_ns <= previous.t_ns {
                    return Err(PlanError::SegmentCollapsed {
                        segment: source(i - 1),
                    });
                }
            }
            knots.push(Knot {
                t_ns,
                coordinate: p.coordinate,
                x: geodetic_to_ecef(p.coordinate, 0.0),
                v: ZERO,
                altitude_m: p.altitude_m.unwrap_or(default_altitude_m),
            });
        }

        let duration: Vec<f64> = knots
            .windows(2)
            .map(|w| (w[1].t_ns - w[0].t_ns) as f64 / 1e9)
            .collect();
        let chord: Vec<Vec3> = knots.windows(2).map(|w| sub(w[1].x, w[0].x)).collect();
        // A leg between two identical coordinates is a wait.
        let stationary: Vec<bool> = knots
            .windows(2)
            .map(|w| w[0].coordinate == w[1].coordinate)
            .collect();
        for (j, d) in chord.iter().enumerate() {
            let length_m = norm(*d);
            if length_m > MAX_SEGMENT_CHORD_M {
                return Err(PlanError::SegmentTooLong {
                    segment: source(j),
                    length_m,
                    limit_m: MAX_SEGMENT_CHORD_M,
                });
            }
        }

        // Knot velocities. An open route starts and ends at rest; a loop
        // carries its velocity across the seam.
        let velocity_between = |before: usize, after: usize, at: &Knot| -> Vec3 {
            if stationary[before] || stationary[after] {
                // Arriving at or leaving a wait: at rest.
                return ZERO;
            }
            // Three-point (non-uniform Catmull-Rom) estimate: the two leg
            // velocities, each weighted by the other leg's duration.
            let (hb, ha) = (duration[before], duration[after]);
            let vb = scale(chord[before], 1.0 / hb);
            let va = scale(chord[after], 1.0 / ha);
            let v = scale(add(scale(vb, ha), scale(va, hb)), 1.0 / (hb + ha));
            // Chords cut through the Earth; keep only the part of the
            // velocity that lies in the ground plane at the knot.
            let up = local_axes(at.coordinate).up;
            sub(v, scale(up, dot(v, up)))
        };
        let interior: Vec<Vec3> = (1..count - 1)
            .map(|k| velocity_between(k - 1, k, &knots[k]))
            .collect();
        for (knot, v) in knots[1..count - 1].iter_mut().zip(interior) {
            knot.v = v;
        }
        if playback.looping {
            let v = velocity_between(legs - 1, 0, &knots[0]);
            knots[0].v = v;
            knots[count - 1].v = v;
        }

        let segments: Vec<Segment> = (0..legs)
            .map(|j| {
                let h = duration[j];
                let (v0, v1) = (scale(knots[j].v, h), scale(knots[j + 1].v, h));
                let d = chord[j];
                // Cubic Hermite in power form.
                let c = [
                    knots[j].x,
                    v0,
                    sub(sub(scale(d, 3.0), scale(v0, 2.0)), v1),
                    add(add(scale(d, -2.0), v0), v1),
                ];
                Segment {
                    h,
                    c,
                    mirror: mirrored_at_rest(c),
                    stationary: stationary[j],
                    rest_at_start: knots[j].v == ZERO,
                    rest_at_end: knots[j + 1].v == ZERO,
                }
            })
            .collect();

        let kinematics = (0..legs)
            .map(|j| analyse(source(j), &segments[j], &knots[j], &knots[j + 1]))
            .collect();

        Ok(Self {
            total_ns: knots[count - 1].t_ns,
            knots,
            segments,
            kinematics,
            looping: playback.looping,
        })
    }

    /// Per-segment kinematics, in played order.
    pub fn kinematics(&self) -> &[SegmentKinematics] {
        &self.kinematics
    }

    pub fn is_looping(&self) -> bool {
        self.looping
    }

    /// Length of one pass in played time.
    pub fn duration_ns(&self) -> i64 {
        self.total_ns
    }

    pub fn duration_s(&self) -> f64 {
        self.total_ns as f64 / 1e9
    }

    pub(super) fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Coefficients of segment `j` relative to a point `origin` in ECEF.
    pub(super) fn offset_coefficients(&self, j: usize, origin: Vec3) -> [Vec3; 4] {
        let c = self.segments[j].c;
        [sub(c[0], origin), c[1], c[2], c[3]]
    }

    pub(super) fn knot_is_at_rest(&self, k: usize) -> bool {
        self.knots[k].v == ZERO
    }

    /// Trajectory state `elapsed_ns` of played time after the route starts.
    ///
    /// * Before the start (negative time) and exactly at it: the first point,
    ///   at rest for an open route.
    /// * Between recorded points: the interpolant.
    /// * Exactly at a recorded point's time: exactly that point's coordinate.
    /// * At and after the end of an open route: the final point, at rest,
    ///   with `complete` set. There is no extrapolation.
    /// * A looping route wraps: time is taken modulo the lap duration.
    pub fn state_at(&self, elapsed_ns: i64) -> Result<RouteState, GeoError> {
        let last = self.knots.len() - 1;
        let (t_ns, complete) = if self.looping {
            (elapsed_ns.rem_euclid(self.total_ns), false)
        } else if elapsed_ns >= self.total_ns {
            (self.total_ns, true)
        } else {
            (elapsed_ns.max(0), false)
        };

        // Index of the segment containing `t_ns`; an exact hit on a recorded
        // time returns the recorded point itself.
        let j = match self.knots.binary_search_by(|k| k.t_ns.cmp(&t_ns)) {
            Ok(k) => {
                // Acceleration just after the point (just before, at the end).
                let (segment, tau) = if k == last { (k - 1, 1.0) } else { (k, 0.0) };
                return Ok(self.at_knot(k, &self.segments[segment], tau, complete));
            }
            Err(insert) => insert - 1,
        };
        let seg = &self.segments[j];
        let (k0, k1) = (&self.knots[j], &self.knots[j + 1]);
        let span = (k1.t_ns - k0.t_ns) as f64;
        // Both fractions straight from the integer times: near the end of a
        // segment `1 − tau` would lose the last nanoseconds to rounding.
        let tau = (t_ns - k0.t_ns) as f64 / span;
        let sigma = (k1.t_ns - t_ns) as f64 / span;
        let altitude_m = k0.altitude_m + (k1.altitude_m - k0.altitude_m) * tau;
        if seg.stationary {
            return Ok(resting(k0.coordinate, altitude_m, complete));
        }
        let c = seg.c;
        let position = add(
            c[0],
            scale(add(c[1], scale(add(c[2], scale(c[3], tau)), tau)), tau),
        );
        let (coordinate, _) = ecef_to_geodetic(position)?;
        Ok(moving(
            coordinate,
            altitude_m,
            motion(seg, tau, sigma),
            complete,
        ))
    }

    fn at_knot(&self, k: usize, seg: &Segment, tau: f64, complete: bool) -> RouteState {
        let knot = &self.knots[k];
        if knot.v == ZERO {
            return resting(knot.coordinate, knot.altitude_m, complete);
        }
        // The stored knot velocity, not the polynomial's value, so the two
        // segments meeting here report exactly the same speed and course.
        let acceleration = scale(
            add(scale(seg.c[2], 2.0), scale(seg.c[3], 6.0 * tau)),
            1.0 / (seg.h * seg.h),
        );
        let speed = norm(knot.v);
        let motion = Motion {
            speed,
            direction: knot.v,
            acceleration: dot(knot.v, acceleration) / speed,
            rotation: scale(cross(knot.v, acceleration), 1.0 / (speed * speed)),
        };
        moving(knot.coordinate, knot.altitude_m, motion, complete)
    }
}

/// Velocity-level description of the trajectory at one instant.
struct Motion {
    speed: f64,
    /// Any positive multiple of the velocity vector.
    direction: Vec3,
    /// Rate of change of speed, m/s².
    acceleration: f64,
    /// `V × A / |V|²` in rad/s: its component along the local vertical is
    /// the (counter-clockwise) turn rate.
    rotation: Vec3,
}

/// Speed, direction, tangential acceleration and rotation on a moving
/// segment, at `tau` from its start (`sigma = 1 − tau` from its end).
///
/// Near a point where the trajectory is at rest the velocity `V` is the
/// difference of terms that nearly cancel, and both `V·A/|V|` and
/// `V × A/|V|²` divide by it. There the factored forms are used instead —
/// `V = τ·U` leaving rest, `V = −σ·Ũ` arriving at rest — in which the small
/// factor cancels analytically (see [`suprema`] for the algebra). They are
/// the same functions, evaluated without the cancellation.
fn motion(seg: &Segment, tau: f64, sigma: f64) -> Motion {
    let h = seg.h;
    // At rest at both ends: use whichever end is nearer.
    if seg.rest_at_start && (!seg.rest_at_end || tau <= 0.5) {
        let c = seg.c;
        let u = add(scale(c[2], 2.0), scale(c[3], 3.0 * tau));
        let a = add(scale(c[2], 2.0), scale(c[3], 6.0 * tau));
        let size = norm(u);
        if size == 0.0 {
            return at_rest();
        }
        Motion {
            speed: tau * size / h,
            direction: u,
            acceleration: dot(u, a) / (h * h * size),
            rotation: scale(cross(c[2], c[3]), 6.0 / (h * size * size)),
        }
    } else if seg.rest_at_end {
        let m = seg.mirror;
        let u = add(scale(m[2], 2.0), scale(m[3], 3.0 * sigma));
        let a = add(scale(m[2], 2.0), scale(m[3], 6.0 * sigma));
        let size = norm(u);
        if size == 0.0 {
            return at_rest();
        }
        // Read backwards the velocity is σ·Ũ, so forwards it is −σ·Ũ; the
        // second derivative is the same either way.
        Motion {
            speed: sigma * size / h,
            direction: scale(u, -1.0),
            acceleration: -dot(u, a) / (h * h * size),
            rotation: scale(cross(m[2], m[3]), -6.0 / (h * size * size)),
        }
    } else {
        let c = seg.c;
        let v = scale(
            add(
                c[1],
                scale(add(scale(c[2], 2.0), scale(c[3], 3.0 * tau)), tau),
            ),
            1.0 / h,
        );
        let a = scale(add(scale(c[2], 2.0), scale(c[3], 6.0 * tau)), 1.0 / (h * h));
        let speed = norm(v);
        if speed == 0.0 {
            return at_rest();
        }
        Motion {
            speed,
            direction: v,
            acceleration: dot(v, a) / speed,
            rotation: scale(cross(v, a), 1.0 / (speed * speed)),
        }
    }
}

fn at_rest() -> Motion {
    Motion {
        speed: 0.0,
        direction: ZERO,
        acceleration: 0.0,
        rotation: ZERO,
    }
}

fn resting(coordinate: Coordinate, altitude_m: f64, complete: bool) -> RouteState {
    RouteState {
        coordinate,
        altitude_m,
        speed_mps: 0.0,
        course_deg: None,
        acceleration_mps2: 0.0,
        heading_rate_dps: 0.0,
        complete,
    }
}

fn moving(coordinate: Coordinate, altitude_m: f64, motion: Motion, complete: bool) -> RouteState {
    if motion.speed == 0.0 {
        return resting(coordinate, altitude_m, complete);
    }
    let axes = local_axes(coordinate);
    let course = dot(motion.direction, axes.east).atan2(dot(motion.direction, axes.north));
    RouteState {
        coordinate,
        altitude_m,
        speed_mps: motion.speed,
        course_deg: Some(normalize_bearing(course.to_degrees())),
        acceleration_mps2: motion.acceleration,
        // Clockwise seen from above is positive, hence the sign.
        heading_rate_dps: (-dot(axes.up, motion.rotation)).to_degrees(),
        complete,
    }
}

fn bearing_of(direction: Vec3, at: Coordinate) -> Option<f64> {
    if norm(direction) == 0.0 {
        return None;
    }
    let axes = local_axes(at);
    let bearing = dot(direction, axes.east).atan2(dot(direction, axes.north));
    Some(normalize_bearing(bearing.to_degrees()))
}

/// Mirror image of a cubic: coefficients of `p(1 − s)`.
fn mirrored(c: [Vec3; 4]) -> [Vec3; 4] {
    [
        add(add(c[0], c[1]), add(c[2], c[3])),
        scale(add(c[1], add(scale(c[2], 2.0), scale(c[3], 3.0))), -1.0),
        add(c[2], scale(c[3], 3.0)),
        scale(c[3], -1.0),
    ]
}

/// Suprema of speed, acceleration, deceleration and heading rate over one
/// segment. Speeds are in m/s, rates in rad/s here; `at` is the parameter.
struct Suprema {
    speed: (f64, f64),
    acceleration: (f64, f64),
    deceleration: (f64, f64),
    heading_rate: (f64, f64),
}

/// The general case, and the case of a segment that starts at rest.
///
/// With `V = dP/dτ` and `A = d²P/dτ²` (so velocity is `V/h`):
///
/// * speed            = |V| / h
/// * tangential accel = (V·A) / (h² |V|)
/// * heading rate     = |V × A| / (h |V|²)
///
/// When the segment starts at rest `V = τ·W`, and the common factor cancels:
/// the same formulas hold with `W` in place of `V` in the two ratios and
/// `V × A = 6τ²(c₂ × c₃)` reduced to the constant `6(c₂ × c₃)`. Without that
/// cancellation both ratios would be 0/0 at the start.
fn suprema(c: [Vec3; 4], h: f64, starts_at_rest: bool) -> Suprema {
    let v = [c[1], scale(c[2], 2.0), scale(c[3], 3.0)];
    let a = [scale(c[2], 2.0), scale(c[3], 6.0)];
    let speed_squared = dot_poly(&v, &v);

    let (denominator, tangential, turning): (Poly, Poly, Poly) = if starts_at_rest {
        let w = [scale(c[2], 2.0), scale(c[3], 3.0)];
        let k = scale(cross(c[2], c[3]), 6.0);
        (dot_poly(&w, &w), dot_poly(&w, &a), Poly::new(&[dot(k, k)]))
    } else {
        let g = [
            scale(cross(c[1], c[2]), 2.0),
            scale(cross(c[1], c[3]), 6.0),
            scale(cross(c[2], c[3]), 6.0),
        ];
        (speed_squared, dot_poly(&v, &a), dot_poly(&g, &g))
    };

    let speed = supremum(
        &|x, y| speed_squared.range(x, y).1.max(0.0).sqrt() / h,
        &|x| speed_squared.eval(x).max(0.0).sqrt() / h,
        1e-12,
    );
    // `sign` selects acceleration (+1) or deceleration (−1).
    let tangential_supremum = |sign: f64| {
        supremum(
            &|x, y| {
                let (low, high) = tangential.range(x, y);
                let numerator = if sign > 0.0 { high } else { -low };
                if numerator <= 0.0 {
                    return 0.0;
                }
                let (den, _) = denominator.range(x, y);
                if den <= 0.0 {
                    f64::INFINITY
                } else {
                    numerator / (h * h * den.sqrt())
                }
            },
            &|x| {
                let den = denominator.eval(x);
                if den <= 0.0 {
                    0.0
                } else {
                    (sign * tangential.eval(x) / (h * h * den.sqrt())).max(0.0)
                }
            },
            1e-12,
        )
    };
    let acceleration = tangential_supremum(1.0);
    let deceleration = tangential_supremum(-1.0);
    let heading_rate = supremum(
        &|x, y| {
            let (_, high) = turning.range(x, y);
            if high <= 0.0 {
                return 0.0;
            }
            let (den, _) = denominator.range(x, y);
            if den <= 0.0 {
                f64::INFINITY
            } else {
                high.sqrt() / (h * den)
            }
        },
        &|x| {
            let den = denominator.eval(x);
            if den <= 0.0 {
                0.0
            } else {
                turning.eval(x).max(0.0).sqrt() / (h * den)
            }
        },
        1e-15,
    );
    Suprema {
        speed: (speed.value, speed.at),
        acceleration: (acceleration.value, acceleration.at),
        deceleration: (deceleration.value, deceleration.at),
        heading_rate: (heading_rate.value, heading_rate.at),
    }
}

fn analyse(source: usize, seg: &Segment, start: &Knot, end: &Knot) -> SegmentKinematics {
    let h = seg.h;
    let start_s = start.t_ns as f64 / 1e9;
    let chord = sub(end.x, start.x);
    let chord_m = norm(chord);
    let peak = |(value, tau): (f64, f64)| Peak {
        value,
        at_elapsed_s: start_s + tau * h,
    };
    let extent = chord_m + h * (norm(start.v) + norm(end.v));
    let mut out = SegmentKinematics {
        segment: source,
        start_elapsed_s: start_s,
        duration_s: h,
        chord_m,
        mean_speed_mps: chord_m / h,
        stationary: seg.stationary,
        peak_speed_mps: peak((0.0, 0.0)),
        peak_acceleration_mps2: peak((0.0, 0.0)),
        peak_deceleration_mps2: peak((0.0, 0.0)),
        peak_heading_rate_dps: peak((0.0, 0.0)),
        entry_bearing_deg: None,
        exit_bearing_deg: None,
        surface_scale: 1.0 + (extent / wgs84::MIN_CURVATURE_RADIUS).powi(2),
    };
    if seg.stationary {
        return out;
    }
    let c = seg.c;
    let (rest_at_start, rest_at_end) = (start.v == ZERO, end.v == ZERO);

    // Direction of travel just after the start and just before the end.
    // At rest the velocity vanishes and the direction is that of the
    // acceleration: V ≈ τ·A(0) leaving, V ≈ −(1 − τ)·A(1) arriving.
    let leaving = if rest_at_start { c[2] } else { start.v };
    let arriving = if rest_at_end {
        scale(add(scale(c[2], 2.0), scale(c[3], 6.0)), -1.0)
    } else {
        end.v
    };
    out.entry_bearing_deg = bearing_of(leaving, start.coordinate);
    out.exit_bearing_deg = bearing_of(arriving, end.coordinate);

    let s = match (rest_at_start, rest_at_end) {
        (true, true) => {
            // V = 6·chord·τ(1 − τ): a straight line in space, covered with
            // a parabolic speed profile. Everything is closed form.
            Suprema {
                speed: (1.5 * chord_m / h, 0.5),
                acceleration: (6.0 * chord_m / (h * h), 0.0),
                deceleration: (6.0 * chord_m / (h * h), 1.0),
                heading_rate: (0.0, 0.0),
            }
        }
        (false, true) => {
            // Read the segment backwards so that it starts at rest. Time
            // runs the other way, so acceleration and deceleration swap.
            let m = suprema(seg.mirror, h, true);
            let flip = |(value, tau): (f64, f64)| (value, 1.0 - tau);
            Suprema {
                speed: flip(m.speed),
                acceleration: flip(m.deceleration),
                deceleration: flip(m.acceleration),
                heading_rate: flip(m.heading_rate),
            }
        }
        (starts_at_rest, false) => suprema(c, h, starts_at_rest),
    };
    out.peak_speed_mps = peak(s.speed);
    out.peak_acceleration_mps2 = peak(s.acceleration);
    out.peak_deceleration_mps2 = peak(s.deceleration);
    out.peak_heading_rate_dps = peak((s.heading_rate.0.to_degrees(), s.heading_rate.1));
    out
}

/// [`mirrored`] with the linear term forced to exactly zero: it is the
/// (vanishing) end velocity, and rounding must not turn it into a tiny
/// non-zero vector that defeats the at-rest factorisation.
fn mirrored_at_rest(c: [Vec3; 4]) -> [Vec3; 4] {
    let mut m = mirrored(c);
    m[1] = ZERO;
    m
}
