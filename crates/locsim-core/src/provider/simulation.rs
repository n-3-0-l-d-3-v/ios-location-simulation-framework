use super::{LocationProvider, ProviderError, ProviderStatus};
use crate::consistency::{KinematicsDeriver, ObservationNoise};
use crate::domain::{
    InvalidTransition, LocationSource, Scenario, SimulationState, SyntheticLocation, Timestamp,
};
use crate::movement::{model_for, MovementError, MovementModel};
use crate::noise::NoiseEngine;
use crate::scheduler::{Poll, ScheduleError, Tick, TickSchedule};
use crate::validation::{SampleLimits, SampleValidator};

/// Builds the movement model for a run. Replaceable so a different movement
/// engine can be plugged in without touching the provider.
pub type ModelFactory =
    Box<dyn Fn(&Scenario) -> Result<Box<dyn MovementModel + Send>, MovementError> + Send>;

/// Everything that exists only while a run is active.
struct Run {
    /// When the run began; the origin of simulated time.
    started: Timestamp,
    schedule: TickSchedule,
    model: Box<dyn MovementModel + Send>,
    noise: NoiseEngine,
    deriver: KinematicsDeriver,
    validator: SampleValidator,
}

/// Provider that generates samples from a [`Scenario`].
///
/// Pipeline per due tick: movement model → noise → final position →
/// derive speed and course from emitted positions → final validation →
/// emit. Nothing touches a sample after validation, and nothing touches a
/// position after noise. The scenario is immutable for the provider's lifetime.
pub struct SimulationProvider {
    scenario: Scenario,
    model_factory: ModelFactory,
    state: SimulationState,
    run: Option<Run>,
    last: Option<SyntheticLocation>,
    sample_count: u64,
    failed_count: u64,
    missed_ticks: u64,
}

impl SimulationProvider {
    /// The scenario is validated at `start`, not here, so a provider can be
    /// constructed for a scenario that is still being edited.
    pub fn new(scenario: Scenario) -> Self {
        Self::with_model_factory(scenario, Box::new(model_for))
    }

    /// Like [`SimulationProvider::new`] with a custom movement engine.
    pub fn with_model_factory(scenario: Scenario, model_factory: ModelFactory) -> Self {
        Self {
            scenario,
            model_factory,
            state: SimulationState::Idle,
            run: None,
            last: None,
            sample_count: 0,
            failed_count: 0,
            missed_ticks: 0,
        }
    }

    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    /// When the next sample is due, for drivers that want to sleep until then.
    /// `None` unless running.
    pub fn next_deadline(&self) -> Option<Timestamp> {
        if self.state != SimulationState::Running {
            return None;
        }
        self.run.as_ref()?.schedule.next_target().ok()
    }

    fn require(&self, next: SimulationState) -> Result<(), InvalidTransition> {
        self.state.transition(next).map(|_| ())
    }

    fn generate(&mut self, tick: Tick) -> Result<SyntheticLocation, ProviderError> {
        // Invariant: `run` is Some whenever state is Running; checked by caller.
        let run = self.run.as_mut().expect("run exists while running");
        // The model runs on simulated time, `tick index × interval`, which
        // stands still while paused; samples are stamped with wall time.
        // Without this a pause would make every mover leap ahead on resume.
        let simulated = i64::try_from(tick.index)
            .ok()
            .and_then(|n| n.checked_mul(run.schedule.interval_nanos()))
            .and_then(|elapsed| run.started.checked_add_nanos(elapsed))
            .ok_or(ScheduleError::Overflow)?;
        let base = run.model.sample_at(simulated)?;
        let noisy = run.noise.apply(
            tick.target,
            &base,
            self.scenario.horizontal_accuracy_m,
            self.scenario.vertical_accuracy_m,
        )?;
        // From here on the position is final. Speed and course are derived
        // from it and the previously emitted fix; the model's own speed and
        // course never reach the output.
        let kinematics = run.deriver.derive(
            tick.target,
            noisy.coordinate,
            ObservationNoise {
                speed_mps: noisy.speed_noise_mps,
                heading_deg: noisy.heading_noise_deg,
            },
        )?;
        let sample = SyntheticLocation {
            timestamp: tick.target,
            coordinate: noisy.coordinate,
            altitude_m: noisy.altitude_m,
            horizontal_accuracy_m: noisy.horizontal_accuracy_m,
            vertical_accuracy_m: noisy.vertical_accuracy_m,
            speed_mps: kinematics.speed_mps,
            course_deg: kinematics.course_deg,
            source: LocationSource::Simulation,
            simulation_state: SimulationState::Running,
        };
        run.validator.validate(&sample)?;
        // Only an emitted fix becomes the reference for the next one.
        run.deriver.accept(sample.timestamp, sample.coordinate);
        Ok(sample)
    }

    /// Records a failure and moves to `Error`. Nothing is emitted; recovery
    /// policy belongs to the health subsystem, and `stop` remains available.
    fn fail(&mut self, error: ProviderError) -> ProviderError {
        self.failed_count += 1;
        self.state = SimulationState::Error;
        error
    }
}

impl LocationProvider for SimulationProvider {
    fn start(&mut self, now: Timestamp) -> Result<(), ProviderError> {
        self.require(SimulationState::Starting)?;
        // Build everything before changing state: a rejected configuration
        // leaves the provider Idle and untouched.
        self.scenario
            .validate()
            .map_err(ProviderError::InvalidScenario)?;
        let model = (self.model_factory)(&self.scenario)?;
        let noise = NoiseEngine::for_scenario(&self.scenario)?;
        let schedule = TickSchedule::new(now, self.scenario.update_interval_s)?;

        self.state = SimulationState::Starting;
        self.run = Some(Run {
            started: now,
            schedule,
            model,
            noise,
            deriver: KinematicsDeriver::new(),
            validator: SampleValidator::with_limits(SampleLimits::for_scenario(&self.scenario)),
        });
        self.last = None;
        self.sample_count = 0;
        self.failed_count = 0;
        self.missed_ticks = 0;
        self.state = SimulationState::Running;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), ProviderError> {
        self.require(SimulationState::Stopping)?;
        self.state = SimulationState::Stopping;
        self.run = None;
        self.last = None;
        self.state = SimulationState::Idle;
        Ok(())
    }

    fn pause(&mut self, now: Timestamp) -> Result<(), ProviderError> {
        if self.state != SimulationState::Running {
            return Err(InvalidTransition {
                from: self.state,
                to: SimulationState::Paused,
            }
            .into());
        }
        let run = self.run.as_mut().expect("run exists while running");
        run.schedule.pause(now)?;
        self.state = SimulationState::Paused;
        Ok(())
    }

    fn resume(&mut self, now: Timestamp) -> Result<(), ProviderError> {
        if self.state != SimulationState::Paused {
            return Err(InvalidTransition {
                from: self.state,
                to: SimulationState::Running,
            }
            .into());
        }
        let run = self.run.as_mut().expect("run exists while paused");
        run.schedule.resume(now)?;
        self.state = SimulationState::Running;
        Ok(())
    }

    fn poll(&mut self, now: Timestamp) -> Result<Option<SyntheticLocation>, ProviderError> {
        if self.state != SimulationState::Running {
            return Ok(None);
        }
        let run = self.run.as_mut().expect("run exists while running");
        let tick = match run.schedule.poll(now) {
            Ok(Poll::Due(tick)) => tick,
            Ok(Poll::Wait { .. } | Poll::Paused) => return Ok(None),
            Err(e) => return Err(self.fail(e.into())),
        };
        self.missed_ticks = self.missed_ticks.saturating_add(tick.missed);
        match self.generate(tick) {
            Ok(sample) => {
                self.sample_count += 1;
                self.last = Some(sample);
                Ok(Some(sample))
            }
            Err(e) => Err(self.fail(e)),
        }
    }

    fn current_location(&self) -> Option<SyntheticLocation> {
        self.last
    }

    fn status(&self) -> ProviderStatus {
        ProviderStatus {
            state: self.state,
            sample_count: self.sample_count,
            failed_count: self.failed_count,
            missed_ticks: self.missed_ticks,
            last_sample_time: self.last.map(|s| s.timestamp),
            trajectory_complete: self.run.as_ref().is_some_and(|r| r.model.is_complete()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        Coordinate, LocationError, MovementMode, MovementParameters, NoiseParameters,
        PlaybackParameters, RotationDirection, Route, RoutePoint, CURRENT_SCHEMA_VERSION,
    };
    use crate::geographic::GeoError;
    use crate::movement::MovementSample;
    use crate::scheduler::ScheduleError;
    use crate::validation::ValidationError;

    const SEC: i64 = 1_000_000_000;
    const T0: i64 = 1_700_000_000 * SEC;

    fn t(nanos: i64) -> Timestamp {
        Timestamp::from_nanos(nanos)
    }

    fn fixed_scenario() -> Scenario {
        Scenario {
            schema_version: CURRENT_SCHEMA_VERSION,
            name: "Fixed".into(),
            origin: Coordinate::new(12.9352, 77.6245).unwrap(),
            altitude_m: 920.0,
            mode: MovementMode::Fixed,
            movement: MovementParameters {
                min_speed_mps: 0.0,
                max_speed_mps: 0.0,
                max_acceleration_mps2: 0.0,
                max_deceleration_mps2: 0.0,
                max_heading_rate_dps: 0.0,
                radius_m: None,
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
            horizontal_accuracy_m: 5.0,
            vertical_accuracy_m: 8.0,
            update_interval_s: 1.0,
            seed: 1,
            route: None,
            playback: PlaybackParameters::REAL_TIME,
        }
    }

    fn running() -> SimulationProvider {
        let mut p = SimulationProvider::new(fixed_scenario());
        p.start(t(T0)).unwrap();
        p
    }

    #[test]
    fn idle_provider_emits_nothing() {
        let mut p = SimulationProvider::new(fixed_scenario());
        assert_eq!(p.status().state, SimulationState::Idle);
        assert_eq!(p.poll(t(T0)).unwrap(), None);
        assert_eq!(p.current_location(), None);
        assert_eq!(p.next_deadline(), None);
    }

    #[test]
    fn emits_fixed_samples_on_the_tick_grid() {
        let mut p = running();
        assert_eq!(p.next_deadline(), Some(t(T0)));
        for n in 0..5 {
            // Poll a little late; the sample is still stamped on the grid.
            let s = p.poll(t(T0 + n * SEC + 1234)).unwrap().unwrap();
            assert_eq!(s.timestamp, t(T0 + n * SEC));
            assert_eq!(s.coordinate, fixed_scenario().origin);
            assert_eq!(s.altitude_m, 920.0);
            assert_eq!((s.horizontal_accuracy_m, s.vertical_accuracy_m), (5.0, 8.0));
            // The first fix has no predecessor; after that the position
            // demonstrably does not move.
            let expected = if n == 0 { None } else { Some(0.0) };
            assert_eq!((s.speed_mps, s.course_deg), (expected, None));
            assert_eq!(s.source, LocationSource::Simulation);
            assert_eq!(s.simulation_state, SimulationState::Running);
            assert_eq!(s.validate(), Ok(()));
            assert_eq!(p.current_location(), Some(s));
            // Nothing more until the next slot.
            assert_eq!(p.poll(t(T0 + n * SEC + 5678)).unwrap(), None);
        }
        let st = p.status();
        assert_eq!(
            (st.sample_count, st.failed_count, st.missed_ticks),
            (5, 0, 0)
        );
        assert_eq!(st.last_sample_time, Some(t(T0 + 4 * SEC)));
    }

    #[test]
    fn stalled_polling_is_counted_as_missed_ticks() {
        let mut p = running();
        p.poll(t(T0)).unwrap().unwrap();
        let s = p.poll(t(T0 + 10 * SEC)).unwrap().unwrap();
        assert_eq!(s.timestamp, t(T0 + 10 * SEC));
        let st = p.status();
        assert_eq!((st.sample_count, st.missed_ticks), (2, 9));
    }

    #[test]
    fn pause_and_resume() {
        let mut p = running();
        p.poll(t(T0)).unwrap().unwrap();
        p.pause(t(T0 + SEC / 4)).unwrap();
        assert_eq!(p.status().state, SimulationState::Paused);
        assert_eq!(p.poll(t(T0 + 100 * SEC)).unwrap(), None);
        assert_eq!(p.next_deadline(), None);
        // The last sample stays readable while paused.
        assert!(p.current_location().is_some());

        let resumed = T0 + 500 * SEC;
        p.resume(t(resumed)).unwrap();
        assert_eq!(p.status().state, SimulationState::Running);
        assert_eq!(p.poll(t(resumed)).unwrap(), None);
        let s = p.poll(t(resumed + 3 * SEC / 4)).unwrap().unwrap();
        assert_eq!(s.timestamp, t(resumed + 3 * SEC / 4));
        assert_eq!(p.status().missed_ticks, 0);
    }

    #[test]
    fn stop_disables_cleanly_and_restart_resets_counters() {
        let mut p = running();
        p.poll(t(T0)).unwrap().unwrap();
        p.stop().unwrap();
        assert_eq!(p.status().state, SimulationState::Idle);
        assert_eq!(p.current_location(), None);
        assert_eq!(p.poll(t(T0 + SEC)).unwrap(), None);

        // A new run may start earlier in time than the old one ended.
        p.start(t(T0 - 50 * SEC)).unwrap();
        assert_eq!(p.status().sample_count, 0);
        let s = p.poll(t(T0 - 50 * SEC)).unwrap().unwrap();
        assert_eq!(s.timestamp, t(T0 - 50 * SEC));
    }

    #[test]
    fn stop_works_from_paused() {
        let mut p = running();
        p.pause(t(T0)).unwrap();
        p.stop().unwrap();
        assert_eq!(p.status().state, SimulationState::Idle);
    }

    #[test]
    fn illegal_lifecycle_calls_are_rejected_without_side_effects() {
        use SimulationState::*;
        let transition = |from, to| Err(ProviderError::Transition(InvalidTransition { from, to }));

        let mut p = SimulationProvider::new(fixed_scenario());
        assert_eq!(p.stop(), transition(Idle, Stopping));
        assert_eq!(p.pause(t(T0)), transition(Idle, Paused));
        assert_eq!(p.resume(t(T0)), transition(Idle, Running));

        p.start(t(T0)).unwrap();
        assert_eq!(p.start(t(T0)), transition(Running, Starting));
        assert_eq!(p.resume(t(T0)), transition(Running, Running));
        p.pause(t(T0)).unwrap();
        assert_eq!(p.pause(t(T0)), transition(Paused, Paused));
        assert_eq!(p.start(t(T0)), transition(Paused, Starting));
        assert_eq!(p.status().state, Paused);
    }

    #[test]
    fn invalid_scenario_is_rejected_and_provider_stays_idle() {
        let mut bad = fixed_scenario();
        bad.update_interval_s = 0.0;
        bad.horizontal_accuracy_m = f64::NAN;
        let mut p = SimulationProvider::new(bad);
        match p.start(t(T0)) {
            Err(ProviderError::InvalidScenario(errors)) => {
                let fields: Vec<_> = errors.iter().map(|e| e.field).collect();
                assert_eq!(fields, ["horizontal_accuracy_m", "update_interval_s"]);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(p.status().state, SimulationState::Idle);
        assert_eq!(p.poll(t(T0)).unwrap(), None);
    }

    fn route_scenario(points: Vec<RoutePoint>) -> Scenario {
        let mut s = fixed_scenario();
        s.mode = MovementMode::RouteReplay;
        s.movement.max_speed_mps = 5.0;
        s.movement.max_acceleration_mps2 = 3.0;
        s.movement.max_deceleration_mps2 = 3.0;
        s.movement.max_heading_rate_dps = 90.0;
        s.route = Some(Route::new(points).unwrap());
        s
    }

    fn point(seconds: i64, lon: f64) -> RoutePoint {
        RoutePoint::new(seconds * SEC, Coordinate::new(0.0, lon).unwrap())
    }

    #[test]
    fn route_replay_runs_to_completion_and_holds_the_final_point() {
        // 11 m east in 10 s, from rest to rest.
        let mut p = SimulationProvider::new(route_scenario(vec![point(0, 0.0), point(10, 0.0001)]));
        p.start(t(T0)).unwrap();
        let first = p.poll(t(T0)).unwrap().unwrap();
        assert_eq!(first.coordinate, Coordinate::new(0.0, 0.0).unwrap());
        assert_eq!((first.speed_mps, first.course_deg), (None, None));
        assert!(!p.status().trajectory_complete);

        // Half-way after 5 s: 5.57 m covered, so 1.11 m/s over that interval.
        let mid = p.poll(t(T0 + 5 * SEC)).unwrap().unwrap();
        assert!((mid.coordinate.longitude() - 0.00005).abs() < 1e-9);
        assert!((mid.speed_mps.unwrap() - 1.11319).abs() < 1e-4);
        assert!((mid.course_deg.unwrap() - 90.0).abs() < 1e-6);
        assert!(!p.status().trajectory_complete);

        let end = Coordinate::new(0.0, 0.0001).unwrap();
        for n in 10..15 {
            let s = p.poll(t(T0 + n * SEC)).unwrap().unwrap();
            assert_eq!(s.coordinate, end);
            assert!(p.status().trajectory_complete);
            if n == 10 {
                // Arriving: the second half was covered since the last fix.
                assert!((s.speed_mps.unwrap() - 1.11319).abs() < 1e-4);
                assert!((s.course_deg.unwrap() - 90.0).abs() < 1e-6);
            } else {
                assert_eq!((s.speed_mps, s.course_deg), (Some(0.0), None));
            }
        }
        // Completion is not a stop: the provider is still running.
        assert_eq!(p.status().state, SimulationState::Running);
        assert_eq!(p.status().failed_count, 0);
    }

    #[test]
    fn inadmissible_route_is_rejected_at_start_with_its_violations() {
        use crate::route::{RouteConstraint, RouteRejection};
        // 111 m in 10 s from rest to rest peaks at 16.7 m/s: over the 5 m/s limit.
        let mut p = SimulationProvider::new(route_scenario(vec![point(0, 0.0), point(10, 0.001)]));
        match p.start(t(T0)) {
            Err(ProviderError::InvalidScenario(_)) => {
                // The scenario's own cheap check (mean speed) fires first.
            }
            other => panic!("unexpected {other:?}"),
        }
        // 33 m in 10 s: mean 3.3 m/s passes the cheap check, peak 5.0 does not.
        let mut p = SimulationProvider::new(route_scenario(vec![point(0, 0.0), point(10, 0.0003)]));
        match p.start(t(T0)) {
            Err(ProviderError::Movement(MovementError::RouteRejected(
                RouteRejection::Violations(violations),
            ))) => {
                assert_eq!(violations.len(), 1);
                assert_eq!(violations[0].segment, 0);
                assert_eq!(violations[0].constraint, RouteConstraint::MaxSpeed);
                assert!((violations[0].observed - 5.0097).abs() < 1e-3);
                assert_eq!(violations[0].limit, 5.0);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(p.status().state, SimulationState::Idle);
        assert!(!p.status().trajectory_complete);
    }

    fn orbit_scenario() -> Scenario {
        let mut s = fixed_scenario();
        s.mode = MovementMode::Circular;
        s.movement.max_speed_mps = 5.0;
        s.movement.max_heading_rate_dps = 10.0;
        s.movement.radius_m = Some(50.0);
        s.movement.angular_velocity_dps = Some(2.0);
        s
    }

    #[test]
    fn moving_model_runs_through_the_same_pipeline() {
        let mut p = SimulationProvider::new(orbit_scenario());
        p.start(t(T0)).unwrap();
        let origin = fixed_scenario().origin;
        for n in 0..180 {
            let s = p.poll(t(T0 + n * SEC)).unwrap().unwrap();
            let d = crate::geographic::distance(origin, s.coordinate).unwrap();
            assert!((d - 50.0).abs() < 1e-6);
            if n == 0 {
                assert_eq!((s.speed_mps, s.course_deg), (None, None));
            } else {
                // 2°/s on a 50 m circle: 1.745 m/s along the arc, a hair
                // less along the chord the two fixes actually span.
                let speed = s.speed_mps.unwrap();
                assert!(speed > 1.7 && speed < 1.7454, "{speed}");
                assert!(s.course_deg.is_some());
            }
        }
        assert_eq!(p.status().failed_count, 0);
    }

    #[test]
    fn simulated_time_stands_still_while_paused() {
        let mut p = SimulationProvider::new(orbit_scenario());
        p.start(t(T0)).unwrap();
        let before = p.poll(t(T0)).unwrap().unwrap();
        let second = p.poll(t(T0 + SEC)).unwrap().unwrap();
        let one_step = crate::geographic::distance(before.coordinate, second.coordinate).unwrap();

        // Pause for an hour (20 revolutions' worth of wall time).
        p.pause(t(T0 + SEC)).unwrap();
        let resumed = T0 + 3_601 * SEC;
        p.resume(t(resumed)).unwrap();
        let after = p.poll(t(resumed + SEC)).unwrap().unwrap();
        // The timestamp jumped by the pause; the orbit advanced by one step.
        assert_eq!(after.timestamp, t(resumed + SEC));
        let moved = crate::geographic::distance(second.coordinate, after.coordinate).unwrap();
        assert!((moved - one_step).abs() < 1e-6, "{moved} vs {one_step}");
    }

    #[test]
    fn sub_nanosecond_interval_is_rejected_at_start() {
        let mut s = fixed_scenario();
        s.update_interval_s = 1e-12;
        let mut p = SimulationProvider::new(s);
        assert_eq!(
            p.start(t(T0)),
            Err(ProviderError::Schedule(ScheduleError::InvalidInterval))
        );
        assert_eq!(p.status().state, SimulationState::Idle);
    }

    /// Model that behaves for `good` samples and then misbehaves.
    struct Faulty {
        good: u32,
        hard_error: bool,
    }

    impl MovementModel for Faulty {
        fn sample_at(&mut self, _t: Timestamp) -> Result<MovementSample, MovementError> {
            let ok = self.good > 0;
            self.good = self.good.saturating_sub(1);
            if !ok && self.hard_error {
                return Err(MovementError::Geo(GeoError::NotConverged));
            }
            Ok(MovementSample {
                coordinate: Coordinate::new(1.0, 2.0).unwrap(),
                altitude_m: if ok { 10.0 } else { f64::NAN },
                speed_mps: Some(0.0),
                course_deg: None,
            })
        }
    }

    fn faulty(hard_error: bool) -> SimulationProvider {
        let mut p = SimulationProvider::with_model_factory(
            fixed_scenario(),
            Box::new(move |_| {
                Ok(Box::new(Faulty {
                    good: 2,
                    hard_error,
                }))
            }),
        );
        p.start(t(T0)).unwrap();
        p
    }

    #[test]
    fn invalid_sample_is_never_emitted_and_enters_error() {
        let mut p = faulty(false);
        p.poll(t(T0)).unwrap().unwrap();
        let last_good = p.poll(t(T0 + SEC)).unwrap().unwrap();

        assert_eq!(
            p.poll(t(T0 + 2 * SEC)),
            Err(ProviderError::Validation(ValidationError::Location(
                LocationError::NonFinite("altitude")
            )))
        );
        let st = p.status();
        assert_eq!(st.state, SimulationState::Error);
        assert_eq!((st.sample_count, st.failed_count), (2, 1));
        assert_eq!(p.current_location(), Some(last_good));
        // No samples while in Error, and stop still works.
        assert_eq!(p.poll(t(T0 + 3 * SEC)).unwrap(), None);
        assert!(p.pause(t(T0 + 3 * SEC)).is_err());
        p.stop().unwrap();
        assert_eq!(p.status().state, SimulationState::Idle);
    }

    #[test]
    fn model_error_is_reported_and_enters_error() {
        let mut p = faulty(true);
        p.poll(t(T0)).unwrap().unwrap();
        p.poll(t(T0 + SEC)).unwrap().unwrap();
        assert_eq!(
            p.poll(t(T0 + 2 * SEC)),
            Err(ProviderError::Movement(MovementError::Geo(
                GeoError::NotConverged
            )))
        );
        assert_eq!(p.status().state, SimulationState::Error);
        assert_eq!(p.status().failed_count, 1);
    }

    #[test]
    fn custom_factory_errors_surface_at_start() {
        let mut p = SimulationProvider::with_model_factory(
            fixed_scenario(),
            Box::new(|s| Err(MovementError::UnsupportedMode(s.mode))),
        );
        assert_eq!(
            p.start(t(T0)),
            Err(ProviderError::Movement(MovementError::UnsupportedMode(
                MovementMode::Fixed
            )))
        );
        assert_eq!(p.status().state, SimulationState::Idle);
    }
}
