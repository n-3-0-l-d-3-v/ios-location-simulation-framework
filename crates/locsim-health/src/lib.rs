//! Health supervision of a location provider.
//!
//! [`Supervisor`] wraps any `LocationProvider` and is one itself. It hands
//! samples on unchanged, notices when the wrapped provider fails, and
//! reports what it sees as a [`HealthReport`] and as [`Event`]s.
//!
//! It never reads a clock and never sleeps: like the core, everything is
//! driven by the time passed to each call, so its behaviour is a function
//! of its policy, the wrapped provider and the sequence of calls.

#![forbid(unsafe_code)]

mod event;
mod policy;
mod report;
mod supervisor;

pub use event::{
    Event, EventKind, EventSink, FailReason, Level, MemorySink, NullSink, Operation, RunEndReason,
};
pub use policy::{HealthPolicy, PolicyError};
pub use report::{Cause, Emission, HealthReport, Totals};
pub use supervisor::Supervisor;
