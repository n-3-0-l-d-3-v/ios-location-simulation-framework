//! A session's health, and the faults of other components reported to it.
//!
//! The report is the supervisor's `HealthReport` as a flat struct. What
//! does not cross: the order in which causes and faults arose (they are
//! bit sets here), and everything an error carries besides its kind.
//!
//! A fault is reported for one of a fixed list of components, by its code.
//! There is no free text: the supervisor's detail string is always empty
//! through this library, so nothing a caller writes can reach an event.

use crate::codes::*;
use crate::session::{contain, with_session, LocsimSession};
use crate::status::*;
use locsim_core::domain::Timestamp;
use locsim_health::{Cause, Emission, HealthReport};

/// The state of a session as of the latest time passed to it. 192 bytes,
/// aligned to 8, no padding other than `reserved`.
///
/// A time is meaningful only when its `has_` flag is 1, and is 0 otherwise;
/// `silent_for_ns` is meaningful only when `emission` is
/// `LOCSIM_EMISSION_STALLED`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocsimHealth {
    /// Set by the caller before the call; must be the size of this struct.
    pub struct_size: u32,
    /// What the session is doing: a `LOCSIM_STATE_*` value.
    pub lifecycle: u32,
    /// One word for the whole: a `LOCSIM_HEALTH_*` value.
    pub health: u32,
    /// Whether samples are arriving: a `LOCSIM_EMISSION_*` value.
    pub emission: u32,
    /// `LOCSIM_CAUSE_*` bits. Zero unless `health` is degraded.
    pub causes: u32,
    /// One bit per component with a reported, uncleared fault:
    /// `1 << (component - 1)`.
    pub active_faults: u32,
    /// Restart attempts used in the current failure episode.
    pub attempts_used: u32,
    pub max_attempts: u32,
    /// The status code of the error that ended the latest failed run or
    /// restart attempt, or 0.
    pub last_error: u32,
    /// The state of the current run: a `LOCSIM_STATE_*` value.
    pub current_state: u32,
    pub has_observed_at: u8,
    pub has_retry_at: u8,
    pub has_last_sample_time: u8,
    /// The trajectory has reached its end; the session keeps emitting the
    /// position it holds.
    pub trajectory_complete: u8,
    /// Zero.
    pub reserved: [u8; 4],
    /// The latest time passed to any call.
    pub observed_at_ns: i64,
    /// When the next restart attempt is due.
    pub retry_at_ns: i64,
    pub silent_for_ns: i64,
    pub last_sample_time_ns: i64,
    /// The number of the current run, or of the latest; 0 before any.
    pub run: u64,
    pub current_sample_count: u64,
    pub current_failed_count: u64,
    pub current_missed_ticks: u64,
    pub total_runs_started: u64,
    pub total_runs_ended: u64,
    pub total_samples: u64,
    pub total_withheld: u64,
    pub total_missed_ticks: u64,
    pub total_failures: u64,
    pub total_recovery_attempts: u64,
    pub total_recoveries_completed: u64,
    pub total_terminal_failures: u64,
    pub total_rejected_operations: u64,
}

const HEALTH_SIZE: u32 = 192;
const _: () = assert!(std::mem::size_of::<LocsimHealth>() == HEALTH_SIZE as usize);
const _: () = assert!(std::mem::align_of::<LocsimHealth>() == 8);

/// An optional time as a flag and a number.
fn optional(time: Option<Timestamp>) -> (u8, i64) {
    match time {
        Some(time) => (1, time.as_nanos()),
        None => (0, 0),
    }
}

impl LocsimHealth {
    fn of(report: &HealthReport) -> Self {
        let (emission, silent_for_ns) = match report.emission {
            Emission::NotExpected => (LOCSIM_EMISSION_NOT_EXPECTED, 0),
            Emission::Flowing => (LOCSIM_EMISSION_FLOWING, 0),
            Emission::Stalled { silent_for_ns } => (LOCSIM_EMISSION_STALLED, silent_for_ns),
        };
        let causes = report.causes.iter().fold(0, |bits, cause| {
            bits | match cause {
                Cause::Stalled => LOCSIM_CAUSE_STALLED,
                Cause::FallingBehind => LOCSIM_CAUSE_FALLING_BEHIND,
                Cause::Probation => LOCSIM_CAUSE_PROBATION,
                Cause::ExternalFault(_) => LOCSIM_CAUSE_EXTERNAL_FAULT,
            }
        });
        let active_faults = report
            .active_faults
            .iter()
            .fold(0, |bits, component| bits | component_bit(component));
        let (has_observed_at, observed_at_ns) = optional(report.observed_at);
        let (has_retry_at, retry_at_ns) = optional(report.retry_at);
        let (has_last_sample_time, last_sample_time_ns) = optional(report.current.last_sample_time);
        Self {
            struct_size: HEALTH_SIZE,
            lifecycle: state_code(report.lifecycle),
            health: health_code(report.health),
            emission,
            causes,
            active_faults,
            attempts_used: report.attempts_used,
            max_attempts: report.max_attempts,
            last_error: report.last_error.as_ref().map_or(0, error_code),
            current_state: state_code(report.current.state),
            has_observed_at,
            has_retry_at,
            has_last_sample_time,
            trajectory_complete: u8::from(report.current.trajectory_complete),
            reserved: [0; 4],
            observed_at_ns,
            retry_at_ns,
            silent_for_ns,
            last_sample_time_ns,
            run: report.run,
            current_sample_count: report.current.sample_count,
            current_failed_count: report.current.failed_count,
            current_missed_ticks: report.current.missed_ticks,
            total_runs_started: report.totals.runs_started,
            total_runs_ended: report.totals.runs_ended,
            total_samples: report.totals.samples,
            total_withheld: report.totals.withheld,
            total_missed_ticks: report.totals.missed_ticks,
            total_failures: report.totals.failures,
            total_recovery_attempts: report.totals.recovery_attempts,
            total_recoveries_completed: report.totals.recoveries_completed,
            total_terminal_failures: report.totals.terminal_failures,
            total_rejected_operations: report.totals.rejected_operations,
        }
    }
}

/// The checks every function that fills a health report makes before it
/// looks at the session.
///
/// # Safety
///
/// `out_health`, if not null, is readable for its first four bytes.
unsafe fn refuse_health(out_health: *mut LocsimHealth) -> Option<LocsimStatus> {
    if out_health.is_null() {
        return Some(LOCSIM_ERR_NULL_POINTER);
    }
    // SAFETY: not null, and readable for the struct's first four bytes by
    // the caller's guarantee. No alignment is assumed.
    let struct_size = unsafe { out_health.cast::<u32>().read_unaligned() };
    (struct_size != HEALTH_SIZE).then_some(LOCSIM_ERR_STRUCT_SIZE)
}

/// # Safety
///
/// `out_health` is not null and writable for 192 bytes.
unsafe fn write_health(
    report: Result<LocsimHealth, LocsimStatus>,
    out_health: *mut LocsimHealth,
) -> LocsimStatus {
    match report {
        Err(status) => status,
        Ok(report) => {
            // SAFETY: the caller's guarantee. No alignment is assumed.
            unsafe { out_health.write_unaligned(report) };
            LOCSIM_OK
        }
    }
}

/// Lets the session act on the time without asking for a sample, and
/// reports its state afterwards: a restart attempt that has come due is
/// made, and a running session is checked for a stall.
///
/// Nothing in the library calls this. A session notices a stall only when
/// it is called.
///
/// # Safety
///
/// `out_health` is aligned, readable and writable for 192 bytes and its
/// `struct_size` is set.
#[no_mangle]
pub unsafe extern "C" fn locsim_session_check(
    session: LocsimSession,
    now_ns: i64,
    out_health: *mut LocsimHealth,
) -> LocsimStatus {
    // SAFETY: the caller's guarantee, passed on.
    if let Some(status) = unsafe { refuse_health(out_health) } {
        return status;
    }
    let report = contain(|| {
        with_session(session, |session| {
            let report = session.supervisor.check(Timestamp::from_nanos(now_ns));
            Ok(LocsimHealth::of(&report))
        })
    });
    // SAFETY: not null, checked above; writable by the caller's guarantee.
    unsafe { write_health(report, out_health) }
}

/// Reports the session's state as of the latest time passed to it. Changes
/// nothing.
///
/// # Safety
///
/// As for [`locsim_session_check`].
#[no_mangle]
pub unsafe extern "C" fn locsim_session_health(
    session: LocsimSession,
    out_health: *mut LocsimHealth,
) -> LocsimStatus {
    // SAFETY: the caller's guarantee, passed on.
    if let Some(status) = unsafe { refuse_health(out_health) } {
        return status;
    }
    let report = contain(|| {
        with_session(session, |session| {
            Ok(LocsimHealth::of(&session.supervisor.health()))
        })
    });
    // SAFETY: not null, checked above; writable by the caller's guarantee.
    unsafe { write_health(report, out_health) }
}

/// Notes that a component other than the session is at fault. The fault
/// stands until it is cleared; while a run exists it makes the health
/// degraded, and it does nothing else.
///
/// `component` is a `LOCSIM_COMPONENT_*` value; any other is
/// `LOCSIM_ERR_INVALID_ARGUMENT`.
#[no_mangle]
pub extern "C" fn locsim_session_report_fault(
    session: LocsimSession,
    now_ns: i64,
    component: u32,
) -> LocsimStatus {
    let Some(name) = component_name(component) else {
        return LOCSIM_ERR_INVALID_ARGUMENT;
    };
    let outcome = contain(|| {
        with_session(session, |session| {
            session
                .supervisor
                .report_fault(Timestamp::from_nanos(now_ns), name, String::new());
            Ok(())
        })
    });
    match outcome {
        Ok(()) => LOCSIM_OK,
        Err(status) => status,
    }
}

/// Notes that a reported fault is over. `*out_was_set` is 1 if there was
/// one, and 0 if there was none, in which case nothing changed.
///
/// # Safety
///
/// `out_was_set` is writable.
#[no_mangle]
pub unsafe extern "C" fn locsim_session_clear_fault(
    session: LocsimSession,
    now_ns: i64,
    component: u32,
    out_was_set: *mut u8,
) -> LocsimStatus {
    if out_was_set.is_null() {
        return LOCSIM_ERR_NULL_POINTER;
    }
    let Some(name) = component_name(component) else {
        return LOCSIM_ERR_INVALID_ARGUMENT;
    };
    let cleared = contain(|| {
        with_session(session, |session| {
            Ok(session
                .supervisor
                .clear_fault(Timestamp::from_nanos(now_ns), name))
        })
    });
    match cleared {
        Err(status) => status,
        Ok(was_set) => {
            // SAFETY: not null and writable by the caller's guarantee.
            unsafe { out_was_set.write(u8::from(was_set)) };
            LOCSIM_OK
        }
    }
}
