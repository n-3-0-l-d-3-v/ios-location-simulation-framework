//! Panic injection and stopping points for this crate's own tests.
//!
//! The module exists only under `cfg(test)`: it is not behind a feature, it
//! exports no symbol, and no build other than this crate's unit tests
//! contains it or the calls to it.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::{self, Receiver, Sender};

/// A place in the library where a test can make the next pass panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Point {
    /// Inside `locsim_session_create`, before anything is looked at.
    Create,
    /// Inside the destructor of a session, wherever that runs.
    SessionDrop,
    /// At the start of an operation on a session: its lock is held, and
    /// the panic is caught around the operation.
    Operation,
    /// With a session's lock held but outside the operation, where only
    /// the catch around the whole call is left.
    HoldingLock,
}

/// A place where a test can hold a call still while it does something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The session's cell has been found; its lock has not been tried.
    BeforeLock,
    /// The session's lock is held; the operation has not begun.
    HoldingLock,
}

/// The test's end of a stopping point.
pub(crate) struct Gate {
    reached: Receiver<()>,
    go: Sender<()>,
}

impl Gate {
    /// Returns once the other thread stands at the stopping point.
    pub(crate) fn wait_until_reached(&self) {
        self.reached.recv().unwrap();
    }

    pub(crate) fn open(self) {
        self.go.send(()).unwrap();
    }
}

/// The end that goes to the thread to be stopped.
pub(crate) struct Latch {
    reached: Sender<()>,
    go: Receiver<()>,
}

impl Latch {
    /// Makes the calling thread stop the next time it passes `point`.
    pub(crate) fn stop_at(self, point: Stop) {
        STOP.with(|stop| *stop.borrow_mut() = Some((point, self)));
    }
}

pub(crate) fn gate() -> (Gate, Latch) {
    let (reached, is_reached) = mpsc::channel();
    let (go, may_go) = mpsc::channel();
    (
        Gate {
            reached: is_reached,
            go,
        },
        Latch {
            reached,
            go: may_go,
        },
    )
}

pub(crate) fn reach(point: Stop) {
    let latch = STOP.with(|stop| {
        let mut stop = stop.borrow_mut();
        match *stop {
            Some((at, _)) if at == point => stop.take().map(|(_, latch)| latch),
            _ => None,
        }
    });
    if let Some(latch) = latch {
        // Neither can fail while the test holds its end; if it has gone,
        // there is nobody to wait for.
        let _ = latch.reached.send(());
        let _ = latch.go.recv();
    }
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
    static STOP: RefCell<Option<(Stop, Latch)>> = const { RefCell::new(None) };
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
