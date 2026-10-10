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
//! # Recovery is a new run
//!
//! When a run fails and the policy allows, the supervisor waits out a
//! backoff and then starts **a new run of the same scenario**. That is all
//! recovery is. It does not resume anything:
//!
//! - The new run begins where every run of the scenario begins, with fresh
//!   random streams, noise and validation history. Its first sample has no
//!   speed and no course.
//! - Between the last sample of the failed run and the first of the new one
//!   there is a jump in position. No continuity is claimed or checked
//!   across it; each run is validated on its own, exactly as before.
//! - The run number in the report increases. A consumer must treat a new
//!   run number as a discontinuity.
//! - A run is a deterministic function of the scenario and the calls made.
//!   A failure that does not depend on timing will happen again in the new
//!   run. The bound on attempts is what limits that; nothing is reseeded or
//!   altered to make a retry differ.
//!
//! What is kept across runs is bookkeeping only: the totals, the last
//! error, and the timestamp of the last sample handed on.
//!
//! ## Attempts
//!
//! `max_recovery_attempts` counts restart attempts in one failure episode.
//! An episode begins at a failure and ends when a restarted run has emitted
//! for `stable_after_s` (the count returns to zero), when the supervisor
//! gives up, or when it is stopped. A restart that cannot start a run uses
//! up its attempt like any other. A failure while a restarted run is still
//! on probation continues the same episode.
//!
//! ## Which errors are retried
//!
//! All of them, within the bound, except one that a later start provably
//! cannot cure: tick times that are no longer representable
//! (`ScheduleError::Overflow`). Errors are told apart by their variant;
//! nothing is read from their text. Retrying is not a claim that the error
//! is recoverable, only that it is not known to be permanent.
//!
//! # Timestamps
//!
//! Within one session (from `start` to `stop`) the timestamps of the
//! samples handed on strictly increase. A sample whose timestamp is not
//! later than the previous one is withheld and ends the run, reported as
//! `ValidationError::NonMonotonicTimestamp`. This holds across automatic
//! restarts too.
//!
//! With `SimulationProvider` the check never has to act. A restarted run is
//! started at the time of the call that restarts it, which is not before
//! the failing poll, and every sample of a run is stamped at or after the
//! start of its run: with the ideal time of its tick, so later than the
//! start if the first poll comes late, never earlier. The last sample
//! handed on belongs to a tick before the one that failed. For any other
//! provider the check is what makes the statement true.
//!
//! Nothing is promised across a `stop` and a later `start`: like the core
//! provider, a new session may start at any time.
//!
//! # The watchdog
//!
//! Two things are watched while a run is running. Neither ends the run or
//! starts a restart: they make the health `Degraded` and say why.
//!
//! **Stall.** The run is stalled when nothing has been handed on for longer
//! than `stall_after_s`, measured between the times passed to calls: from
//! the call that last handed on a sample (or that started, restarted or
//! resumed the run) to the latest call. It is evaluated by `check`, and by
//! a `poll` that yields nothing. It is not evaluated while paused, and
//! pausing clears it: nothing is expected then. The report says so twice,
//! as a cause and as `Emission::Stalled`, because the lifecycle still says
//! running. A stall can only be noticed by a call; if nothing calls the
//! supervisor, nothing is noticed.
//!
//! **Falling behind.** The provider counts the tick slots a late poll
//! skipped. After every poll that produced a sample or an error the
//! supervisor reads that count and takes the increase. A poll that hands on
//! a sample advances the run by that increase plus one slot. Slots are
//! grouped into consecutive windows: a window closes at the first sample
//! with which the slots since the last close reach `missed_window_ticks`.
//! (A late poll can carry a window past that length; the excess is not
//! carried into the next.) The run is falling behind as soon as the missed
//! slots in the open window exceed `max_missed_in_window`, without waiting
//! for the window to close, and stops being so only when a window closes
//! within the limit. Paused time adds no slots, so a window continues
//! across a pause. A new run starts with an empty window.
//!
//! # Faults of other components
//!
//! The supervisor watches one provider. Whatever else can go wrong around
//! it (storing a record, delivering a sample) it learns only if the caller
//! says so, with [`Supervisor::report_fault`], and forgets when the caller
//! says so, with [`Supervisor::clear_fault`]. A reported fault makes a run
//! `Degraded` for as long as it stands. It never stops samples, never ends
//! a run and never causes a restart: a store that cannot write is not a
//! simulation that cannot run. This crate knows no other component by
//! name and depends on none.
//!
//! # One place for state, totals and events
//!
//! Every event passes through [`Supervisor::record`], and the cumulative
//! totals are updated there and nowhere else, from the event itself. A
//! total therefore cannot disagree with the events. Health is never
//! stored: it is computed from the state when asked.

use crate::event::{Event, EventKind, EventSink, FailReason, Operation, RunEndReason};
use crate::policy::{Backoff, HealthPolicy, Limits, PolicyError};
use crate::report::{Cause, Emission, HealthReport, Totals};
use locsim_core::domain::{
    HealthState, InvalidTransition, SimulationState, SyntheticLocation, Timestamp,
};
use locsim_core::provider::{LocationProvider, ProviderError, ProviderStatus, SimulationProvider};
use locsim_core::scheduler::ScheduleError;
use locsim_core::validation::ValidationError;

/// What the supervisor is doing. Its public face is a `SimulationState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    /// No run and none scheduled.
    Stopped,
    Running,
    Paused,
    /// The run failed; a restart is due at `at`.
    AwaitingRetry {
        at: Timestamp,
    },
    /// The run failed for good; waiting to be stopped.
    Failed,
}

impl Lifecycle {
    fn state(self) -> SimulationState {
        match self {
            Lifecycle::Stopped => SimulationState::Idle,
            Lifecycle::Running => SimulationState::Running,
            Lifecycle::Paused => SimulationState::Paused,
            Lifecycle::AwaitingRetry { .. } => SimulationState::Recovering,
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
    /// Time of the call that last handed on a sample, or that started,
    /// restarted or resumed the run.
    last_progress_at: Option<Timestamp>,
    stalled: bool,
    /// Tick slots, and missed ones among them, since the window opened.
    window_slots: u64,
    window_missed: u64,
    falling_behind: bool,
    /// Set for a run started by a recovery, until it has proved stable.
    /// Holds the timestamp of its first sample once there is one.
    probation: Option<Option<Timestamp>>,
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
    /// Restart attempts used in the current failure episode.
    attempts_used: u32,
    /// The delays of the current failure episode; `None` outside one.
    backoff: Option<Backoff>,
    /// Totals of the runs that are over. The current run is added on read.
    ended: Totals,
    /// Latest time passed to any call.
    observed_at: Option<Timestamp>,
    clock_regressed: bool,
    /// The health last announced with a `HealthChanged` event.
    announced: HealthState,
    /// Faults the caller has reported and not cleared, in the order they
    /// were first reported, with their latest detail.
    faults: Vec<(&'static str, String)>,
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
            attempts_used: 0,
            backoff: None,
            ended: Totals::default(),
            observed_at: None,
            clock_regressed: false,
            announced: HealthState::Stopped,
            faults: Vec::new(),
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
            attempts_used: self.attempts_used,
            max_attempts: self.limits.max_recovery_attempts,
            retry_at: self.next_deadline(),
            last_error: self.last_error.clone(),
            totals: self.totals(),
            current: self.status(),
            active_faults: self
                .faults
                .iter()
                .map(|(component, _)| *component)
                .collect(),
        }
    }

    /// Lets the supervisor act on the time without asking for a sample: a
    /// restart attempt that has come due is made (the new run is started
    /// but not polled), and a running run is checked for a stall. Returns
    /// the state afterwards.
    ///
    /// Nothing in this crate calls it. Whoever owns a timer does.
    pub fn check(&mut self, now: Timestamp) -> HealthReport {
        self.observe(now);
        if let Lifecycle::AwaitingRetry { at } = self.lifecycle {
            if now >= at {
                self.attempt_restart(now);
            }
        }
        self.watch_for_stall(now);
        self.announce(now);
        self.health()
    }

    /// Notes that another component is at fault. The fault stands until it
    /// is cleared. Reporting the same component again replaces the detail.
    ///
    /// While a run exists, a standing fault makes the health `Degraded`; it
    /// does nothing else. The detail is recorded as given: keep positions
    /// out of it.
    pub fn report_fault(&mut self, now: Timestamp, component: &'static str, detail: String) {
        self.observe(now);
        match self.faults.iter_mut().find(|(c, _)| *c == component) {
            Some(fault) => fault.1.clone_from(&detail),
            None => self.faults.push((component, detail.clone())),
        }
        self.record(now, EventKind::FaultReported { component, detail });
        self.announce(now);
    }

    /// Notes that a reported fault is over. Returns whether there was one.
    pub fn clear_fault(&mut self, now: Timestamp, component: &'static str) -> bool {
        self.observe(now);
        let before = self.faults.len();
        self.faults.retain(|(c, _)| *c != component);
        let cleared = self.faults.len() < before;
        if cleared {
            self.record(now, EventKind::FaultCleared { component });
            self.announce(now);
        }
        cleared
    }

    /// When the supervisor next needs to be called for something it has
    /// scheduled itself: the time of the pending restart attempt, if any.
    pub fn next_deadline(&self) -> Option<Timestamp> {
        match self.lifecycle {
            Lifecycle::AwaitingRetry { at } => Some(at),
            _ => None,
        }
    }

    // --- Derived, never stored ----------------------------------------------------

    fn causes(&self) -> Vec<Cause> {
        let mut causes = Vec::new();
        if !self.lifecycle.has_run() {
            return causes;
        }
        if self.run.stalled {
            causes.push(Cause::Stalled);
        }
        if self.run.falling_behind {
            causes.push(Cause::FallingBehind);
        }
        if self.run.probation.is_some() {
            causes.push(Cause::Probation);
        }
        causes.extend(
            self.faults
                .iter()
                .map(|(component, _)| Cause::ExternalFault(component)),
        );
        causes
    }

    fn derive_health(&self, causes: &[Cause]) -> HealthState {
        match self.lifecycle {
            Lifecycle::Stopped => HealthState::Stopped,
            Lifecycle::Failed => HealthState::Failed,
            Lifecycle::AwaitingRetry { .. } => HealthState::Recovering,
            Lifecycle::Running | Lifecycle::Paused if causes.is_empty() => HealthState::Healthy,
            Lifecycle::Running | Lifecycle::Paused => HealthState::Degraded,
        }
    }

    fn emission(&self) -> Emission {
        match self.lifecycle {
            Lifecycle::Running if self.run.stalled => Emission::Stalled {
                silent_for_ns: self.silent_for_ns(),
            },
            Lifecycle::Running => Emission::Flowing,
            _ => Emission::NotExpected,
        }
    }

    /// Time from the last progress of the run to the latest call.
    fn silent_for_ns(&self) -> i64 {
        match (self.observed_at, self.run.last_progress_at) {
            (Some(latest), Some(progress)) => {
                let silent = latest.as_nanos() as i128 - progress.as_nanos() as i128;
                silent.clamp(0, i64::MAX as i128) as i64
            }
            _ => 0,
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
        self.run = Run {
            last_progress_at: self.observed_at,
            ..Run::default()
        };
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
        self.after_failure(at, error);
    }

    /// Decides what follows a failed run or a failed restart: another
    /// attempt, or the end.
    fn after_failure(&mut self, at: Timestamp, error: ProviderError) {
        let permanent = is_permanent(&error);
        self.last_error = Some(error);
        if permanent {
            return self.give_up(at, FailReason::Permanent);
        }
        if self.attempts_used >= self.limits.max_recovery_attempts {
            return self.give_up(at, FailReason::AttemptsExhausted);
        }
        let limits = self.limits;
        let delay_ns = self.backoff.get_or_insert_with(|| limits.backoff()).take();
        match at.checked_add_nanos(delay_ns) {
            Some(retry_at) => {
                self.lifecycle = Lifecycle::AwaitingRetry { at: retry_at };
                self.record(
                    at,
                    EventKind::RecoveryScheduled {
                        attempt: self.attempts_used + 1,
                        at: retry_at,
                    },
                );
            }
            None => self.give_up(at, FailReason::RetryTimeUnrepresentable),
        }
    }

    fn give_up(&mut self, at: Timestamp, reason: FailReason) {
        self.lifecycle = Lifecycle::Failed;
        self.backoff = None;
        self.record(at, EventKind::Failed { reason });
    }

    /// Makes the restart attempt that has come due: stops whatever is left
    /// of the failed run and starts a new one at `now`.
    fn attempt_restart(&mut self, now: Timestamp) {
        self.attempts_used += 1;
        let attempt = self.attempts_used;
        self.record(now, EventKind::RecoveryAttempted { attempt });
        // After a failed poll the provider is in `Error` and must be
        // stopped; after a start that failed it is already idle, and
        // stopping it then would itself be refused.
        let stopped = if self.provider.status().state == SimulationState::Idle {
            Ok(())
        } else {
            self.provider.stop()
        };
        match stopped.and_then(|()| self.provider.start(now)) {
            Ok(()) => {
                self.begin_run(now, Some(attempt));
                self.run.probation = Some(None);
                self.verify(now);
            }
            Err(error) => {
                self.record(
                    now,
                    EventKind::RecoveryStartFailed {
                        attempt,
                        error: error.clone(),
                    },
                );
                self.after_failure(now, error);
            }
        }
    }

    /// Ends the failure episode once the restarted run has emitted for long
    /// enough. Called with the timestamp of a sample just handed on.
    fn note_progress(&mut self, at: Timestamp, sample_time: Timestamp) {
        let Some(first) = &mut self.run.probation else {
            return;
        };
        let first = *first.get_or_insert(sample_time);
        let emitted_for = sample_time.as_nanos() as i128 - first.as_nanos() as i128;
        if emitted_for >= self.limits.stable_after_ns as i128 {
            self.run.probation = None;
            self.backoff = None;
            let attempts = std::mem::take(&mut self.attempts_used);
            self.record(at, EventKind::Recovered { attempts });
        }
    }

    /// Forgets the failure episode, if there is one.
    fn end_episode(&mut self) {
        self.attempts_used = 0;
        self.backoff = None;
    }

    /// Reads the wrapped provider's missed-tick count into the run and
    /// returns by how much it grew.
    fn read_missed(&mut self) -> u64 {
        let missed = self.provider.status().missed_ticks;
        let grew = missed.saturating_sub(self.run.missed);
        self.run.missed = missed;
        grew
    }

    /// Accounts for a sample just handed on: the slots it advanced the run
    /// by, `missed` of them skipped, and the verdict on the window.
    fn note_slots(&mut self, at: Timestamp, missed: u64) {
        let run = &mut self.run;
        run.window_slots = run.window_slots.saturating_add(missed).saturating_add(1);
        run.window_missed = run.window_missed.saturating_add(missed);
        let (slots, in_window) = (run.window_slots, run.window_missed);
        let over = in_window > self.limits.max_missed_in_window;
        let closes = slots >= self.limits.missed_window_ticks;
        if over && !self.run.falling_behind {
            self.run.falling_behind = true;
            self.record(
                at,
                EventKind::FallingBehind {
                    missed: in_window,
                    slots,
                },
            );
        }
        if closes {
            if !over && self.run.falling_behind {
                self.run.falling_behind = false;
                self.record(at, EventKind::CaughtUp);
            }
            self.run.window_slots = 0;
            self.run.window_missed = 0;
        }
    }

    /// Marks the run stalled if nothing has been handed on for too long.
    fn watch_for_stall(&mut self, at: Timestamp) {
        if self.lifecycle != Lifecycle::Running || self.run.stalled {
            return;
        }
        let silent_for_ns = self.silent_for_ns();
        if silent_for_ns > self.limits.stall_after_ns {
            self.run.stalled = true;
            self.record(at, EventKind::Stalled { silent_for_ns });
        }
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

impl Supervisor<SimulationProvider> {
    /// [`Supervisor::new`] for the simulation provider, which additionally
    /// checks what only it makes checkable: that the stall threshold is
    /// longer than the scenario's update interval. A shorter one would
    /// report a provider that is exactly on time as stalled between two
    /// samples.
    pub fn for_simulation(
        provider: SimulationProvider,
        policy: HealthPolicy,
        sink: Box<dyn EventSink + Send>,
    ) -> Result<Self, Vec<PolicyError>> {
        policy.validate_for_interval(provider.scenario().update_interval_s)?;
        Self::new(provider, policy, sink)
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
    /// including after it has failed and while a restart is pending, which
    /// it cancels.
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
        self.end_episode();
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
        // Nothing is expected of a paused run.
        self.run.stalled = false;
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
        self.run.last_progress_at = self.observed_at;
        self.record(now, EventKind::Resumed);
        self.verify(now);
        self.announce(now);
        Ok(())
    }

    /// Hands on the sample due at `now`, if there is one and it may be
    /// handed on. An `Err` means the run has just failed: it is the wrapped
    /// provider's error, or `NonMonotonicTimestamp` for a withheld sample.
    /// Afterwards, and whenever no run is running, the answer is `Ok(None)`.
    ///
    /// If a restart attempt is due it is made first, and the new run is
    /// polled in the same call.
    fn poll(&mut self, now: Timestamp) -> Result<Option<SyntheticLocation>, ProviderError> {
        self.observe(now);
        if let Lifecycle::AwaitingRetry { at } = self.lifecycle {
            if now >= at {
                self.attempt_restart(now);
            }
        }
        if self.lifecycle != Lifecycle::Running {
            self.announce(now);
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
                if self.verify(now) {
                    self.watch_for_stall(now);
                }
                Ok(None)
            }
            Ok(Some(sample)) => {
                let missed = self.read_missed();
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
                        self.run.last_progress_at = self.observed_at;
                        if std::mem::take(&mut self.run.stalled) {
                            self.record(now, EventKind::StallCleared);
                        }
                        self.note_slots(now, missed);
                        self.note_progress(now, sample.timestamp);
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
