//! Shared by the supervisor's integration tests: a provider that does what
//! a script says, a movement model that fails on request, and checks that
//! are applied after every step of every test.
#![allow(dead_code)]

/// Examples and the core's independent re-check of a stream.
#[path = "../../../locsim-scenario/tests/support/mod.rs"]
pub mod scenario;

use locsim_core::domain::{
    Coordinate, HealthState, InvalidTransition, LocationSource, Scenario, SimulationState,
    SyntheticLocation, Timestamp,
};
use locsim_core::movement::{model_for, MovementError, MovementModel, MovementSample};
use locsim_core::provider::{LocationProvider, ProviderError, ProviderStatus, SimulationProvider};
use locsim_core::rng::Rng;
use locsim_health::{
    Cause, Emission, Event, EventKind, HealthPolicy, HealthReport, MemorySink, RunEndReason,
    Supervisor,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

pub const SEC: i64 = 1_000_000_000;
pub const T0: i64 = 1_700_000_000_000_000_000;

pub fn t(nanos: i64) -> Timestamp {
    Timestamp::from_nanos(nanos)
}

/// A policy with round numbers; tests change the field they are about.
pub fn policy() -> HealthPolicy {
    HealthPolicy {
        max_recovery_attempts: 3,
        backoff_initial_s: 1.0,
        backoff_multiplier: 2.0,
        backoff_max_s: 60.0,
        stable_after_s: 10.0,
        stall_after_s: 5.0,
        missed_window_ticks: 10,
        max_missed_in_window: 2,
    }
}

/// The same, with a failure being final.
pub fn no_recovery() -> HealthPolicy {
    HealthPolicy {
        max_recovery_attempts: 0,
        ..policy()
    }
}

// --- A provider that follows a script ---------------------------------------------

/// What the scripted provider does at one poll while running.
#[derive(Debug, Clone)]
pub enum Step {
    /// Emit a sample stamped `at` (or with the poll's time), after counting
    /// `missed` skipped ticks.
    Emit { at: Option<i64>, missed: u64 },
    /// Nothing is due.
    Nothing,
    /// Count `missed`, then fail.
    Fail { error: ProviderError, missed: u64 },
}

#[derive(Debug)]
pub struct Script {
    pub state: SimulationState,
    /// Steps for the coming polls. When empty, a poll emits a sample
    /// stamped with its own time.
    pub steps: VecDeque<Step>,
    /// One entry per coming `start`: `Some(error)` makes it fail.
    pub start_results: VecDeque<Option<ProviderError>>,
    pub stop_error: Option<ProviderError>,
    pub pause_error: Option<ProviderError>,
    pub resume_error: Option<ProviderError>,
    /// After a failed poll the provider normally enters `Error`; with this
    /// it claims to be running still.
    pub keeps_running_after_failure: bool,
    /// If set, `status()` reports this state whatever the real one is.
    pub claims: Option<SimulationState>,
    pub sample_count: u64,
    pub failed_count: u64,
    pub missed_ticks: u64,
    pub last: Option<SyntheticLocation>,
    /// Every call made on the provider, in order.
    pub calls: Vec<String>,
}

/// A `LocationProvider` driven by a [`Script`]. Clones share the script, so
/// a test keeps one clone to steer and inspect the provider it gave away.
#[derive(Debug, Clone)]
pub struct Scripted(Arc<Mutex<Script>>);

impl Scripted {
    pub fn new() -> Self {
        Scripted(Arc::new(Mutex::new(Script {
            state: SimulationState::Idle,
            steps: VecDeque::new(),
            start_results: VecDeque::new(),
            stop_error: None,
            pause_error: None,
            resume_error: None,
            keeps_running_after_failure: false,
            claims: None,
            sample_count: 0,
            failed_count: 0,
            missed_ticks: 0,
            last: None,
            calls: Vec::new(),
        })))
    }

    pub fn script(&self) -> MutexGuard<'_, Script> {
        self.0.lock().unwrap()
    }

    pub fn push(&self, step: Step) {
        self.script().steps.push_back(step);
    }

    pub fn fail_next(&self, error: ProviderError) {
        self.push(Step::Fail { error, missed: 0 });
    }

    pub fn calls(&self) -> Vec<String> {
        self.script().calls.clone()
    }

    pub fn state(&self) -> SimulationState {
        self.script().state
    }
}

/// An error the real pipeline can produce, with nothing special about it.
pub fn pipeline_error() -> ProviderError {
    ProviderError::Movement(MovementError::InvalidConfiguration("injected failure"))
}

pub fn sample_at(nanos: i64) -> SyntheticLocation {
    SyntheticLocation {
        timestamp: t(nanos),
        coordinate: Coordinate::new(12.9352, 77.6245).unwrap(),
        altitude_m: 920.0,
        horizontal_accuracy_m: 5.0,
        vertical_accuracy_m: 8.0,
        speed_mps: None,
        course_deg: None,
        source: LocationSource::Test,
        simulation_state: SimulationState::Running,
    }
}

fn refuse(from: SimulationState, to: SimulationState) -> ProviderError {
    ProviderError::Transition(InvalidTransition { from, to })
}

impl LocationProvider for Scripted {
    fn start(&mut self, now: Timestamp) -> Result<(), ProviderError> {
        let mut s = self.script();
        s.calls.push(format!("start@{}", now.as_nanos() - T0));
        if s.state != SimulationState::Idle {
            return Err(refuse(s.state, SimulationState::Starting));
        }
        if let Some(Some(error)) = s.start_results.pop_front() {
            return Err(error);
        }
        s.state = SimulationState::Running;
        s.sample_count = 0;
        s.failed_count = 0;
        s.missed_ticks = 0;
        s.last = None;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), ProviderError> {
        let mut s = self.script();
        s.calls.push("stop".into());
        if let Some(error) = s.stop_error.clone() {
            return Err(error);
        }
        if s.state == SimulationState::Idle {
            return Err(refuse(s.state, SimulationState::Stopping));
        }
        s.state = SimulationState::Idle;
        s.last = None;
        Ok(())
    }

    fn pause(&mut self, now: Timestamp) -> Result<(), ProviderError> {
        let mut s = self.script();
        s.calls.push(format!("pause@{}", now.as_nanos() - T0));
        if let Some(error) = s.pause_error.clone() {
            return Err(error);
        }
        if s.state != SimulationState::Running {
            return Err(refuse(s.state, SimulationState::Paused));
        }
        s.state = SimulationState::Paused;
        Ok(())
    }

    fn resume(&mut self, now: Timestamp) -> Result<(), ProviderError> {
        let mut s = self.script();
        s.calls.push(format!("resume@{}", now.as_nanos() - T0));
        if let Some(error) = s.resume_error.clone() {
            return Err(error);
        }
        if s.state != SimulationState::Paused {
            return Err(refuse(s.state, SimulationState::Running));
        }
        s.state = SimulationState::Running;
        Ok(())
    }

    fn poll(&mut self, now: Timestamp) -> Result<Option<SyntheticLocation>, ProviderError> {
        let mut s = self.script();
        s.calls.push(format!("poll@{}", now.as_nanos() - T0));
        if s.state != SimulationState::Running {
            return Ok(None);
        }
        let step = s.steps.pop_front().unwrap_or(Step::Emit {
            at: None,
            missed: 0,
        });
        match step {
            Step::Nothing => Ok(None),
            Step::Emit { at, missed } => {
                s.missed_ticks += missed;
                s.sample_count += 1;
                let sample = sample_at(at.unwrap_or(now.as_nanos()));
                s.last = Some(sample);
                Ok(Some(sample))
            }
            Step::Fail { error, missed } => {
                s.missed_ticks += missed;
                s.failed_count += 1;
                if !s.keeps_running_after_failure {
                    s.state = SimulationState::Error;
                }
                Err(error)
            }
        }
    }

    fn current_location(&self) -> Option<SyntheticLocation> {
        self.script().last
    }

    fn status(&self) -> ProviderStatus {
        let s = self.script();
        ProviderStatus {
            state: s.claims.unwrap_or(s.state),
            sample_count: s.sample_count,
            failed_count: s.failed_count,
            missed_ticks: s.missed_ticks,
            last_sample_time: s.last.map(|l| l.timestamp),
            trajectory_complete: false,
        }
    }
}

/// A supervised scripted provider, the handle that steers it, and its events.
pub fn scripted(policy: HealthPolicy) -> (Supervisor<Scripted>, Scripted, MemorySink) {
    let provider = Scripted::new();
    let events = MemorySink::new();
    let supervisor = Supervisor::new(provider.clone(), policy, Box::new(events.clone())).unwrap();
    (supervisor, provider, events)
}

// --- The real provider, made to fail ----------------------------------------------

/// A movement model that behaves like the real one until its `fail_at`-th
/// sample (counting from 0) and then fails.
struct FailsAt {
    real: Box<dyn MovementModel + Send>,
    calls: u64,
    fail_at: u64,
}

impl MovementModel for FailsAt {
    fn sample_at(&mut self, at: Timestamp) -> Result<MovementSample, MovementError> {
        let call = self.calls;
        self.calls += 1;
        if call == self.fail_at {
            return Err(MovementError::InvalidConfiguration("injected failure"));
        }
        self.real.sample_at(at)
    }

    fn is_complete(&self) -> bool {
        self.real.is_complete()
    }
}

/// A real `SimulationProvider` whose first `failing_runs` runs fail at
/// their `fail_at`-th sample; later runs are sound. The counter says how
/// many runs have been built.
pub fn failing_provider(
    scenario: &Scenario,
    failing_runs: u32,
    fail_at: u64,
) -> (SimulationProvider, Arc<AtomicU32>) {
    let built = Arc::new(AtomicU32::new(0));
    let counter = built.clone();
    let provider = SimulationProvider::with_model_factory(
        scenario.clone(),
        Box::new(move |scenario| {
            let run = counter.fetch_add(1, Ordering::Relaxed);
            let real = model_for(scenario)?;
            Ok(if run < failing_runs {
                Box::new(FailsAt {
                    real,
                    calls: 0,
                    fail_at,
                })
            } else {
                real
            })
        }),
    );
    (provider, built)
}

// --- Checks applied everywhere -----------------------------------------------------

/// What must hold of any report, whatever happened before it. A report
/// that fails this is misleading.
pub fn assert_consistent(report: &HealthReport) {
    use HealthState::*;
    let r = report;
    let has_run = matches!(
        r.lifecycle,
        SimulationState::Running | SimulationState::Paused
    );
    // Health follows lifecycle; it can never contradict it.
    match r.health {
        Stopped => assert_eq!(r.lifecycle, SimulationState::Idle, "{r:#?}"),
        Failed => assert_eq!(r.lifecycle, SimulationState::Error, "{r:#?}"),
        Recovering => assert_eq!(r.lifecycle, SimulationState::Recovering, "{r:#?}"),
        Healthy | Degraded => assert!(has_run, "{r:#?}"),
    }
    match r.lifecycle {
        SimulationState::Idle => assert_eq!(r.health, Stopped, "{r:#?}"),
        SimulationState::Error => assert_eq!(r.health, Failed, "{r:#?}"),
        SimulationState::Recovering => assert_eq!(r.health, Recovering, "{r:#?}"),
        SimulationState::Running | SimulationState::Paused => {
            assert!(matches!(r.health, Healthy | Degraded), "{r:#?}")
        }
        other => panic!("the supervisor never reports {other:?}: {r:#?}"),
    }
    // Healthy means nothing is wrong; degraded means something named is.
    if has_run {
        assert_eq!(r.health == Healthy, r.causes.is_empty(), "{r:#?}");
    } else {
        assert!(r.causes.is_empty(), "{r:#?}");
    }
    // A retry time exists exactly while recovering.
    assert_eq!(r.retry_at.is_some(), r.health == Recovering, "{r:#?}");
    // Samples are expected only from a running run, and "stalled" is said
    // in both places or in neither.
    match r.emission {
        Emission::NotExpected => assert_ne!(r.lifecycle, SimulationState::Running, "{r:#?}"),
        Emission::Flowing => {
            assert_eq!(r.lifecycle, SimulationState::Running, "{r:#?}");
            assert!(!r.causes.contains(&Cause::Stalled), "{r:#?}");
        }
        Emission::Stalled { silent_for_ns } => {
            assert_eq!(r.lifecycle, SimulationState::Running, "{r:#?}");
            assert!(r.causes.contains(&Cause::Stalled), "{r:#?}");
            assert!(silent_for_ns > 0, "{r:#?}");
        }
    }
    assert!(r.attempts_used <= r.max_attempts, "{r:#?}");
    assert_eq!(r.current.state, r.lifecycle, "{r:#?}");
    if !has_run {
        assert_eq!(r.current.sample_count, 0, "{r:#?}");
        assert_eq!(r.current.last_sample_time, None, "{r:#?}");
    }
    // Every external fault listed is a cause while a run exists.
    for component in &r.active_faults {
        assert_eq!(
            r.causes.contains(&Cause::ExternalFault(component)),
            has_run,
            "{r:#?}"
        );
    }
}

/// Recomputes every total from the events alone and compares. `current` is
/// the run in progress, which has no `RunEnded` yet.
pub fn assert_reconciles(events: &[Event], report: &HealthReport) {
    let count = |f: fn(&EventKind) -> bool| events.iter().filter(|e| f(&e.kind)).count() as u64;
    let mut ended = (0u64, 0u64, 0u64, 0u64); // samples, withheld, missed, failed runs
    for event in events {
        if let EventKind::RunEnded {
            samples,
            withheld,
            missed,
            reason,
        } = &event.kind
        {
            ended.0 += samples;
            ended.1 += withheld;
            ended.2 += missed;
            ended.3 += u64::from(*reason == RunEndReason::Failure);
        }
    }
    let t = &report.totals;
    let has_run = matches!(
        report.lifecycle,
        SimulationState::Running | SimulationState::Paused
    );
    let (current_samples, current_missed) = if has_run {
        (report.current.sample_count, report.current.missed_ticks)
    } else {
        (0, 0)
    };
    assert_eq!(
        t.runs_started,
        count(|k| matches!(k, EventKind::RunStarted { .. }))
    );
    assert_eq!(
        t.runs_ended,
        count(|k| matches!(k, EventKind::RunEnded { .. }))
    );
    assert_eq!(t.samples, ended.0 + current_samples, "samples");
    assert_eq!(t.missed_ticks, ended.2 + current_missed, "missed ticks");
    // A withheld sample ends its run at once, so the current run has none.
    assert_eq!(t.withheld, ended.1, "withheld");
    assert_eq!(t.failures, ended.3, "failures");
    assert_eq!(
        t.failures,
        count(|k| matches!(k, EventKind::RunFailed { .. }))
    );
    assert_eq!(
        t.recovery_attempts,
        count(|k| matches!(k, EventKind::RecoveryAttempted { .. }))
    );
    assert_eq!(
        t.recoveries_completed,
        count(|k| matches!(k, EventKind::Recovered { .. }))
    );
    assert_eq!(
        t.terminal_failures,
        count(|k| matches!(k, EventKind::Failed { .. }))
    );
    assert_eq!(
        t.rejected_operations,
        count(|k| matches!(k, EventKind::OperationRejected { .. }))
    );
    // Runs are either over or the one in progress.
    assert_eq!(t.runs_started, t.runs_ended + u64::from(has_run));
    // Health changes form a chain that ends at the reported health.
    let mut health = HealthState::Stopped;
    for event in events {
        if let EventKind::HealthChanged { from, to } = &event.kind {
            assert_eq!(*from, health, "broken chain of health changes");
            assert_ne!(from, to);
            health = *to;
        }
    }
    assert_eq!(
        health, report.health,
        "the last announced health is not the reported one"
    );
}

/// Both checks, on a supervisor and its recorded events.
pub fn check<P: LocationProvider>(supervisor: &Supervisor<P>, events: &MemorySink) -> HealthReport {
    let report = supervisor.health();
    assert_consistent(&report);
    assert_reconciles(&events.events(), &report);
    report
}

/// The kinds of the recorded events, without health changes, which are a
/// consequence of the others.
pub fn kinds(events: &MemorySink) -> Vec<EventKind> {
    events
        .events()
        .into_iter()
        .map(|e| e.kind)
        .filter(|k| !matches!(k, EventKind::HealthChanged { .. }))
        .collect()
}

/// Polls `polls` times at the scenario's interval, punctually or, with
/// `jitter`, at gaps between 0.3 and 2.5 intervals. Returns what came out.
pub fn drive<P: LocationProvider>(
    provider: &mut P,
    interval_s: f64,
    polls: usize,
    jitter: Option<u64>,
) -> Vec<SyntheticLocation> {
    let interval = interval_s * 1e9;
    let mut rng = Rng::from_seed(jitter.unwrap_or(0));
    let mut now = T0;
    let mut stream = Vec::new();
    for _ in 0..polls {
        if let Some(sample) = provider.poll(t(now)).expect("no failure") {
            stream.push(sample);
        }
        let gap = if jitter.is_some() {
            rng.uniform(0.3, 2.5)
        } else {
            1.0
        };
        now += (gap * interval) as i64;
    }
    stream
}
