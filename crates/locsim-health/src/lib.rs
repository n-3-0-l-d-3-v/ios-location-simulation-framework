//! Health supervision of a location provider.

#![forbid(unsafe_code)]
// The supervisor that uses the policy lands in the next commit.
#![allow(dead_code)]

mod event;
mod policy;

pub use event::{
    Event, EventKind, EventSink, FailReason, Level, MemorySink, NullSink, Operation, RunEndReason,
};
pub use policy::{HealthPolicy, PolicyError};
