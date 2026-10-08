use crate::domain::Timestamp;
use std::cell::Cell;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Source of "now". Everything time-dependent takes a clock or an explicit
/// timestamp, so tests never sleep and never depend on the host's time.
pub trait Clock {
    fn now(&self) -> Timestamp;
}

/// Wall-clock epoch captured once, then advanced by a monotonic timer.
/// The result never goes backwards even if the system time is adjusted.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    epoch_nanos_at_start: i64,
    started: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        // A system clock set before 1970 is treated as the epoch itself; the
        // monotonic part still advances correctly from there.
        let epoch_nanos_at_start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Self {
            epoch_nanos_at_start,
            started: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let elapsed = i64::try_from(self.started.elapsed().as_nanos()).unwrap_or(i64::MAX);
        Timestamp::from_nanos(self.epoch_nanos_at_start.saturating_add(elapsed))
    }
}

/// Clock that only moves when told to. For tests and deterministic replays.
#[derive(Debug)]
pub struct ManualClock {
    now: Cell<i64>,
}

impl ManualClock {
    pub fn new(start: Timestamp) -> Self {
        Self {
            now: Cell::new(start.as_nanos()),
        }
    }

    pub fn set(&self, t: Timestamp) {
        self.now.set(t.as_nanos());
    }

    pub fn advance_nanos(&self, nanos: i64) {
        self.now.set(self.now.get().saturating_add(nanos));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_nanos(self.now.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_on_request() {
        let c = ManualClock::new(Timestamp::from_nanos(100));
        assert_eq!(c.now().as_nanos(), 100);
        assert_eq!(c.now().as_nanos(), 100);
        c.advance_nanos(50);
        assert_eq!(c.now().as_nanos(), 150);
        c.set(Timestamp::from_nanos(7));
        assert_eq!(c.now().as_nanos(), 7);
        c.advance_nanos(i64::MAX);
        assert_eq!(c.now().as_nanos(), i64::MAX);
    }

    #[test]
    fn system_clock_is_monotonic_and_plausible() {
        let c = SystemClock::new();
        let a = c.now();
        let b = c.now();
        assert!(b >= a);
        // After 2020-01-01; guards against unit mistakes (ms vs ns).
        assert!(a.as_secs_f64() > 1_577_836_800.0);
    }
}
