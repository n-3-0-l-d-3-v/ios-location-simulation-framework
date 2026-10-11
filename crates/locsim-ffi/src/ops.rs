//! Driving a session: its lifecycle, its samples, and when to call next.
//!
//! Time. Every `now_ns` becomes the core's `Timestamp` unchanged: a count
//! of nanoseconds. The library reads no clock and does no arithmetic on the
//! number. Any `int64_t` is accepted, negative ones included, and nothing
//! here depends on where the count starts; the times of one session mean
//! nothing to another. A caller should pass times that do not decrease
//! within a session; the library does not enforce it, and what the
//! supervisor does with a time that went backwards is what it does for any
//! caller.
//!
//! Outputs. A call that does not return `LOCSIM_OK` writes nothing. A call
//! that returns `LOCSIM_OK` with nothing to report (no sample is due, no
//! deadline exists) writes the flag as 0 and leaves the value alone.
//!
//! Order of checks: null pointers, then `struct_size`, then the handle,
//! then the session's own state, then the operation.
//!
//! No call waits for a session: one that another call is using is
//! `LOCSIM_ERR_BUSY`, and nothing was done.

use crate::codes::{source_code, state_code, status_of};
use crate::session::{contain, with_session, LocsimSession};
use crate::status::*;
use locsim_core::domain::{SyntheticLocation, Timestamp};
use locsim_core::provider::{LocationProvider, ProviderError, SimulationProvider};
use locsim_health::Supervisor;

/// One emitted sample. 88 bytes, aligned to 8, no padding.
///
/// `speed_mps` and `course_deg` are known only when their flag is 1. When
/// it is 0 the number is NaN: there is no value, and zero would be one.
/// What an unknown speed or course becomes on a platform is the platform
/// adapter's decision, not this library's.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LocsimSample {
    /// Set by the caller before the call; must be the size of this struct.
    pub struct_size: u32,
    /// A `LOCSIM_SOURCE_*` value.
    pub source: u32,
    /// A `LOCSIM_STATE_*` value.
    pub simulation_state: u32,
    pub has_speed: u8,
    pub has_course: u8,
    /// Zero.
    pub reserved: [u8; 2],
    pub timestamp_ns: i64,
    /// The number of the run the sample belongs to. **A change means a new
    /// run**: positions start again, and nothing connects the samples
    /// before the change to those after it.
    pub run: u64,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub altitude_m: f64,
    pub horizontal_accuracy_m: f64,
    pub vertical_accuracy_m: f64,
    pub speed_mps: f64,
    pub course_deg: f64,
}

const SAMPLE_SIZE: u32 = 88;
const _: () = assert!(std::mem::size_of::<LocsimSample>() == SAMPLE_SIZE as usize);
const _: () = assert!(std::mem::align_of::<LocsimSample>() == 8);

impl LocsimSample {
    fn of(sample: &SyntheticLocation, run: u64) -> Self {
        Self {
            struct_size: SAMPLE_SIZE,
            source: source_code(sample.source),
            simulation_state: state_code(sample.simulation_state),
            has_speed: u8::from(sample.speed_mps.is_some()),
            has_course: u8::from(sample.course_deg.is_some()),
            reserved: [0; 2],
            timestamp_ns: sample.timestamp.as_nanos(),
            run,
            latitude_deg: sample.coordinate.latitude(),
            longitude_deg: sample.coordinate.longitude(),
            altitude_m: sample.altitude_m,
            horizontal_accuracy_m: sample.horizontal_accuracy_m,
            vertical_accuracy_m: sample.vertical_accuracy_m,
            speed_mps: sample.speed_mps.unwrap_or(f64::NAN),
            course_deg: sample.course_deg.unwrap_or(f64::NAN),
        }
    }
}

/// A lifecycle call: the supervisor's answer as a status. A call it
/// refuses changes nothing in the session.
fn lifecycle(
    session: LocsimSession,
    call: impl FnOnce(&mut Supervisor<SimulationProvider>) -> Result<(), ProviderError>,
) -> LocsimStatus {
    let outcome = contain(|| {
        with_session(session, |session| {
            call(&mut session.supervisor).map_err(|error| status_of(&error))
        })
    });
    match outcome {
        Ok(()) => LOCSIM_OK,
        Err(status) => status,
    }
}

/// Begins a run whose first sample is stamped `now_ns`. The session must be
/// stopped; otherwise `LOCSIM_ERR_PROVIDER_TRANSITION`.
#[no_mangle]
pub extern "C" fn locsim_session_start(session: LocsimSession, now_ns: i64) -> LocsimStatus {
    lifecycle(session, |supervisor| {
        supervisor.start(Timestamp::from_nanos(now_ns))
    })
}

/// Ends the session's run, whatever state it is in, and cancels a pending
/// restart. `LOCSIM_ERR_PROVIDER_TRANSITION` if the session is stopped.
#[no_mangle]
pub extern "C" fn locsim_session_stop(session: LocsimSession) -> LocsimStatus {
    lifecycle(session, |supervisor| supervisor.stop())
}

/// Pauses a running session.
#[no_mangle]
pub extern "C" fn locsim_session_pause(session: LocsimSession, now_ns: i64) -> LocsimStatus {
    lifecycle(session, |supervisor| {
        supervisor.pause(Timestamp::from_nanos(now_ns))
    })
}

/// Resumes a paused session. A time earlier than that of the pause is
/// `LOCSIM_ERR_SCHEDULE_CLOCK_WENT_BACKWARDS`, and the session stays paused.
#[no_mangle]
pub extern "C" fn locsim_session_resume(session: LocsimSession, now_ns: i64) -> LocsimStatus {
    lifecycle(session, |supervisor| {
        supervisor.resume(Timestamp::from_nanos(now_ns))
    })
}

/// Asks for the sample due at `now_ns`.
///
/// - `LOCSIM_OK`, `*out_has_sample == 1`: `*out_sample` is the sample.
/// - `LOCSIM_OK`, `*out_has_sample == 0`: nothing is due, or the session
///   is not running. `*out_sample` is not written.
/// - A provider status: the run has just failed and nothing was emitted.
///   It is reported by this one call; later calls return `LOCSIM_OK`
///   without a sample until a run is running again.
///
/// # Safety
///
/// `out_sample` is aligned, readable and writable for 88 bytes and its
/// `struct_size` is set; `out_has_sample` is writable. See the module
/// documentation of `session.rs` for what holds for every pointer.
#[no_mangle]
pub unsafe extern "C" fn locsim_session_poll(
    session: LocsimSession,
    now_ns: i64,
    out_sample: *mut LocsimSample,
    out_has_sample: *mut u8,
) -> LocsimStatus {
    if out_sample.is_null() || out_has_sample.is_null() {
        return LOCSIM_ERR_NULL_POINTER;
    }
    // SAFETY: not null, and readable for the struct's first four bytes by
    // the caller's guarantee. No alignment is assumed.
    let struct_size = unsafe { out_sample.cast::<u32>().read_unaligned() };
    if struct_size != SAMPLE_SIZE {
        return LOCSIM_ERR_STRUCT_SIZE;
    }

    let polled = contain(|| {
        with_session(session, |session| {
            let supervisor = &mut session.supervisor;
            let sample = supervisor
                .poll(Timestamp::from_nanos(now_ns))
                .map_err(|error| status_of(&error))?;
            // Read under the same lock as the sample it describes.
            Ok(sample.map(|sample| LocsimSample::of(&sample, supervisor.health().run)))
        })
    });
    match polled {
        Err(status) => status,
        Ok(None) => {
            // SAFETY: not null and writable by the caller's guarantee.
            unsafe { out_has_sample.write(0) };
            LOCSIM_OK
        }
        Ok(Some(sample)) => {
            // SAFETY: both are not null and writable for their size by the
            // caller's guarantee. No alignment is assumed.
            unsafe {
                out_sample.write_unaligned(sample);
                out_has_sample.write(1);
            }
            LOCSIM_OK
        }
    }
}

/// Says when the session next needs a call: the earlier of the time the
/// next sample is due (only while running) and the time of a pending
/// restart attempt (only while recovering).
///
/// - `LOCSIM_OK`, `*out_has_deadline == 1`: `*out_deadline_ns` is that time.
/// - `LOCSIM_OK`, `*out_has_deadline == 0`: there is none, because the
///   session is stopped, paused or has failed, or because the next sample's
///   time cannot be represented; in that last case the next poll reports
///   it. `*out_deadline_ns` is not written.
///
/// # Safety
///
/// Both pointers are writable.
#[no_mangle]
pub unsafe extern "C" fn locsim_session_next_deadline(
    session: LocsimSession,
    out_deadline_ns: *mut i64,
    out_has_deadline: *mut u8,
) -> LocsimStatus {
    if out_deadline_ns.is_null() || out_has_deadline.is_null() {
        return LOCSIM_ERR_NULL_POINTER;
    }
    let found = contain(|| {
        with_session(session, |session| {
            let supervisor = &session.supervisor;
            let tick = supervisor.provider().next_deadline();
            let retry = supervisor.next_deadline();
            Ok(match (tick, retry) {
                (Some(tick), Some(retry)) => Some(tick.min(retry)),
                (tick, retry) => tick.or(retry),
            })
        })
    });
    match found {
        Err(status) => status,
        Ok(None) => {
            // SAFETY: not null and writable by the caller's guarantee.
            unsafe { out_has_deadline.write(0) };
            LOCSIM_OK
        }
        Ok(Some(deadline)) => {
            // SAFETY: both are not null and writable by the caller's
            // guarantee. No alignment is assumed.
            unsafe {
                out_deadline_ns.write_unaligned(deadline.as_nanos());
                out_has_deadline.write(1);
            }
            LOCSIM_OK
        }
    }
}

#[cfg(test)]
mod tests;
