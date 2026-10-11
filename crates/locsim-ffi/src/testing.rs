//! What the tests of more than one module share. Test builds only.

use crate::hooks::DropConditions;
use crate::session::{locsim_session_create, locsim_session_destroy, LocsimPolicy};
use crate::status::LOCSIM_OK;
use std::cell::RefCell;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub(crate) const FIXED: &str = include_str!("../../../Examples/Scenarios/fixed.json");
pub(crate) const EXAMPLES: [&str; 8] = [
    include_str!("../../../Examples/Scenarios/antimeridian_loop.json"),
    include_str!("../../../Examples/Scenarios/circular.json"),
    include_str!("../../../Examples/Scenarios/driving.json"),
    FIXED,
    include_str!("../../../Examples/Scenarios/high_latitude.json"),
    include_str!("../../../Examples/Scenarios/random_walk.json"),
    include_str!("../../../Examples/Scenarios/route_replay.json"),
    include_str!("../../../Examples/Scenarios/walking.json"),
];

// The registry is one per process and the tests run side by side. A test
// that needs to know exactly what the registry holds, or that nobody else
// is using it, takes `alone`; every other test that creates a session
// takes `shared`.
//
// Both guards destroy, when they are dropped, every session the test's
// thread created and did not destroy. A test that fails half way would
// otherwise leave its sessions registered, and the tests that count
// sessions or fill the registry would then fail for no fault of their own.
static GLOBAL: RwLock<()> = RwLock::new(());

thread_local! {
    static MADE: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// Notes a session the calling thread has just been given.
pub(crate) fn made(session: u64) {
    MADE.with(|made| made.borrow_mut().push(session));
}

/// Destroys what the calling thread created. A handle already destroyed is
/// refused and nothing happens: a handle is never issued twice.
fn sweep() {
    for session in MADE.with(RefCell::take) {
        locsim_session_destroy(session);
    }
}

pub(crate) struct Shared(#[allow(dead_code)] RwLockReadGuard<'static, ()>);
pub(crate) struct Alone(#[allow(dead_code)] RwLockWriteGuard<'static, ()>);

// The sweep runs before the lock inside the guard is let go.
impl Drop for Shared {
    fn drop(&mut self) {
        sweep();
    }
}

impl Drop for Alone {
    fn drop(&mut self) {
        sweep();
    }
}

pub(crate) fn shared() -> Shared {
    Shared(GLOBAL.read().unwrap_or_else(PoisonError::into_inner))
}

pub(crate) fn alone() -> Alone {
    Alone(GLOBAL.write().unwrap_or_else(PoisonError::into_inner))
}

pub(crate) fn policy() -> LocsimPolicy {
    LocsimPolicy {
        struct_size: 64,
        max_recovery_attempts: 3,
        backoff_initial_s: 1.0,
        backoff_multiplier: 2.0,
        backoff_max_s: 60.0,
        stable_after_s: 10.0,
        stall_after_s: 5.0,
        missed_window_ticks: 10,
        max_missed_in_window: 2,
    }
}

/// A session for `json`, which must be acceptable.
pub(crate) fn open_with(json: &str) -> u64 {
    let policy = policy();
    let mut session = 0;
    let mut err_len = 0;
    // SAFETY: every pointer is to a live local or to `json`, with its length.
    let status = unsafe {
        locsim_session_create(
            json.as_ptr(),
            json.len(),
            &policy,
            16,
            &mut session,
            std::ptr::null_mut(),
            0,
            &mut err_len,
        )
    };
    assert_eq!(status, LOCSIM_OK);
    session
}

pub(crate) fn open() -> u64 {
    open_with(FIXED)
}

/// What must hold whenever a session is released: no lock is held and no
/// other panic is unwinding.
pub(crate) const RELEASED_ALONE: DropConditions = DropConditions {
    during_a_panic: false,
    registry_locked: false,
};
