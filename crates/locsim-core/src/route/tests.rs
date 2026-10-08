use super::*;
use crate::domain::{Boundary, Coordinate, PlaybackParameters, Route, RoutePoint};
use crate::geographic::{bearing_difference, distance, inverse, EnuFrame};

const SEC: i64 = 1_000_000_000;
const MS: i64 = 1_000_000;

fn base() -> Coordinate {
    Coordinate::new(12.9352, 77.6245).unwrap()
}

/// A point `east`/`north` metres from the base, `seconds` into the route.
fn pt(seconds: f64, east: f64, north: f64) -> RoutePoint {
    let frame = EnuFrame::new(base(), 0.0).unwrap();
    // The base itself, exactly, so that routes returning to it are closed.
    let coordinate = if east == 0.0 && north == 0.0 {
        base()
    } else {
        frame.horizontal_to_coordinate(east, north).unwrap()
    };
    RoutePoint::new((seconds * 1e9).round() as i64, coordinate)
}

fn forward() -> PlaybackParameters {
    PlaybackParameters::REAL_TIME
}

fn plan_of(points: Vec<RoutePoint>) -> RoutePlan {
    RoutePlan::build(&Route::new(points).unwrap(), &forward(), 100.0).unwrap()
}

/// Accelerate east, cruise, curve north, brake: a plausible short walk.
fn walk_points() -> Vec<RoutePoint> {
    vec![
        pt(0.0, 0.0, 0.0),
        pt(2.0, 1.5, 0.0),
        pt(4.0, 4.5, 0.0),
        pt(6.0, 7.5, 0.0),
        pt(8.0, 10.2, 1.0),
        pt(10.0, 12.0, 3.2),
        pt(12.0, 12.8, 6.0),
        pt(14.0, 13.0, 8.5),
        pt(16.0, 13.0, 9.5),
    ]
}

fn walk() -> RoutePlan {
    plan_of(walk_points())
}

const NO_LIMITS: RouteLimits = RouteLimits {
    max_speed_mps: 1e9,
    displacement_cap: None,
    max_acceleration_mps2: 1e9,
    max_deceleration_mps2: 1e9,
    max_heading_rate_dps: 1e9,
    boundary: None,
};

/// Largest peak of one kind over the plan, and the segment it is on.
fn worst(plan: &RoutePlan, pick: fn(&SegmentKinematics) -> f64) -> (usize, f64) {
    plan.kinematics()
        .iter()
        .map(|k| (k.segment, pick(k)))
        .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a })
}

// --- Interpolation -----------------------------------------------------------

#[test]
fn two_point_route_goes_from_rest_to_rest_along_the_line() {
    let plan = plan_of(vec![pt(0.0, 0.0, 0.0), pt(10.0, 20.0, 0.0)]);
    assert_eq!(plan.duration_ns(), 10 * SEC);
    assert!(!plan.is_looping());

    let start = plan.state_at(0).unwrap();
    assert_eq!(start.coordinate, base());
    assert_eq!(
        (start.speed_mps, start.course_deg, start.complete),
        (0.0, None, false)
    );

    // Half-way in time is half-way in distance, at the peak speed 1.5 × mean.
    let mid = plan.state_at(5 * SEC).unwrap();
    assert!((distance(base(), mid.coordinate).unwrap() - 10.0).abs() < 1e-6);
    assert!((mid.speed_mps - 3.0).abs() < 1e-6);
    assert!((mid.course_deg.unwrap() - 90.0).abs() < 1e-4);
    assert!(mid.acceleration_mps2.abs() < 1e-9 && mid.heading_rate_dps.abs() < 1e-6);

    // Speeding up in the first half, slowing in the second.
    assert!(plan.state_at(2 * SEC).unwrap().acceleration_mps2 > 0.0);
    assert!(plan.state_at(8 * SEC).unwrap().acceleration_mps2 < 0.0);

    let k = &plan.kinematics()[0];
    assert!((k.chord_m - 20.0).abs() < 1e-6 && (k.mean_speed_mps - 2.0).abs() < 1e-6);
    assert!((k.peak_speed_mps.value - 3.0).abs() < 1e-6);
    assert_eq!(k.peak_speed_mps.at_elapsed_s, 5.0);
    // 6·d/h² = 1.2 m/s², at the very start and the very end.
    assert!((k.peak_acceleration_mps2.value - 1.2).abs() < 1e-6);
    assert_eq!(k.peak_acceleration_mps2.at_elapsed_s, 0.0);
    assert!((k.peak_deceleration_mps2.value - 1.2).abs() < 1e-6);
    assert_eq!(k.peak_deceleration_mps2.at_elapsed_s, 10.0);
    assert_eq!(k.peak_heading_rate_dps.value, 0.0);
    assert!((k.entry_bearing_deg.unwrap() - 90.0).abs() < 1e-4);
    assert!((k.exit_bearing_deg.unwrap() - 90.0).abs() < 1e-4);
}

#[test]
fn before_start_at_end_and_after_end() {
    let points = walk_points();
    let plan = walk();
    let (first, last) = (points[0].coordinate, points[8].coordinate);

    // Before the start: the first point, at rest, not complete.
    for t in [-1, -5 * SEC, i64::MIN] {
        let s = plan.state_at(t).unwrap();
        assert_eq!((s.coordinate, s.speed_mps, s.complete), (first, 0.0, false));
    }
    // One nanosecond before the end: still moving, not complete.
    let almost = plan.state_at(16 * SEC - 1).unwrap();
    assert!(!almost.complete && almost.speed_mps > 0.0);
    // Exactly at the end and for ever after: the last point, at rest, complete.
    for t in [16 * SEC, 16 * SEC + 1, 500 * SEC, i64::MAX] {
        let s = plan.state_at(t).unwrap();
        assert_eq!(s.coordinate, last);
        assert_eq!((s.speed_mps, s.course_deg), (0.0, None));
        assert!(s.complete);
    }
}

#[test]
fn passes_exactly_through_every_recorded_point_at_its_time() {
    let points = walk_points();
    let plan = walk();
    for p in &points {
        let s = plan.state_at(p.elapsed_ns).unwrap();
        assert_eq!(s.coordinate, p.coordinate, "at {} ns", p.elapsed_ns);
    }
    // And only a hair's breadth away a nanosecond either side.
    for p in &points[1..8] {
        for dt in [-1, 1] {
            let s = plan.state_at(p.elapsed_ns + dt).unwrap();
            assert!(distance(s.coordinate, p.coordinate).unwrap() < 1e-7);
        }
    }
}

#[test]
fn speed_and_course_are_continuous_across_recorded_points() {
    let plan = walk();
    for k in 1..8i64 {
        let t = 2 * k * SEC;
        let (before, at, after) = (
            plan.state_at(t - 1).unwrap(),
            plan.state_at(t).unwrap(),
            plan.state_at(t + 1).unwrap(),
        );
        assert!(at.speed_mps > 0.3, "point {k} should be passed at speed");
        for neighbour in [before, after] {
            // One nanosecond at ≤ 1 m/s²: at most 1e-9 m/s of change.
            assert!(
                (neighbour.speed_mps - at.speed_mps).abs() < 1e-8,
                "point {k}"
            );
            let turn = bearing_difference(neighbour.course_deg.unwrap(), at.course_deg.unwrap());
            assert!(turn.abs() < 1e-6, "point {k}: {turn}");
        }
    }
}

#[test]
fn reported_speed_and_course_are_the_derivative_of_the_position() {
    let plan = walk();
    let mut checked = 0;
    for step in 1..160 {
        let t = step * 100 * MS; // every 0.1 s, avoiding the two ends
        let (a, s, b) = (
            plan.state_at(t - MS).unwrap(),
            plan.state_at(t).unwrap(),
            plan.state_at(t + MS).unwrap(),
        );
        let line = inverse(a.coordinate, b.coordinate).unwrap();
        let measured_speed = line.distance_m / 0.002;
        assert!(
            (measured_speed - s.speed_mps).abs() < 1e-4 * (1.0 + s.speed_mps),
            "t={t}: {measured_speed} vs {}",
            s.speed_mps
        );
        if s.speed_mps > 0.05 {
            let turn = bearing_difference(line.initial_bearing_deg, s.course_deg.unwrap());
            assert!(turn.abs() < 0.05, "t={t}: course off by {turn}");
            checked += 1;
        }
        // Reported rates are the derivatives of the reported speed and course
        // (not compared at recorded points, where acceleration may step).
        if t % (2 * SEC) == 0 {
            continue;
        }
        let dv = (b.speed_mps - a.speed_mps) / 0.002;
        assert!((dv - s.acceleration_mps2).abs() < 1e-3, "t={t}: {dv}");
        if let (Some(c0), Some(c1)) = (a.course_deg, b.course_deg) {
            if s.speed_mps > 0.05 {
                let rate = bearing_difference(c0, c1) / 0.002;
                assert!((rate - s.heading_rate_dps).abs() < 0.05, "t={t}: {rate}");
            }
        }
    }
    assert!(checked > 140);
}

#[test]
fn a_wait_is_spent_at_rest_and_entered_and_left_at_rest() {
    // Walk 6 m east, wait 5 s, walk 6 m north.
    let plan = plan_of(vec![
        pt(0.0, 0.0, 0.0),
        pt(6.0, 6.0, 0.0),
        pt(11.0, 6.0, 0.0),
        pt(17.0, 6.0, 6.0),
    ]);
    let corner = pt(0.0, 6.0, 0.0).coordinate;
    for t in [6 * SEC, 7 * SEC, 8 * SEC + 123, 11 * SEC] {
        let s = plan.state_at(t).unwrap();
        assert_eq!(
            (s.coordinate, s.speed_mps, s.course_deg),
            (corner, 0.0, None)
        );
        assert!(!s.complete);
    }
    let kin = plan.kinematics();
    assert!(kin[1].stationary && !kin[0].stationary && !kin[2].stationary);
    assert_eq!(
        (kin[1].entry_bearing_deg, kin[1].peak_speed_mps.value),
        (None, 0.0)
    );
    // Arrives heading east, leaves heading north.
    assert!((kin[0].exit_bearing_deg.unwrap() - 90.0).abs() < 1e-3);
    assert!(kin[2].entry_bearing_deg.unwrap().abs() < 1e-3);
    // Approaching and leaving speeds tend to zero.
    assert!(plan.state_at(6 * SEC - MS).unwrap().speed_mps < 0.01);
    assert!(plan.state_at(11 * SEC + MS).unwrap().speed_mps < 0.01);
}

#[test]
fn the_trajectory_is_a_function_of_time_alone() {
    // Sampling coarsely, finely or out of order reads the same curve.
    let plan = walk();
    let coarse: Vec<RouteState> = (0..=16).map(|s| plan.state_at(s * SEC).unwrap()).collect();
    for step in (0..=16_000).rev() {
        let fine = plan.state_at(step * MS).unwrap();
        if step % 1_000 == 0 {
            assert_eq!(fine, coarse[(step / 1_000) as usize]);
        }
    }
    let again = walk();
    for t in [0, 37 * MS, 5 * SEC + 1, 9_999_999_999, 16 * SEC] {
        assert_eq!(plan.state_at(t).unwrap(), again.state_at(t).unwrap());
    }
}

#[test]
fn altitude_is_interpolated_linearly_or_defaults_to_the_scenario() {
    let with = Route::new(vec![
        pt(0.0, 0.0, 0.0).with_altitude(10.0),
        pt(4.0, 4.0, 0.0).with_altitude(18.0),
        pt(8.0, 8.0, 0.0).with_altitude(14.0),
    ])
    .unwrap();
    let plan = RoutePlan::build(&with, &forward(), 100.0).unwrap();
    for (t, expected) in [
        (0, 10.0),
        (SEC, 12.0),
        (4 * SEC, 18.0),
        (6 * SEC, 16.0),
        (9 * SEC, 14.0),
    ] {
        assert!((plan.state_at(t).unwrap().altitude_m - expected).abs() < 1e-9);
    }
    assert_eq!(walk().state_at(3 * SEC).unwrap().altitude_m, 100.0);
    assert_eq!(walk().state_at(99 * SEC).unwrap().altitude_m, 100.0);
}

// --- Playback ------------------------------------------------------------------

#[test]
fn reverse_playback_is_the_same_curve_read_backwards() {
    let route = Route::new(walk_points()).unwrap();
    let fwd = RoutePlan::build(&route, &forward(), 0.0).unwrap();
    let reverse = PlaybackParameters {
        reverse: true,
        ..forward()
    };
    let rev = RoutePlan::build(&route, &reverse, 0.0).unwrap();
    assert_eq!(rev.duration_ns(), fwd.duration_ns());
    for step in 0..=160 {
        let t = step * 100 * MS;
        let (a, b) = (
            rev.state_at(t).unwrap(),
            fwd.state_at(16 * SEC - t).unwrap(),
        );
        assert!(
            distance(a.coordinate, b.coordinate).unwrap() < 1e-6,
            "t={t}"
        );
        assert!((a.speed_mps - b.speed_mps).abs() < 1e-9);
        if let (Some(ca), Some(cb)) = (a.course_deg, b.course_deg) {
            assert!((bearing_difference(ca, cb).abs() - 180.0).abs() < 1e-6);
        }
    }
    // Acceleration and braking swap; segments keep the route's own indices.
    let (f, r) = (fwd.kinematics(), rev.kinematics());
    for j in 0..8 {
        let mirror = &r[7 - j];
        assert_eq!(mirror.segment, f[j].segment);
        assert!((mirror.peak_speed_mps.value - f[j].peak_speed_mps.value).abs() < 1e-8);
        let (a, d) = (
            f[j].peak_acceleration_mps2.value,
            f[j].peak_deceleration_mps2.value,
        );
        // Each is a certified bound within 1e-7 of the true peak.
        assert!((mirror.peak_deceleration_mps2.value - a).abs() < 1e-6 * (1.0 + a));
        assert!((mirror.peak_acceleration_mps2.value - d).abs() < 1e-6 * (1.0 + d));
    }
}

#[test]
fn playback_speed_compresses_time_and_scales_the_kinematics() {
    let route = Route::new(walk_points()).unwrap();
    let normal = RoutePlan::build(&route, &forward(), 0.0).unwrap();
    let double = PlaybackParameters {
        speed: 2.0,
        ..forward()
    };
    let fast = RoutePlan::build(&route, &double, 0.0).unwrap();
    assert_eq!(fast.duration_ns(), 8 * SEC);
    for step in 0..=80 {
        let t = step * 100 * MS;
        let (a, b) = (fast.state_at(t).unwrap(), normal.state_at(2 * t).unwrap());
        assert!(distance(a.coordinate, b.coordinate).unwrap() < 1e-6);
        assert!((a.speed_mps - 2.0 * b.speed_mps).abs() < 1e-8);
    }
    for (f, n) in fast.kinematics().iter().zip(normal.kinematics()) {
        assert!((f.peak_speed_mps.value - 2.0 * n.peak_speed_mps.value).abs() < 1e-7);
        let (fa, na) = (
            f.peak_acceleration_mps2.value,
            n.peak_acceleration_mps2.value,
        );
        assert!((fa - 4.0 * na).abs() < 1e-7 * (1.0 + fa));
        let (fh, nh) = (f.peak_heading_rate_dps.value, n.peak_heading_rate_dps.value);
        assert!((fh - 2.0 * nh).abs() < 1e-6 * (1.0 + fh));
    }
}

#[test]
fn structural_rejections() {
    let route = Route::new(walk_points()).unwrap();
    let looped = PlaybackParameters {
        looping: true,
        ..forward()
    };
    assert_eq!(
        RoutePlan::build(&route, &looped, 0.0).unwrap_err(),
        PlanError::OpenRouteCannotLoop
    );
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let playback = PlaybackParameters {
            speed: bad,
            ..forward()
        };
        assert!(matches!(
            RoutePlan::build(&route, &playback, 0.0),
            Err(PlanError::InvalidPlaybackSpeed(_))
        ));
    }
    assert_eq!(
        RoutePlan::build(&route, &forward(), f64::NAN).unwrap_err(),
        PlanError::NonFiniteAltitude
    );

    // A leg of 1 ns played 100× faster cannot be represented.
    let tiny = Route::new(vec![
        pt(0.0, 0.0, 0.0),
        pt(5.0, 5.0, 0.0),
        RoutePoint::new(5 * SEC + 1, pt(0.0, 5.0, 0.0).coordinate),
        pt(10.0, 10.0, 0.0),
    ])
    .unwrap();
    let fast = PlaybackParameters {
        speed: 100.0,
        ..forward()
    };
    assert_eq!(
        RoutePlan::build(&tiny, &fast, 0.0).unwrap_err(),
        PlanError::SegmentCollapsed { segment: 1 }
    );

    // Bengaluru to Chennai (~290 km) in one leg is beyond the interpolant.
    let far = Route::new(vec![
        RoutePoint::new(0, base()),
        RoutePoint::new(20_000 * SEC, Coordinate::new(13.0827, 80.2707).unwrap()),
    ])
    .unwrap();
    match RoutePlan::build(&far, &forward(), 0.0) {
        Err(PlanError::SegmentTooLong {
            segment: 0,
            length_m,
            limit_m,
        }) => {
            assert!(length_m > 280_000.0 && length_m < 300_000.0);
            assert_eq!(limit_m, MAX_SEGMENT_CHORD_M);
        }
        other => panic!("unexpected {other:?}"),
    }
}

// --- Looping ---------------------------------------------------------------------

/// A 20 m square walked in 40 s, starting and ending at the same corner.
fn square() -> Route {
    Route::new(vec![
        pt(0.0, 0.0, 0.0),
        pt(5.0, 10.0, -1.0),
        pt(10.0, 20.0, 0.0),
        pt(15.0, 21.0, 10.0),
        pt(20.0, 20.0, 20.0),
        pt(25.0, 10.0, 21.0),
        pt(30.0, 0.0, 20.0),
        pt(35.0, -1.0, 10.0),
        pt(40.0, 0.0, 0.0),
    ])
    .unwrap()
}

#[test]
fn closed_route_loops_seamlessly_and_never_completes() {
    assert!(square().is_closed());
    let looped = PlaybackParameters {
        looping: true,
        ..forward()
    };
    let plan = RoutePlan::build(&square(), &looped, 0.0).unwrap();
    assert!(plan.is_looping());

    // Periodic: lap 7 is lap 1.
    for step in 0..400 {
        let t = step * 100 * MS + 13;
        let (a, b) = (
            plan.state_at(t).unwrap(),
            plan.state_at(t + 7 * 40 * SEC).unwrap(),
        );
        assert_eq!(a, b);
        assert!(!a.complete);
    }
    // The seam is like any other point: passed at speed, with no jump in
    // position, speed or course.
    let (before, at, after) = (
        plan.state_at(40 * SEC - 1).unwrap(),
        plan.state_at(40 * SEC).unwrap(),
        plan.state_at(40 * SEC + 1).unwrap(),
    );
    assert_eq!(at.coordinate, base());
    assert_eq!(at, plan.state_at(0).unwrap());
    assert!(at.speed_mps > 1.0);
    for neighbour in [before, after] {
        assert!(distance(neighbour.coordinate, at.coordinate).unwrap() < 1e-7);
        assert!((neighbour.speed_mps - at.speed_mps).abs() < 1e-8);
        let turn = bearing_difference(neighbour.course_deg.unwrap(), at.course_deg.unwrap());
        assert!(turn.abs() < 1e-6);
    }
    // Negative time wraps the same way.
    assert_eq!(
        plan.state_at(-10 * SEC).unwrap(),
        plan.state_at(30 * SEC).unwrap()
    );
}

#[test]
fn closed_route_without_looping_is_an_ordinary_open_route() {
    let plan = RoutePlan::build(&square(), &forward(), 0.0).unwrap();
    assert_eq!(plan.state_at(0).unwrap().speed_mps, 0.0);
    let end = plan.state_at(40 * SEC).unwrap();
    assert_eq!(
        (end.coordinate, end.speed_mps, end.complete),
        (base(), 0.0, true)
    );
    assert!(plan.state_at(41 * SEC).unwrap().complete);
}

// --- Admission ---------------------------------------------------------------------

#[test]
fn a_route_within_all_limits_is_admitted() {
    let plan = walk();
    assert_eq!(plan.violations(&NO_LIMITS), vec![]);
    let (_, speed) = worst(&plan, |k| k.peak_speed_mps.value);
    let (_, accel) = worst(&plan, |k| k.peak_acceleration_mps2.value);
    let (_, decel) = worst(&plan, |k| k.peak_deceleration_mps2.value);
    let (_, turn) = worst(&plan, |k| k.peak_heading_rate_dps.value);
    // A plausible walk: under 2.5 m/s, 1.5 m/s² and 60°/s.
    assert!(
        speed < 2.5 && accel < 1.5 && decel < 1.5 && turn < 60.0,
        "{speed} {accel} {decel} {turn}"
    );
    let snug = RouteLimits {
        max_speed_mps: speed * 1.001,
        displacement_cap: Some((speed * 1.001 * 0.25, 0.25)),
        max_acceleration_mps2: accel * 1.001,
        max_deceleration_mps2: decel * 1.001,
        max_heading_rate_dps: turn * 1.001,
        boundary: Some(Boundary {
            center: base(),
            radius_m: 16.2, // the far end is 16.10 m away
        }),
    };
    assert_eq!(plan.violations(&snug), vec![]);
}

/// Tightening exactly one limit to just under its peak yields exactly that
/// violation, on the right segment, with the observed value and the limit.
#[test]
fn each_kinematic_limit_is_detected_independently() {
    type Pick = fn(&SegmentKinematics) -> f64;
    type Case = (RouteConstraint, Pick, fn(f64) -> RouteLimits);
    let plan = walk();
    let cases: [Case; 4] = [
        (
            RouteConstraint::MaxSpeed,
            |k| k.peak_speed_mps.value,
            |v| RouteLimits {
                max_speed_mps: v,
                ..NO_LIMITS
            },
        ),
        (
            RouteConstraint::Acceleration,
            |k| k.peak_acceleration_mps2.value,
            |v| RouteLimits {
                max_acceleration_mps2: v,
                ..NO_LIMITS
            },
        ),
        (
            RouteConstraint::Deceleration,
            |k| k.peak_deceleration_mps2.value,
            |v| RouteLimits {
                max_deceleration_mps2: v,
                ..NO_LIMITS
            },
        ),
        (
            RouteConstraint::HeadingRate,
            |k| k.peak_heading_rate_dps.value,
            |v| RouteLimits {
                max_heading_rate_dps: v,
                ..NO_LIMITS
            },
        ),
    ];
    for (constraint, pick, limits) in cases {
        let (segment, peak) = worst(&plan, pick);
        assert!(peak > 0.0, "{constraint:?}");
        // Comfortably above the peak: admitted.
        assert_eq!(
            plan.violations(&limits(peak * 1.001)),
            vec![],
            "{constraint:?}"
        );
        // Just below it: that constraint, that segment, nothing else.
        let found = plan.violations(&limits(peak * 0.999));
        assert_eq!(found.len(), 1, "{constraint:?}: {found:?}");
        let v = found[0];
        assert_eq!((v.constraint, v.segment), (constraint, segment));
        assert_eq!(v.limit, peak * 0.999);
        assert!(
            (v.observed / peak - 1.0).abs() < 1e-6,
            "{constraint:?}: {} vs {peak}",
            v.observed
        );
        let k = plan.kinematics()[segment];
        assert!(
            v.at_elapsed_s >= k.start_elapsed_s
                && v.at_elapsed_s <= k.start_elapsed_s + k.duration_s
        );
        // The limit itself is too tight as well: models keep a margin below.
        assert_eq!(plan.violations(&limits(peak)).len(), 1, "{constraint:?}");
    }
}

#[test]
fn displacement_cap_is_checked_per_update_interval() {
    let plan = walk();
    let (segment, speed) = worst(&plan, |k| k.peak_speed_mps.value);
    let with_cap = |cap: f64, interval: f64| RouteLimits {
        displacement_cap: Some((cap, interval)),
        ..NO_LIMITS
    };
    // At 2 Hz the fastest half-second covers speed/2 metres.
    assert_eq!(plan.violations(&with_cap(speed * 0.5 * 1.001, 0.5)), vec![]);
    let found = plan.violations(&with_cap(speed * 0.5 * 0.999, 0.5));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].constraint, RouteConstraint::DisplacementPerSample);
    assert_eq!(found[0].segment, segment);
    assert!((found[0].observed - speed * 0.5).abs() < 1e-6);
    assert_eq!(found[0].limit, speed * 0.5 * 0.999);
    // The same cap is fine at 4 Hz.
    assert_eq!(
        plan.violations(&with_cap(speed * 0.5 * 0.999, 0.25)),
        vec![]
    );
}

#[test]
fn boundary_is_checked_along_the_curve_not_only_at_the_points() {
    let fence = |radius_m: f64| RouteLimits {
        boundary: Some(Boundary {
            center: base(),
            radius_m,
        }),
        ..NO_LIMITS
    };
    // The walk ends 16.10 m from the base, its farthest point.
    let plan = walk();
    assert_eq!(plan.violations(&fence(16.15)), vec![]);
    let found = plan.violations(&fence(16.05));
    assert_eq!(found.len(), 1);
    let v = found[0];
    assert_eq!((v.constraint, v.segment), (RouteConstraint::Boundary, 7));
    assert!((v.observed - 16.1012).abs() < 1e-3, "{}", v.observed);
    assert_eq!(v.limit, 16.05);
    assert!((v.at_elapsed_s - 16.0).abs() < 1e-6);

    // Every recorded point is within 9.6 m (the farthest, at 9.55 m), but
    // the walker arrives at the second point still moving east and swings
    // wide before turning north: the curve leaves the fence between points.
    let swing = plan_of(vec![
        pt(0.0, 0.0, 0.0),
        pt(4.0, 9.5, 0.0),
        pt(6.0, 9.5, 1.0),
        pt(10.0, 7.0, 3.0),
    ]);
    let found = swing.violations(&fence(9.6));
    assert_eq!(found.len(), 1, "{found:?}");
    let v = found[0];
    assert_eq!((v.constraint, v.segment), (RouteConstraint::Boundary, 1));
    assert!(v.observed > 9.6 && v.observed < 10.0, "{}", v.observed);
    // The farthest point lies strictly inside the segment (4 s … 6 s).
    assert!(
        v.at_elapsed_s > 4.1 && v.at_elapsed_s < 5.9,
        "{}",
        v.at_elapsed_s
    );
    // Confirmed by the trajectory itself.
    let there = swing.state_at((v.at_elapsed_s * 1e9) as i64).unwrap();
    let actual = distance(base(), there.coordinate).unwrap();
    assert!(
        (actual - v.observed).abs() < 1e-3 && actual > 9.6,
        "{actual}"
    );
    assert_eq!(swing.violations(&fence(10.0)), vec![]);
}

#[test]
fn turning_round_at_a_stop_needs_time_to_turn() {
    // Out 8 m east, wait, back again: a 180° turn made while stopped.
    let out_and_back = |wait_s: f64| {
        plan_of(vec![
            pt(0.0, 0.0, 0.0),
            pt(8.0, 8.0, 0.0),
            pt(8.0 + wait_s, 8.0, 0.0),
            pt(16.0 + wait_s, 0.0, 0.0),
        ])
    };
    let turning = |rate: f64| RouteLimits {
        max_heading_rate_dps: rate,
        ..NO_LIMITS
    };
    // 90°/s turns 180° in 2 s: a 3 s wait is enough, a 1 s wait is not.
    assert_eq!(out_and_back(3.0).violations(&turning(90.0)), vec![]);
    let found = out_and_back(1.0).violations(&turning(90.0));
    assert_eq!(found.len(), 1, "{found:?}");
    let v = found[0];
    assert_eq!((v.constraint, v.segment), (RouteConstraint::TurnAtStop, 0));
    assert!((v.observed - 180.0).abs() < 1e-6);
    assert_eq!(v.limit, 90.0);
    assert_eq!(v.at_elapsed_s, 8.0);
    // A quarter turn at the same stop is fine.
    let corner = plan_of(vec![
        pt(0.0, 0.0, 0.0),
        pt(8.0, 8.0, 0.0),
        pt(9.0, 8.0, 0.0),
        pt(17.0, 8.0, 8.0),
    ]);
    assert_eq!(corner.violations(&turning(90.1)), vec![]);
    assert_eq!(corner.violations(&turning(89.0)).len(), 1);
}

#[test]
fn reversing_without_stopping_at_a_recorded_point_is_never_admitted() {
    // Out and straight back with no wait: the interpolant passes through
    // zero speed and flips direction instantly.
    let plan = plan_of(vec![
        pt(0.0, 0.0, 0.0),
        pt(8.0, 8.0, 0.0),
        pt(16.0, 0.0, 0.0),
    ]);
    let found = plan.violations(&RouteLimits {
        max_heading_rate_dps: 1e6,
        ..NO_LIMITS
    });
    assert!(!found.is_empty());
    assert!(found.iter().all(|v| matches!(
        v.constraint,
        RouteConstraint::TurnAtStop | RouteConstraint::HeadingRate
    )));
    assert!(found.iter().any(|v| v.observed >= 180.0 - 1e-6));
}

#[test]
fn violations_refer_to_the_routes_own_segments_under_reverse_playback() {
    let route = Route::new(walk_points()).unwrap();
    let fwd = RoutePlan::build(&route, &forward(), 0.0).unwrap();
    let reverse = PlaybackParameters {
        reverse: true,
        ..forward()
    };
    let rev = RoutePlan::build(&route, &reverse, 0.0).unwrap();
    let (segment, peak) = worst(&fwd, |k| k.peak_heading_rate_dps.value);
    let limits = RouteLimits {
        max_heading_rate_dps: peak * 0.999,
        ..NO_LIMITS
    };
    let (f, r) = (fwd.violations(&limits), rev.violations(&limits));
    assert_eq!((f.len(), r.len()), (1, 1));
    assert_eq!(f[0].segment, segment);
    assert_eq!(r[0].segment, segment);
    // …at mirrored times.
    assert!((f[0].at_elapsed_s + r[0].at_elapsed_s - 16.0).abs() < 1e-2);
}

#[test]
fn a_loop_seam_is_validated_like_any_other_point() {
    let looped = PlaybackParameters {
        looping: true,
        ..forward()
    };
    let good = RoutePlan::build(&square(), &looped, 0.0).unwrap();
    let (_, turn) = worst(&good, |k| k.peak_heading_rate_dps.value);
    let limits = RouteLimits {
        max_heading_rate_dps: turn * 1.5,
        ..NO_LIMITS
    };
    assert_eq!(good.violations(&limits), vec![]);

    // Same square, but the walker stops dead at the start corner for only
    // 0.5 s per lap and leaves at right angles to the way it arrived.
    let stop_at_seam = Route::new(vec![
        pt(0.0, 0.0, 0.0),
        pt(10.0, 20.0, 0.0),
        pt(20.0, 20.0, 20.0),
        pt(30.0, 0.0, 20.0),
        pt(40.0, 0.0, 0.0),
        pt(40.5, 0.0, 0.0),
        pt(50.5, 20.0, 0.0),
        pt(60.5, 20.0, 20.0),
        pt(70.5, 0.0, 20.0),
        pt(80.5, 0.0, 0.0),
    ])
    .unwrap();
    assert!(stop_at_seam.is_closed());
    let plan = RoutePlan::build(&stop_at_seam, &looped, 0.0).unwrap();
    // The mid-route stop (0.5 s) needs 90° of turn: 180°/s. So does nothing
    // at the seam, which is passed at speed. With 100°/s the stop fails.
    let found = plan.violations(&RouteLimits {
        max_heading_rate_dps: 100.0,
        ..NO_LIMITS
    });
    assert!(found
        .iter()
        .any(|v| v.constraint == RouteConstraint::TurnAtStop && v.segment == 3));
}
