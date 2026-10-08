//! Property tests for the route engine.
//!
//! The central claim of admission is tested from both sides: a route
//! admitted under some limits really keeps to them at every instant, and
//! passes the independent validation gate at any sampling interval; and a
//! route is rejected as soon as any one limit is set below what it needs.

mod common;

use common::*;
use locsim_core::domain::{
    Boundary, LocationSource, PlaybackParameters, Route, SimulationState, SyntheticLocation,
    Timestamp,
};
use locsim_core::geographic::{bearing_difference, distance, inverse};
use locsim_core::rng::Rng;
use locsim_core::route::{RouteConstraint, RouteLimits, RoutePlan, RouteState};
use locsim_core::validation::{SampleLimits, SampleValidator};

fn forward() -> PlaybackParameters {
    PlaybackParameters::REAL_TIME
}

fn random_playback(rng: &mut Rng, case: u64, closed: bool) -> PlaybackParameters {
    PlaybackParameters {
        speed: if case % 3 == 0 {
            log_uniform(rng, -0.5, 0.5)
        } else {
            1.0
        },
        looping: closed,
        reverse: case % 4 == 1,
    }
}

/// A random route (open, or a closed circuit for every fifth case) and its plan.
fn random_plan(rng: &mut Rng, case: u64) -> (Route, PlaybackParameters, RoutePlan) {
    let origin = special_origin(rng, case);
    let closed = case % 5 == 4;
    let route = if closed {
        let legs = 6 + (rng.next_u64() % 20) as usize;
        let radius = log_uniform(rng, 1.0, 2.5);
        let lap_s = rng.uniform(40.0, 400.0);
        closed_route(rng, origin, legs, radius, lap_s)
    } else {
        let legs = 3 + (rng.next_u64() % 40) as usize;
        let cruise = log_uniform(rng, -0.3, 1.5);
        smooth_route(rng, origin, legs, cruise, case % 2 == 0)
    };
    let playback = random_playback(rng, case, closed);
    let plan = RoutePlan::build(&route, &playback, 30.0).unwrap();
    (route, playback, plan)
}

/// The gate's limits for a route admitted under `limits`, with no noise.
fn gate_limits(limits: &RouteLimits) -> SampleLimits {
    SampleLimits {
        max_speed_mps: limits.max_speed_mps,
        max_displacement_rate_mps: limits.max_speed_mps,
        boundary: limits.boundary,
        max_acceleration_mps2: limits.max_acceleration_mps2,
        max_deceleration_mps2: limits.max_deceleration_mps2,
        max_heading_rate_dps: limits.max_heading_rate_dps,
        speed_noise_allowance_mps: 0.0,
        heading_noise_allowance_deg: 0.0,
        position_noise_reach_m: 0.0,
    }
}

fn as_sample(state: &RouteState, t_ns: i64) -> SyntheticLocation {
    SyntheticLocation {
        timestamp: Timestamp::from_nanos(START + t_ns),
        coordinate: state.coordinate,
        altitude_m: state.altitude_m,
        horizontal_accuracy_m: 5.0,
        vertical_accuracy_m: 8.0,
        speed_mps: Some(state.speed_mps),
        course_deg: state.course_deg,
        source: LocationSource::Replay,
        simulation_state: SimulationState::Running,
    }
}

#[test]
fn admitted_routes_keep_their_declared_limits_at_every_instant() {
    let mut rng = Rng::from_seed(0x0611);
    let (mut admitted, mut unbounded, mut instants) = (0u32, 0u32, 0u64);
    let mut closest = [0.0f64; 5]; // speed, accel, decel, turn, radius as fractions of the limit
    for case in 0..400u64 {
        let (_, _, plan) = random_plan(&mut rng, case);
        let centre = plan.state_at(0).unwrap().coordinate;
        let slack = if case % 2 == 0 { TIGHT } else { 1.01 };
        let Some(limits) = declared_limits(&plan, centre, slack, 1.0) else {
            unbounded += 1;
            continue;
        };
        assert_eq!(plan.violations(&limits), vec![], "case {case}");
        admitted += 1;
        let boundary = limits.boundary.unwrap();

        // Random instants over two passes, plus instants hugging every
        // recorded point, where the interpolant changes segment.
        let span = 2 * plan.duration_ns();
        let mut times: Vec<i64> = (0..600)
            .map(|_| (rng.next_f64() * span as f64) as i64)
            .collect();
        times.push(plan.duration_ns());
        for k in plan.kinematics() {
            let knot = (k.start_elapsed_s * 1e9).round() as i64;
            times.extend([knot - 1, knot, knot + 1]);
        }
        for t in times {
            let s = plan.state_at(t).unwrap();
            let ctx = || format!("case {case} at {t} ns: {s:?}");
            assert!(s.speed_mps.is_finite() && s.speed_mps >= 0.0, "{}", ctx());
            assert!(s.speed_mps <= limits.max_speed_mps, "{}", ctx());
            assert!(
                s.acceleration_mps2 <= limits.max_acceleration_mps2,
                "{}",
                ctx()
            );
            assert!(
                -s.acceleration_mps2 <= limits.max_deceleration_mps2,
                "{}",
                ctx()
            );
            assert!(
                s.heading_rate_dps.abs() <= limits.max_heading_rate_dps,
                "{}",
                ctx()
            );
            assert_eq!(s.course_deg.is_some(), s.speed_mps > 0.0, "{}", ctx());
            let d = distance(boundary.center, s.coordinate).unwrap();
            assert!(
                d <= boundary.radius_m,
                "{}: {d} > {}",
                ctx(),
                boundary.radius_m
            );
            for (slot, fraction) in [
                (0, s.speed_mps / limits.max_speed_mps),
                (1, s.acceleration_mps2 / limits.max_acceleration_mps2),
                (2, -s.acceleration_mps2 / limits.max_deceleration_mps2),
                (3, s.heading_rate_dps.abs() / limits.max_heading_rate_dps),
                (4, d / boundary.radius_m),
            ] {
                closest[slot] = closest[slot].max(fraction);
            }
            instants += 1;
        }
    }
    println!(
        "admitted {admitted} of 400 random routes ({unbounded} unbounded), {instants} instants; \
         closest approach to speed/accel/decel/turn/radius limits: {closest:?}"
    );
    assert!(admitted >= 380, "only {admitted} admitted");
    // The declared limits are tight: sampling comes within a whisker of
    // each, so the checks above are not vacuous.
    assert!(
        closest.iter().all(|f| *f > 0.99 && *f <= 1.0),
        "{closest:?}"
    );
}

#[test]
fn admitted_routes_pass_the_gate_at_any_sampling_interval() {
    let mut rng = Rng::from_seed(0x0612);
    let (mut pairs, mut streams) = (0u64, 0u32);
    for case in 0..300u64 {
        let (_, _, plan) = random_plan(&mut rng, case);
        let centre = plan.state_at(0).unwrap().coordinate;
        let slack = if case % 2 == 0 { TIGHT } else { 1.01 };
        // The same route, read at three unrelated cadences: sub-millisecond
        // to seconds, regular and irregular. It is admitted for the shortest
        // interval each cadence will use.
        for cadence in 0..3 {
            let step_s = log_uniform(&mut rng, -3.5, 1.0); // 0.3 ms … 10 s
            let shortest = if cadence == 2 { 0.2 * step_s } else { step_s };
            let Some(limits) = declared_limits(&plan, centre, slack, shortest) else {
                continue;
            };
            assert_eq!(plan.violations(&limits), vec![], "case {case}");
            let mut gate = SampleValidator::with_limits(gate_limits(&limits));
            let mut t = 0i64;
            let end = plan.duration_ns() + 3 * SEC;
            let mut count = 0u64;
            while t <= end && count < 4_000 {
                let state = plan.state_at(t).unwrap();
                if let Err(e) = gate.validate(&as_sample(&state, t)) {
                    panic!(
                        "case {case} cadence {cadence} (step {step_s} s) at {t} ns: {e}\n{state:?}"
                    );
                }
                let jitter = if cadence == 2 {
                    rng.uniform(0.2, 1.8)
                } else {
                    1.0
                };
                t += ((step_s * jitter * 1e9) as i64).max(1);
                count += 1;
            }
            pairs += count.saturating_sub(1);
            streams += 1;
        }
    }
    println!("gate accepted {pairs} consecutive pairs over {streams} streams");
    assert!(streams > 800 && pairs > 500_000);
}

#[test]
fn any_single_limit_set_below_the_route_is_detected() {
    let mut rng = Rng::from_seed(0x0613);
    let mut detected = [0u32; 6];
    for case in 0..200u64 {
        let (_, _, plan) = random_plan(&mut rng, case);
        let centre = plan.state_at(0).unwrap().coordinate;
        let Some(limits) = declared_limits(&plan, centre, 1.01, 0.5) else {
            continue;
        };
        assert_eq!(plan.violations(&limits), vec![], "case {case}");
        let speed = peak(&plan, |k| k.peak_speed_mps.value);
        let radius = limits.boundary.unwrap().radius_m;
        let tightened: [(RouteConstraint, RouteLimits); 6] = [
            (
                RouteConstraint::MaxSpeed,
                RouteLimits {
                    max_speed_mps: speed * 0.99,
                    ..limits
                },
            ),
            (
                RouteConstraint::DisplacementPerSample,
                RouteLimits {
                    displacement_cap_m: Some(speed * 0.5 * 0.99),
                    ..limits
                },
            ),
            (
                RouteConstraint::Acceleration,
                RouteLimits {
                    max_acceleration_mps2: peak(&plan, |k| k.peak_acceleration_mps2.value) * 0.99,
                    ..limits
                },
            ),
            (
                RouteConstraint::Deceleration,
                RouteLimits {
                    max_deceleration_mps2: peak(&plan, |k| k.peak_deceleration_mps2.value) * 0.99,
                    ..limits
                },
            ),
            (
                RouteConstraint::HeadingRate,
                RouteLimits {
                    max_heading_rate_dps: peak(&plan, |k| k.peak_heading_rate_dps.value) * 0.99,
                    ..limits
                },
            ),
            (
                RouteConstraint::Boundary,
                RouteLimits {
                    boundary: Some(Boundary {
                        center: centre,
                        radius_m: radius * 0.98,
                    }),
                    ..limits
                },
            ),
        ];
        for (slot, (constraint, tight)) in tightened.into_iter().enumerate() {
            let found = plan.violations(&tight);
            // Lowering the heading rate may also invalidate a turn at a stop,
            // which is the same limit; nothing else may appear.
            let allowed = |c: RouteConstraint| {
                c == constraint
                    || (constraint == RouteConstraint::HeadingRate
                        && c == RouteConstraint::TurnAtStop)
            };
            assert!(
                found.iter().any(|v| v.constraint == constraint),
                "case {case}: {constraint:?} not reported in {found:?}"
            );
            assert!(
                found.iter().all(|v| allowed(v.constraint)),
                "case {case}: {found:?}"
            );
            for v in &found {
                assert!(v.observed > v.limit * (1.0 - 1e-6), "case {case}: {v:?}");
                assert!(v.at_elapsed_s >= 0.0 && v.at_elapsed_s <= plan.duration_s() + 1e-9);
            }
            detected[slot] += 1;
        }
    }
    println!(
        "single-limit detections (speed/displacement/accel/decel/turn/boundary): {detected:?}"
    );
    assert!(detected.iter().all(|n| *n >= 180), "{detected:?}");
}

#[test]
fn replay_is_deterministic_and_independent_of_sampling_history() {
    let mut rng = Rng::from_seed(0x0614);
    for case in 0..60u64 {
        let origin = special_origin(&mut rng, case);
        let route = smooth_route(&mut rng, origin, 12, 3.0, true);
        let playback = random_playback(&mut rng, case, false);
        let a = RoutePlan::build(&route, &playback, 5.0).unwrap();
        let b = RoutePlan::build(&route, &playback, 5.0).unwrap();
        assert_eq!(a.kinematics(), b.kinematics());
        let span = a.duration_ns();
        // b is read densely and in order, a at scattered instants.
        let mut dense = Vec::new();
        for step in 0..=2_000 {
            let t = span / 2_000 * step;
            dense.push((t, b.state_at(t).unwrap()));
        }
        for _ in 0..200 {
            let (t, expected) = dense[(rng.next_u64() % 2_001) as usize];
            assert_eq!(a.state_at(t).unwrap(), expected, "case {case} at {t}");
        }
    }
}

#[test]
fn every_recorded_point_is_reproduced_exactly_for_random_routes() {
    let mut rng = Rng::from_seed(0x0615);
    for case in 0..200u64 {
        let origin = special_origin(&mut rng, case);
        let cruise = log_uniform(&mut rng, -0.3, 1.5);
        let route = smooth_route(&mut rng, origin, 20, cruise, true);
        let plan = RoutePlan::build(&route, &forward(), 0.0).unwrap();
        for p in route.points() {
            let s = plan.state_at(p.elapsed_ns).unwrap();
            assert_eq!(s.coordinate, p.coordinate, "case {case}");
        }
        // Reverse playback reproduces them too, at mirrored times.
        let reverse = PlaybackParameters {
            reverse: true,
            ..forward()
        };
        let back = RoutePlan::build(&route, &reverse, 0.0).unwrap();
        for p in route.points() {
            let s = back.state_at(route.duration_ns() - p.elapsed_ns).unwrap();
            assert_eq!(s.coordinate, p.coordinate, "case {case}");
        }
    }
}

#[test]
fn antimeridian_and_polar_routes_are_smooth() {
    // A route that crosses the date line and one that passes 300 m from the
    // pole: no jump in position, no jump in course beyond real turning.
    let date_line = Route::new(
        (0..12)
            .map(|k| {
                locsim_core::domain::RoutePoint::new(
                    k * 5 * SEC,
                    locsim_core::domain::Coordinate::new(
                        10.0 + 0.0001 * k as f64,
                        locsim_core::geographic::normalize_longitude(179.9992 + 0.00015 * k as f64),
                    )
                    .unwrap(),
                )
            })
            .collect(),
    )
    .unwrap();
    let near_pole = {
        let start = locsim_core::domain::Coordinate::new(89.99, 0.0).unwrap();
        let mut rng = Rng::from_seed(9);
        // Head for a point past the pole, slightly to one side.
        let mut points = vec![locsim_core::domain::RoutePoint::new(0, start)];
        let mut position = start;
        let mut heading = 345.0;
        for k in 1..=30i64 {
            let ramp = (k as f64 / 4.0).min((31 - k) as f64 / 4.0).min(1.0);
            let (next, arrival) =
                locsim_core::geographic::direct(position, heading, 80.0 * ramp).unwrap();
            position = next;
            heading = arrival + rng.uniform(-2.0, 2.0);
            points.push(locsim_core::domain::RoutePoint::new(k * 4 * SEC, position));
        }
        Route::new(points).unwrap()
    };
    for (label, route) in [("date line", date_line), ("near pole", near_pole)] {
        let plan = RoutePlan::build(&route, &forward(), 0.0).unwrap();
        let limits = declared_limits(&plan, route.points()[0].coordinate, TIGHT, 0.02).unwrap();
        assert_eq!(plan.violations(&limits), vec![], "{label}");
        let mut gate = SampleValidator::with_limits(gate_limits(&limits));
        let mut previous: Option<RouteState> = None;
        let mut signs = [false, false];
        let mut swing = 0.0;
        for step in 0..=(plan.duration_ns() / (20 * 1_000_000)) {
            let t = step * 20 * 1_000_000;
            let s = plan.state_at(t).unwrap();
            gate.validate(&as_sample(&s, t))
                .unwrap_or_else(|e| panic!("{label} at {t}: {e}"));
            signs[(s.coordinate.longitude() < 0.0) as usize] = true;
            if let Some(p) = previous {
                let line = inverse(p.coordinate, s.coordinate).unwrap();
                assert!(line.distance_m <= limits.max_speed_mps * 0.02, "{label}");
                if let (Some(c0), Some(c1)) = (p.course_deg, s.course_deg) {
                    swing += bearing_difference(c0, c1).abs();
                }
            }
            previous = Some(s);
        }
        // Both hemispheres of longitude were visited: the line was crossed.
        assert!(signs[0] && signs[1], "{label}");
        if label == "near pole" {
            // The raw bearing swings far more than the route ever turns.
            assert!(swing > 90.0, "{label}: bearing swung {swing}");
        }
    }
}
