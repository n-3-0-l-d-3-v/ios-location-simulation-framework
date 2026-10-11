//! Status codes. The numbers are part of the interface and never change
//! within one [`crate::LOCSIM_ABI_VERSION`].

/// What every function of the interface returns.
pub type LocsimStatus = i32;

pub const LOCSIM_OK: LocsimStatus = 0;

// --- The call itself was not acceptable ---------------------------------

/// A pointer that must not be null was null, or a null pointer came with a
/// length or capacity other than zero.
pub const LOCSIM_ERR_NULL_POINTER: LocsimStatus = 1;
/// The handle was never issued, or its session has been destroyed.
pub const LOCSIM_ERR_INVALID_HANDLE: LocsimStatus = 2;
pub const LOCSIM_ERR_INVALID_UTF8: LocsimStatus = 3;
/// A length, capacity or code outside what the function accepts.
pub const LOCSIM_ERR_INVALID_ARGUMENT: LocsimStatus = 4;
/// A struct's `struct_size` is not the size this version defines.
pub const LOCSIM_ERR_STRUCT_SIZE: LocsimStatus = 5;
/// Another call is using the session. Nothing was done.
pub const LOCSIM_ERR_BUSY: LocsimStatus = 6;
/// As many sessions exist as the library allows.
pub const LOCSIM_ERR_LIMIT: LocsimStatus = 7;

// --- The session could not be built -------------------------------------

/// The scenario document was refused.
pub const LOCSIM_ERR_SCENARIO: LocsimStatus = 20;
/// The health policy was refused.
pub const LOCSIM_ERR_POLICY: LocsimStatus = 21;

// --- The provider's errors, one code per `ProviderError` variant --------

pub const LOCSIM_ERR_PROVIDER_INVALID_SCENARIO: LocsimStatus = 30;
pub const LOCSIM_ERR_PROVIDER_TRANSITION: LocsimStatus = 31;
pub const LOCSIM_ERR_PROVIDER_MOVEMENT: LocsimStatus = 33;
pub const LOCSIM_ERR_PROVIDER_NOISE: LocsimStatus = 34;
pub const LOCSIM_ERR_PROVIDER_DERIVATION: LocsimStatus = 35;
pub const LOCSIM_ERR_PROVIDER_VALIDATION: LocsimStatus = 36;

// `ProviderError::Schedule`, one code per `ScheduleError` variant.
pub const LOCSIM_ERR_SCHEDULE_INVALID_INTERVAL: LocsimStatus = 40;
pub const LOCSIM_ERR_SCHEDULE_OVERFLOW: LocsimStatus = 41;
pub const LOCSIM_ERR_SCHEDULE_ALREADY_PAUSED: LocsimStatus = 42;
pub const LOCSIM_ERR_SCHEDULE_NOT_PAUSED: LocsimStatus = 43;
pub const LOCSIM_ERR_SCHEDULE_CLOCK_WENT_BACKWARDS: LocsimStatus = 44;

// --- The library itself failed ------------------------------------------

/// A panic was caught at the boundary during this call.
pub const LOCSIM_ERR_PANIC: LocsimStatus = 90;
/// An earlier call on this session panicked; only destroying it is left.
pub const LOCSIM_ERR_POISONED: LocsimStatus = 91;
