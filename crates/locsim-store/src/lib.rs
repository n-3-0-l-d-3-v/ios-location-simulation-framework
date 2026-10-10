//! File storage for the simulation framework: a scenario kept in a
//! directory so that it survives the process, written so that a failed or
//! interrupted save does not destroy what was there.
//!
//! This crate is the only one that touches files. It has no third-party
//! dependency: text formats come from `locsim-scenario`, everything else is
//! the standard library and code here. It contains nothing specific to a
//! platform's location services.
//!
//! # What a stored file is
//!
//! An envelope around a payload (module `envelope`): a four-line header
//! naming the format, the SHA-256 of the payload and its length, then the
//! payload byte for byte. The digest is checked before the payload is
//! interpreted, because a document codec alone cannot tell a damaged digit
//! from the original. See [`EnvelopeError`] for what is recognised.
//!
//! # Replacing a record
//!
//! Temporary file in the same directory, write, sync, read back and
//! compare, rename over the record, sync the directory where possible. The
//! record is not touched before the rename.
//!
//! | | Unix-like | Windows |
//! |---|---|---|
//! | A failed save leaves the old record | yes | yes |
//! | A reader never finds a half-written record | yes | yes |
//! | A reader always finds *a* record during a replacement | yes (POSIX `rename`) | not documented by Microsoft; a read may fail and be retried |
//! | Content flushed before it becomes visible | requested | requested |
//! | The change of name flushed before `save` returns | requested (directory sync) | **not requested**; see [`DIRECTORY_SYNC`] |
//!
//! "Requested" means the operating system was asked. No behaviour under
//! power loss has been tested, on any platform.
//!
//! # What this crate does not do
//!
//! - It does not checkpoint a running simulation. Nothing stored here can
//!   resume a run.
//! - It keeps no backup generation: a successful save replaces the record.
//! - It does not lock the directory. One writer is a precondition.
//! - The digest detects accidental damage. It does not authenticate.

#![forbid(unsafe_code)]

mod atomic;
mod envelope;
mod error;
mod sha256;
mod store;

pub use atomic::DIRECTORY_SYNC;
pub use envelope::{EnvelopeError, ENVELOPE_VERSION};
pub use error::{Corruption, Operation, Record, StoreError};
pub use store::Store;
