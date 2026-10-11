//! Rust values as the numbers C sees.
//!
//! Every function here is a `match` without a wildcard: a variant added to
//! one of these types in another crate stops this crate from compiling
//! until it has been given a number, instead of crossing the boundary as
//! one nobody documented.
//!
//! Only the kind of an error crosses. What an error carries (the states of
//! a refused transition, the numbers of a failed validation, a coordinate
//! that was out of range) stays on this side.

use crate::status::*;
use locsim_core::domain::{LocationSource, SimulationState};
use locsim_core::provider::ProviderError;
use locsim_core::scheduler::ScheduleError;

// `SimulationState`
pub const LOCSIM_STATE_IDLE: u32 = 0;
pub const LOCSIM_STATE_STARTING: u32 = 1;
pub const LOCSIM_STATE_RUNNING: u32 = 2;
pub const LOCSIM_STATE_PAUSED: u32 = 3;
pub const LOCSIM_STATE_STOPPING: u32 = 4;
pub const LOCSIM_STATE_ERROR: u32 = 5;
pub const LOCSIM_STATE_RECOVERING: u32 = 6;

// `LocationSource`
pub const LOCSIM_SOURCE_SIMULATION: u32 = 0;
pub const LOCSIM_SOURCE_REPLAY: u32 = 1;
pub const LOCSIM_SOURCE_TEST: u32 = 2;

pub(crate) fn state_code(state: SimulationState) -> u32 {
    match state {
        SimulationState::Idle => LOCSIM_STATE_IDLE,
        SimulationState::Starting => LOCSIM_STATE_STARTING,
        SimulationState::Running => LOCSIM_STATE_RUNNING,
        SimulationState::Paused => LOCSIM_STATE_PAUSED,
        SimulationState::Stopping => LOCSIM_STATE_STOPPING,
        SimulationState::Error => LOCSIM_STATE_ERROR,
        SimulationState::Recovering => LOCSIM_STATE_RECOVERING,
    }
}

pub(crate) fn source_code(source: LocationSource) -> u32 {
    match source {
        LocationSource::Simulation => LOCSIM_SOURCE_SIMULATION,
        LocationSource::Replay => LOCSIM_SOURCE_REPLAY,
        LocationSource::Test => LOCSIM_SOURCE_TEST,
    }
}

/// The status a provider's error is reported as.
pub(crate) fn status_of(error: &ProviderError) -> LocsimStatus {
    match error {
        ProviderError::InvalidScenario(_) => LOCSIM_ERR_PROVIDER_INVALID_SCENARIO,
        ProviderError::Transition(_) => LOCSIM_ERR_PROVIDER_TRANSITION,
        ProviderError::Schedule(schedule) => match schedule {
            ScheduleError::InvalidInterval => LOCSIM_ERR_SCHEDULE_INVALID_INTERVAL,
            ScheduleError::Overflow => LOCSIM_ERR_SCHEDULE_OVERFLOW,
            ScheduleError::AlreadyPaused => LOCSIM_ERR_SCHEDULE_ALREADY_PAUSED,
            ScheduleError::NotPaused => LOCSIM_ERR_SCHEDULE_NOT_PAUSED,
            ScheduleError::ClockWentBackwards => LOCSIM_ERR_SCHEDULE_CLOCK_WENT_BACKWARDS,
        },
        ProviderError::Movement(_) => LOCSIM_ERR_PROVIDER_MOVEMENT,
        ProviderError::Noise(_) => LOCSIM_ERR_PROVIDER_NOISE,
        ProviderError::Derivation(_) => LOCSIM_ERR_PROVIDER_DERIVATION,
        ProviderError::Validation(_) => LOCSIM_ERR_PROVIDER_VALIDATION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use locsim_core::consistency::DeriveError;
    use locsim_core::domain::{InvalidTransition, LocationError, Timestamp};
    use locsim_core::geographic::GeoError;
    use locsim_core::movement::MovementError;
    use locsim_core::noise::NoiseError;
    use locsim_core::validation::ValidationError;

    // The numbers are written out here, not taken from the constants: this
    // is the table the header will state.

    #[test]
    fn every_provider_error_has_its_documented_number() {
        let earlier = Timestamp::from_nanos(1);
        let later = Timestamp::from_nanos(2);
        let cases = [
            (ProviderError::InvalidScenario(Vec::new()), 30),
            (
                ProviderError::Transition(InvalidTransition {
                    from: SimulationState::Idle,
                    to: SimulationState::Paused,
                }),
                31,
            ),
            (
                ProviderError::Movement(MovementError::InvalidConfiguration("x")),
                33,
            ),
            (
                ProviderError::Noise(NoiseError::NonIncreasingTime {
                    previous: later,
                    current: earlier,
                }),
                34,
            ),
            (
                ProviderError::Derivation(DeriveError::Geo(GeoError::NotConverged)),
                35,
            ),
            (
                ProviderError::Validation(ValidationError::Location(
                    LocationError::CourseWithoutMotion,
                )),
                36,
            ),
            (
                ProviderError::Validation(ValidationError::NonMonotonicTimestamp {
                    previous: later,
                    current: earlier,
                }),
                36,
            ),
            (ProviderError::Schedule(ScheduleError::InvalidInterval), 40),
            (ProviderError::Schedule(ScheduleError::Overflow), 41),
            (ProviderError::Schedule(ScheduleError::AlreadyPaused), 42),
            (ProviderError::Schedule(ScheduleError::NotPaused), 43),
            (
                ProviderError::Schedule(ScheduleError::ClockWentBackwards),
                44,
            ),
        ];
        for (error, number) in cases {
            assert_eq!(status_of(&error), number, "{error:?}");
        }
    }

    #[test]
    fn every_state_and_source_has_its_documented_number() {
        let states = [
            (SimulationState::Idle, 0),
            (SimulationState::Starting, 1),
            (SimulationState::Running, 2),
            (SimulationState::Paused, 3),
            (SimulationState::Stopping, 4),
            (SimulationState::Error, 5),
            (SimulationState::Recovering, 6),
        ];
        // The table above names every state there is.
        assert_eq!(states.len(), SimulationState::ALL.len());
        for state in SimulationState::ALL {
            assert!(states.iter().any(|(named, _)| *named == state));
        }
        for (state, number) in states {
            assert_eq!(state_code(state), number, "{state:?}");
        }
        assert_eq!(source_code(LocationSource::Simulation), 0);
        assert_eq!(source_code(LocationSource::Replay), 1);
        assert_eq!(source_code(LocationSource::Test), 2);
    }
}
