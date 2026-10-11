//! C ABI over one supervised simulation session.
//!
//! A session is a `Supervisor<SimulationProvider>` built from a scenario
//! document and a health policy. A caller holds it as an integer handle and
//! never as a pointer: a handle that was destroyed, or never issued, is
//! refused with a status code instead of being undefined behaviour.
//!
//! No Rust type crosses the boundary. Every function returns a
//! [`LocsimStatus`]; data goes out through pointers the caller supplies, and
//! the library keeps none of them after it returns. A panic never unwinds
//! into the caller: it becomes [`LOCSIM_ERR_PANIC`].
//!
//! What this is not: it is not an iOS location provider and it does not make
//! any location appear anywhere. It is the boundary a platform adapter can
//! call. It reads no clock and no file.
//!
//! Present so far (T11A, stage 3): creating and destroying a session, its
//! lifecycle, polling it for samples, asking when to call next, its health,
//! reporting the faults of other components, and reading its events. There
//! is no C header yet, and nothing has been called from C.
//!
//! This is the only crate of the workspace that contains `unsafe`. It is
//! confined to reading and writing through the caller's pointers.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

mod codes;
mod events;
mod health;
#[cfg(test)]
mod hooks;
mod ops;
mod registry;
mod session;
mod status;
#[cfg(test)]
mod testing;

pub use codes::{
    LOCSIM_CAUSE_EXTERNAL_FAULT, LOCSIM_CAUSE_FALLING_BEHIND, LOCSIM_CAUSE_PROBATION,
    LOCSIM_CAUSE_STALLED, LOCSIM_COMPONENT_ADAPTER, LOCSIM_COMPONENT_DELIVERY,
    LOCSIM_COMPONENT_STORE, LOCSIM_EMISSION_FLOWING, LOCSIM_EMISSION_NOT_EXPECTED,
    LOCSIM_EMISSION_STALLED, LOCSIM_EVENT_CAUGHT_UP, LOCSIM_EVENT_CLOCK_WENT_BACKWARDS,
    LOCSIM_EVENT_FAILED, LOCSIM_EVENT_FALLING_BEHIND, LOCSIM_EVENT_FAULT_CLEARED,
    LOCSIM_EVENT_FAULT_REPORTED, LOCSIM_EVENT_HEALTH_CHANGED, LOCSIM_EVENT_OPERATION_REJECTED,
    LOCSIM_EVENT_PAUSED, LOCSIM_EVENT_RECOVERED, LOCSIM_EVENT_RECOVERY_ATTEMPTED,
    LOCSIM_EVENT_RECOVERY_SCHEDULED, LOCSIM_EVENT_RECOVERY_START_FAILED, LOCSIM_EVENT_RESUMED,
    LOCSIM_EVENT_RUN_ENDED, LOCSIM_EVENT_RUN_FAILED, LOCSIM_EVENT_RUN_STARTED,
    LOCSIM_EVENT_STALLED, LOCSIM_EVENT_STALL_CLEARED, LOCSIM_EVENT_STOPPED, LOCSIM_HEALTH_DEGRADED,
    LOCSIM_HEALTH_FAILED, LOCSIM_HEALTH_HEALTHY, LOCSIM_HEALTH_RECOVERING, LOCSIM_HEALTH_STOPPED,
    LOCSIM_LEVEL_DEBUG, LOCSIM_LEVEL_ERROR, LOCSIM_LEVEL_INFO, LOCSIM_LEVEL_WARN,
    LOCSIM_SOURCE_REPLAY, LOCSIM_SOURCE_SIMULATION, LOCSIM_SOURCE_TEST, LOCSIM_STATE_ERROR,
    LOCSIM_STATE_IDLE, LOCSIM_STATE_PAUSED, LOCSIM_STATE_RECOVERING, LOCSIM_STATE_RUNNING,
    LOCSIM_STATE_STARTING, LOCSIM_STATE_STOPPING,
};
pub use events::{locsim_session_events_dropped, locsim_session_next_event, LocsimEvent};
pub use health::{
    locsim_session_check, locsim_session_clear_fault, locsim_session_health,
    locsim_session_report_fault, LocsimHealth,
};
pub use ops::{
    locsim_session_next_deadline, locsim_session_pause, locsim_session_poll, locsim_session_resume,
    locsim_session_start, locsim_session_stop, LocsimSample,
};

pub use session::{
    locsim_session_create, locsim_session_destroy, LocsimPolicy, LocsimSession,
    LOCSIM_CREATE_ERROR_TEXT_MAX, LOCSIM_EVENT_CAPACITY_MAX, LOCSIM_SCENARIO_MAX_BYTES,
    LOCSIM_SESSION_LIMIT,
};
pub use status::*;

/// The version of this interface. Any change to it, of any kind, is a new
/// number; a caller compares for equality and does not continue otherwise.
pub const LOCSIM_ABI_VERSION: u32 = 1;

/// Returns [`LOCSIM_ABI_VERSION`] of the library that was linked.
#[no_mangle]
pub extern "C" fn locsim_abi_version() -> u32 {
    LOCSIM_ABI_VERSION
}
