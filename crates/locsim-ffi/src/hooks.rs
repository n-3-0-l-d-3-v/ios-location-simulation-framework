//! Panic injection for this crate's own tests.
//!
//! The module exists only under `cfg(test)`: it is not behind a feature, it
//! exports no symbol, and no build other than this crate's unit tests
//! contains it or the calls to it.

use std::cell::Cell;

/// A place in the library where a test can make the next pass panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Point {
    /// Inside `locsim_session_create`, before anything is looked at.
    Create,
    /// Inside the destructor of a session, wherever that runs.
    SessionDrop,
}

/// What was true when a session's destructor started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DropConditions {
    /// Another panic was already unwinding on this thread.
    pub(crate) during_a_panic: bool,
    /// Some thread held the registry.
    pub(crate) registry_locked: bool,
}

thread_local! {
    static ARMED: Cell<Option<Point>> = const { Cell::new(None) };
    static LAST_DROP: Cell<Option<DropConditions>> = const { Cell::new(None) };
}

/// Makes the calling thread's next pass through `point` panic, once.
pub(crate) fn arm(point: Point) {
    ARMED.with(|armed| armed.set(Some(point)));
}

pub(crate) fn fire(point: Point) {
    if ARMED.with(|armed| armed.get()) == Some(point) {
        ARMED.with(|armed| armed.set(None));
        panic!("injected by a test at {point:?}");
    }
}

pub(crate) fn note_session_drop(conditions: DropConditions) {
    LAST_DROP.with(|last| last.set(Some(conditions)));
}

/// The conditions of the latest session destructor run on the calling
/// thread since this was last asked, if any ran.
pub(crate) fn take_session_drop() -> Option<DropConditions> {
    LAST_DROP.with(Cell::take)
}
