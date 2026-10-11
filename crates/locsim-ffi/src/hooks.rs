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
    /// Inside `locsim_session_destroy`, after the handle has been
    /// invalidated and before the session is dropped.
    DestroyDrop,
}

thread_local! {
    static ARMED: Cell<Option<Point>> = const { Cell::new(None) };
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
