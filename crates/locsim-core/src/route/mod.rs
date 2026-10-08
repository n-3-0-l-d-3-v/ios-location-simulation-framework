//! Route engine: turning a recorded [`Route`](crate::domain::Route) into a
//! trajectory that can be replayed under a scenario's movement limits.
//!
//! Three separate things happen to a route, and they are kept separate:
//!
//! 1. **Structural validation** — `domain::Route::new`. Is this a recording
//!    at all? (enough points, increasing times, finite values)
//! 2. **Admission** — [`admit`] / [`RoutePlan::violations`], in this module.
//!    Does the trajectory the recording describes respect the movement
//!    limits *everywhere*, not just at the recorded points?
//! 3. **Final per-sample validation** — `validation::SampleValidator`, after
//!    noise, exactly as for every other movement model. Admission is not a
//!    licence to skip it: the gate knows nothing about routes.
//!
//! # Interpolation contract
//!
//! Recorded points are joined by a cubic Hermite spline in Earth-centred
//! Cartesian (ECEF) coordinates, evaluated at the requested time and dropped
//! onto the ellipsoid with the geographic engine's own conversion. Latitude
//! and longitude are never interpolated directly, so the antimeridian and
//! the poles need no special cases.
//!
//! * **Through the points.** The trajectory passes exactly through every
//!   recorded coordinate at exactly its recorded time.
//! * **Velocity is continuous.** The velocity at a recorded point is the
//!   three-point estimate from its neighbours (each neighbouring leg's mean
//!   velocity weighted by the other leg's duration), kept tangent to the
//!   ground. Both segments meeting at the point use that same vector, so
//!   speed and course have no jumps there.
//! * **At rest at the ends.** An open route starts from rest at its first
//!   point and comes to rest at its last. The first and last legs are
//!   therefore an acceleration and a braking leg; a recording that begins or
//!   ends at speed will usually exceed the acceleration limits there and be
//!   rejected.
//! * **Waits.** Two consecutive points with the same coordinate are a wait:
//!   the trajectory is at rest for that leg and arrives and leaves at rest.
//! * **Speed and course are derived, not recorded.** They are the analytic
//!   derivative of the interpolated position — the magnitude and the true
//!   bearing of the velocity vector at the interpolated point. Recorded
//!   speed or course values are not part of a route.
//! * **Altitude** is interpolated linearly in time between recorded
//!   altitudes, or is the scenario's altitude when the route has none.
//!
//! The trajectory is a function of played time alone. Sampling it at a
//! different interval reads different instants of the same curve; it never
//! changes the curve.
//!
//! # Playback
//!
//! Playback speed divides every recorded time (rounded to the nanosecond);
//! reverse playback reads the route from its last point to its first. Both
//! happen before interpolation, so the result is validated like any route.
//!
//! # Completion and looping
//!
//! An open route holds its final point, at rest, from its end time onwards
//! and reports itself complete. There is no extrapolation. Before the start
//! it holds the first point.
//!
//! A route may loop only if it is closed (last coordinate exactly equal to
//! the first). Then the velocity is carried across the seam like at any
//! other point and the seam is validated like any other point. An open route
//! with looping requested is rejected: it would jump back to its start.
//!
//! # Why admission can promise anything
//!
//! On each segment speed, tangential acceleration and turn rate are
//! polynomials or ratios of polynomials in the segment parameter. Their
//! suprema are bounded rigorously (Bernstein coefficients plus bisection, see
//! `poly`), so an admitted route stays within its limits between samples too,
//! at any sampling interval. Admission keeps the same
//! [`KINEMATIC_MARGIN`](crate::domain::KINEMATIC_MARGIN) below each limit as
//! the generated movement models do.

mod admission;
mod plan;
mod poly;
#[cfg(test)]
mod tests;

pub use admission::{
    admit, RouteConstraint, RouteLimits, RouteRejection, RouteViolation, COORDINATE_RESOLUTION_M,
    COURSE_RESOLUTION_DEG,
};
pub use plan::{Peak, PlanError, RoutePlan, RouteState, SegmentKinematics, MAX_SEGMENT_CHORD_M};
