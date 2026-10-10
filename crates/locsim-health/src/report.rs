//! What the supervisor says about itself when asked.

use locsim_core::domain::{HealthState, SimulationState, Timestamp};
use locsim_core::provider::{ProviderError, ProviderStatus};

/// Whether samples are actually arriving, as of the latest call.
///
/// This is an observation, kept apart from the lifecycle on purpose: a
/// provider can be running by its lifecycle and silent in fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emission {
    /// No sample is expected: nothing is running, or the run is paused.
    NotExpected,
    /// A run is running and is not stalled.
    Flowing,
    /// A run is running, but nothing has been emitted for this long, which
    /// is longer than the policy's stall threshold. **Samples are not
    /// arriving.**
    Stalled { silent_for_ns: i64 },
}

/// A reason a run that exists is not simply healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// Running, but no sample has arrived for longer than the threshold.
    Stalled,
    /// More ticks were missed in a window than the policy allows.
    FallingBehind,
    /// The run was started by a recovery and has not yet been stable for
    /// the configured time.
    Probation,
    /// The caller has reported a fault of this component and not cleared it.
    ExternalFault(&'static str),
}

/// Cumulative counts over the supervisor's whole life: every session and
/// every run, including the current one.
///
/// Each can be recomputed from the event stream:
///
/// | Total | Equals |
/// |---|---|
/// | `runs_started` | number of `RunStarted` |
/// | `runs_ended` | number of `RunEnded` |
/// | `samples` | sum of `RunEnded.samples`, plus the current run's forwarded samples |
/// | `withheld` | sum of `RunEnded.withheld`, plus the current run's |
/// | `missed_ticks` | sum of `RunEnded.missed`, plus the current run's |
/// | `failures` | number of `RunFailed` = number of `RunEnded` with reason `Failure` |
/// | `recovery_attempts` | number of `RecoveryAttempted` |
/// | `recoveries_completed` | number of `Recovered` |
/// | `terminal_failures` | number of `Failed` |
/// | `rejected_operations` | number of `OperationRejected` |
///
/// Counters saturate instead of wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    pub runs_started: u64,
    pub runs_ended: u64,
    /// Samples handed to the caller.
    pub samples: u64,
    /// Samples the supervisor refused to hand on (a timestamp not later
    /// than the one before it).
    pub withheld: u64,
    pub missed_ticks: u64,
    /// Runs that ended in failure.
    pub failures: u64,
    pub recovery_attempts: u64,
    pub recoveries_completed: u64,
    /// Times the supervisor gave up.
    pub terminal_failures: u64,
    /// Lifecycle calls that were refused without changing anything.
    pub rejected_operations: u64,
}

/// The supervisor's state as of the latest time passed to it.
///
/// Three different questions are answered by three fields:
///
/// - `lifecycle`: what is the supervisor doing?
/// - `emission`: are samples actually arriving?
/// - `health`: one word, derived from the two and from `causes`.
///
/// | `health` | When | `lifecycle` |
/// |---|---|---|
/// | `Stopped` | No run exists and none is scheduled: never started, stopped, or a start was refused | `Idle` |
/// | `Healthy` | A run exists, has not failed, and no cause is active | `Running`, `Paused` |
/// | `Degraded` | A run exists, has not failed, and `causes` is not empty | `Running`, `Paused` |
/// | `Recovering` | The run failed and a restart is scheduled; `retry_at` says when | `Recovering` |
/// | `Failed` | The run failed and no restart will happen until `stop` and `start` | `Error` |
///
/// A stalled provider is `Degraded` with `Cause::Stalled`, and `emission`
/// is `Stalled`: the lifecycle says running, the observation says nothing
/// is coming.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthReport {
    /// The latest time passed to any call; `None` before the first.
    pub observed_at: Option<Timestamp>,
    pub lifecycle: SimulationState,
    pub health: HealthState,
    /// Every active cause. Empty unless `health` is `Degraded`.
    pub causes: Vec<Cause>,
    pub emission: Emission,
    /// Number of the current run, or of the latest one; `0` before any.
    /// It never repeats. **A change means a new run**: positions restart
    /// and nothing connects the samples before it to those after.
    pub run: u64,
    /// Restart attempts used in the current failure episode.
    pub attempts_used: u32,
    pub max_attempts: u32,
    /// When the next restart attempt is due. `Some` exactly while
    /// `health` is `Recovering`.
    pub retry_at: Option<Timestamp>,
    /// The error that ended the latest failed run or restart attempt.
    pub last_error: Option<ProviderError>,
    pub totals: Totals,
    /// The current run as the supervisor's own `status()` reports it.
    pub current: ProviderStatus,
    /// Components with a reported, uncleared fault. Listed whatever the
    /// lifecycle; they affect `health` only while a run exists.
    pub active_faults: Vec<&'static str>,
}
