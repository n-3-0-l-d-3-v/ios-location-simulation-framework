//! Route admission: may this route be replayed under these movement limits?

use super::plan::{PlanError, RoutePlan, SegmentKinematics};
use super::poly::{dot_poly, supremum};
use crate::domain::{Boundary, Scenario, KINEMATIC_MARGIN};
use crate::geographic::{bearing_difference, geodetic_to_ecef, wgs84};
use std::fmt;

/// The movement limits a replayed route must respect. The same quantities
/// the final validation gate enforces on every emitted sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteLimits {
    pub max_speed_mps: f64,
    /// `(cap in metres, update interval in seconds)`, when the scenario caps
    /// the displacement per sample.
    pub displacement_cap: Option<(f64, f64)>,
    pub max_acceleration_mps2: f64,
    pub max_deceleration_mps2: f64,
    pub max_heading_rate_dps: f64,
    pub boundary: Option<Boundary>,
}

impl RouteLimits {
    pub fn for_scenario(scenario: &Scenario) -> Self {
        let m = &scenario.movement;
        Self {
            max_speed_mps: m.max_speed_mps,
            displacement_cap: m
                .max_displacement_per_sample_m
                .map(|cap| (cap, scenario.update_interval_s)),
            max_acceleration_mps2: m.max_acceleration_mps2,
            max_deceleration_mps2: m.max_deceleration_mps2,
            max_heading_rate_dps: m.max_heading_rate_dps,
            boundary: scenario.boundary(),
        }
    }
}

/// Which limit a route breaks. Units of `observed` and `limit` in a
/// [`RouteViolation`] follow the constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteConstraint {
    /// Peak speed, m/s.
    MaxSpeed,
    /// Peak distance covered in one update interval, metres.
    DisplacementPerSample,
    /// Peak rate of increase of speed, m/s².
    Acceleration,
    /// Peak rate of decrease of speed, m/s².
    Deceleration,
    /// Peak turn rate while moving, degrees per second.
    HeadingRate,
    /// Change of direction across a stop, degrees; the limit is what the
    /// heading-rate limit allows during the time spent stopped.
    TurnAtStop,
    /// Greatest distance from the boundary centre, metres.
    Boundary,
}

/// One broken limit, located.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteViolation {
    /// Index of the route leg (between points `segment` and `segment + 1`,
    /// in the route's own order). For [`RouteConstraint::TurnAtStop`], the
    /// leg that arrives at the stop.
    pub segment: usize,
    pub constraint: RouteConstraint,
    pub observed: f64,
    pub limit: f64,
    /// Played time at which the peak occurs.
    pub at_elapsed_s: f64,
}

impl fmt::Display for RouteViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (what, unit) = match self.constraint {
            RouteConstraint::MaxSpeed => ("speed", "m/s"),
            RouteConstraint::DisplacementPerSample => ("displacement per sample", "m"),
            RouteConstraint::Acceleration => ("acceleration", "m/s^2"),
            RouteConstraint::Deceleration => ("deceleration", "m/s^2"),
            RouteConstraint::HeadingRate => ("heading rate", "deg/s"),
            RouteConstraint::TurnAtStop => ("turn at a stop", "deg"),
            RouteConstraint::Boundary => ("distance from the boundary centre", "m"),
        };
        write!(
            f,
            "segment {}: {what} reaches {} {unit} at {:.3} s, limit is {} {unit}",
            self.segment, self.observed, self.at_elapsed_s, self.limit
        )
    }
}

/// Why a route was not admitted for replay.
#[derive(Debug, Clone, PartialEq)]
pub enum RouteRejection {
    /// No trajectory could be built at all.
    Plan(PlanError),
    /// The trajectory exists but breaks movement limits. Never empty.
    Violations(Vec<RouteViolation>),
}

impl fmt::Display for RouteRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouteRejection::Plan(e) => write!(f, "route rejected: {e}"),
            RouteRejection::Violations(violations) => {
                write!(f, "route rejected, {} violation(s):", violations.len())?;
                for v in violations {
                    write!(f, " [{v}]")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for RouteRejection {}

impl From<PlanError> for RouteRejection {
    fn from(e: PlanError) -> Self {
        RouteRejection::Plan(e)
    }
}

/// `observed` is acceptable if it stays the kinematic margin below `limit`.
/// Written so that NaN is a violation.
fn exceeds(observed: f64, limit: f64) -> bool {
    observed.is_nan() || limit.is_nan() || observed > limit * (1.0 - KINEMATIC_MARGIN)
}

impl RoutePlan {
    /// Every way the trajectory breaks `limits`, segment by segment. Empty
    /// means the route is admissible.
    ///
    /// Each comparison is on a supremum over the whole segment, so a route
    /// that passes cannot exceed a limit between two samples whatever the
    /// sampling interval. Ground speed and turn rate are inflated by the
    /// segment's `surface_scale` first (see [`SegmentKinematics`]).
    pub fn violations(&self, limits: &RouteLimits) -> Vec<RouteViolation> {
        let mut out = Vec::new();
        for k in self.kinematics() {
            if k.stationary {
                continue;
            }
            let mut check = |constraint, observed: f64, limit: f64, at_elapsed_s: f64| {
                if exceeds(observed, limit) {
                    out.push(RouteViolation {
                        segment: k.segment,
                        constraint,
                        observed,
                        limit,
                        at_elapsed_s,
                    });
                }
            };
            let speed = k.peak_speed_mps.value * k.surface_scale;
            check(
                RouteConstraint::MaxSpeed,
                speed,
                limits.max_speed_mps,
                k.peak_speed_mps.at_elapsed_s,
            );
            if let Some((cap_m, interval_s)) = limits.displacement_cap {
                check(
                    RouteConstraint::DisplacementPerSample,
                    speed * interval_s,
                    cap_m,
                    k.peak_speed_mps.at_elapsed_s,
                );
            }
            check(
                RouteConstraint::Acceleration,
                k.peak_acceleration_mps2.value,
                limits.max_acceleration_mps2,
                k.peak_acceleration_mps2.at_elapsed_s,
            );
            check(
                RouteConstraint::Deceleration,
                k.peak_deceleration_mps2.value,
                limits.max_deceleration_mps2,
                k.peak_deceleration_mps2.at_elapsed_s,
            );
            check(
                RouteConstraint::HeadingRate,
                k.peak_heading_rate_dps.value * k.surface_scale * k.surface_scale,
                limits.max_heading_rate_dps,
                k.peak_heading_rate_dps.at_elapsed_s,
            );
        }
        self.turns_at_stops(limits, &mut out);
        if let Some(boundary) = limits.boundary {
            self.boundary_violations(boundary, &mut out);
        }
        out.sort_by_key(|v| v.segment);
        out
    }

    /// At every point where the trajectory is at rest between two moving
    /// segments, the direction it leaves in may differ from the direction it
    /// arrived in only by what the heading-rate limit allows during the wait.
    ///
    /// Without this an about-turn at zero speed would be admitted: speed and
    /// heading rate are both fine on either side, yet two samples just
    /// before and after the stop show an instantaneous reversal.
    fn turns_at_stops(&self, limits: &RouteLimits, out: &mut Vec<RouteViolation>) {
        let kin = self.kinematics();
        let count = kin.len();
        let moving: Vec<usize> = (0..count).filter(|j| !kin[*j].stationary).collect();
        if moving.is_empty() {
            return;
        }
        let mut pairs: Vec<(usize, usize, bool)> =
            moving.windows(2).map(|w| (w[0], w[1], false)).collect();
        if self.is_looping() {
            pairs.push((moving[moving.len() - 1], moving[0], true));
        }
        for (before, after, wraps) in pairs {
            // Knot `before + 1` ends the arriving segment. If the mover is
            // not at rest there, velocity is continuous and there is no stop.
            if !self.knot_is_at_rest(before + 1) {
                continue;
            }
            let (arrive, leave) = (&kin[before], &kin[after]);
            let (Some(from), Some(to)) = (arrive.exit_bearing_deg, leave.entry_bearing_deg) else {
                continue;
            };
            let arrival_s = arrive.start_elapsed_s + arrive.duration_s;
            let wait_s = if wraps {
                self.duration_s() - arrival_s + leave.start_elapsed_s
            } else {
                leave.start_elapsed_s - arrival_s
            };
            let turn = bearing_difference(from, to).abs();
            let allowed = limits.max_heading_rate_dps * wait_s;
            // The 1e-9° covers the rounding of two bearings that should be equal.
            if turn.is_nan() || turn > allowed * (1.0 - KINEMATIC_MARGIN) + 1e-9 {
                out.push(RouteViolation {
                    segment: arrive.segment,
                    constraint: RouteConstraint::TurnAtStop,
                    observed: turn,
                    limit: allowed,
                    at_elapsed_s: arrival_s,
                });
            }
        }
    }

    fn boundary_violations(&self, boundary: Boundary, out: &mut Vec<RouteViolation>) {
        let centre = geodetic_to_ecef(boundary.center, 0.0);
        for j in 0..self.segment_count() {
            let k: &SegmentKinematics = &self.kinematics()[j];
            // Squared straight-line distance from the centre: a polynomial of
            // degree 6 along the segment, so its supremum is rigorous too.
            let offset = self.offset_coefficients(j, centre);
            let squared = dot_poly(&offset, &offset);
            let farthest = supremum(
                &|a, b| squared.range(a, b).1.max(0.0).sqrt(),
                &|x| squared.eval(x).max(0.0).sqrt(),
                1e-9,
            );
            // A chord is shorter than the arc over it; (d/R)² is a generous
            // bound on the relative difference (the true one is d²/24R²).
            let chord = farthest.value;
            let observed = chord * (1.0 + (chord / wgs84::MIN_CURVATURE_RADIUS).powi(2));
            if exceeds(observed, boundary.radius_m) {
                out.push(RouteViolation {
                    segment: k.segment,
                    constraint: RouteConstraint::Boundary,
                    observed,
                    limit: boundary.radius_m,
                    at_elapsed_s: k.start_elapsed_s + farthest.at * k.duration_s,
                });
            }
        }
    }
}

/// Builds the trajectory for a scenario's route and admits it only if it
/// respects the scenario's movement limits everywhere.
pub fn admit(scenario: &Scenario) -> Result<RoutePlan, RouteRejection> {
    let route = scenario
        .route
        .as_ref()
        .ok_or(RouteRejection::Plan(PlanError::MissingRoute))?;
    let plan = RoutePlan::build(route, &scenario.playback, scenario.altitude_m)?;
    let violations = plan.violations(&RouteLimits::for_scenario(scenario));
    if violations.is_empty() {
        Ok(plan)
    } else {
        Err(RouteRejection::Violations(violations))
    }
}
