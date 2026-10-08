//! Location abstraction: the interface every source of locations implements,
//! whether simulated, replayed, test-injected or platform-backed.
//!
//! Time is always passed in. A provider never reads a clock or sleeps, so a
//! run is a pure function of (scenario, start time, poll times), and sample
//! content does not depend on poll times at all.

mod simulation;

pub use simulation::{ModelFactory, SimulationProvider};

use crate::domain::{
    ConfigError, InvalidTransition, SimulationState, SyntheticLocation, Timestamp,
};
use crate::movement::MovementError;
use crate::noise::NoiseError;
use crate::scheduler::ScheduleError;
use crate::validation::ValidationError;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderError {
    InvalidScenario(Vec<ConfigError>),
    Transition(InvalidTransition),
    Schedule(ScheduleError),
    Movement(MovementError),
    Noise(NoiseError),
    /// A generated sample was rejected by the validation gate.
    Validation(ValidationError),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::InvalidScenario(errors) => {
                write!(f, "invalid scenario:")?;
                for e in errors {
                    write!(f, " [{e}]")?;
                }
                Ok(())
            }
            ProviderError::Transition(e) => write!(f, "{e}"),
            ProviderError::Schedule(e) => write!(f, "scheduler: {e}"),
            ProviderError::Movement(e) => write!(f, "{e}"),
            ProviderError::Noise(e) => write!(f, "{e}"),
            ProviderError::Validation(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProviderError {}

impl From<InvalidTransition> for ProviderError {
    fn from(e: InvalidTransition) -> Self {
        ProviderError::Transition(e)
    }
}

impl From<ScheduleError> for ProviderError {
    fn from(e: ScheduleError) -> Self {
        ProviderError::Schedule(e)
    }
}

impl From<MovementError> for ProviderError {
    fn from(e: MovementError) -> Self {
        ProviderError::Movement(e)
    }
}

impl From<NoiseError> for ProviderError {
    fn from(e: NoiseError) -> Self {
        ProviderError::Noise(e)
    }
}

impl From<ValidationError> for ProviderError {
    fn from(e: ValidationError) -> Self {
        ProviderError::Validation(e)
    }
}

/// Observable counters for one provider. Counters cover the current run and
/// are reset by `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderStatus {
    pub state: SimulationState,
    /// Samples that passed validation and were emitted.
    pub sample_count: u64,
    /// Samples that were generated but rejected (never emitted).
    pub failed_count: u64,
    /// Scheduler slots skipped because polling fell behind.
    pub missed_ticks: u64,
    pub last_sample_time: Option<Timestamp>,
}

pub trait LocationProvider {
    /// Begins a run whose first sample is stamped `now`.
    fn start(&mut self, now: Timestamp) -> Result<(), ProviderError>;

    /// Ends the run and returns to `Idle`. Legal from every non-idle state.
    fn stop(&mut self) -> Result<(), ProviderError>;

    fn pause(&mut self, now: Timestamp) -> Result<(), ProviderError>;

    fn resume(&mut self, now: Timestamp) -> Result<(), ProviderError>;

    /// Emits the sample due at `now`, if any. `Ok(None)` means nothing is
    /// due (or the provider is not running). An `Err` means a sample could
    /// not be produced validly; nothing was emitted.
    fn poll(&mut self, now: Timestamp) -> Result<Option<SyntheticLocation>, ProviderError>;

    /// Most recently emitted sample of the current run.
    fn current_location(&self) -> Option<SyntheticLocation>;

    fn status(&self) -> ProviderStatus;
}
