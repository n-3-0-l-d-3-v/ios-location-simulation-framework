//! Crash-safe file storage for the simulation framework.

#![forbid(unsafe_code)]
// The store that uses these layers lands in the next commit.
#![allow(dead_code)]

mod atomic;
mod envelope;
mod error;
mod sha256;

pub use atomic::DIRECTORY_SYNC;
pub use envelope::{EnvelopeError, ENVELOPE_VERSION};
pub use error::{Corruption, Operation, Record, StoreError};
