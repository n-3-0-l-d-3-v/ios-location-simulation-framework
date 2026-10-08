//! Shared helpers for the route integration tests: random routes and the
//! tightest limits a given route can be admitted under.
#![allow(dead_code, unused_imports)]

mod recheck;

pub use recheck::{recheck_stream, Recheck};

use locsim_core::domain::{
    Boundary, Coordinate, MovementMode, MovementParameters, NoiseParameters, PlaybackParameters,
    RotationDirection, Route, RoutePoint, Scenario, CURRENT_SCHEMA_VERSION,
};
use locsim_core::geographic::destination;
use locsim_core::rng::Rng;
use locsim_core::route::{
    RouteConstraint, RouteLimits, RoutePlan, COORDINATE_RESOLUTION_M, COURSE_RESOLUTION_DEG,
};

pub const SEC: i64 = 1_000_000_000;
pub const START: i64 = 1_700_000_000_000_000_000;

/// The smallest factor above a route's own peaks at which admission must
/// accept it: the kinematic margin (1e-6) plus the certified-bound gap
/// (1e-7), with a little room.
pub const TIGHT: f64 = 1.0 + 3e-6;

pub fn log_uniform(rng: &mut Rng, low_exp: f64, high_exp: f64) -> f64 {
    10f64.powf(rng.uniform(low_exp, high_exp))
}

pub fn special_origin(rng: &mut Rng, case: u64) -> Coordinate {
    let (lat, lon) = match case % 8 {
        0 => (rng.uniform(89.9, 89.98), rng.uniform(-180.0, 180.0)),
        1 => (rng.uniform(-89.98, -89.9), rng.uniform(-180.0, 180.0)),
        2 => (rng.uniform(-70.0, 70.0), 180.0),
        3 => (rng.uniform(-70.0, 70.0), -179.99999),
        _ => (rng.uniform(-85.0, 85.0), rng.uniform(-180.0, 180.0)),
    };
    Coordinate::new(lat, lon).unwrap()
}

/// A plausible recording: starts and ends gently, cruises at `cruise` m/s
/// with ±15 % variation, wanders by up to 25° per leg, legs of 0.5–5 s, and
/// optionally a few waits. Built leg by leg with the geodesic direct solution.
pub fn smooth_route(
    rng: &mut Rng,
    origin: Coordinate,
    legs: usize,
    cruise: f64,
    waits: bool,
) -> Route {
    let mut points = vec![RoutePoint::new(0, origin)];
    let mut position = origin;
    let mut heading = rng.uniform(0.0, 360.0);
    let mut t_ns = 0i64;
    for j in 0..legs {
        let dt = rng.uniform(0.5, 5.0);
        // Ramp up over the first legs and down over the last ones.
        let ramp = (1.0f64)
            .min((j as f64 + 1.0) / 4.0)
            .min((legs - j) as f64 / 4.0);
        let speed = cruise * ramp * rng.uniform(0.85, 1.15);
        heading += rng.uniform(-25.0, 25.0);
        position = destination(position, heading, speed * dt).unwrap();
        t_ns += (dt * 1e9) as i64;
        points.push(RoutePoint::new(t_ns, position));
        if waits && j + 1 < legs && rng.next_f64() < 0.1 {
            t_ns += (rng.uniform(1.0, 10.0) * 1e9) as i64;
            points.push(RoutePoint::new(t_ns, position));
            heading += rng.uniform(-120.0, 120.0);
        }
    }
    Route::new(points).unwrap()
}

/// A closed circuit: `legs` points around a wobbly ring of about `radius`
/// metres, returning exactly to the first point.
pub fn closed_route(
    rng: &mut Rng,
    centre: Coordinate,
    legs: usize,
    radius: f64,
    lap_s: f64,
) -> Route {
    let phase = rng.uniform(0.0, 360.0);
    let first = destination(centre, phase, radius).unwrap();
    let mut points = vec![RoutePoint::new(0, first)];
    for j in 1..legs {
        let bearing = phase + 360.0 * j as f64 / legs as f64;
        let r = radius * rng.uniform(0.9, 1.1);
        let t_ns = (lap_s * j as f64 / legs as f64 * 1e9) as i64;
        points.push(RoutePoint::new(
            t_ns,
            destination(centre, bearing, r).unwrap(),
        ));
    }
    points.push(RoutePoint::new((lap_s * 1e9) as i64, first));
    Route::new(points).unwrap()
}

pub fn peak(plan: &RoutePlan, pick: fn(&locsim_core::route::SegmentKinematics) -> f64) -> f64 {
    plan.kinematics().iter().map(pick).fold(0.0, f64::max)
}

/// The limits a plan can just be admitted under: each of its own peaks times
/// `slack`, a heading rate sufficient for every turn made at a stop, and a
/// fence around `centre` just beyond the farthest point of the curve.
/// `interval_s` is the shortest interval the route will be sampled at;
/// admission's rounding headroom for it is added on top.
/// `None` if the trajectory has an unbounded peak (a reversal through zero
/// speed between recorded points) and so can be admitted under no limits.
pub fn declared_limits(
    plan: &RoutePlan,
    centre: Coordinate,
    slack: f64,
    interval_s: f64,
) -> Option<RouteLimits> {
    let speed = peak(plan, |k| k.peak_speed_mps.value);
    let accel = peak(plan, |k| k.peak_acceleration_mps2.value);
    let decel = peak(plan, |k| k.peak_deceleration_mps2.value);
    let turn = peak(plan, |k| k.peak_heading_rate_dps.value);
    if ![speed, accel, decel, turn].iter().all(|v| v.is_finite()) {
        return None;
    }
    let speed_headroom = 2.0 * COORDINATE_RESOLUTION_M / interval_s;
    let turn_headroom = COURSE_RESOLUTION_DEG / interval_s;
    let mut limits = RouteLimits {
        max_speed_mps: (speed + speed_headroom) * slack,
        displacement_cap_m: None,
        update_interval_s: interval_s,
        // Floors keep a perfectly straight or steady route from declaring
        // a limit of exactly zero.
        max_acceleration_mps2: (accel * slack).max(1e-9),
        max_deceleration_mps2: (decel * slack).max(1e-9),
        max_heading_rate_dps: (turn + turn_headroom) * slack + 1e-12,
        boundary: None,
    };
    // Turns at stops: the limit reported is `rate × wait`.
    for v in plan.violations(&limits) {
        if v.constraint != RouteConstraint::TurnAtStop {
            return None;
        }
        let wait_s = v.limit / limits.max_heading_rate_dps;
        if wait_s <= 0.0 {
            return None;
        }
        limits.max_heading_rate_dps = limits
            .max_heading_rate_dps
            .max(v.observed / wait_s * slack + 1e-6);
    }
    // Farthest point of the curve from the centre: ask with a vanishing fence.
    let probe = RouteLimits {
        boundary: Some(Boundary {
            center: centre,
            radius_m: 1e-9,
        }),
        ..limits
    };
    let farthest = plan
        .violations(&probe)
        .iter()
        .filter(|v| v.constraint == RouteConstraint::Boundary)
        .map(|v| v.observed)
        .fold(0.0, f64::max);
    limits.boundary = Some(Boundary {
        center: centre,
        radius_m: farthest * slack + 1e-6,
    });
    Some(limits)
}

/// A route-replay scenario whose movement limits are exactly `limits`.
pub fn route_scenario(
    route: Route,
    playback: PlaybackParameters,
    limits: &RouteLimits,
    update_interval_s: f64,
) -> Scenario {
    let boundary = limits.boundary.expect("declared limits carry a boundary");
    Scenario {
        schema_version: CURRENT_SCHEMA_VERSION,
        name: "route replay".into(),
        origin: boundary.center,
        altitude_m: 25.0,
        mode: MovementMode::RouteReplay,
        movement: MovementParameters {
            min_speed_mps: 0.0,
            max_speed_mps: limits.max_speed_mps,
            max_acceleration_mps2: limits.max_acceleration_mps2,
            max_deceleration_mps2: limits.max_deceleration_mps2,
            max_heading_rate_dps: limits.max_heading_rate_dps,
            radius_m: Some(boundary.radius_m),
            step_distance_m: None,
            heading_persistence: 0.0,
            pause_probability: 0.0,
            max_pause_s: 0.0,
            angular_velocity_dps: None,
            direction: RotationDirection::Clockwise,
            start_phase_deg: 0.0,
            speed_change_interval_s: 0.0,
            max_displacement_per_sample_m: None,
        },
        noise: NoiseParameters::NONE,
        horizontal_accuracy_m: 6.0,
        vertical_accuracy_m: 9.0,
        update_interval_s,
        seed: 1,
        route: Some(route),
        playback,
    }
}
