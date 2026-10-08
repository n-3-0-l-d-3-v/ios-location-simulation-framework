//! Property tests for the movement models in isolation (no provider, no
//! noise, no validation gate): every limit is recomputed here from the
//! trajectory itself and asserted strictly.

use locsim_core::domain::{Boundary, Coordinate, RotationDirection, Timestamp};
use locsim_core::geographic::{bearing_difference, distance, inverse};
use locsim_core::movement::{
    CircularModel, Cruise, Kinematics, MovementModel, MovementSample, SteeredConfig, SteeredModel,
};
use locsim_core::rng::Rng;

const START: i64 = 1_700_000_000_000_000_000;

fn log_uniform(rng: &mut Rng, low_exp: f64, high_exp: f64) -> f64 {
    10f64.powf(rng.uniform(low_exp, high_exp))
}

fn special_origin(rng: &mut Rng, case: u64) -> Coordinate {
    let (lat, lon) = match case % 8 {
        0 => (rng.uniform(89.9, 89.99), rng.uniform(-180.0, 180.0)), // ~1–11 km from the pole
        1 => (rng.uniform(-89.99, -89.9), rng.uniform(-180.0, 180.0)),
        2 => (rng.uniform(-70.0, 70.0), 180.0),
        3 => (rng.uniform(-70.0, 70.0), -179.99999),
        _ => (rng.uniform(-85.0, 85.0), rng.uniform(-180.0, 180.0)),
    };
    Coordinate::new(lat, lon).unwrap()
}

fn random_config(rng: &mut Rng, case: u64) -> SteeredConfig {
    let max_speed = log_uniform(rng, -0.5, 1.7); // 0.3 … 50 m/s
    let min_speed = max_speed * rng.uniform(0.1, 0.9);
    SteeredConfig {
        origin: special_origin(rng, case),
        altitude_m: 10.0,
        max_speed_mps: max_speed,
        max_acceleration_mps2: log_uniform(rng, -1.0, 1.0),
        max_deceleration_mps2: log_uniform(rng, -1.0, 1.0),
        max_heading_rate_dps: log_uniform(rng, 0.0, 2.3),
        boundary: None, // filled in by the caller, it needs the origin
        cruise: if case % 3 == 0 {
            Cruise::Constant(rng.uniform(min_speed, max_speed))
        } else {
            Cruise::Varying {
                min_mps: min_speed,
                max_mps: max_speed,
                mean_hold_s: rng.uniform(0.0, 60.0),
            }
        },
        heading_persistence: rng.uniform(0.0, 1.0),
        pause_probability: if case % 2 == 0 {
            rng.uniform(0.0, 0.1)
        } else {
            0.0
        },
        max_pause_s: rng.uniform(0.0, 30.0),
        seed: case,
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Stats {
    steps: u64,
    clamps: u64,
    moved_steps: u64,
    max_speed_fraction: f64,
    max_accel_fraction: f64,
    max_decel_fraction: f64,
    max_turn_fraction: f64,
    max_radius_fraction: f64,
}

impl Stats {
    fn absorb(&mut self, other: Stats) {
        self.steps += other.steps;
        self.clamps += other.clamps;
        self.moved_steps += other.moved_steps;
        for (a, b) in [
            (&mut self.max_speed_fraction, other.max_speed_fraction),
            (&mut self.max_accel_fraction, other.max_accel_fraction),
            (&mut self.max_decel_fraction, other.max_decel_fraction),
            (&mut self.max_turn_fraction, other.max_turn_fraction),
            (&mut self.max_radius_fraction, other.max_radius_fraction),
        ] {
            *a = a.max(b);
        }
    }
}

/// Runs a steered model with the given time steps and checks every limit on
/// every step. Returns the trajectory and how close it came to each limit.
fn run_steered(
    config: SteeredConfig,
    steps: usize,
    mut next_dt_s: impl FnMut() -> f64,
    label: &str,
) -> (Vec<(Timestamp, MovementSample, Kinematics)>, Stats) {
    let mut model = SteeredModel::new(config);
    let mut now = START;
    let mut out: Vec<(Timestamp, MovementSample, Kinematics)> = Vec::with_capacity(steps);
    let mut stats = Stats::default();

    for i in 0..steps {
        if i > 0 {
            now += ((next_dt_s() * 1e9) as i64).max(1);
        }
        let t = Timestamp::from_nanos(now);
        let sample = model
            .sample_at(t)
            .unwrap_or_else(|e| panic!("{label} step {i}: {e}"));
        let k = model.kinematics().unwrap();
        let ctx = || format!("{label} step {i}: {k:?} ({config:?})");

        // The sample and the kinematic state agree.
        assert_eq!(sample.coordinate, k.position, "{}", ctx());
        assert_eq!(sample.speed_mps, Some(k.speed_mps), "{}", ctx());
        assert_eq!(sample.course_deg.is_some(), k.speed_mps > 0.0, "{}", ctx());
        if let Some(course) = sample.course_deg {
            assert_eq!(course, k.heading_deg, "{}", ctx());
        }

        // Geographic validity.
        let (lat, lon) = (k.position.latitude(), k.position.longitude());
        assert!(
            lat.is_finite() && (-90.0..=90.0).contains(&lat),
            "{}",
            ctx()
        );
        assert!(
            lon.is_finite() && (-180.0..=180.0).contains(&lon),
            "{}",
            ctx()
        );
        assert!((0.0..360.0).contains(&k.heading_deg), "{}", ctx());
        assert!(k.acceleration_mps2.is_finite() && k.heading_rate_dps.is_finite());

        // Speed bound.
        assert!(
            k.speed_mps >= 0.0 && k.speed_mps <= config.max_speed_mps,
            "{}",
            ctx()
        );
        stats.max_speed_fraction = stats
            .max_speed_fraction
            .max(k.speed_mps / config.max_speed_mps);

        // Boundary.
        if let Some(b) = config.boundary {
            let d = distance(b.center, k.position).unwrap();
            assert!(d <= b.radius_m, "outside by {}: {}", d - b.radius_m, ctx());
            stats.max_radius_fraction = stats.max_radius_fraction.max(d / b.radius_m);
        }

        if let Some((prev_t, _, prev)) = out.last().copied() {
            let dt = t.seconds_since(prev_t);
            assert!(dt > 0.0);
            let line = inverse(prev.position, k.position).unwrap();

            // Displacement: no teleporting.
            let limit = config.max_speed_mps * dt;
            assert!(
                line.distance_m <= limit,
                "moved {} > {limit}: {}",
                line.distance_m,
                ctx()
            );
            stats.moved_steps += (line.distance_m > 0.0) as u64;

            // Acceleration and deceleration, from the speeds themselves.
            let dv = k.speed_mps - prev.speed_mps;
            let (up, down) = (
                config.max_acceleration_mps2 * dt,
                config.max_deceleration_mps2 * dt,
            );
            assert!(dv <= up, "accelerated {dv} > {up}: {}", ctx());
            assert!(-dv <= down, "braked {} > {down}: {}", -dv, ctx());
            if dv > 0.0 {
                stats.max_accel_fraction = stats.max_accel_fraction.max(dv / up);
            } else {
                stats.max_decel_fraction = stats.max_decel_fraction.max(-dv / down);
            }
            assert!((k.acceleration_mps2 - dv / dt).abs() <= 1e-9 * (1.0 + (dv / dt).abs()));

            // Heading rate: turning relative to the geodesic just travelled.
            let turn_limit = config.max_heading_rate_dps * dt;
            if turn_limit < 180.0 {
                let carried = prev.heading_deg + line.convergence_deg;
                let turn = bearing_difference(carried, k.heading_deg).abs();
                assert!(
                    turn <= turn_limit,
                    "turned {turn} > {turn_limit}: {}",
                    ctx()
                );
                stats.max_turn_fraction = stats.max_turn_fraction.max(turn / turn_limit);
                // …and the model's own figure describes the same turn.
                assert!(
                    (k.heading_rate_dps.abs() * dt - turn).abs() < 1e-6,
                    "reported {} vs measured {turn}: {}",
                    k.heading_rate_dps.abs() * dt,
                    ctx()
                );
            }
        } else {
            // First sample: at the origin, at rest.
            assert_eq!(k.position, config.origin, "{}", ctx());
            assert_eq!(k.speed_mps, 0.0, "{}", ctx());
        }
        out.push((t, sample, k));
    }
    stats.steps = steps as u64;
    stats.clamps = model.boundary_clamps();
    (out, stats)
}

#[test]
fn steered_models_respect_every_limit_for_random_configurations() {
    let mut rng = Rng::from_seed(0x0501);
    let mut total = Stats::default();
    let cases = 500u64;
    for case in 0..cases {
        let mut config = random_config(&mut rng, case);
        if case % 2 == 0 {
            config.boundary = Some(Boundary {
                center: config.origin,
                radius_m: log_uniform(&mut rng, 0.7, 3.5), // 5 m … 3 km
            });
        }
        let mut dt_rng = Rng::from_seed(case ^ 0xD7);
        // 1 ms … 100 s, irregular.
        let (_, stats) = run_steered(
            config,
            300,
            || log_uniform(&mut dt_rng, -3.0, 2.0),
            &format!("case {case}"),
        );
        total.absorb(stats);
    }
    println!("steered random configurations: {total:?}");
    assert_eq!(total.steps, cases * 300);
    // The models really move and really use their envelopes…
    assert!(total.moved_steps > total.steps / 2, "{total:?}");
    assert!(total.max_speed_fraction > 0.99, "{total:?}");
    assert!(
        total.max_accel_fraction > 0.99 && total.max_decel_fraction > 0.99,
        "{total:?}"
    );
    assert!(total.max_turn_fraction > 0.99, "{total:?}");
    // …yet never exceed them (asserted per step) and stay off the fence.
    assert!(total.max_radius_fraction <= 1.0, "{total:?}");
    // Planned steps almost never need the exact re-check to shorten them.
    assert!(total.clamps * 1000 <= total.steps, "{total:?}");
}

fn walker(origin: Coordinate, seed: u64) -> SteeredConfig {
    SteeredConfig {
        origin,
        altitude_m: 0.0,
        max_speed_mps: 1.8,
        max_acceleration_mps2: 0.8,
        max_deceleration_mps2: 1.2,
        max_heading_rate_dps: 60.0,
        boundary: None,
        cruise: Cruise::Varying {
            min_mps: 0.8,
            max_mps: 1.8,
            mean_hold_s: 30.0,
        },
        heading_persistence: 0.9,
        pause_probability: 0.01,
        max_pause_s: 20.0,
        seed,
    }
}

fn driver(origin: Coordinate, seed: u64) -> SteeredConfig {
    SteeredConfig {
        max_speed_mps: 30.0,
        max_acceleration_mps2: 2.5,
        max_deceleration_mps2: 4.5,
        max_heading_rate_dps: 25.0,
        cruise: Cruise::Varying {
            min_mps: 5.0,
            max_mps: 30.0,
            mean_hold_s: 60.0,
        },
        heading_persistence: 0.97,
        pause_probability: 0.005,
        max_pause_s: 45.0,
        ..walker(origin, seed)
    }
}

fn bengaluru() -> Coordinate {
    Coordinate::new(12.9352, 77.6245).unwrap()
}

fn fenced(config: SteeredConfig, radius_m: f64) -> SteeredConfig {
    SteeredConfig {
        boundary: Some(Boundary {
            center: config.origin,
            radius_m,
        }),
        ..config
    }
}

#[test]
fn extreme_but_valid_configurations_and_time_steps() {
    let near_pole = Coordinate::new(89.995, -40.0).unwrap(); // 560 m from the pole
    let date_line = Coordinate::new(-16.5, 179.9999).unwrap();
    let cases: Vec<(&str, SteeredConfig, f64, usize)> = vec![
        ("microsecond steps", walker(bengaluru(), 1), 1e-6, 20_000),
        (
            "millisecond steps, driving",
            driver(bengaluru(), 2),
            1e-3,
            20_000,
        ),
        (
            "ten-minute steps, fenced",
            fenced(walker(bengaluru(), 3), 80.0),
            600.0,
            2_000,
        ),
        (
            "day-long steps, unfenced driving",
            driver(bengaluru(), 4),
            86_400.0,
            300,
        ),
        (
            "driver in a 3 m pen",
            fenced(driver(bengaluru(), 5), 3.0),
            0.5,
            5_000,
        ),
        (
            "walker in a 20 cm pen",
            fenced(walker(bengaluru(), 6), 0.2),
            0.1,
            5_000,
        ),
        (
            "violent dynamics",
            SteeredConfig {
                max_acceleration_mps2: 500.0,
                max_deceleration_mps2: 500.0,
                max_heading_rate_dps: 5_000.0,
                heading_persistence: 0.0,
                ..fenced(driver(bengaluru(), 7), 150.0)
            },
            0.05,
            10_000,
        ),
        (
            "barely able to turn or accelerate",
            SteeredConfig {
                max_acceleration_mps2: 1e-3,
                max_deceleration_mps2: 1e-3,
                max_heading_rate_dps: 0.01,
                ..fenced(walker(bengaluru(), 8), 500.0)
            },
            1.0,
            20_000,
        ),
        (
            "fenced walker next to the pole",
            fenced(walker(near_pole, 9), 300.0),
            1.0,
            10_000,
        ),
        (
            "fence enclosing the pole",
            fenced(driver(near_pole, 10), 2_000.0),
            1.0,
            10_000,
        ),
        (
            "driver straddling the date line",
            fenced(driver(date_line, 11), 400.0),
            0.2,
            10_000,
        ),
        (
            "no wander, no pauses, tiny fence",
            SteeredConfig {
                heading_persistence: 1.0,
                pause_probability: 0.0,
                ..fenced(walker(bengaluru(), 12), 10.0)
            },
            0.5,
            10_000,
        ),
    ];
    for (label, config, dt, steps) in cases {
        let (trajectory, stats) = run_steered(config, steps, || dt, label);
        println!("{label}: {stats:?}");
        assert_eq!(trajectory.len(), steps);
        // Nothing may deadlock: every one of these keeps moving.
        assert!(stats.moved_steps > 0, "{label} never moved");
        assert!(stats.clamps * 100 <= stats.steps, "{label}: {stats:?}");
    }
}

#[test]
fn bounded_random_walk_never_wanders_off_and_uses_its_area() {
    // A long walk: 200 000 steps of 1.2 m in a 25 m disc (67 hours).
    let config = SteeredConfig {
        cruise: Cruise::Constant(1.2),
        heading_persistence: 0.7,
        pause_probability: 0.0,
        ..fenced(walker(bengaluru(), 77), 25.0)
    };
    let (trajectory, stats) = run_steered(config, 200_000, || 1.0, "long walk");
    assert_eq!(stats.clamps, 0);

    let mut radial_sum = 0.0;
    let mut quadrants = [0u32; 4];
    let mut travelled = 0.0;
    let mut previous = config.origin;
    for (_, _, k) in &trajectory {
        let g = inverse(config.origin, k.position).unwrap();
        radial_sum += g.distance_m;
        if g.distance_m > 1.0 {
            quadrants[(g.initial_bearing_deg / 90.0) as usize % 4] += 1;
        }
        travelled += distance(previous, k.position).unwrap();
        previous = k.position;
    }
    let mean_radius = radial_sum / trajectory.len() as f64;
    // Walked ~240 km, yet never more than 25 m from the start.
    assert!(travelled > 200_000.0, "travelled {travelled}");
    assert!(stats.max_radius_fraction <= 1.0);
    // Neither stuck at the centre nor pinned to the fence; all sides visited.
    assert!(
        (8.0..20.0).contains(&mean_radius),
        "mean radius {mean_radius}"
    );
    assert!(stats.max_radius_fraction > 0.9);
    let least = *quadrants.iter().min().unwrap() as f64;
    assert!(least > 0.15 * trajectory.len() as f64, "{quadrants:?}");

    // No slow outward creep: the last tenth looks like the first tenth.
    let mean_of = |range: std::ops::Range<usize>| {
        let n = range.len() as f64;
        trajectory[range]
            .iter()
            .map(|(_, _, k)| distance(config.origin, k.position).unwrap())
            .sum::<f64>()
            / n
    };
    let (early, late) = (mean_of(1_000..21_000), mean_of(180_000..200_000));
    assert!((early - late).abs() < 2.0, "early {early} late {late}");
}

#[test]
fn unfenced_walker_is_free_to_leave() {
    // The fence, not some hidden tether, is what bounds a walk.
    let (trajectory, _) = run_steered(walker(bengaluru(), 5), 3_600, || 1.0, "free walk");
    let furthest = trajectory
        .iter()
        .map(|(_, _, k)| distance(bengaluru(), k.position).unwrap())
        .fold(0.0f64, f64::max);
    assert!(furthest > 200.0, "only got {furthest} m away in an hour");
}

#[test]
fn trajectories_are_reproducible_and_seeds_matter() {
    let mut rng = Rng::from_seed(0x0502);
    for case in 0..30u64 {
        let mut config = random_config(&mut rng, case);
        config.origin = bengaluru();
        config.heading_persistence = rng.uniform(0.0, 0.98);
        let run = |seed: u64| {
            let mut dt_rng = Rng::from_seed(3);
            let c = SteeredConfig { seed, ..config };
            run_steered(c, 400, || log_uniform(&mut dt_rng, -1.0, 1.0), "repro").0
        };
        let (a, b, c) = (run(10), run(10), run(11));
        assert_eq!(a, b, "case {case}");
        // A different seed gives a different path, not a perturbed copy: the
        // very first heading differs, so the tracks separate immediately.
        let apart = a
            .iter()
            .zip(&c)
            .map(|(x, y)| distance(x.2.position, y.2.position).unwrap())
            .fold(0.0f64, f64::max);
        let reach = a
            .iter()
            .map(|x| distance(config.origin, x.2.position).unwrap())
            .fold(0.0f64, f64::max);
        assert!(
            apart > 0.2 * reach && apart > 0.0,
            "case {case}: {apart} vs {reach}"
        );
    }
}

#[test]
fn straight_line_crosses_the_antimeridian_without_a_jump() {
    let start = Coordinate::new(10.0, 179.9995).unwrap(); // 55 m short of 180°
    let config = SteeredConfig {
        cruise: Cruise::Constant(1.5),
        heading_persistence: 1.0,
        pause_probability: 0.0,
        ..walker(start, 0)
    };
    // Find a seed that happens to head roughly east.
    let seed = (0..200u64)
        .find(|seed| {
            let mut m = SteeredModel::new(SteeredConfig {
                seed: *seed,
                ..config
            });
            m.sample_at(Timestamp::from_nanos(START)).unwrap();
            let h = m.kinematics().unwrap().heading_deg;
            (60.0..120.0).contains(&h)
        })
        .expect("some seed heads east");
    let (trajectory, _) = run_steered(SteeredConfig { seed, ..config }, 300, || 1.0, "date line");
    let east = trajectory
        .iter()
        .filter(|(_, _, k)| k.position.longitude() > 0.0)
        .count();
    let west = trajectory
        .iter()
        .filter(|(_, _, k)| k.position.longitude() < 0.0)
        .count();
    assert!(east > 10 && west > 100, "east {east} west {west}");
    // The longitude flips sign; the ground track does not notice.
    for pair in trajectory.windows(2) {
        let step = distance(pair[0].2.position, pair[1].2.position).unwrap();
        assert!(step <= 1.8, "step {step}");
    }
}

#[test]
fn straight_travel_at_high_latitude_keeps_zero_heading_rate() {
    // 2 km from the pole the bearing of a straight line swings quickly, but
    // the model's turn rate must stay zero and the path must stay geodesic.
    let start = Coordinate::new(89.982, 20.0).unwrap();
    let config = SteeredConfig {
        cruise: Cruise::Constant(20.0),
        heading_persistence: 1.0,
        pause_probability: 0.0,
        ..driver(start, 3)
    };
    let (trajectory, _) = run_steered(config, 200, || 1.0, "polar straight");
    let headings: Vec<f64> = trajectory.iter().map(|(_, _, k)| k.heading_deg).collect();
    let swing: f64 = headings
        .windows(2)
        .map(|w| bearing_difference(w[0], w[1]).abs())
        .sum();
    assert!(swing > 20.0, "bearing swung only {swing}");
    assert!(trajectory.iter().all(|(_, _, k)| k.heading_rate_dps == 0.0));
    // Geodesic: the end point is where the first leg's line leads.
    let a = trajectory[20].2.position;
    let leg = inverse(a, trajectory[21].2.position).unwrap();
    let whole = inverse(a, trajectory[199].2.position).unwrap();
    assert!(bearing_difference(leg.initial_bearing_deg, whole.initial_bearing_deg).abs() < 1e-6);
}

#[test]
fn circular_orbits_are_exact_for_random_configurations() {
    let mut rng = Rng::from_seed(0x0503);
    for case in 0..300u64 {
        let center = special_origin(&mut rng, case);
        let radius = log_uniform(&mut rng, 0.0, 3.7); // 1 m … 5 km
        let omega = log_uniform(&mut rng, -2.0, 1.5); // 0.01 … 30 °/s
        let direction = if case % 2 == 0 {
            RotationDirection::Clockwise
        } else {
            RotationDirection::CounterClockwise
        };
        let phase = rng.uniform(-720.0, 720.0);
        let mut m = CircularModel::new(center, 5.0, radius, omega, direction, phase);
        let sign = if case % 2 == 0 { 1.0 } else { -1.0 };
        let nominal = radius * omega.to_radians();

        let mut now = START;
        let mut previous: Option<(Timestamp, Kinematics)> = None;
        for i in 0..200 {
            if i > 0 {
                now += ((log_uniform(&mut rng, -2.0, 1.0) * 1e9) as i64).max(1);
            }
            let t = Timestamp::from_nanos(now);
            let s = m.sample_at(t).unwrap();
            let k = m.kinematics().unwrap();
            let ctx = || format!("case {case} sample {i}: r={radius} w={omega} {center:?}");

            // Geometry: on the circle, at the bearing time dictates.
            let radial = inverse(center, k.position).unwrap();
            assert!((radial.distance_m - radius).abs() < 1e-6, "{}", ctx());
            let elapsed = t.seconds_since(Timestamp::from_nanos(START));
            let expected = phase + sign * omega * elapsed;
            // Bearing error tolerance equivalent to 1 µm on the circle.
            let tolerance = (1e-6 / radius).to_degrees().max(1e-9);
            assert!(
                bearing_difference(expected, radial.initial_bearing_deg).abs() < tolerance,
                "{}",
                ctx()
            );
            assert_eq!(s.speed_mps, Some(nominal));
            assert_eq!(k.heading_rate_dps, sign * omega);

            if let Some((prev_t, prev)) = previous {
                let dt = t.seconds_since(prev_t);
                let line = inverse(prev.position, k.position).unwrap();
                // Never faster over the ground than the nominal speed (a
                // chord is shorter than its arc), within coordinate rounding.
                assert!(
                    line.distance_m <= nominal * dt * (1.0 + 1e-6) + 1e-8,
                    "{}",
                    ctx()
                );
                // Turning matches the angular velocity, never exceeds it.
                let swept = omega * dt;
                if swept < 170.0 {
                    let carried = prev.heading_deg + line.convergence_deg;
                    let turn = bearing_difference(carried, k.heading_deg) * sign;
                    assert!(
                        (turn - swept).abs() < 1e-6 * (1.0 + swept),
                        "{turn} vs {swept}: {}",
                        ctx()
                    );
                }
            }
            previous = Some((t, k));
        }
    }
}
