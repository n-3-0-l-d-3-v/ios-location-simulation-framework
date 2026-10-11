//! A session's events: a bounded queue the caller empties.
//!
//! The supervisor reports what happens to a sink. Here the sink turns each
//! event into a record of fixed size and puts it in a queue; the caller
//! takes records out one at a time. Nothing is called back.
//!
//! Bounded. The queue holds as many records as the session was created
//! with room for. When a record arrives and the queue is full, the oldest
//! one is discarded and counted: [`locsim_session_events_dropped`] says how
//! many. The supervisor's totals in the health report do not depend on the
//! queue; comparing them with the events is sound only while that count
//! is zero.
//!
//! No text and no position. A record is times, counts and codes, all
//! integers. An error is reported as its status code and nothing it
//! carries; a fault is reported as the code of its component.

use crate::codes::*;
use crate::session::{contain, with_session, LocsimSession};
use crate::status::*;
use locsim_health::{Event, EventKind, EventSink, FailReason, Level, Operation, RunEndReason};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// One thing that happened to a session. 64 bytes, aligned to 8, no
/// padding. What `arg` holds depends on `kind`; see the `LOCSIM_EVENT_*`
/// constants. Arguments a kind does not use are 0.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocsimEvent {
    /// Set by the caller before the call; must be the size of this struct.
    pub struct_size: u32,
    /// A `LOCSIM_LEVEL_*` value.
    pub level: u32,
    /// A `LOCSIM_EVENT_*` value.
    pub kind: u32,
    /// The status code of the error the event carries, or 0.
    pub error: u32,
    /// The time passed to the call during which it happened.
    pub at_ns: i64,
    /// The number of the run it concerns; 0 before any run.
    pub run: u64,
    pub arg: [i64; 4],
}

const EVENT_SIZE: u32 = 64;
const _: () = assert!(std::mem::size_of::<LocsimEvent>() == EVENT_SIZE as usize);
const _: () = assert!(std::mem::align_of::<LocsimEvent>() == 8);

/// A count as an argument. One that does not fit is the largest that does.
fn count(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

impl LocsimEvent {
    pub(crate) fn of(event: &Event) -> Self {
        let mut error = 0;
        let mut arg = [0; 4];
        let kind = match &event.kind {
            EventKind::RunStarted { recovery_attempt } => {
                if let Some(attempt) = recovery_attempt {
                    arg[0] = 1;
                    arg[1] = i64::from(*attempt);
                }
                LOCSIM_EVENT_RUN_STARTED
            }
            EventKind::RunEnded {
                samples,
                withheld,
                missed,
                reason,
            } => {
                arg[0] = count(*samples);
                arg[1] = count(*withheld);
                arg[2] = count(*missed);
                arg[3] = match reason {
                    RunEndReason::Failure => 0,
                    RunEndReason::Stopped => 1,
                };
                LOCSIM_EVENT_RUN_ENDED
            }
            EventKind::Stopped => LOCSIM_EVENT_STOPPED,
            EventKind::Paused => LOCSIM_EVENT_PAUSED,
            EventKind::Resumed => LOCSIM_EVENT_RESUMED,
            EventKind::OperationRejected {
                operation,
                error: refused,
            } => {
                arg[0] = match operation {
                    Operation::Start => 0,
                    Operation::Stop => 1,
                    Operation::Pause => 2,
                    Operation::Resume => 3,
                };
                error = error_code(refused);
                LOCSIM_EVENT_OPERATION_REJECTED
            }
            EventKind::RunFailed { error: failure } => {
                error = error_code(failure);
                LOCSIM_EVENT_RUN_FAILED
            }
            EventKind::RecoveryScheduled { attempt, at } => {
                arg[0] = i64::from(*attempt);
                arg[1] = at.as_nanos();
                LOCSIM_EVENT_RECOVERY_SCHEDULED
            }
            EventKind::RecoveryAttempted { attempt } => {
                arg[0] = i64::from(*attempt);
                LOCSIM_EVENT_RECOVERY_ATTEMPTED
            }
            EventKind::RecoveryStartFailed {
                attempt,
                error: failure,
            } => {
                arg[0] = i64::from(*attempt);
                error = error_code(failure);
                LOCSIM_EVENT_RECOVERY_START_FAILED
            }
            EventKind::Recovered { attempts } => {
                arg[0] = i64::from(*attempts);
                LOCSIM_EVENT_RECOVERED
            }
            EventKind::Failed { reason } => {
                arg[0] = match reason {
                    FailReason::AttemptsExhausted => 0,
                    FailReason::Permanent => 1,
                    FailReason::RetryTimeUnrepresentable => 2,
                };
                LOCSIM_EVENT_FAILED
            }
            EventKind::Stalled { silent_for_ns } => {
                arg[0] = *silent_for_ns;
                LOCSIM_EVENT_STALLED
            }
            EventKind::StallCleared => LOCSIM_EVENT_STALL_CLEARED,
            EventKind::FallingBehind { missed, slots } => {
                arg[0] = count(*missed);
                arg[1] = count(*slots);
                LOCSIM_EVENT_FALLING_BEHIND
            }
            EventKind::CaughtUp => LOCSIM_EVENT_CAUGHT_UP,
            // The detail of a fault is text and does not cross. Through
            // this library it is always empty.
            EventKind::FaultReported {
                component,
                detail: _,
            } => {
                arg[0] = i64::from(component_code(component));
                LOCSIM_EVENT_FAULT_REPORTED
            }
            EventKind::FaultCleared { component } => {
                arg[0] = i64::from(component_code(component));
                LOCSIM_EVENT_FAULT_CLEARED
            }
            EventKind::HealthChanged { from, to } => {
                arg[0] = i64::from(health_code(*from));
                arg[1] = i64::from(health_code(*to));
                LOCSIM_EVENT_HEALTH_CHANGED
            }
            EventKind::ClockWentBackwards { latest } => {
                arg[0] = latest.as_nanos();
                LOCSIM_EVENT_CLOCK_WENT_BACKWARDS
            }
        };
        Self {
            struct_size: EVENT_SIZE,
            level: match event.level {
                Level::Error => LOCSIM_LEVEL_ERROR,
                Level::Warn => LOCSIM_LEVEL_WARN,
                Level::Info => LOCSIM_LEVEL_INFO,
                Level::Debug => LOCSIM_LEVEL_DEBUG,
            },
            kind,
            error,
            at_ns: event.at.as_nanos(),
            run: event.run,
            arg,
        }
    }
}

/// The records a session has not handed out yet, oldest first.
pub(crate) struct EventQueue {
    capacity: usize,
    records: VecDeque<LocsimEvent>,
    dropped: u64,
}

impl EventQueue {
    /// `capacity` is at least 1.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            records: VecDeque::with_capacity(capacity),
            dropped: 0,
        }
    }

    fn push(&mut self, record: LocsimEvent) {
        if self.records.len() >= self.capacity {
            self.records.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.records.push_back(record);
    }

    fn pop(&mut self) -> Option<LocsimEvent> {
        self.records.pop_front()
    }
}

/// The queue, whether or not a thread once panicked while holding it: a
/// queue is valid after every step of every change to it.
///
/// It is only ever locked by the call that holds its session, so this
/// never waits.
fn lock(queue: &Mutex<EventQueue>) -> MutexGuard<'_, EventQueue> {
    queue.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The sink a session's supervisor reports to.
pub(crate) struct QueueSink(pub(crate) Arc<Mutex<EventQueue>>);

impl EventSink for QueueSink {
    fn record(&mut self, event: &Event) {
        let mut queue = lock(&self.0);
        #[cfg(test)]
        crate::hooks::fire(crate::hooks::Point::EventRecord);
        queue.push(LocsimEvent::of(event));
    }
}

/// Takes the oldest event out of the session's queue.
///
/// - `LOCSIM_OK`, `*out_has_event == 1`: `*out_event` is that event.
/// - `LOCSIM_OK`, `*out_has_event == 0`: the queue is empty. `*out_event`
///   is not written.
///
/// # Safety
///
/// `out_event` is aligned, readable and writable for 64 bytes and its
/// `struct_size` is set; `out_has_event` is writable.
#[no_mangle]
pub unsafe extern "C" fn locsim_session_next_event(
    session: LocsimSession,
    out_event: *mut LocsimEvent,
    out_has_event: *mut u8,
) -> LocsimStatus {
    if out_event.is_null() || out_has_event.is_null() {
        return LOCSIM_ERR_NULL_POINTER;
    }
    // SAFETY: not null, and readable for the struct's first four bytes by
    // the caller's guarantee. No alignment is assumed.
    let struct_size = unsafe { out_event.cast::<u32>().read_unaligned() };
    if struct_size != EVENT_SIZE {
        return LOCSIM_ERR_STRUCT_SIZE;
    }
    let taken = contain(|| with_session(session, |session| Ok(lock(&session.events).pop())));
    match taken {
        Err(status) => status,
        Ok(None) => {
            // SAFETY: not null and writable by the caller's guarantee.
            unsafe { out_has_event.write(0) };
            LOCSIM_OK
        }
        Ok(Some(event)) => {
            // SAFETY: both are not null and writable for their size by the
            // caller's guarantee. No alignment is assumed.
            unsafe {
                out_event.write_unaligned(event);
                out_has_event.write(1);
            }
            LOCSIM_OK
        }
    }
}

/// Says how many events the session has discarded, since it was created,
/// because its queue was full. The count never goes down.
///
/// # Safety
///
/// `out_count` is writable.
#[no_mangle]
pub unsafe extern "C" fn locsim_session_events_dropped(
    session: LocsimSession,
    out_count: *mut u64,
) -> LocsimStatus {
    if out_count.is_null() {
        return LOCSIM_ERR_NULL_POINTER;
    }
    let counted = contain(|| with_session(session, |session| Ok(lock(&session.events).dropped)));
    match counted {
        Err(status) => status,
        Ok(dropped) => {
            // SAFETY: not null and writable by the caller's guarantee. No
            // alignment is assumed.
            unsafe { out_count.write_unaligned(dropped) };
            LOCSIM_OK
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(n: i64) -> LocsimEvent {
        LocsimEvent {
            struct_size: EVENT_SIZE,
            level: 0,
            kind: 0,
            error: 0,
            at_ns: n,
            run: 0,
            arg: [0; 4],
        }
    }

    fn drain(queue: &mut EventQueue) -> Vec<i64> {
        std::iter::from_fn(|| queue.pop())
            .map(|event| event.at_ns)
            .collect()
    }

    #[test]
    fn a_queue_keeps_the_newest_records_in_order_and_counts_the_rest() {
        for capacity in [1, 2, 3, 7] {
            for pushed in 0..=20 {
                let mut queue = EventQueue::new(capacity);
                (0..pushed).for_each(|n| queue.push(record(n)));
                let kept = pushed.min(capacity as i64);
                assert_eq!(queue.dropped, (pushed - kept) as u64);
                assert_eq!(
                    drain(&mut queue),
                    (pushed - kept..pushed).collect::<Vec<_>>()
                );
                // Emptying a queue does not forget what it dropped.
                assert_eq!(queue.dropped, (pushed - kept) as u64);
                assert!(queue.pop().is_none());
            }
        }
    }

    #[test]
    fn room_made_by_taking_a_record_out_is_room_for_the_next() {
        let mut queue = EventQueue::new(2);
        queue.push(record(0));
        queue.push(record(1));
        assert_eq!(queue.pop().unwrap().at_ns, 0);
        queue.push(record(2));
        assert_eq!(queue.dropped, 0);
        queue.push(record(3));
        assert_eq!(queue.dropped, 1);
        assert_eq!(drain(&mut queue), [2, 3]);
    }

    #[test]
    fn the_queue_never_holds_more_than_its_capacity() {
        let mut queue = EventQueue::new(3);
        for n in 0..1000 {
            queue.push(record(n));
            assert!(queue.records.len() <= 3);
        }
        assert_eq!(queue.dropped, 997);
        queue.dropped = u64::MAX;
        queue.push(record(0));
        assert_eq!(queue.dropped, u64::MAX);
    }

    #[test]
    fn a_count_that_does_not_fit_is_the_largest_that_does() {
        assert_eq!(count(0), 0);
        assert_eq!(count(i64::MAX as u64), i64::MAX);
        assert_eq!(count(i64::MAX as u64 + 1), i64::MAX);
        assert_eq!(count(u64::MAX), i64::MAX);
    }

    #[test]
    fn the_event_struct_has_the_documented_layout() {
        let event = record(0);
        let base = std::ptr::addr_of!(event) as usize;
        let offsets = [
            (std::ptr::addr_of!(event.struct_size) as usize, 0),
            (std::ptr::addr_of!(event.level) as usize, 4),
            (std::ptr::addr_of!(event.kind) as usize, 8),
            (std::ptr::addr_of!(event.error) as usize, 12),
            (std::ptr::addr_of!(event.at_ns) as usize, 16),
            (std::ptr::addr_of!(event.run) as usize, 24),
            (std::ptr::addr_of!(event.arg) as usize, 32),
        ];
        for (address, offset) in offsets {
            assert_eq!(address - base, offset);
        }
        assert_eq!(std::mem::size_of::<LocsimEvent>(), 64);
    }
}
