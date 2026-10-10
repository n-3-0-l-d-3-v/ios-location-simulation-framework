//! The supervisor: a provider wrapped in a state machine that watches it.
//!
//! It is itself a [`LocationProvider`], so it goes wherever a provider
//! goes. It never changes a sample; it decides only whether one is handed
//! on and what to do when the wrapped provider fails.
//!
//! # Time
//!
//! Like the core, the supervisor never reads a clock and never sleeps.
//! Everything it does happens inside a call, at the time that call was
//! given. A deadline that nobody calls it at is not noticed.
//!
//! # Rejected operations and failed runs
//!
//! These are different things and are reported differently.
//!
//! - A **rejected operation** is a lifecycle call (`start`, `stop`,
//!   `pause`, `resume`) that returns an error. Nothing changes: the run, if
//!   there is one, goes on. The error is returned as it is and one
//!   `OperationRejected` event is recorded. A call that is illegal for the
//!   supervisor's own lifecycle is refused with `ProviderError::Transition`
//!   without reaching the wrapped provider.
//! - A **failed run** is a run that cannot be trusted to continue. There
//!   are exactly three sources: the wrapped provider's `poll` returned an
//!   error; it returned a sample that had to be withheld; or its state is
//!   not the one the supervisor's lifecycle implies. `RunFailed` and
//!   `RunEnded` are recorded.
//!
//! # Timestamps
//!
//! Within one session (from `start` to `stop`) the timestamps of the
//! samples handed on strictly increase. A sample whose timestamp is not
//! later than the previous one is withheld and ends the run, reported as
//! `ValidationError::NonMonotonicTimestamp`. Nothing is promised across a
//! `stop` and a later `start`: like the core provider, a new session may
//! start at any time.
//!
//! # One place for state, totals and events
//!
//! Every event passes through [`Supervisor::record`], and the cumulative
//! totals are updated there and nowhere else, from the event itself. A
//! total therefore cannot disagree with the events. Health is never
//! stored: it is computed from the state when asked.

use crate::event::{Event, EventKind, EventSink, FailReason, Operation, RunEndReason};
use crate::policy::{HealthPolicy, Limits, PolicyError};
use crate::report::{Cause, Emission, HealthReport, Totals};
use locsim_core::domain::{
    HealthState, InvalidTransition, SimulationState, SyntheticLocation, Timestamp,
};
use locsim_core::provider::{LocationProvider, ProviderError, ProviderStatus};
use locsim_core::scheduler::ScheduleError;
use locsim_core::validation::ValidationError;

/// What the supervisor is doing. Its public face is a `SimulationState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    /// No run and none scheduled.
    Stopped,
    Running,
    Paused,
    /// The run failed for good; waiting to be stopped.
    Failed,
}

impl Lifecycle {
    fn state(self) -> SimulationState {
        match self {
            Lifecycle::Stopped => SimulationState::Idle,
            Lifecycle::Running => SimulationState::Running,
            Lifecycle::Paused => SimulationState::Paused,
            Lifecycle::Failed => SimulationState::Error,
        }
    }

    /// Whether a run exists (and so the wrapped provider is in a run).
    fn has_run(self) -> bool {
        matches!(self, Lifecycle::Running | Lifecycle::Paused)
    }
}

/// The current run, as the supervisor has seen it.
#[derive(Debug, Clone, Copy, Default)]
struct Run {
    /// Samples handed to the caller.
    forwarded: u64,
    withheld: u64,
    /// The wrapped provider's missed-tick count as last read.
    missed: u64,
    last_sample: Option<SyntheticLocation>,
}

/// Wraps a provider and supervises it. See the module documentation.
pub struct Supervisor<P: LocationProvider> {
    provider: P,
    limits: Limits,
    sink: Box<dyn EventSink + Send>,
    lifecycle: Lifecycle,
    /// Number of the current or latest run.
    run_number: u64,
    run: Run,
    /// Timestamp of the last sample handed on in this session.
    last_forwarded: Option<Timestamp>,
    last_error: Option<ProviderError>,
    /// Totals of the runs that are over. The current run is added on read.
    ended: Totals,
    /// Latest time passed to any call.
    observed_at: Option<Timestamp>,
    clock_regressed: bool,
    /// The health last announced with a `HealthChanged` event.
    announced: HealthState,
}

impl<P: LocationProvider> Supervisor<P> {
    /// Wraps `provider`, which must be idle. The policy is validated here;
    /// an invalid one is refused with every problem it has.
    pub fn new(
        provider: P,
        policy: HealthPolicy,
        sink: Box<dyn EventSink + Send>,
    ) -> Result<Self, Vec<PolicyError>> {
        Ok(Self {
            provider,
            limits: policy.limits()?,
            sink,
            lifecycle: Lifecycle::Stopped,
            run_number: 0,
            run: Run::default(),
            last_forwarded: None,
            last_error: None,
            ended: Totals::default(),
            observed_at: None,
            clock_regressed: false,
            announced: HealthState::Stopped,
        })
    }

    /// The wrapped provider, for looking at. There is no mutable access:
    /// every way of driving a provider needs it, so the supervisor cannot
    /// be bypassed.
    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// The state as of the latest time passed to any call.
    pub fn health(&self) -> HealthReport {
        let causes = self.causes();
        HealthReport {
            observed_at: self.observed_at,
            lifecycle: self.lifecycle.state(),
            health: self.derive_health(&causes),
            causes,
            emission: self.emission(),
            run: self.run_number,
            attempts_used: 0,
            max_attempts: self.limits.max_recovery_attempts,
            retry_at: None,
            last_error: self.last_error.clone(),
            totals: self.totals(),
            current: self.status(),
            active_faults: Vec::new(),
        }
    }

    // --- Derived, never stored ----------------------------------------------------

    fn causes(&self) -> Vec<Cause> {
        Vec::new()
    }

    fn derive_health(&self, causes: &[Cause]) -> HealthState {
        match self.lifecycle {
            Lifecycle::Stopped => HealthState::Stopped,
            Lifecycle::Failed => HealthState::Failed,
            Lifecycle::Running | Lifecycle::Paused if causes.is_empty() => HealthState::Healthy,
            Lifecycle::Running | Lifecycle::Paused => HealthState::Degraded,
        }
    }

    fn emission(&self) -> Emission {
        match self.lifecycle {
            Lifecycle::Running => Emission::Flowing,
            _ => Emission::NotExpected,
        }
    }

    fn totals(&self) -> Totals {
        let mut totals = self.ended;
        if self.lifecycle.has_run() {
            totals.samples = totals.samples.saturating_add(self.run.forwarded);
            totals.withheld = totals.withheld.saturating_add(self.run.withheld);
            totals.missed_ticks = totals.missed_ticks.saturating_add(self.run.missed);
        }
        totals
    }

    // --- The one place events and totals are produced ------------------------------

    fn record(&mut self, at: Timestamp, kind: EventKind) {
        let t = &mut self.ended;
        let bump = |n: &mut u64| *n = n.saturating_add(1);
        match &kind {
            EventKind::RunStarted { .. } => bump(&mut t.runs_started),
            EventKind::RunEnded {
                samples,
                withheld,
                missed,
                reason,
            } => {
                bump(&mut t.runs_ended);
                t.samples = t.samples.saturating_add(*samples);
                t.withheld = t.withheld.saturating_add(*withheld);
                t.missed_ticks = t.missed_ticks.saturating_add(*missed);
                if *reason == RunEndReason::Failure {
                    bump(&mut t.failures);
                }
            }
            EventKind::RecoveryAttempted { .. } => bump(&mut t.recovery_attempts),
            EventKind::Recovered { .. } => bump(&mut t.recoveries_completed),
            EventKind::Failed { .. } => bump(&mut t.terminal_failures),
            EventKind::OperationRejected { .. } => bump(&mut t.rejected_operations),
            _ => {}
        }
        self.sink.record(&Event {
            at,
            level: kind.level(),
            run: self.run_number,
            kind,
        });
    }

    /// Announces a change of health, if there was one. Called at the end of
    /// every call that can change anything.
    fn announce(&mut self, at: Timestamp) {
        let now = self.derive_health(&self.causes());
        if now != self.announced {
            let from = std::mem::replace(&mut self.announced, now);
            self.record(at, EventKind::HealthChanged { from, to: now });
        }
    }

    /// Notes the time of a call.
    fn observe(&mut self, now: Timestamp) {
        match self.observed_at {
            Some(latest) if now < latest => {
                if !self.clock_regressed {
                    self.clock_regressed = true;
                    self.record(now, EventKind::ClockWentBackwards { latest });
                }
            }
            _ => {
                self.observed_at = Some(now);
                self.clock_regressed = false;
            }
        }
    }

    /// The time to put on events of a call that was given none.
    fn latest(&self) -> Timestamp {
        self.observed_at.unwrap_or(Timestamp::from_nanos(0))
    }

    // --- Transitions ----------------------------------------------------------------

    fn reject(
        &mut self,
        at: Timestamp,
        operation: Operation,
        error: ProviderError,
    ) -> ProviderError {
        self.record(
            at,
            EventKind::OperationRejected {
                operation,
                error: error.clone(),
            },
        );
        error
    }

    /// The error for a call the supervisor's own lifecycle does not allow.
    fn illegal(&self, to: SimulationState) -> ProviderError {
        ProviderError::Transition(InvalidTransition {
            from: self.lifecycle.state(),
            to,
        })
    }

    fn begin_run(&mut self, at: Timestamp, recovery_attempt: Option<u32>) {
        self.run_number = self.run_number.saturating_add(1);
        self.run = Run::default();
        self.lifecycle = Lifecycle::Running;
        self.record(at, EventKind::RunStarted { recovery_attempt });
    }

    fn end_run(&mut self, at: Timestamp, reason: RunEndReason) {
        self.record(
            at,
            EventKind::RunEnded {
                samples: self.run.forwarded,
                withheld: self.run.withheld,
                missed: self.run.missed,
                reason,
            },
        );
        self.run = Run::default();
    }

    /// The current run cannot continue.
    fn fail_run(&mut self, at: Timestamp, error: ProviderError) {
        self.record(
            at,
            EventKind::RunFailed {
                error: error.clone(),
            },
        );
        self.end_run(at, RunEndReason::Failure);
        let permanent = is_permanent(&error);
        self.last_error = Some(error);
        self.give_up(
            at,
            if permanent {
                FailReason::Permanent
            } else {
                FailReason::AttemptsExhausted
            },
        );
    }

    fn give_up(&mut self, at: Timestamp, reason: FailReason) {
        self.lifecycle = Lifecycle::Failed;
        self.record(at, EventKind::Failed { reason });
    }

    /// Reads the wrapped provider's missed-tick count into the run.
    fn read_missed(&mut self) {
        self.run.missed = self.provider.status().missed_ticks;
    }

    /// Fails the run if the wrapped provider is not in the state the
    /// supervisor's lifecycle implies. Returns whether the run survived.
    fn verify(&mut self, at: Timestamp) -> bool {
        if !self.lifecycle.has_run() {
            return true;
        }
        let expected = self.lifecycle.state();
        let actual = self.provider.status().state;
        if actual == expected {
            return true;
        }
        self.fail_run(
            at,
            ProviderError::Transition(InvalidTransition {
                from: actual,
                to: expected,
            }),
        );
        false
    }
}

/// Errors a new run cannot cure. Only what can be shown to be so:
/// once tick times stop being representable, a later start is no better.
fn is_permanent(error: &ProviderError) -> bool {
    matches!(error, ProviderError::Schedule(ScheduleError::Overflow))
}

impl<P: LocationProvider> LocationProvider for Supervisor<P> {
    /// Starts a session and its first run. An error from the wrapped
    /// provider is returned as it is and the supervisor stays stopped: a
    /// refused start is a rejected operation, not a failure.
    fn start(&mut self, now: Timestamp) -> Result<(), ProviderError> {
        self.observe(now);
        if self.lifecycle != Lifecycle::Stopped {
            let error = self.illegal(SimulationState::Starting);
            return Err(self.reject(now, Operation::Start, error));
        }
        if let Err(error) = self.provider.start(now) {
            return Err(self.reject(now, Operation::Start, error));
        }
        // A new session: nothing is carried over from the one before.
        self.last_forwarded = None;
        self.last_error = None;
        self.begin_run(now, None);
        self.verify(now);
        self.announce(now);
        Ok(())
    }

    /// Ends the session. Legal whenever the supervisor is not stopped,
    /// including after it has failed.
    fn stop(&mut self) -> Result<(), ProviderError> {
        let at = self.latest();
        if self.lifecycle == Lifecycle::Stopped {
            let error = self.illegal(SimulationState::Stopping);
            return Err(self.reject(at, Operation::Stop, error));
        }
        // The wrapped provider may already be idle (a restart that failed).
        if self.provider.status().state != SimulationState::Idle {
            if let Err(error) = self.provider.stop() {
                return Err(self.reject(at, Operation::Stop, error));
            }
        }
        if self.lifecycle.has_run() {
            self.end_run(at, RunEndReason::Stopped);
        }
        self.lifecycle = Lifecycle::Stopped;
        self.record(at, EventKind::Stopped);
        self.announce(at);
        Ok(())
    }

    fn pause(&mut self, now: Timestamp) -> Result<(), ProviderError> {
        self.observe(now);
        if self.lifecycle != Lifecycle::Running {
            let error = self.illegal(SimulationState::Paused);
            return Err(self.reject(now, Operation::Pause, error));
        }
        if let Err(error) = self.provider.pause(now) {
            return Err(self.reject(now, Operation::Pause, error));
        }
        self.lifecycle = Lifecycle::Paused;
        self.record(now, EventKind::Paused);
        self.verify(now);
        self.announce(now);
        Ok(())
    }

    fn resume(&mut self, now: Timestamp) -> Result<(), ProviderError> {
        self.observe(now);
        if self.lifecycle != Lifecycle::Paused {
            let error = self.illegal(SimulationState::Running);
            return Err(self.reject(now, Operation::Resume, error));
        }
        if let Err(error) = self.provider.resume(now) {
            return Err(self.reject(now, Operation::Resume, error));
        }
        self.lifecycle = Lifecycle::Running;
        self.record(now, EventKind::Resumed);
        self.verify(now);
        self.announce(now);
        Ok(())
    }

    /// Hands on the sample due at `now`, if there is one and it may be
    /// handed on. An `Err` means the run has just failed: it is the wrapped
    /// provider's error, or `NonMonotonicTimestamp` for a withheld sample.
    /// Afterwards, and whenever no run is running, the answer is `Ok(None)`.
    fn poll(&mut self, now: Timestamp) -> Result<Option<SyntheticLocation>, ProviderError> {
        self.observe(now);
        if self.lifecycle != Lifecycle::Running {
            return Ok(None);
        }
        let result = match self.provider.poll(now) {
            Err(error) => {
                // The provider counts the ticks a late poll skipped before
                // it tries to produce the sample; they are part of the run.
                self.read_missed();
                self.fail_run(now, error.clone());
                Err(error)
            }
            Ok(None) => {
                self.verify(now);
                Ok(None)
            }
            Ok(Some(sample)) => {
                self.read_missed();
                match self.last_forwarded {
                    Some(previous) if sample.timestamp <= previous => {
                        self.run.withheld = self.run.withheld.saturating_add(1);
                        let error =
                            ProviderError::Validation(ValidationError::NonMonotonicTimestamp {
                                previous,
                                current: sample.timestamp,
                            });
                        self.fail_run(now, error.clone());
                        Err(error)
                    }
                    _ => {
                        self.run.forwarded = self.run.forwarded.saturating_add(1);
                        self.run.last_sample = Some(sample);
                        self.last_forwarded = Some(sample.timestamp);
                        self.verify(now);
                        Ok(Some(sample))
                    }
                }
            }
        };
        self.announce(now);
        result
    }

    /// The latest sample handed on in the current run.
    fn current_location(&self) -> Option<SyntheticLocation> {
        self.run.last_sample
    }

    /// The current run as the supervisor counts it: samples handed on, not
    /// samples the wrapped provider produced. Zero when no run exists; the
    /// cumulative numbers are in [`Supervisor::health`].
    fn status(&self) -> ProviderStatus {
        let inner = self.provider.status();
        let running = self.lifecycle.has_run();
        ProviderStatus {
            state: self.lifecycle.state(),
            sample_count: self.run.forwarded,
            failed_count: if running { inner.failed_count } else { 0 },
            missed_ticks: self.run.missed,
            last_sample_time: self.run.last_sample.map(|s| s.timestamp),
            trajectory_complete: running && inner.trajectory_complete,
        }
    }
}
