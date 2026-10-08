//! Sampling scheduler: clocks and a drift-free fixed-rate tick schedule.

mod clock;
mod schedule;

pub use clock::{Clock, ManualClock, SystemClock};
pub use schedule::{Poll, ScheduleError, Tick, TickSchedule};
