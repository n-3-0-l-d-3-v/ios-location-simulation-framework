//! Route replay through the real pipeline:
//! route → noise → final validation → provider.
//!
//! Besides showing that admitted routes run cleanly at any sample interval,
//! this file corrupts route output on purpose and checks that the final
//! validation gate — which knows nothing about routes or admission —
//! rejects each kind of corruption by itself.

mod common;

use common::*;
use locsim_core::domain::{
    Coordinate, LocationSource, NoiseParameters, PlaybackParameters, Route, Scenario,
    SimulationState, SyntheticLocation, Timestamp, NOISE_CLIP_SIGMA,
};
use locsim_core::geographic::{bearing_difference, destination, distance, inverse};
use locsim_core::movement::{model_for, MovementError, MovementModel, MovementSample};
use locsim_core::noise::NoiseError;
use locsim_core::provider::{
    LocationProvider, ModelFactory, ProviderError, ProviderStatus, SimulationProvider,
};
use locsim_core::rng::Rng;
use locsim_core::route::{RouteConstraint, RoutePlan, RouteRejection};
use locsim_core::validation::{SampleLimits, SampleValidator, ValidationError};

fn bengaluru() -> Coordinate {
    Coordinate::new(12.9352, 77.6245).unwrap()
}

/// A 30-leg walk at about 1.4 m/s with a few waits.
fn walk_route() -> Route {
    smooth_route(&mut Rng::from_seed(0x0620), bengaluru(), 30, 1.4, true)
}

/// A circuit of about 60 m radius walked in two minutes.
fn circuit_route() -> Route {
    closed_route(&mut Rng::from_seed(0x0621), bengaluru(), 24, 60.0, 120.0)
}

/// A scenario replaying `route` under limits `slack` above what it needs.
fn scenario_for(
    route: &Route,
    playback: PlaybackParameters,
    interval_s: f64,
    slack: f64,
) -> Scenario {
    let plan = RoutePlan::build(route, &playback, 25.0).unwrap();
    let centre = plan.state_at(0).unwrap().coordinate;
    let limits = declared_limits(&plan, centre, slack, interval_s).expect("bounded route");
    let sc = route_scenario(route.clone(), playback, &limits, interval_s);
    assert_eq!(sc.validate(), Ok(()));
    sc
}

/// Polls punctually (one poll per interval) for `seconds` of wall time.
fn run(sc: &Scenario, seconds: f64) -> (Vec<SyntheticLocation>, ProviderStatus) {
    let interval_ns = (sc.update_interval_s * 1e9).round() as i64;
    let polls = (seconds / sc.update_interval_s).ceil() as i64;
    let mut p = SimulationProvider::new(sc.clone());
    p.start(Timestamp::from_nanos(START)).unwrap();
    let mut out = Vec::new();
    for n in 0..=polls {
        match p.poll(Timestamp::from_nanos(START + n * interval_ns)) {
            Ok(Some(s)) => out.push(s),
            Ok(None) => panic!("a sample is due at every poll"),
            Err(e) => panic!("sample {n} rejected: {e}"),
        }
    }
    let status = p.status();
    assert_eq!(
        (status.state, status.failed_count),
        (SimulationState::Running, 0)
    );
    (out, status)
}

/// Independent re-check of an emitted stream against the scenario's limits
/// (with the allowances configured noise is entitled to). Returns the number
/// of consecutive pairs examined.
fn check_stream(sc: &Scenario, stream: &[SyntheticLocation]) -> usize {
    let m = &sc.movement;
    let noisy = sc.noise.has_position_noise();
    let offset_rate = if noisy {
        sc.noise.max_offset_rate_mps
    } else {
        0.0
    };
    let reach = if noisy {
        sc.noise.max_position_offset_m
    } else {
        0.0
    };
    let speed_slack = 2.0 * NOISE_CLIP_SIGMA * sc.noise.speed_noise_mps;
    let heading_slack = 2.0 * NOISE_CLIP_SIGMA * sc.noise.heading_noise_deg;
    let boundary = sc.boundary().unwrap();
    for s in stream {
        assert_eq!(s.validate(), Ok(()));
        assert!(s.speed_mps.unwrap() <= m.max_speed_mps);
        assert!(distance(boundary.center, s.coordinate).unwrap() <= boundary.radius_m);
    }
    for pair in stream.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(b.timestamp > a.timestamp);
        let dt = b.timestamp.seconds_since(a.timestamp);
        let line = inverse(a.coordinate, b.coordinate).unwrap();
        assert!(line.distance_m <= (m.max_speed_mps + offset_rate) * dt);
        let dv = b.speed_mps.unwrap() - a.speed_mps.unwrap();
        assert!(dv <= m.max_acceleration_mps2 * dt + speed_slack, "dv {dv}");
        assert!(-dv <= m.max_deceleration_mps2 * dt + speed_slack, "dv {dv}");
        if let (Some(c0), Some(c1)) = (a.course_deg, b.course_deg) {
            let lat = a
                .coordinate
                .latitude()
                .abs()
                .max(b.coordinate.latitude().abs());
            let convergence_slack = (2.0 * reach / 6.3e6 * lat.to_radians().tan()).to_degrees();
            let limit = m.max_heading_rate_dps * dt + heading_slack + convergence_slack;
            if limit < 180.0 {
                let turn = bearing_difference(c0 + line.convergence_deg, c1).abs();
                assert!(turn <= limit, "turned {turn} > {limit}");
            }
        }
    }
    stream.len() - 1
}

#[test]
fn route_without_noise_at_different_sample_intervals() {
    let route = walk_route();
    let duration = route.duration_s();
    let mut pairs = 0;
    let mut streams = Vec::new();
    for interval in [0.05, 0.37, 1.0, 7.0] {
        let sc = scenario_for(&route, PlaybackParameters::REAL_TIME, interval, 1.0 + 3e-6);
        let (stream, status) = run(&sc, duration + 20.0);
        assert!(status.trajectory_complete);
        pairs += check_stream(&sc, &stream);
        assert!(stream
            .iter()
            .all(|s| s.source == LocationSource::Simulation));
        streams.push((interval, stream));
    }
    println!("route without noise: {pairs} consecutive pairs re-checked over 4 intervals");

    // The interval decides which instants are read, never what is there:
    // at every whole second the 20 Hz and the 1 Hz streams agree exactly.
    let (fine, coarse) = (&streams[0].1, &streams[2].1);
    for (k, c) in coarse.iter().enumerate() {
        let Some(f) = fine.get(k * 20) else {
            break;
        };
        assert_eq!(f.timestamp, c.timestamp);
        assert_eq!(
            (f.coordinate, f.speed_mps, f.course_deg),
            (c.coordinate, c.speed_mps, c.course_deg)
        );
    }
    // And every recorded point that falls on a sampled instant is emitted exactly.
    let mut exact = 0;
    for p in route.points() {
        if p.elapsed_ns % 50_000_000 == 0 {
            let s = &fine[(p.elapsed_ns / 50_000_000) as usize];
            assert_eq!(s.coordinate, p.coordinate);
            exact += 1;
        }
    }
    assert!(exact >= 1);
}

#[test]
fn route_with_noise() {
    let route = walk_route();
    let mut pairs = 0;
    for (interval, seed) in [(0.2, 1u64), (1.0, 2), (3.0, 3)] {
        let mut sc = scenario_for(&route, PlaybackParameters::REAL_TIME, interval, 1.05);
        sc.seed = seed;
        sc.noise = NoiseParameters {
            position_noise_m: 1.5,
            max_position_offset_m: 8.0,
            speed_noise_mps: 0.1,
            heading_noise_deg: 4.0,
            accuracy_noise_m: 0.8,
            drift_rate_mps: 0.05,
            position_correlation_time_s: 6.0,
            max_offset_rate_mps: 1.0,
        };
        assert_eq!(sc.validate(), Ok(()));
        let (stream, status) = run(&sc, route.duration_s() + 30.0);
        assert!(status.trajectory_complete);
        pairs += check_stream(&sc, &stream);

        // Noise really was applied, around the route, and reproducibly.
        let plan = RoutePlan::build(&route, &sc.playback, 25.0).unwrap();
        let mut largest = 0.0f64;
        for (n, s) in stream.iter().enumerate() {
            let truth = plan
                .state_at((n as f64 * interval * 1e9).round() as i64)
                .unwrap();
            let offset = distance(truth.coordinate, s.coordinate).unwrap();
            assert!(offset <= 8.0, "offset {offset}");
            largest = largest.max(offset);
        }
        assert!(largest > 1.0, "largest offset {largest}");
        assert_eq!(run(&sc, route.duration_s() + 30.0).0, stream);
    }
    println!("route with noise: {pairs} consecutive pairs re-checked over 3 intervals");
}

#[test]
fn route_completion_holds_the_final_point() {
    let route = walk_route();
    let last = route.points().last().unwrap().coordinate;
    let sc = scenario_for(&route, PlaybackParameters::REAL_TIME, 0.5, 1.01);
    let interval_ns = 500_000_000i64;
    let mut p = SimulationProvider::new(sc);
    p.start(Timestamp::from_nanos(START)).unwrap();
    let end_tick = (route.duration_ns() as f64 / interval_ns as f64).ceil() as i64;
    for n in 0..end_tick + 40 {
        let s = p
            .poll(Timestamp::from_nanos(START + n * interval_ns))
            .unwrap()
            .unwrap();
        let done = p.status().trajectory_complete;
        assert_eq!(done, n * interval_ns >= route.duration_ns(), "tick {n}");
        if done {
            // No extrapolation, no stop, no error: the last point, at rest.
            assert_eq!(s.coordinate, last);
            assert_eq!((s.speed_mps, s.course_deg), (Some(0.0), None));
            assert_eq!(p.status().state, SimulationState::Running);
        }
    }
    // Stopping and restarting replays from the beginning.
    p.stop().unwrap();
    assert!(!p.status().trajectory_complete);
    p.start(Timestamp::from_nanos(START + 10_000 * SEC))
        .unwrap();
    let first = p
        .poll(Timestamp::from_nanos(START + 10_000 * SEC))
        .unwrap()
        .unwrap();
    assert_eq!(first.coordinate, route.points()[0].coordinate);
    assert!(!p.status().trajectory_complete);
}

#[test]
fn closed_route_loops_without_a_seam() {
    let route = circuit_route();
    let looping = PlaybackParameters {
        looping: true,
        ..PlaybackParameters::REAL_TIME
    };
    let sc = scenario_for(&route, looping, 1.0, 1.0 + 3e-6);
    let (stream, status) = run(&sc, 5.0 * 120.0);
    assert!(!status.trajectory_complete);
    let pairs = check_stream(&sc, &stream);
    println!("closed loop: {pairs} consecutive pairs re-checked over 5 laps");
    // Each lap repeats the first exactly, and the seam is crossed at speed.
    for (k, s) in stream.iter().enumerate().skip(120) {
        let earlier = &stream[k - 120];
        assert_eq!(
            (s.coordinate, s.speed_mps, s.course_deg),
            (earlier.coordinate, earlier.speed_mps, earlier.course_deg)
        );
    }
    for lap in 1..5 {
        let seam = &stream[lap * 120];
        assert_eq!(seam.coordinate, route.points()[0].coordinate);
        assert!(seam.speed_mps.unwrap() > 1.0);
    }
}

#[test]
fn reverse_and_fast_playback_are_ordinary_routes() {
    let route = walk_route();
    let (first, last) = (
        route.points()[0].coordinate,
        route.points().last().unwrap().coordinate,
    );
    let reverse = PlaybackParameters {
        reverse: true,
        ..PlaybackParameters::REAL_TIME
    };
    let sc = scenario_for(&route, reverse, 1.0, 1.01);
    let (stream, status) = run(&sc, route.duration_s() + 5.0);
    check_stream(&sc, &stream);
    assert!(status.trajectory_complete);
    assert_eq!(stream[0].coordinate, last);
    assert_eq!(stream.last().unwrap().coordinate, first);

    let double = PlaybackParameters {
        speed: 2.0,
        ..PlaybackParameters::REAL_TIME
    };
    let sc = scenario_for(&route, double, 1.0, 1.01);
    let (stream, _) = run(&sc, route.duration_s() / 2.0 + 5.0);
    check_stream(&sc, &stream);
    assert_eq!(stream.last().unwrap().coordinate, last);
    // Twice as fast needs roughly twice the speed limit.
    let normal = scenario_for(&route, PlaybackParameters::REAL_TIME, 1.0, 1.01);
    let ratio = sc.movement.max_speed_mps / normal.movement.max_speed_mps;
    assert!((ratio - 2.0).abs() < 1e-3, "{ratio}");
}

#[test]
fn a_route_that_breaks_a_limit_never_starts() {
    let route = walk_route();
    let good = scenario_for(&route, PlaybackParameters::REAL_TIME, 1.0, 1.01);
    type Tighten = fn(&mut Scenario);
    let cases: [(RouteConstraint, Tighten); 5] = [
        (RouteConstraint::MaxSpeed, |s| {
            s.movement.max_speed_mps *= 0.97
        }),
        (RouteConstraint::Acceleration, |s| {
            s.movement.max_acceleration_mps2 *= 0.97
        }),
        (RouteConstraint::Deceleration, |s| {
            s.movement.max_deceleration_mps2 *= 0.97
        }),
        (RouteConstraint::Boundary, |s| {
            s.movement.radius_m = s.movement.radius_m.map(|r| r * 0.97)
        }),
        (RouteConstraint::DisplacementPerSample, |s| {
            s.movement.max_displacement_per_sample_m = Some(s.movement.max_speed_mps * 0.97)
        }),
    ];
    for (constraint, tighten) in cases {
        let mut sc = good.clone();
        tighten(&mut sc);
        let mut p = SimulationProvider::new(sc);
        match p.start(Timestamp::from_nanos(START)) {
            Err(ProviderError::Movement(MovementError::RouteRejected(
                RouteRejection::Violations(violations),
            ))) => {
                assert!(
                    violations.iter().all(|v| v.constraint == constraint),
                    "{violations:?}"
                );
                let v = violations[0];
                assert!(
                    v.observed > v.limit * (1.0 - 1e-6) && v.segment < route.points().len() - 1
                );
            }
            other => panic!("{constraint:?}: unexpected {other:?}"),
        }
        assert_eq!(p.status().state, SimulationState::Idle);
        assert_eq!(p.poll(Timestamp::from_nanos(START)).unwrap(), None);
    }
    // Looping an open route is refused before admission is even attempted.
    let mut open_loop = good.clone();
    open_loop.playback.looping = true;
    let mut p = SimulationProvider::new(open_loop);
    match p.start(Timestamp::from_nanos(START)) {
        Err(ProviderError::InvalidScenario(errors)) => {
            assert_eq!(errors[0].field, "playback.looping");
        }
        other => panic!("unexpected {other:?}"),
    }
}

// --- Corrupted route output must be caught by the gate itself. -------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Corruption {
    Teleport,
    ExcessiveSpeed,
    ExcessiveAcceleration,
    ExcessiveDeceleration,
    ExcessiveTurn,
    BoundaryEscape,
}

const CORRUPTIONS: [Corruption; 6] = [
    Corruption::Teleport,
    Corruption::ExcessiveSpeed,
    Corruption::ExcessiveAcceleration,
    Corruption::ExcessiveDeceleration,
    Corruption::ExcessiveTurn,
    Corruption::BoundaryEscape,
];

/// A scenario with room around the route, so that each corruption below
/// trips exactly the check it is aimed at and no other.
fn roomy_scenario() -> Scenario {
    let route = smooth_route(&mut Rng::from_seed(0x0622), bengaluru(), 30, 1.4, false);
    let mut sc = scenario_for(&route, PlaybackParameters::REAL_TIME, 0.5, 1.05);
    sc.movement.max_speed_mps *= 4.0;
    sc.movement.radius_m = sc.movement.radius_m.map(|r| r * 1.5);
    assert_eq!(sc.validate(), Ok(()));
    sc
}

/// Applies one corruption to an otherwise correct route sample.
fn corrupt(kind: Corruption, sc: &Scenario, clean: MovementSample) -> MovementSample {
    let m = &sc.movement;
    let dt = sc.update_interval_s;
    let speed = clean.speed_mps.unwrap();
    let course = clean.course_deg.unwrap();
    match kind {
        // Three times as far sideways as the speed limit allows in one step.
        Corruption::Teleport => MovementSample {
            coordinate: destination(clean.coordinate, course + 90.0, 3.0 * m.max_speed_mps * dt)
                .unwrap(),
            ..clean
        },
        Corruption::ExcessiveSpeed => MovementSample {
            speed_mps: Some(m.max_speed_mps * 1.2),
            ..clean
        },
        Corruption::ExcessiveAcceleration => MovementSample {
            speed_mps: Some(speed + 3.0 * m.max_acceleration_mps2 * dt),
            ..clean
        },
        Corruption::ExcessiveDeceleration => MovementSample {
            speed_mps: Some(1e-3),
            ..clean
        },
        Corruption::ExcessiveTurn => MovementSample {
            course_deg: Some((course + 150.0) % 360.0),
            ..clean
        },
        // A hand's breadth outside the fence, on the same bearing.
        Corruption::BoundaryEscape => {
            let b = sc.boundary().unwrap();
            let bearing = inverse(b.center, clean.coordinate)
                .unwrap()
                .initial_bearing_deg;
            MovementSample {
                coordinate: destination(b.center, bearing, b.radius_m + 0.1).unwrap(),
                ..clean
            }
        }
    }
}

/// The clean route samples at the scenario interval, and the index of one
/// taken at walking pace well inside the fence, with a moving predecessor.
fn clean_samples(sc: &Scenario) -> (Vec<MovementSample>, usize) {
    let mut model = model_for(sc).unwrap();
    let interval_ns = (sc.update_interval_s * 1e9) as i64;
    let samples: Vec<MovementSample> = (0..150)
        .map(|n| {
            model
                .sample_at(Timestamp::from_nanos(START + n * interval_ns))
                .unwrap()
        })
        .collect();
    let b = sc.boundary().unwrap();
    let limit = sc.movement.max_deceleration_mps2 * sc.update_interval_s;
    let target = (5..samples.len())
        .find(|&i| {
            let ok =
                |s: &MovementSample| s.speed_mps.unwrap() > limit + 0.5 && s.course_deg.is_some();
            ok(&samples[i])
                && ok(&samples[i - 1])
                && distance(b.center, samples[i].coordinate).unwrap() < 0.4 * b.radius_m
        })
        .expect("a cruising sample well inside the fence");
    (samples, target)
}

fn as_location(sc: &Scenario, n: usize, s: &MovementSample) -> SyntheticLocation {
    let interval_ns = (sc.update_interval_s * 1e9) as i64;
    SyntheticLocation {
        timestamp: Timestamp::from_nanos(START + n as i64 * interval_ns),
        coordinate: s.coordinate,
        altitude_m: s.altitude_m,
        horizontal_accuracy_m: sc.horizontal_accuracy_m,
        vertical_accuracy_m: sc.vertical_accuracy_m,
        speed_mps: s.speed_mps,
        course_deg: s.course_deg,
        source: LocationSource::Simulation,
        simulation_state: SimulationState::Running,
    }
}

fn expected(kind: Corruption, e: &ValidationError) -> bool {
    matches!(
        (kind, e),
        (
            Corruption::Teleport,
            ValidationError::ImpossibleDisplacement { .. }
        ) | (
            Corruption::ExcessiveSpeed,
            ValidationError::SpeedAboveMaximum { .. }
        ) | (
            Corruption::ExcessiveAcceleration,
            ValidationError::AccelerationExceeded { .. }
        ) | (
            Corruption::ExcessiveDeceleration,
            ValidationError::DecelerationExceeded { .. }
        ) | (
            Corruption::ExcessiveTurn,
            ValidationError::HeadingRateExceeded { .. }
        ) | (
            Corruption::BoundaryEscape,
            ValidationError::OutsideBoundary { .. }
        )
    )
}

#[test]
fn the_gate_alone_rejects_each_kind_of_corrupted_route_output() {
    // No provider, no noise stage, no admission in the loop: route samples
    // are handed straight to the validator the provider would use.
    let sc = roomy_scenario();
    let (samples, target) = clean_samples(&sc);

    // The clean stream passes in full.
    let mut gate = SampleValidator::with_limits(SampleLimits::for_scenario(&sc));
    for (n, s) in samples.iter().enumerate() {
        assert_eq!(
            gate.validate(&as_location(&sc, n, s)),
            Ok(()),
            "clean sample {n}"
        );
    }

    for kind in CORRUPTIONS {
        let mut gate = SampleValidator::with_limits(SampleLimits::for_scenario(&sc));
        for (n, s) in samples.iter().enumerate().take(target) {
            gate.validate(&as_location(&sc, n, s)).unwrap();
        }
        let bad = corrupt(kind, &sc, samples[target]);
        let error = gate
            .validate(&as_location(&sc, target, &bad))
            .expect_err("corrupted sample accepted");
        assert!(expected(kind, &error), "{kind:?} was reported as {error:?}");
        // The rejected sample left no trace: the true one is still accepted.
        assert_eq!(
            gate.validate(&as_location(&sc, target, &samples[target])),
            Ok(())
        );
    }
}

/// A route model whose `target`-th sample is corrupted.
struct Corrupting {
    inner: Box<dyn MovementModel + Send>,
    scenario: Scenario,
    kind: Corruption,
    target: usize,
    count: usize,
}

impl MovementModel for Corrupting {
    fn sample_at(&mut self, t: Timestamp) -> Result<MovementSample, MovementError> {
        let clean = self.inner.sample_at(t)?;
        self.count += 1;
        if self.count - 1 == self.target {
            Ok(corrupt(self.kind, &self.scenario, clean))
        } else {
            Ok(clean)
        }
    }
}

#[test]
fn corrupted_route_output_never_leaves_the_provider() {
    let sc = roomy_scenario();
    let (_, target) = clean_samples(&sc);
    let interval_ns = (sc.update_interval_s * 1e9) as i64;
    for kind in CORRUPTIONS {
        let for_factory = sc.clone();
        let factory: ModelFactory = Box::new(move |scenario| {
            Ok(Box::new(Corrupting {
                inner: model_for(scenario)?,
                scenario: for_factory.clone(),
                kind,
                target,
                count: 0,
            }))
        });
        let mut p = SimulationProvider::with_model_factory(sc.clone(), factory);
        p.start(Timestamp::from_nanos(START)).unwrap();
        for n in 0..target {
            p.poll(Timestamp::from_nanos(START + n as i64 * interval_ns))
                .unwrap()
                .unwrap();
        }
        let before = p.current_location();
        let error = p
            .poll(Timestamp::from_nanos(START + target as i64 * interval_ns))
            .expect_err("corrupted sample emitted");
        match (&error, kind) {
            // The noise stage sits upstream of the gate and refuses an
            // out-of-bounds base position before the gate sees it; the
            // gate's own boundary check is exercised in the test above.
            (
                ProviderError::Noise(NoiseError::BaseOutsideBoundary { .. }),
                Corruption::BoundaryEscape,
            ) => {}
            (ProviderError::Validation(e), _) if expected(kind, e) => {}
            _ => panic!("{kind:?} was reported as {error:?}"),
        }
        let status = p.status();
        assert_eq!(status.state, SimulationState::Error, "{kind:?}");
        assert_eq!(
            (status.sample_count, status.failed_count),
            (target as u64, 1)
        );
        assert_eq!(p.current_location(), before);
    }
}
