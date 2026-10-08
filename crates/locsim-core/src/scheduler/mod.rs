//! Sampling scheduler: clocks and (next) a drift-free tick schedule.

mod clock;

pub use clock::{Clock, ManualClock, SystemClock};
