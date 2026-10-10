//! What the supervisor reports as it happens: structured events, handed to
//! a sink the caller supplies.
//!
//! # What an event may contain
//!
//! No event carries a coordinate, at any level, and there is no event per
//! sample. An event holds times, counts, states, and the provider's own
//! error values, none of which contains a position. The one free text is
//! the detail of a fault the caller reports
//! ([`EventKind::FaultReported`]); what goes into it is the caller's
//! responsibility.
//!
//! # Events and totals
//!
//! The supervisor's cumulative totals are updated from the events
//! themselves, in the one place that sends an event to the sink. A total
//! can therefore always be recomputed from the event stream; the
//! identities are listed on `Totals`.

use locsim_core::domain::{HealthState, Timestamp};
use locsim_core::provider::ProviderError;
use std::fmt;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// A run ended in failure, or the supervisor gave up.
    Error,
    /// Something is wrong but samples may still flow.
    Warn,
    /// Lifecycle and recovery progress.
    Info,
    /// Detail for diagnosis.
    Debug,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        })
    }
}

/// A lifecycle call made on the supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    Start,
    Stop,
    Pause,
    Resume,
}

/// Why a run is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunEndReason {
    /// The run failed.
    Failure,
    /// The caller stopped the supervisor.
    Stopped,
}

/// Why the supervisor will not try again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailReason {
    /// Every restart attempt the policy allows has been used.
    AttemptsExhausted,
    /// The error is one a restart cannot cure.
    Permanent,
    /// The time of the next attempt cannot be represented.
    RetryTimeUnrepresentable,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    /// A run began. `recovery_attempt` is `None` for a run the caller
    /// started and the attempt number for one the supervisor restarted.
    RunStarted {
        recovery_attempt: Option<u32>,
    },
    /// A run is over. `samples` were forwarded to the caller, `withheld`
    /// were refused by the supervisor, `missed` ticks were skipped.
    RunEnded {
        samples: u64,
        withheld: u64,
        missed: u64,
        reason: RunEndReason,
    },
    Stopped,
    Paused,
    Resumed,
    /// A lifecycle call was refused. Nothing changed: no run failed, no
    /// restart attempt was used.
    OperationRejected {
        operation: Operation,
        error: ProviderError,
    },
    /// The current run can no longer continue. Always followed by
    /// [`EventKind::RunEnded`].
    RunFailed {
        error: ProviderError,
    },
    RecoveryScheduled {
        attempt: u32,
        at: Timestamp,
    },
    RecoveryAttempted {
        attempt: u32,
    },
    /// The restart itself failed. The attempt is used up.
    RecoveryStartFailed {
        attempt: u32,
        error: ProviderError,
    },
    /// A restarted run has been stable for the configured time; the
    /// failure episode is over. `attempts` is how many restarts it took.
    Recovered {
        attempts: u32,
    },
    /// The supervisor has given up until it is stopped and started again.
    Failed {
        reason: FailReason,
    },
    /// The provider is running but nothing has been emitted for this long.
    Stalled {
        silent_for_ns: i64,
    },
    StallCleared,
    /// More ticks were missed in the current window than the policy allows.
    FallingBehind {
        missed: u64,
        slots: u64,
    },
    CaughtUp,
    FaultReported {
        component: &'static str,
        detail: String,
    },
    FaultCleared {
        component: &'static str,
    },
    HealthChanged {
        from: HealthState,
        to: HealthState,
    },
    /// A call was given a time earlier than one given before. `latest` is
    /// the latest time seen; the event's own time is the earlier one.
    ClockWentBackwards {
        latest: Timestamp,
    },
}

impl EventKind {
    pub fn level(&self) -> Level {
        match self {
            EventKind::RunFailed { .. }
            | EventKind::RecoveryStartFailed { .. }
            | EventKind::Failed { .. } => Level::Error,
            EventKind::OperationRejected { .. }
            | EventKind::Stalled { .. }
            | EventKind::FallingBehind { .. }
            | EventKind::FaultReported { .. }
            | EventKind::ClockWentBackwards { .. } => Level::Warn,
            EventKind::RunStarted { .. }
            | EventKind::Stopped
            | EventKind::Paused
            | EventKind::Resumed
            | EventKind::RecoveryScheduled { .. }
            | EventKind::RecoveryAttempted { .. }
            | EventKind::Recovered { .. }
            | EventKind::StallCleared
            | EventKind::CaughtUp
            | EventKind::FaultCleared { .. }
            | EventKind::HealthChanged { .. } => Level::Info,
            EventKind::RunEnded { .. } => Level::Debug,
        }
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventKind::RunStarted {
                recovery_attempt: None,
            } => write!(f, "run started"),
            EventKind::RunStarted {
                recovery_attempt: Some(n),
            } => write!(f, "run started by recovery attempt {n}"),
            EventKind::RunEnded {
                samples,
                withheld,
                missed,
                reason,
            } => write!(
                f,
                "run ended ({reason:?}): {samples} samples, {withheld} withheld, {missed} missed"
            ),
            EventKind::Stopped => write!(f, "stopped"),
            EventKind::Paused => write!(f, "paused"),
            EventKind::Resumed => write!(f, "resumed"),
            EventKind::OperationRejected { operation, error } => {
                write!(f, "{operation:?} rejected: {error}")
            }
            EventKind::RunFailed { error } => write!(f, "run failed: {error}"),
            EventKind::RecoveryScheduled { attempt, at } => {
                write!(
                    f,
                    "recovery attempt {attempt} scheduled for {} ns",
                    at.as_nanos()
                )
            }
            EventKind::RecoveryAttempted { attempt } => write!(f, "recovery attempt {attempt}"),
            EventKind::RecoveryStartFailed { attempt, error } => {
                write!(
                    f,
                    "recovery attempt {attempt} could not start a run: {error}"
                )
            }
            EventKind::Recovered { attempts } => {
                write!(f, "recovered after {attempts} attempt(s)")
            }
            EventKind::Failed { reason } => write!(f, "failed: {reason:?}"),
            EventKind::Stalled { silent_for_ns } => {
                write!(f, "stalled: no sample for {silent_for_ns} ns")
            }
            EventKind::StallCleared => write!(f, "samples are arriving again"),
            EventKind::FallingBehind { missed, slots } => {
                write!(f, "falling behind: {missed} of {slots} ticks missed")
            }
            EventKind::CaughtUp => write!(f, "no longer falling behind"),
            EventKind::FaultReported { component, detail } => {
                write!(f, "fault reported by {component}: {detail}")
            }
            EventKind::FaultCleared { component } => write!(f, "fault of {component} cleared"),
            EventKind::HealthChanged { from, to } => write!(f, "health {from:?} -> {to:?}"),
            EventKind::ClockWentBackwards { latest } => write!(
                f,
                "time went backwards (latest seen {} ns)",
                latest.as_nanos()
            ),
        }
    }
}

/// One thing that happened.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// The time passed to the call during which it happened. `stop` takes
    /// no time; its events carry the latest time the supervisor had seen.
    pub at: Timestamp,
    pub level: Level,
    /// The number of the run it concerns: the current one, or the latest
    /// if none is current; `0` before any run.
    pub run: u64,
    pub kind: EventKind,
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} run={} {}",
            self.at.as_nanos(),
            self.level,
            self.run,
            self.kind
        )
    }
}

/// Where events go. Implemented by the caller: this crate writes to no
/// file, console or operating-system log.
pub trait EventSink {
    fn record(&mut self, event: &Event);
}

/// Discards every event.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullSink;

impl EventSink for NullSink {
    fn record(&mut self, _event: &Event) {}
}

/// Keeps every event in memory. Clones share the same list, so one clone
/// can be given to a supervisor and another kept to read from.
///
/// It grows without limit; it is meant for tests and short diagnostics.
#[derive(Debug, Clone, Default)]
pub struct MemorySink {
    events: Arc<Mutex<Vec<Event>>>,
}

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    /// A copy of everything recorded so far, oldest first.
    pub fn events(&self) -> Vec<Event> {
        self.lock().clone()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Event>> {
        // The list is only ever appended to, so it is sound after a panic
        // elsewhere.
        self.events.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl EventSink for MemorySink {
    fn record(&mut self, event: &Event) {
        self.lock().push(event.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use locsim_core::domain::{InvalidTransition, SimulationState};

    fn event(kind: EventKind) -> Event {
        Event {
            at: Timestamp::from_nanos(1_700_000_000_000_000_000),
            level: kind.level(),
            run: 2,
            kind,
        }
    }

    #[test]
    fn levels_follow_severity() {
        let transition = ProviderError::Transition(InvalidTransition {
            from: SimulationState::Idle,
            to: SimulationState::Paused,
        });
        assert_eq!(
            EventKind::RunFailed {
                error: transition.clone()
            }
            .level(),
            Level::Error
        );
        assert_eq!(
            EventKind::Failed {
                reason: FailReason::AttemptsExhausted
            }
            .level(),
            Level::Error
        );
        assert_eq!(
            EventKind::OperationRejected {
                operation: Operation::Pause,
                error: transition
            }
            .level(),
            Level::Warn
        );
        assert_eq!(EventKind::Stalled { silent_for_ns: 1 }.level(), Level::Warn);
        assert_eq!(EventKind::Recovered { attempts: 1 }.level(), Level::Info);
        assert_eq!(
            EventKind::RunEnded {
                samples: 1,
                withheld: 0,
                missed: 0,
                reason: RunEndReason::Stopped
            }
            .level(),
            Level::Debug
        );
        assert!(
            Level::Error < Level::Warn && Level::Warn < Level::Info && Level::Info < Level::Debug
        );
    }

    #[test]
    fn an_event_formats_as_one_line() {
        let line = event(EventKind::RecoveryScheduled {
            attempt: 2,
            at: Timestamp::from_nanos(1_700_000_002_000_000_000),
        })
        .to_string();
        assert_eq!(
            line,
            "1700000000000000000 INFO run=2 recovery attempt 2 scheduled for 1700000002000000000 ns"
        );
        let line = event(EventKind::RunEnded {
            samples: 124,
            withheld: 0,
            missed: 3,
            reason: RunEndReason::Failure,
        })
        .to_string();
        assert_eq!(
            line,
            "1700000000000000000 DEBUG run=2 run ended (Failure): 124 samples, 0 withheld, 3 missed"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn a_memory_sink_shares_its_list_between_clones() {
        let kept = MemorySink::new();
        let mut given: Box<dyn EventSink + Send> = Box::new(kept.clone());
        assert!(kept.is_empty());
        given.record(&event(EventKind::Paused));
        given.record(&event(EventKind::Resumed));
        assert_eq!(kept.len(), 2);
        assert_eq!(kept.events()[1].kind, EventKind::Resumed);
        NullSink.record(&event(EventKind::Stopped));
    }
}
