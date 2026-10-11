//! Creating and destroying a session.
//!
//! # What a caller must guarantee
//!
//! The library checks what can be checked: null pointers, lengths, sizes,
//! codes, UTF-8. It cannot tell whether a pointer that is not null points at
//! memory that exists, is large enough, or is still alive. That is the
//! caller's responsibility, and getting it wrong is undefined behaviour.
//!
//! - Every region passed to a call is valid for the whole call and is not
//!   written by another thread during it. No pointer is kept afterwards.
//! - Regions written by a call overlap neither each other nor a region the
//!   same call reads.
//! - Lengths and capacities are in bytes. No terminator is read, written or
//!   counted.
//!
//! # Order of checks
//!
//! Always the same, so that a call with several faults has one answer:
//! null pointers, then `struct_size`, then the other arguments in the order
//! of the parameter list, then the content (UTF-8, the scenario, the
//! policy), then the session limit.

use crate::registry::{self, CloseError, Registry};
use crate::status::*;
use locsim_core::provider::SimulationProvider;
use locsim_health::{HealthPolicy, NullSink, Supervisor};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// A session handle. Zero is never a valid one.
pub type LocsimSession = u64;

/// The longest scenario document accepted, in bytes.
pub const LOCSIM_SCENARIO_MAX_BYTES: usize = 16 * 1024 * 1024;
/// The longest text `locsim_session_create` reports, in bytes.
pub const LOCSIM_CREATE_ERROR_TEXT_MAX: usize = 4096;
/// The largest event queue a session may be given.
pub const LOCSIM_EVENT_CAPACITY_MAX: u32 = 4096;
/// The most sessions that may exist at once.
pub const LOCSIM_SESSION_LIMIT: usize = registry::LIMIT;

/// `HealthPolicy` as C passes it. 64 bytes, aligned to 8, no padding.
/// Every bit pattern is a value of this type, so reading one from the
/// caller's memory cannot itself go wrong; whether the values make a valid
/// policy is decided by `HealthPolicy`'s own validation.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LocsimPolicy {
    /// Must be the size of this struct.
    pub struct_size: u32,
    pub max_recovery_attempts: u32,
    pub backoff_initial_s: f64,
    pub backoff_multiplier: f64,
    pub backoff_max_s: f64,
    pub stable_after_s: f64,
    pub stall_after_s: f64,
    pub missed_window_ticks: u64,
    pub max_missed_in_window: u64,
}

const _: () = assert!(std::mem::size_of::<LocsimPolicy>() == 64);
const _: () = assert!(std::mem::align_of::<LocsimPolicy>() == 8);

impl LocsimPolicy {
    fn to_health(self) -> HealthPolicy {
        HealthPolicy {
            max_recovery_attempts: self.max_recovery_attempts,
            backoff_initial_s: self.backoff_initial_s,
            backoff_multiplier: self.backoff_multiplier,
            backoff_max_s: self.backoff_max_s,
            stable_after_s: self.stable_after_s,
            stall_after_s: self.stall_after_s,
            missed_window_ticks: self.missed_window_ticks,
            max_missed_in_window: self.max_missed_in_window,
        }
    }
}

pub(crate) struct Session {
    // Nothing drives a session yet; the operations of the next stages do.
    #[allow(dead_code)]
    supervisor: Supervisor<SimulationProvider>,
}

/// What a handle's cell holds.
pub(crate) enum State {
    // Not read yet, for the same reason as `Session::supervisor`.
    #[allow(dead_code)]
    Open(Box<Session>),
    /// The session was destroyed. A call that reached the cell before that
    /// and locks it afterwards finds this and does nothing.
    Closed,
}

// In this crate's tests a session's destructor reports the conditions it
// runs under and can be made to panic: the only way to exercise a session
// whose release fails.
#[cfg(test)]
impl Drop for Session {
    fn drop(&mut self) {
        use std::sync::TryLockError;
        crate::hooks::note_session_drop(crate::hooks::DropConditions {
            during_a_panic: std::thread::panicking(),
            registry_locked: matches!(REGISTRY.try_lock(), Err(TryLockError::WouldBlock)),
        });
        crate::hooks::fire(crate::hooks::Point::SessionDrop);
    }
}

// A session is handed between the threads that call the library.
const _: fn() = || {
    fn sendable<T: Send>() {}
    sendable::<Supervisor<SimulationProvider>>();
    sendable::<State>();
};

static REGISTRY: Mutex<Registry<State>> = Mutex::new(Registry::new());

/// The registry, whether or not a thread once panicked while holding it:
/// every change to it leaves it valid at each step (see `registry.rs`).
fn registry() -> MutexGuard<'static, Registry<State>> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why a session was not created.
struct Refusal {
    status: LocsimStatus,
    /// Present for a refused scenario or policy.
    text: Option<String>,
}

impl From<LocsimStatus> for Refusal {
    fn from(status: LocsimStatus) -> Self {
        Self { status, text: None }
    }
}

/// One line per problem, at most [`LOCSIM_CREATE_ERROR_TEXT_MAX`] bytes.
fn explain<E: ToString>(status: LocsimStatus, errors: &[E]) -> Refusal {
    let lines: Vec<String> = errors.iter().map(E::to_string).collect();
    Refusal {
        status,
        text: Some(truncated(lines.join("\n"), LOCSIM_CREATE_ERROR_TEXT_MAX)),
    }
}

/// `text` cut to at most `max` bytes without splitting a character.
fn truncated(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Everything `locsim_session_create` does once its pointers are known not
/// to be null.
///
/// # Safety
///
/// `policy` is readable for 64 bytes, and `scenario_json` for
/// `scenario_len` bytes unless that is zero.
unsafe fn create(
    scenario_json: *const u8,
    scenario_len: usize,
    policy: *const LocsimPolicy,
    event_capacity: u32,
) -> Result<LocsimSession, Refusal> {
    #[cfg(test)]
    crate::hooks::fire(crate::hooks::Point::Create);

    // SAFETY: `policy` is not null and the caller guarantees 64 readable
    // bytes; the first four are `struct_size`. No alignment is assumed.
    let struct_size = unsafe { policy.cast::<u32>().read_unaligned() };
    if usize::try_from(struct_size).ok() != Some(std::mem::size_of::<LocsimPolicy>()) {
        return Err(LOCSIM_ERR_STRUCT_SIZE.into());
    }
    if scenario_len > LOCSIM_SCENARIO_MAX_BYTES {
        return Err(LOCSIM_ERR_INVALID_ARGUMENT.into());
    }
    if !(1..=LOCSIM_EVENT_CAPACITY_MAX).contains(&event_capacity) {
        return Err(LOCSIM_ERR_INVALID_ARGUMENT.into());
    }

    let bytes: &[u8] = if scenario_len == 0 {
        // The pointer may be null here and is not looked at.
        &[]
    } else {
        // SAFETY: not null (checked by the caller of this function, since
        // the length is not zero), readable for `scenario_len` bytes by the
        // caller's guarantee, not written during the call, and at most
        // 16 MiB, which is below `isize::MAX`.
        unsafe { std::slice::from_raw_parts(scenario_json, scenario_len) }
    };
    let text = std::str::from_utf8(bytes).map_err(|_| LOCSIM_ERR_INVALID_UTF8)?;
    let scenario = locsim_scenario::import_scenario(text)
        .map_err(|errors| explain(LOCSIM_ERR_SCENARIO, &errors))?;

    // SAFETY: as for `struct_size`; every bit pattern is a `LocsimPolicy`.
    let policy = unsafe { policy.read_unaligned() }.to_health();
    // No event leaves a session yet: the queue comes with the operations
    // that can produce one. `event_capacity` is checked and not yet used.
    let supervisor = Supervisor::for_simulation(
        SimulationProvider::new(scenario),
        policy,
        Box::new(NullSink),
    )
    .map_err(|errors| explain(LOCSIM_ERR_POLICY, &errors))?;

    // Built before the registry is locked, and, if refused, dropped after
    // it is released: nothing but table work happens under that lock.
    let cell = Arc::new(Mutex::new(State::Open(Box::new(Session { supervisor }))));
    let issued = registry().insert(cell);
    issued.map_err(|_refused| LOCSIM_ERR_LIMIT.into())
}

/// Builds a session from a scenario document and a health policy. The
/// session is stopped.
///
/// `event_capacity` is the length of the session's event queue, from 1 to
/// [`LOCSIM_EVENT_CAPACITY_MAX`].
///
/// # Outputs
///
/// | Status | `*out_session` | `*out_err_len` | `err_buf` |
/// |---|---|---|---|
/// | `LOCSIM_OK` | the handle | 0 | untouched |
/// | `LOCSIM_ERR_NULL_POINTER` | not written | not written | untouched |
/// | `LOCSIM_ERR_SCENARIO`, `LOCSIM_ERR_POLICY` | 0 | length of the text | the whole text if `err_cap` is at least its length, otherwise untouched |
/// | any other | 0 | 0 | untouched |
///
/// The text says what was wrong, one line per problem, and is at most
/// [`LOCSIM_CREATE_ERROR_TEXT_MAX`] bytes of UTF-8, cut at a character if
/// longer. It can repeat values of the document. A call that fails creates
/// nothing, so it can be repeated with a larger buffer.
///
/// # Safety
///
/// See the module documentation. In particular:
///
/// - `scenario_json` is readable for `scenario_len` bytes; it may be null
///   only if `scenario_len` is 0.
/// - `policy` is readable for 64 bytes.
/// - `out_session` and `out_err_len` are writable.
/// - `err_buf` is writable for `err_cap` bytes; it may be null only if
///   `err_cap` is 0.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn locsim_session_create(
    scenario_json: *const u8,
    scenario_len: usize,
    policy: *const LocsimPolicy,
    event_capacity: u32,
    out_session: *mut LocsimSession,
    err_buf: *mut u8,
    err_cap: usize,
    out_err_len: *mut usize,
) -> LocsimStatus {
    // Comparisons only: nothing here can panic, and nothing is written.
    if (scenario_json.is_null() && scenario_len > 0)
        || policy.is_null()
        || out_session.is_null()
        || (err_buf.is_null() && err_cap > 0)
        || out_err_len.is_null()
    {
        return LOCSIM_ERR_NULL_POINTER;
    }

    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the pointers were checked for null above; the rest is the
        // caller's guarantee, passed on unchanged.
        unsafe { create(scenario_json, scenario_len, policy, event_capacity) }
    }));
    let (status, session, text) = match outcome {
        Ok(Ok(session)) => (LOCSIM_OK, session, None),
        Ok(Err(refusal)) => (refusal.status, 0, refusal.text),
        Err(payload) => {
            // Dropping a panic payload can itself panic.
            std::mem::forget(payload);
            (LOCSIM_ERR_PANIC, 0, None)
        }
    };
    let text = text.as_deref().unwrap_or("");

    // SAFETY: both pointers are not null and writable by the caller's
    // guarantee. No alignment is assumed.
    unsafe {
        out_session.write_unaligned(session);
        out_err_len.write_unaligned(text.len());
    }
    if !text.is_empty() && err_cap >= text.len() {
        // SAFETY: `err_cap` is not zero here, so `err_buf` is not null, and
        // it is writable for `err_cap >= text.len()` bytes by the caller's
        // guarantee. It does not overlap `text`, which the library owns.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), err_buf, text.len()) };
    }
    status
}

/// Ends a session and invalidates its handle.
///
/// - `LOCSIM_OK`: the session is gone. Every later use of the handle,
///   including a second destroy, returns `LOCSIM_ERR_INVALID_HANDLE`.
/// - `LOCSIM_ERR_INVALID_HANDLE`: never issued, or already destroyed.
/// - `LOCSIM_ERR_BUSY`: another call is using the session. Nothing was
///   done and the handle is still valid; destroy it again later.
/// - `LOCSIM_ERR_PANIC`: the handle was invalidated, and releasing the
///   session then panicked. Destroying it again is harmless and returns
///   `LOCSIM_ERR_INVALID_HANDLE`.
///
/// What a panic while releasing can and cannot mean. The session is
/// released on its own, after every lock has been let go and with no other
/// panic in flight, so one panicking destructor is caught and reported.
/// The language then releases the rest of the session while that panic
/// unwinds; a second panic there ends the process, as does any panic in a
/// build that aborts instead of unwinding. Neither can be caught here.
#[no_mangle]
pub extern "C" fn locsim_session_destroy(session: LocsimSession) -> LocsimStatus {
    // First the table, and nothing else: the handle is invalidated and both
    // locks are released before the session itself is touched.
    let closed = catch_unwind(AssertUnwindSafe(|| {
        registry().close(session, State::Closed)
    }));
    let state = match closed {
        Ok(Ok(state)) => state,
        Ok(Err(CloseError::Invalid)) => return LOCSIM_ERR_INVALID_HANDLE,
        Ok(Err(CloseError::Busy)) => return LOCSIM_ERR_BUSY,
        Err(payload) => {
            // Not reachable by any path known: closing does table work only.
            std::mem::forget(payload);
            return LOCSIM_ERR_PANIC;
        }
    };
    // Then the session's own destructors, caught separately.
    match catch_unwind(AssertUnwindSafe(move || drop(state))) {
        Ok(()) => LOCSIM_OK,
        Err(payload) => {
            std::mem::forget(payload);
            LOCSIM_ERR_PANIC
        }
    }
}

#[cfg(test)]
mod tests;
