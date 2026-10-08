//! Property and statistical tests for the noise engine in isolation.
//!
//! Every limit is asserted strictly (no tolerance) against exact geodesic
//! distances, recomputed here independently of the engine.

use locsim_core::domain::{Boundary, Coordinate, NoiseParameters, Timestamp, NOISE_CLIP_SIGMA};
use locsim_core::geographic::{bearing_difference, distance, normalize_bearing, EnuFrame};
use locsim_core::movement::MovementSample;
use locsim_core::noise::{NoiseConstraints, NoiseEngine, NoisySample};
use locsim_core::rng::Rng;

const H_ACC: f64 = 6.0;
const V_ACC: f64 = 9.0;

/// One randomised situation: a noise configuration plus a base trajectory
/// (a circle of `path_radius_m` around `center`, or a fixed point).
#[derive(Debug, Clone, Copy)]
struct Case {
    params: NoiseParameters,
    max_speed_mps: f64,
    boundary_radius_m: Option<f64>,
    center: Coordinate,
    path_radius_m: f64,
    base_speed_mps: f64,
}

#[derive(Debug, Default)]
struct Outcome {
    samples: Vec<(Coordinate, NoisySample)>,
    fallbacks: u64,
    max_offset_seen_m: f64,
    moved: bool,
}

/// Runs `n` samples with time steps from `next_dt_s`, asserting every
/// guarantee on every sample.
fn run_and_check(
    case: &Case,
    seed: u64,
    n: usize,
    mut next_dt_s: impl FnMut() -> f64,
    label: &str,
) -> Outcome {
    let boundary = case.boundary_radius_m.map(|radius_m| Boundary {
        center: case.center,
        radius_m,
    });
    let constraints = NoiseConstraints {
        max_speed_mps: case.max_speed_mps,
        boundary,
    };
    let mut engine =
        NoiseEngine::new(case.params, constraints, seed).unwrap_or_else(|e| panic!("{label}: {e}"));
    let frame = EnuFrame::new(case.center, 0.0).unwrap();

    let mut out = Outcome::default();
    let mut now = 1_700_000_000_000_000_000i64;
    let mut theta = 0.0f64;
    let mut previous: Option<(Timestamp, Coordinate, Coordinate)> = None;

    for i in 0..n {
        let dt_s = next_dt_s();
        now += ((dt_s * 1e9) as i64).max(1);
        let t = Timestamp::from_nanos(now);
        let moving = case.path_radius_m > 0.0 && case.base_speed_mps > 0.0;
        if moving {
            theta += case.base_speed_mps * dt_s / case.path_radius_m;
        }
        let base_coordinate = frame
            .horizontal_to_coordinate(
                case.path_radius_m * theta.cos(),
                case.path_radius_m * theta.sin(),
            )
            .unwrap();
        let base = MovementSample {
            coordinate: base_coordinate,
            altitude_m: 100.0,
            speed_mps: Some(if moving { case.base_speed_mps } else { 0.0 }),
            // Direction of travel on a counter-clockwise circle.
            course_deg: moving.then(|| normalize_bearing(-theta.to_degrees())),
        };

        let s = engine
            .apply(t, &base, H_ACC, V_ACC)
            .unwrap_or_else(|e| panic!("{label} sample {i}: {e}"));
        let ctx = || format!("{label} sample {i} ({case:?})");

        // Coordinate bounds and finiteness.
        let (lat, lon) = (s.coordinate.latitude(), s.coordinate.longitude());
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
        assert!(s.altitude_m == 100.0, "{}", ctx());

        // 1. Offset from the true position.
        let offset = distance(base_coordinate, s.coordinate).unwrap();
        assert!(
            offset <= case.params.max_position_offset_m,
            "offset {offset}: {}",
            ctx()
        );
        assert_eq!(offset, s.position_offset_m, "{}", ctx());
        out.max_offset_seen_m = out.max_offset_seen_m.max(offset);
        if !case.params.has_position_noise() {
            assert_eq!(s.coordinate, base_coordinate, "{}", ctx());
        }

        // 2. Scenario boundary.
        if let Some(b) = boundary {
            let d = distance(b.center, s.coordinate).unwrap();
            assert!(d <= b.radius_m, "boundary {d} > {}: {}", b.radius_m, ctx());
        }

        // 3. Displacement since the previous output.
        if let Some((prev_t, prev_base, prev_out)) = previous {
            let dt = t.seconds_since(prev_t);
            assert!(dt > 0.0);
            let limit = distance(prev_base, base_coordinate).unwrap()
                + case.params.max_offset_rate_mps * dt;
            let step = distance(prev_out, s.coordinate).unwrap();
            assert!(step <= limit, "step {step} > {limit}: {}", ctx());
            out.moved |= step > 0.0;
        }

        // Metadata: speed bounded, course consistent with speed.
        let speed = s.speed_mps.expect("speed stays known");
        assert!(speed.is_finite() && speed >= 0.0, "{}", ctx());
        assert!(speed <= case.max_speed_mps, "speed {speed}: {}", ctx());
        match s.course_deg {
            Some(c) => {
                assert!(speed > 0.0 && (0.0..360.0).contains(&c), "{}", ctx());
                let reach = NOISE_CLIP_SIGMA * case.params.heading_noise_deg;
                let delta = bearing_difference(base.course_deg.unwrap(), c).abs();
                assert!(delta <= reach + 1e-9, "course moved {delta}: {}", ctx());
            }
            None => assert!(!moving || speed == 0.0, "{}", ctx()),
        }
        if !moving {
            assert_eq!((s.speed_mps, s.course_deg), (Some(0.0), None), "{}", ctx());
        }

        // Accuracy stays positive and within the clipped noise reach.
        let reach = NOISE_CLIP_SIGMA * case.params.accuracy_noise_m;
        for (value, nominal) in [
            (s.horizontal_accuracy_m, H_ACC),
            (s.vertical_accuracy_m, V_ACC),
        ] {
            assert!(value.is_finite() && value > 0.0, "{}", ctx());
            assert!((value - nominal).abs() <= reach + 1e-12, "{}", ctx());
        }

        previous = Some((t, base_coordinate, s.coordinate));
        out.samples.push((base_coordinate, s));
    }
    out.fallbacks = engine.fallback_count();
    out
}

fn random_center(rng: &mut Rng, case: u64) -> Coordinate {
    let (lat, lon) = match case % 12 {
        0 => (90.0, 0.0),
        1 => (-90.0, 0.0),
        2 => (0.0, 180.0),
        3 => (rng.uniform(-89.0, 89.0), -180.0),
        4 => (89.9999, rng.uniform(-180.0, 180.0)),
        _ => (rng.uniform(-90.0, 90.0), rng.uniform(-180.0, 180.0)),
    };
    Coordinate::new(lat, lon).unwrap()
}

fn log_uniform(rng: &mut Rng, low_exp: f64, high_exp: f64) -> f64 {
    10f64.powf(rng.uniform(low_exp, high_exp))
}

fn random_case(rng: &mut Rng, case: u64) -> Case {
    let sigma = if case % 7 == 0 {
        0.0
    } else {
        log_uniform(rng, -2.0, 1.5)
    };
    let rate = log_uniform(rng, -2.0, 1.5);
    // Bounds from much tighter than the noise to much looser.
    let mut max_offset = sigma.max(0.01) * rng.uniform(0.2, 20.0);
    let drift_wanted = case % 3 == 0;
    if drift_wanted && max_offset <= NOISE_CLIP_SIGMA * sigma {
        max_offset = NOISE_CLIP_SIGMA * sigma * rng.uniform(1.01, 4.0) + 0.01;
    }
    let drift = if drift_wanted {
        rng.uniform(0.0, rate)
    } else {
        0.0
    };
    let max_speed = if case % 5 == 0 {
        0.0
    } else {
        rng.uniform(0.1, 40.0)
    };
    let boundary_radius_m = (case % 2 == 0).then(|| log_uniform(rng, 0.5, 3.5));
    let path_radius_m = match boundary_radius_m {
        // Right up against the edge in some cases.
        Some(r) => {
            r * if case % 4 == 0 {
                0.999
            } else {
                rng.uniform(0.0, 0.9)
            }
        }
        None => rng.uniform(0.0, 800.0),
    };
    Case {
        params: NoiseParameters {
            position_noise_m: sigma,
            max_position_offset_m: max_offset,
            speed_noise_mps: if case % 2 == 1 {
                rng.uniform(0.0, 3.0)
            } else {
                0.0
            },
            heading_noise_deg: if case % 2 == 1 {
                rng.uniform(0.0, 40.0)
            } else {
                0.0
            },
            accuracy_noise_m: rng.uniform(0.0, 1.9),
            drift_rate_mps: drift,
            position_correlation_time_s: if case % 4 == 1 {
                0.0
            } else {
                log_uniform(rng, -1.0, 2.5)
            },
            max_offset_rate_mps: rate,
        },
        max_speed_mps: max_speed,
        boundary_radius_m,
        center: random_center(rng, case),
        path_radius_m,
        base_speed_mps: rng.uniform(0.0, max_speed),
    }
}

#[test]
fn limits_hold_for_random_configurations_motions_and_time_steps() {
    let mut rng = Rng::from_seed(0x0401);
    let (mut total, mut fallbacks, mut moved) = (0u64, 0u64, 0u64);
    for case_index in 0..600u64 {
        let case = random_case(&mut rng, case_index);
        let mut dt_rng = Rng::from_seed(case_index);
        // 10 ms … 30 s, irregular.
        let out = run_and_check(
            &case,
            case_index ^ 0xFEED,
            250,
            || log_uniform(&mut dt_rng, -2.0, 1.5),
            &format!("case {case_index}"),
        );
        total += out.samples.len() as u64;
        fallbacks += out.fallbacks;
        moved += out.moved as u64;
    }
    assert_eq!(total, 600 * 250);
    // The test must actually exercise noise, and the fallback must stay rare.
    assert!(moved > 400, "only {moved} cases produced position noise");
    assert!(
        fallbacks * 1000 < total,
        "fallback used for {fallbacks} of {total} samples"
    );
}

fn base_case() -> Case {
    Case {
        params: NoiseParameters {
            position_noise_m: 2.0,
            max_position_offset_m: 1_000.0,
            max_offset_rate_mps: 1e6,
            ..NoiseParameters::NONE
        },
        max_speed_mps: 0.0,
        boundary_radius_m: None,
        center: Coordinate::new(12.9352, 77.6245).unwrap(),
        path_radius_m: 0.0,
        base_speed_mps: 0.0,
    }
}

#[test]
fn extreme_configurations_stay_within_limits() {
    let base = base_case();
    let with = |f: &dyn Fn(&mut Case)| {
        let mut c = base;
        f(&mut c);
        c
    };
    let cases: Vec<(&str, Case, f64)> = vec![
        (
            "huge noise, tiny bound",
            with(&|c| {
                c.params.position_noise_m = 1e4;
                c.params.max_position_offset_m = 1e-3;
            }),
            1.0,
        ),
        (
            "tiny offset rate",
            with(&|c| c.params.max_offset_rate_mps = 1e-9),
            1.0,
        ),
        (
            "huge time steps",
            with(&|c| {
                c.params.drift_rate_mps = 0.5;
                c.params.max_position_offset_m = 50.0;
                c.params.position_correlation_time_s = 30.0;
            }),
            1e6,
        ),
        (
            "millisecond time steps",
            with(&|c| {
                c.params.max_offset_rate_mps = 2.0;
                c.params.position_correlation_time_s = 5.0;
            }),
            1e-3,
        ),
        (
            "boundary far smaller than noise, at a pole",
            with(&|c| {
                c.center = Coordinate::new(90.0, 0.0).unwrap();
                c.boundary_radius_m = Some(0.5);
                c.params.position_noise_m = 30.0;
                c.params.max_position_offset_m = 200.0;
            }),
            1.0,
        ),
        (
            "fast mover hugging the boundary on the antimeridian",
            with(&|c| {
                c.center = Coordinate::new(-16.5, 180.0).unwrap();
                c.boundary_radius_m = Some(500.0);
                c.path_radius_m = 499.9;
                c.max_speed_mps = 60.0;
                c.base_speed_mps = 60.0;
                c.params.position_noise_m = 5.0;
                c.params.max_position_offset_m = 25.0;
                c.params.max_offset_rate_mps = 3.0;
                c.params.speed_noise_mps = 5.0;
                c.params.heading_noise_deg = 20.0;
                c.params.position_correlation_time_s = 8.0;
            }),
            0.5,
        ),
        (
            "long trajectory forcing frame re-anchoring",
            with(&|c| {
                c.path_radius_m = 50_000.0;
                c.max_speed_mps = 40.0;
                c.base_speed_mps = 40.0;
                c.params.position_noise_m = 3.0;
                c.params.drift_rate_mps = 0.2;
                c.params.max_position_offset_m = 30.0;
                c.params.max_offset_rate_mps = 4.0;
                c.params.position_correlation_time_s = 10.0;
            }),
            10.0,
        ),
        (
            "everything disabled",
            with(&|c| c.params = NoiseParameters::NONE),
            1.0,
        ),
    ];
    for (label, case, dt) in cases {
        let out = run_and_check(&case, 77, 3_000, || dt, label);
        assert_eq!(out.samples.len(), 3_000, "{label}");
        if label == "tiny offset rate" {
            // The first sample may sit anywhere inside the offset bound, but
            // from then on the output can move at most 1 nm per sample.
            let first = out.samples[0].1.coordinate;
            let last = out.samples[2_999].1.coordinate;
            assert!(distance(first, last).unwrap() <= 3_000.0 * 1e-9);
        }
        if label == "long trajectory forcing frame re-anchoring" {
            assert!(out.moved && out.max_offset_seen_m > 1.0, "{label}");
        }
    }
}

#[test]
fn same_seed_and_configuration_reproduce_bit_identical_output() {
    let mut rng = Rng::from_seed(0x0402);
    for case_index in 0..40u64 {
        let case = random_case(&mut rng, case_index);
        let run = |seed| {
            let mut dt_rng = Rng::from_seed(5);
            run_and_check(
                &case,
                seed,
                300,
                || log_uniform(&mut dt_rng, -1.0, 1.0),
                "repro",
            )
            .samples
        };
        let (a, b, c) = (run(1), run(1), run(2));
        assert_eq!(a, b, "case {case_index}");
        let any_noise = case.params.has_position_noise() || case.params.accuracy_noise_m > 0.0;
        if any_noise {
            assert_ne!(a, c, "case {case_index}: seed had no effect");
        }
    }
}

/// East/north offsets of the output relative to its own true position.
fn offsets(out: &Outcome) -> (Vec<f64>, Vec<f64>) {
    out.samples
        .iter()
        .map(|(base, s)| {
            let enu = EnuFrame::new(*base, 0.0)
                .unwrap()
                .to_enu(s.coordinate, 0.0)
                .unwrap();
            (enu.east, enu.north)
        })
        .unzip()
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

fn std_dev(v: &[f64]) -> f64 {
    let m = mean(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

fn lag1_autocorrelation(v: &[f64]) -> f64 {
    let m = mean(v);
    let var: f64 = v.iter().map(|x| (x - m).powi(2)).sum();
    let cov: f64 = v.windows(2).map(|w| (w[0] - m) * (w[1] - m)).sum();
    cov / var
}

fn mean_step(e: &[f64], n: &[f64]) -> f64 {
    let steps: Vec<f64> = e
        .windows(2)
        .zip(n.windows(2))
        .map(|(a, b)| (a[1] - a[0]).hypot(b[1] - b[0]))
        .collect();
    mean(&steps)
}

#[test]
fn uncorrelated_jitter_has_the_configured_statistics() {
    // Limits loose enough not to interfere: pure clipped Gaussian per axis.
    let out = run_and_check(&base_case(), 2024, 50_000, || 1.0, "white");
    assert_eq!(out.fallbacks, 0);
    let (e, n) = offsets(&out);
    for (axis, v) in [("east", &e), ("north", &n)] {
        assert!(mean(v).abs() < 0.03, "{axis} mean {}", mean(v));
        // σ = 2 m clipped at 3 σ has a standard deviation of 0.9866 σ.
        let sd = std_dev(v);
        assert!((1.93..2.01).contains(&sd), "{axis} std {sd}");
        let r = lag1_autocorrelation(v);
        assert!(r.abs() < 0.02, "{axis} lag-1 {r}");
        let max = v.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        assert!(max <= 6.0 + 1e-6 && max > 5.0, "{axis} max {max}");
    }
    // The two axes are independent.
    let cross: f64 =
        e.iter().zip(&n).map(|(a, b)| a * b).sum::<f64>() / e.len() as f64 / (1.973 * 1.973);
    assert!(cross.abs() < 0.02, "cross-correlation {cross}");
}

#[test]
fn correlation_time_produces_smooth_jitter_not_independent_jumps() {
    let white = run_and_check(&base_case(), 31, 50_000, || 1.0, "white");
    let mut case = base_case();
    case.params.position_correlation_time_s = 10.0;
    let smooth = run_and_check(&case, 31, 50_000, || 1.0, "correlated");

    let (we, wn) = offsets(&white);
    let (se, sn) = offsets(&smooth);
    for (axis, v) in [("east", &se), ("north", &sn)] {
        // Same marginal distribution as the white case…
        assert!(mean(v).abs() < 0.2, "{axis} mean {}", mean(v));
        let sd = std_dev(v);
        assert!((1.87..2.07).contains(&sd), "{axis} std {sd}");
        // …but consecutive samples are correlated by exp(−dt/τ) = 0.905.
        let r = lag1_autocorrelation(v);
        assert!((0.88..0.93).contains(&r), "{axis} lag-1 {r}");
    }
    // Sample-to-sample movement shrinks by about sqrt(1 − ρ) ≈ 0.31.
    let ratio = mean_step(&se, &sn) / mean_step(&we, &wn);
    assert!((0.25..0.40).contains(&ratio), "step ratio {ratio}");
}

#[test]
fn correlation_depends_on_elapsed_time_not_on_sample_count() {
    // Halving the time step must raise lag-1 correlation to exp(−0.5/10).
    let mut case = base_case();
    case.params.position_correlation_time_s = 10.0;
    let out = run_and_check(&case, 8, 50_000, || 0.5, "half-step");
    let (e, _) = offsets(&out);
    let r = lag1_autocorrelation(&e);
    assert!((0.935..0.965).contains(&r), "lag-1 {r}");
}

#[test]
fn drift_is_slow_continuous_and_bounded() {
    let mut case = base_case();
    case.params = NoiseParameters {
        drift_rate_mps: 0.05,
        max_position_offset_m: 10.0,
        max_offset_rate_mps: 1.0,
        ..NoiseParameters::NONE
    };
    let out = run_and_check(&case, 5, 20_000, || 1.0, "drift");
    assert_eq!(out.fallbacks, 0);
    let (e, n) = offsets(&out);
    let steps: Vec<f64> = e
        .windows(2)
        .zip(n.windows(2))
        .map(|(a, b)| (a[1] - a[0]).hypot(b[1] - b[0]))
        .collect();
    // Never faster than the drift rate, and almost always exactly at it
    // (slower only on the sample where a waypoint is rounded).
    let fastest = steps.iter().fold(0.0f64, |m, s| m.max(*s));
    assert!(fastest <= 0.05 + 1e-6, "fastest step {fastest}");
    assert!(mean(&steps) > 0.045, "mean step {}", mean(&steps));
    // Wanders over the disc without leaving it; highly correlated in time.
    assert!(out.max_offset_seen_m > 5.0 && out.max_offset_seen_m <= 10.0);
    assert!(lag1_autocorrelation(&e) > 0.999);
    assert!(std_dev(&e) > 1.0 && std_dev(&n) > 1.0);
}

#[test]
fn offset_rate_limit_caps_how_fast_noise_can_move_the_output() {
    // White 2 m jitter wants ~3.5 m jumps per second; the limit allows 0.2.
    let mut case = base_case();
    case.params.max_offset_rate_mps = 0.2;
    let out = run_and_check(&case, 12, 20_000, || 1.0, "rate-limited");
    let (e, n) = offsets(&out);
    let fastest = e
        .windows(2)
        .zip(n.windows(2))
        .map(|(a, b)| (a[1] - a[0]).hypot(b[1] - b[0]))
        .fold(0.0f64, f64::max);
    assert!(fastest <= 0.2, "fastest step {fastest}");
    assert!(fastest > 0.19, "limit never reached: {fastest}");
}
