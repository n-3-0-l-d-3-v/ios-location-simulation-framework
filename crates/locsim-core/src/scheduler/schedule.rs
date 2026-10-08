use crate::domain::Timestamp;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleError {
    /// Interval is non-finite, not positive, or rounds to zero nanoseconds.
    InvalidInterval,
    /// A tick time is not representable.
    Overflow,
    AlreadyPaused,
    NotPaused,
    /// `resume` was given a time earlier than the matching `pause`.
    ClockWentBackwards,
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ScheduleError::InvalidInterval => "update interval must be finite and at least 1 ns",
            ScheduleError::Overflow => "tick time is out of the representable range",
            ScheduleError::AlreadyPaused => "schedule is already paused",
            ScheduleError::NotPaused => "schedule is not paused",
            ScheduleError::ClockWentBackwards => "resume time precedes pause time",
        })
    }
}

impl std::error::Error for ScheduleError {}

/// One due tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// Slot number: this tick's ideal time is `start + index × interval`.
    pub index: u64,
    /// The ideal time. Samples are stamped with this, not with "now", so
    /// output does not depend on how punctually the schedule was polled.
    pub target: Timestamp,
    /// How late the poll was relative to `target` (≥ 0).
    pub lateness_nanos: i64,
    /// Slots skipped immediately before this one because polling fell behind.
    pub missed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    Due(Tick),
    /// Nothing due; the next tick is this many nanoseconds away.
    Wait {
        nanos: i64,
    },
    Paused,
}

/// Fixed-rate tick schedule without cumulative drift.
///
/// Tick `n` is due at `start + n × interval`, computed from the origin each
/// time rather than by adding the interval to the previous wake-up. If the
/// caller falls behind by more than one interval the overdue slots are
/// skipped and counted (never replayed in a burst), which is the
/// backpressure policy: a slow consumer gets fewer samples, not a backlog.
///
/// The schedule is a pure state machine: it never sleeps or reads a clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickSchedule {
    start: Timestamp,
    interval_nanos: i64,
    next_index: u64,
    paused_at: Option<Timestamp>,
    missed_total: u64,
}

impl TickSchedule {
    /// Tick 0 is due at `start`.
    pub fn new(start: Timestamp, interval_s: f64) -> Result<Self, ScheduleError> {
        let nanos = (interval_s * 1e9).round();
        if !nanos.is_finite() || nanos < 1.0 || nanos >= i64::MAX as f64 {
            return Err(ScheduleError::InvalidInterval);
        }
        Ok(Self {
            start,
            interval_nanos: nanos as i64,
            next_index: 0,
            paused_at: None,
            missed_total: 0,
        })
    }

    pub fn interval_nanos(&self) -> i64 {
        self.interval_nanos
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    /// Total slots skipped since construction.
    pub fn missed_total(&self) -> u64 {
        self.missed_total
    }

    /// Ideal time of slot `index`.
    pub fn target_time(&self, index: u64) -> Result<Timestamp, ScheduleError> {
        let t = self.start.as_nanos() as i128 + index as i128 * self.interval_nanos as i128;
        i64::try_from(t)
            .map(Timestamp::from_nanos)
            .map_err(|_| ScheduleError::Overflow)
    }

    /// Ideal time of the next slot that has not fired yet.
    pub fn next_target(&self) -> Result<Timestamp, ScheduleError> {
        self.target_time(self.next_index)
    }

    /// Reports whether a tick is due at `now`. At most one tick is returned
    /// per call; it is always the most recent due slot.
    pub fn poll(&mut self, now: Timestamp) -> Result<Poll, ScheduleError> {
        if self.paused_at.is_some() {
            return Ok(Poll::Paused);
        }
        let next = self.next_target()?;
        if now < next {
            // Fits: both are valid i64 timestamps and next > now.
            let nanos = (next.as_nanos() as i128 - now.as_nanos() as i128).min(i64::MAX as i128);
            return Ok(Poll::Wait {
                nanos: nanos as i64,
            });
        }
        let elapsed = now.as_nanos() as i128 - self.start.as_nanos() as i128;
        let index = u64::try_from(elapsed / self.interval_nanos as i128)
            .map_err(|_| ScheduleError::Overflow)?;
        let target = self.target_time(index)?;
        let missed = index - self.next_index;
        self.missed_total = self.missed_total.saturating_add(missed);
        self.next_index = index.checked_add(1).ok_or(ScheduleError::Overflow)?;
        Ok(Poll::Due(Tick {
            index,
            target,
            lateness_nanos: now.as_nanos() - target.as_nanos(),
            missed,
        }))
    }

    pub fn pause(&mut self, now: Timestamp) -> Result<(), ScheduleError> {
        if self.paused_at.is_some() {
            return Err(ScheduleError::AlreadyPaused);
        }
        self.paused_at = Some(now);
        Ok(())
    }

    /// Resumes by shifting the origin forward by the paused duration, so the
    /// time remaining until the next tick is the same as when pausing and
    /// paused time is neither replayed nor counted as missed.
    pub fn resume(&mut self, now: Timestamp) -> Result<(), ScheduleError> {
        let paused_at = self.paused_at.ok_or(ScheduleError::NotPaused)?;
        if now < paused_at {
            return Err(ScheduleError::ClockWentBackwards);
        }
        let shifted =
            self.start.as_nanos() as i128 + now.as_nanos() as i128 - paused_at.as_nanos() as i128;
        self.start = i64::try_from(shifted)
            .map(Timestamp::from_nanos)
            .map_err(|_| ScheduleError::Overflow)?;
        self.paused_at = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: i64 = 1_000_000_000;

    fn t(nanos: i64) -> Timestamp {
        Timestamp::from_nanos(nanos)
    }

    fn due(s: &mut TickSchedule, now: i64) -> Tick {
        match s.poll(t(now)).unwrap() {
            Poll::Due(tick) => tick,
            other => panic!("expected a due tick at {now}, got {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_intervals() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-10, 1e300] {
            assert_eq!(
                TickSchedule::new(t(0), bad),
                Err(ScheduleError::InvalidInterval),
                "{bad}"
            );
        }
        assert_eq!(
            TickSchedule::new(t(0), 0.25).unwrap().interval_nanos(),
            SEC / 4
        );
    }

    #[test]
    fn first_tick_is_due_at_start_and_then_waits() {
        let mut s = TickSchedule::new(t(10 * SEC), 1.0).unwrap();
        assert_eq!(s.poll(t(9 * SEC)).unwrap(), Poll::Wait { nanos: SEC });
        let tick = due(&mut s, 10 * SEC);
        assert_eq!(
            (tick.index, tick.target, tick.lateness_nanos, tick.missed),
            (0, t(10 * SEC), 0, 0)
        );
        assert_eq!(
            s.poll(t(10 * SEC + 1)).unwrap(),
            Poll::Wait { nanos: SEC - 1 }
        );
        assert_eq!(s.next_target().unwrap(), t(11 * SEC));
    }

    #[test]
    fn late_polls_do_not_accumulate_drift() {
        // Every poll is 30 ms late, yet targets stay on the exact grid.
        let mut s = TickSchedule::new(t(0), 1.0).unwrap();
        for n in 0..10_000i64 {
            let tick = due(&mut s, n * SEC + 30_000_000);
            assert_eq!(tick.index, n as u64);
            assert_eq!(tick.target, t(n * SEC));
            assert_eq!(tick.lateness_nanos, 30_000_000);
            assert_eq!(tick.missed, 0);
        }
        assert_eq!(s.missed_total(), 0);
    }

    #[test]
    fn overdue_slots_are_skipped_and_counted_not_replayed() {
        let mut s = TickSchedule::new(t(0), 1.0).unwrap();
        due(&mut s, 0);
        // Stall for 5.5 s: slots 1–4 are lost, slot 5 fires once.
        let tick = due(&mut s, 5 * SEC + SEC / 2);
        assert_eq!((tick.index, tick.target, tick.missed), (5, t(5 * SEC), 4));
        assert_eq!(tick.lateness_nanos, SEC / 2);
        assert_eq!(s.missed_total(), 4);
        assert_eq!(
            s.poll(t(5 * SEC + SEC / 2)).unwrap(),
            Poll::Wait { nanos: SEC / 2 }
        );
        assert_eq!(due(&mut s, 6 * SEC).index, 6);
    }

    #[test]
    fn pause_preserves_phase_and_counts_nothing_as_missed() {
        let mut s = TickSchedule::new(t(0), 1.0).unwrap();
        due(&mut s, 0);
        // Pause 0.4 s into the interval, for an hour.
        s.pause(t(400_000_000)).unwrap();
        assert!(s.is_paused());
        assert_eq!(s.poll(t(100 * SEC)).unwrap(), Poll::Paused);
        let resumed = 3600 * SEC + 400_000_000;
        s.resume(t(resumed)).unwrap();
        // 0.6 s of the interval remained.
        assert_eq!(
            s.poll(t(resumed)).unwrap(),
            Poll::Wait { nanos: 600_000_000 }
        );
        let tick = due(&mut s, resumed + 600_000_000);
        assert_eq!((tick.index, tick.missed, tick.lateness_nanos), (1, 0, 0));
        assert_eq!(tick.target, t(resumed + 600_000_000));
        assert_eq!(s.missed_total(), 0);
    }

    #[test]
    fn pause_resume_misuse_is_rejected() {
        let mut s = TickSchedule::new(t(0), 1.0).unwrap();
        assert_eq!(s.resume(t(1)), Err(ScheduleError::NotPaused));
        s.pause(t(10)).unwrap();
        assert_eq!(s.pause(t(11)), Err(ScheduleError::AlreadyPaused));
        assert_eq!(s.resume(t(9)), Err(ScheduleError::ClockWentBackwards));
        assert!(s.is_paused());
        s.resume(t(10)).unwrap();
    }

    #[test]
    fn overflow_is_reported() {
        let s = TickSchedule::new(t(i64::MAX - 10), 1.0).unwrap();
        assert_eq!(s.target_time(0), Ok(t(i64::MAX - 10)));
        assert_eq!(s.target_time(1), Err(ScheduleError::Overflow));
        let mut s = TickSchedule::new(t(i64::MAX - 10), 1.0).unwrap();
        due(&mut s, i64::MAX - 10);
        assert_eq!(s.poll(t(i64::MAX)), Err(ScheduleError::Overflow));
    }
}
