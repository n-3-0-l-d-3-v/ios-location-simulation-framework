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
use locsim_core::domain::{HealthState, LocationSource, SimulationState};
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

// `HealthState`
pub const LOCSIM_HEALTH_HEALTHY: u32 = 0;
pub const LOCSIM_HEALTH_DEGRADED: u32 = 1;
pub const LOCSIM_HEALTH_RECOVERING: u32 = 2;
pub const LOCSIM_HEALTH_FAILED: u32 = 3;
pub const LOCSIM_HEALTH_STOPPED: u32 = 4;

// `Emission`
pub const LOCSIM_EMISSION_NOT_EXPECTED: u32 = 0;
pub const LOCSIM_EMISSION_FLOWING: u32 = 1;
pub const LOCSIM_EMISSION_STALLED: u32 = 2;

// `Cause`, as bits
pub const LOCSIM_CAUSE_STALLED: u32 = 1;
pub const LOCSIM_CAUSE_FALLING_BEHIND: u32 = 2;
pub const LOCSIM_CAUSE_PROBATION: u32 = 4;
pub const LOCSIM_CAUSE_EXTERNAL_FAULT: u32 = 8;

// The components a fault can be reported for.
pub const LOCSIM_COMPONENT_STORE: u32 = 1;
pub const LOCSIM_COMPONENT_ADAPTER: u32 = 2;
pub const LOCSIM_COMPONENT_DELIVERY: u32 = 3;

// `Level`
pub const LOCSIM_LEVEL_ERROR: u32 = 0;
pub const LOCSIM_LEVEL_WARN: u32 = 1;
pub const LOCSIM_LEVEL_INFO: u32 = 2;
pub const LOCSIM_LEVEL_DEBUG: u32 = 3;

// `EventKind`. After each, what `arg` holds; what is not named is 0.
/// `arg[0]`: 1 if a recovery started the run; `arg[1]`: the attempt.
pub const LOCSIM_EVENT_RUN_STARTED: u32 = 1;
/// `arg[0..3]`: samples handed on, samples withheld, ticks missed;
/// `arg[3]`: 0 if the run failed, 1 if it was stopped.
pub const LOCSIM_EVENT_RUN_ENDED: u32 = 2;
pub const LOCSIM_EVENT_STOPPED: u32 = 3;
pub const LOCSIM_EVENT_PAUSED: u32 = 4;
pub const LOCSIM_EVENT_RESUMED: u32 = 5;
/// `arg[0]`: 0 start, 1 stop, 2 pause, 3 resume; `error` is set.
pub const LOCSIM_EVENT_OPERATION_REJECTED: u32 = 6;
/// `error` is set.
pub const LOCSIM_EVENT_RUN_FAILED: u32 = 7;
/// `arg[0]`: the attempt; `arg[1]`: when it is due.
pub const LOCSIM_EVENT_RECOVERY_SCHEDULED: u32 = 8;
/// `arg[0]`: the attempt.
pub const LOCSIM_EVENT_RECOVERY_ATTEMPTED: u32 = 9;
/// `arg[0]`: the attempt; `error` is set.
pub const LOCSIM_EVENT_RECOVERY_START_FAILED: u32 = 10;
/// `arg[0]`: how many attempts it took.
pub const LOCSIM_EVENT_RECOVERED: u32 = 11;
/// `arg[0]`: 0 attempts exhausted, 1 the error is permanent, 2 the time of
/// the next attempt cannot be represented.
pub const LOCSIM_EVENT_FAILED: u32 = 12;
/// `arg[0]`: nanoseconds without a sample.
pub const LOCSIM_EVENT_STALLED: u32 = 13;
pub const LOCSIM_EVENT_STALL_CLEARED: u32 = 14;
/// `arg[0]`: ticks missed in the window; `arg[1]`: slots of the window.
pub const LOCSIM_EVENT_FALLING_BEHIND: u32 = 15;
pub const LOCSIM_EVENT_CAUGHT_UP: u32 = 16;
/// `arg[0]`: the component.
pub const LOCSIM_EVENT_FAULT_REPORTED: u32 = 17;
/// `arg[0]`: the component.
pub const LOCSIM_EVENT_FAULT_CLEARED: u32 = 18;
/// `arg[0]`, `arg[1]`: the health before and after.
pub const LOCSIM_EVENT_HEALTH_CHANGED: u32 = 19;
/// `arg[0]`: the latest time the session had been given; `at_ns` is the
/// earlier one it was given now.
pub const LOCSIM_EVENT_CLOCK_WENT_BACKWARDS: u32 = 20;

pub(crate) fn health_code(health: HealthState) -> u32 {
    match health {
        HealthState::Healthy => LOCSIM_HEALTH_HEALTHY,
        HealthState::Degraded => LOCSIM_HEALTH_DEGRADED,
        HealthState::Recovering => LOCSIM_HEALTH_RECOVERING,
        HealthState::Failed => LOCSIM_HEALTH_FAILED,
        HealthState::Stopped => LOCSIM_HEALTH_STOPPED,
    }
}

/// The name the supervisor knows a component by. Only these exist: a code
/// from C that is not one of them names nothing.
pub(crate) fn component_name(code: u32) -> Option<&'static str> {
    match code {
        LOCSIM_COMPONENT_STORE => Some("store"),
        LOCSIM_COMPONENT_ADAPTER => Some("adapter"),
        LOCSIM_COMPONENT_DELIVERY => Some("delivery"),
        _ => None,
    }
}

/// The code of a component name, or 0 for a name this library never gives
/// the supervisor.
pub(crate) fn component_code(name: &str) -> u32 {
    [
        LOCSIM_COMPONENT_STORE,
        LOCSIM_COMPONENT_ADAPTER,
        LOCSIM_COMPONENT_DELIVERY,
    ]
    .into_iter()
    .find(|code| component_name(*code) == Some(name))
    .unwrap_or(0)
}

/// A component's bit in a set of faults; no bit for a name without a code.
pub(crate) fn component_bit(name: &str) -> u32 {
    match component_code(name) {
        0 => 0,
        code => 1 << (code - 1),
    }
}

/// A provider's error as it appears in a report or an event: its status,
/// which is never negative.
pub(crate) fn error_code(error: &ProviderError) -> u32 {
    u32::try_from(status_of(error)).unwrap_or(0)
}

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

    #[test]
    fn every_health_has_its_documented_number() {
        assert_eq!(health_code(HealthState::Healthy), 0);
        assert_eq!(health_code(HealthState::Degraded), 1);
        assert_eq!(health_code(HealthState::Recovering), 2);
        assert_eq!(health_code(HealthState::Failed), 3);
        assert_eq!(health_code(HealthState::Stopped), 4);
    }

    #[test]
    fn exactly_three_components_exist_and_no_other_code_names_one() {
        assert_eq!(component_name(1), Some("store"));
        assert_eq!(component_name(2), Some("adapter"));
        assert_eq!(component_name(3), Some("delivery"));
        for code in [0, 4, 5, 255, 256, u32::MAX] {
            assert_eq!(component_name(code), None, "{code}");
        }
        for (name, code, bit) in [("store", 1, 1), ("adapter", 2, 2), ("delivery", 3, 4)] {
            assert_eq!(component_code(name), code);
            assert_eq!(component_bit(name), bit);
        }
        // A name the library never uses has no code and no bit.
        for name in ["", "Store", "store ", "other"] {
            assert_eq!(
                (component_code(name), component_bit(name)),
                (0, 0),
                "{name:?}"
            );
        }
    }

    #[test]
    fn an_error_in_a_report_is_its_status() {
        assert_eq!(
            error_code(&ProviderError::Schedule(ScheduleError::Overflow)),
            41
        );
        assert_eq!(error_code(&ProviderError::InvalidScenario(Vec::new())), 30);
    }
}
