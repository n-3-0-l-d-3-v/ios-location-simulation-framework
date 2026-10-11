//! C ABI over one supervised simulation session.
//!
//! A session is a `Supervisor<SimulationProvider>` built from a scenario
//! document and a health policy. A caller holds it as an integer handle and
//! never as a pointer: a handle that was destroyed, or never issued, is
//! refused with a status code instead of being undefined behaviour.
//!
//! No Rust type crosses the boundary. Every function returns a
//! [`LocsimStatus`]; data goes out through pointers the caller supplies, and
//! the library keeps none of them after it returns. A panic never unwinds
//! into the caller: it becomes [`LOCSIM_ERR_PANIC`].
//!
//! What this is not: it is not an iOS location provider and it does not make
//! any location appear anywhere. It is the boundary a platform adapter can
//! call. It reads no clock and no file.
//!
//! Present so far (T11A, stage 1): the handle registry, the status codes,
//! [`locsim_abi_version`], [`locsim_session_create`] and
//! [`locsim_session_destroy`]. A session cannot be driven yet.
//!
//! This is the only crate of the workspace that contains `unsafe`. It is
//! confined to reading and writing through the caller's pointers.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

#[cfg(test)]
mod hooks;
mod registry;
mod session;
mod status;

pub use session::{
    locsim_session_create, locsim_session_destroy, LocsimPolicy, LocsimSession,
    LOCSIM_CREATE_ERROR_TEXT_MAX, LOCSIM_EVENT_CAPACITY_MAX, LOCSIM_SCENARIO_MAX_BYTES,
    LOCSIM_SESSION_LIMIT,
};
pub use status::*;

/// The version of this interface. Any change to it, of any kind, is a new
/// number; a caller compares for equality and does not continue otherwise.
pub const LOCSIM_ABI_VERSION: u32 = 1;

/// Returns [`LOCSIM_ABI_VERSION`] of the library that was linked.
#[no_mangle]
pub extern "C" fn locsim_abi_version() -> u32 {
    LOCSIM_ABI_VERSION
}
