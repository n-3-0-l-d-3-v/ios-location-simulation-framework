use super::*;
use crate::events::{locsim_session_events_dropped, locsim_session_next_event, LocsimEvent};
use crate::health::{
    locsim_session_check, locsim_session_clear_fault, locsim_session_health,
    locsim_session_report_fault, LocsimHealth,
};
use crate::hooks::{arm, gate, take_session_drop, Point, Stop};
use crate::session::{adopt, locsim_session_destroy, registry};
use crate::testing::*;
use locsim_core::domain::{HealthState, LocationSource, Scenario, SimulationState};
use locsim_core::movement::{model_for, MovementError, MovementModel, MovementSample};
use locsim_core::scheduler::ScheduleError;
use locsim_health::{
    Cause, Emission, Event, EventKind, EventSink, FailReason, HealthReport, Level, MemorySink,
    Operation, RunEndReason,
};
use std::collections::BTreeSet;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};

const SECOND: i64 = 1_000_000_000;
const MS: i64 = 1_000_000;

// What the outputs hold before a call. Still there afterwards means the
// call did not write.
const BYTE_BEFORE: u8 = 0xA5;
const DEADLINE_BEFORE: i64 = 0x5A5A_5A5A_5A5A_5A5A;

/// Room for a sample, filled with the marker except for `struct_size`.
#[repr(C, align(8))]
struct SampleRoom([u8; 88]);

impl SampleRoom {
    fn new(struct_size: u32) -> Self {
        let mut room = Self([BYTE_BEFORE; 88]);
        room.0[..4].copy_from_slice(&struct_size.to_ne_bytes());
        room
    }
}

struct Polled {
    status: LocsimStatus,
    has: u8,
    room: SampleRoom,
    struct_size: u32,
}

impl Polled {
    /// Neither the flag nor the sample was written.
    fn untouched(&self) -> bool {
        self.has == BYTE_BEFORE && self.sample_untouched()
    }

    fn sample_untouched(&self) -> bool {
        self.room.0[..4] == self.struct_size.to_ne_bytes()
            && self.room.0[4..].iter().all(|byte| *byte == BYTE_BEFORE)
    }

    fn sample(&self) -> Option<LocsimSample> {
        assert_eq!(self.status, LOCSIM_OK);
        match self.has {
            0 => {
                assert!(self.sample_untouched());
                None
            }
            1 => {
                // SAFETY: the room is 88 aligned bytes that the call has
                // just filled with a `LocsimSample`.
                Some(unsafe { self.room.0.as_ptr().cast::<LocsimSample>().read() })
            }
            other => panic!("the flag is {other}"),
        }
    }
}

fn poll_sized(session: u64, now_ns: i64, struct_size: u32) -> Polled {
    let mut room = SampleRoom::new(struct_size);
    let mut has = BYTE_BEFORE;
    // SAFETY: both pointers are to live locals of the sizes required.
    let status =
        unsafe { locsim_session_poll(session, now_ns, room.0.as_mut_ptr().cast(), &mut has) };
    Polled {
        status,
        has,
        room,
        struct_size,
    }
}

fn poll(session: u64, now_ns: i64) -> Polled {
    poll_sized(session, now_ns, 88)
}

struct Deadline {
    status: LocsimStatus,
    has: u8,
    at: i64,
}

impl Deadline {
    fn untouched(&self) -> bool {
        self.has == BYTE_BEFORE && self.at == DEADLINE_BEFORE
    }

    fn value(&self) -> Option<i64> {
        assert_eq!(self.status, LOCSIM_OK);
        match self.has {
            0 => {
                assert_eq!(self.at, DEADLINE_BEFORE);
                None
            }
            1 => Some(self.at),
            other => panic!("the flag is {other}"),
        }
    }
}

fn deadline(session: u64) -> Deadline {
    let mut at = DEADLINE_BEFORE;
    let mut has = BYTE_BEFORE;
    // SAFETY: both pointers are to live locals.
    let status = unsafe { locsim_session_next_deadline(session, &mut at, &mut has) };
    Deadline { status, has, at }
}

const COUNT_BEFORE: u64 = 0x5A5A_5A5A_5A5A_5A5A;

/// Room for a struct of `N` bytes, filled with the marker except for
/// `struct_size`.
#[repr(C, align(8))]
struct Room<const N: usize>([u8; N]);

impl<const N: usize> Room<N> {
    fn new(struct_size: u32) -> Self {
        let mut room = Self([BYTE_BEFORE; N]);
        room.0[..4].copy_from_slice(&struct_size.to_ne_bytes());
        room
    }

    fn untouched(&self, struct_size: u32) -> bool {
        self.0[..4] == struct_size.to_ne_bytes()
            && self.0[4..].iter().all(|byte| *byte == BYTE_BEFORE)
    }
}

struct Checked {
    status: LocsimStatus,
    room: Room<192>,
    struct_size: u32,
}

impl Checked {
    fn untouched(&self) -> bool {
        self.room.untouched(self.struct_size)
    }

    fn report(&self) -> LocsimHealth {
        assert_eq!(self.status, LOCSIM_OK);
        // SAFETY: the room is 192 aligned bytes that the call has just
        // filled with a `LocsimHealth`.
        unsafe { self.room.0.as_ptr().cast::<LocsimHealth>().read() }
    }
}

fn health_sized(session: u64, struct_size: u32) -> Checked {
    let mut room = Room::<192>::new(struct_size);
    // SAFETY: the pointer is to a live local of the size required.
    let status = unsafe { locsim_session_health(session, room.0.as_mut_ptr().cast()) };
    Checked {
        status,
        room,
        struct_size,
    }
}

fn health(session: u64) -> Checked {
    health_sized(session, 192)
}

fn check_sized(session: u64, now_ns: i64, struct_size: u32) -> Checked {
    let mut room = Room::<192>::new(struct_size);
    // SAFETY: the pointer is to a live local of the size required.
    let status = unsafe { locsim_session_check(session, now_ns, room.0.as_mut_ptr().cast()) };
    Checked {
        status,
        room,
        struct_size,
    }
}

fn check(session: u64, now_ns: i64) -> Checked {
    check_sized(session, now_ns, 192)
}

struct Taken {
    status: LocsimStatus,
    has: u8,
    room: Room<64>,
    struct_size: u32,
}

impl Taken {
    fn untouched(&self) -> bool {
        self.has == BYTE_BEFORE && self.room.untouched(self.struct_size)
    }

    fn event(&self) -> Option<LocsimEvent> {
        assert_eq!(self.status, LOCSIM_OK);
        match self.has {
            0 => {
                assert!(self.room.untouched(self.struct_size));
                None
            }
            1 => {
                // SAFETY: the room is 64 aligned bytes that the call has
                // just filled with a `LocsimEvent`.
                Some(unsafe { self.room.0.as_ptr().cast::<LocsimEvent>().read() })
            }
            other => panic!("the flag is {other}"),
        }
    }
}

fn next_event_sized(session: u64, struct_size: u32) -> Taken {
    let mut room = Room::<64>::new(struct_size);
    let mut has = BYTE_BEFORE;
    // SAFETY: both pointers are to live locals of the sizes required.
    let status =
        unsafe { locsim_session_next_event(session, room.0.as_mut_ptr().cast(), &mut has) };
    Taken {
        status,
        has,
        room,
        struct_size,
    }
}

fn next_event(session: u64) -> Taken {
    next_event_sized(session, 64)
}

/// Every event waiting in the session's queue, oldest first.
fn drain(session: u64) -> Vec<LocsimEvent> {
    std::iter::from_fn(|| next_event(session).event()).collect()
}

fn dropped(session: u64) -> (LocsimStatus, u64) {
    let mut count = COUNT_BEFORE;
    // SAFETY: the pointer is to a live local.
    let status = unsafe { locsim_session_events_dropped(session, &mut count) };
    (status, count)
}

fn clear_fault(session: u64, now_ns: i64, component: u32) -> (LocsimStatus, u8) {
    let mut was_set = BYTE_BEFORE;
    // SAFETY: the pointer is to a live local.
    let status = unsafe { locsim_session_clear_fault(session, now_ns, component, &mut was_set) };
    (status, was_set)
}

/// Every operation on `session` but destroy, each expected to be refused
/// with `status` and to have written nothing.
fn assert_all_refused(session: u64, status: LocsimStatus) {
    let checked = check(session, 0);
    assert_eq!(checked.status, status);
    assert!(checked.untouched());
    let reported = health(session);
    assert_eq!(reported.status, status);
    assert!(reported.untouched());
    assert_eq!(locsim_session_report_fault(session, 0, 1), status);
    assert_eq!(clear_fault(session, 0, 1), (status, BYTE_BEFORE));
    let taken = next_event(session);
    assert_eq!(taken.status, status);
    assert!(taken.untouched());
    assert_eq!(dropped(session), (status, COUNT_BEFORE));

    assert_eq!(locsim_session_start(session, 0), status);
    assert_eq!(locsim_session_stop(session), status);
    assert_eq!(locsim_session_pause(session, 0), status);
    assert_eq!(locsim_session_resume(session, 0), status);
    let polled = poll(session, 0);
    assert_eq!(polled.status, status);
    assert!(polled.untouched());
    let next = deadline(session);
    assert_eq!(next.status, status);
    assert!(next.untouched());
}

// --- The same calls made on the supervisor directly ---------------------

fn reference(json: &str) -> (Supervisor<SimulationProvider>, MemorySink) {
    let scenario = locsim_scenario::import_scenario(json).unwrap();
    let events = MemorySink::new();
    let supervisor = Supervisor::for_simulation(
        SimulationProvider::new(scenario),
        policy().to_health(),
        Box::new(events.clone()),
    )
    .unwrap();
    (supervisor, events)
}

/// The names of the three components, written out here a second time.
fn component(code: u32) -> &'static str {
    match code {
        1 => "store",
        2 => "adapter",
        3 => "delivery",
        other => panic!("no component {other}"),
    }
}

fn error_number(error: &ProviderError) -> u32 {
    expected_status(&Err(error.clone())) as u32
}

fn state_number(state: SimulationState) -> u32 {
    match state {
        SimulationState::Idle => 0,
        SimulationState::Starting => 1,
        SimulationState::Running => 2,
        SimulationState::Paused => 3,
        SimulationState::Stopping => 4,
        SimulationState::Error => 5,
        SimulationState::Recovering => 6,
    }
}

fn health_number(health: HealthState) -> u32 {
    match health {
        HealthState::Healthy => 0,
        HealthState::Degraded => 1,
        HealthState::Recovering => 2,
        HealthState::Failed => 3,
        HealthState::Stopped => 4,
    }
}

/// The record an event must become: the table of `events.rs`, written out
/// a second time and independently, with the numbers as numbers.
fn expected_event(event: &Event) -> LocsimEvent {
    let (kind, error, arg): (u32, u32, [i64; 4]) = match &event.kind {
        EventKind::RunStarted {
            recovery_attempt: None,
        } => (1, 0, [0, 0, 0, 0]),
        EventKind::RunStarted {
            recovery_attempt: Some(attempt),
        } => (1, 0, [1, i64::from(*attempt), 0, 0]),
        EventKind::RunEnded {
            samples,
            withheld,
            missed,
            reason,
        } => {
            let reason = match reason {
                RunEndReason::Failure => 0,
                RunEndReason::Stopped => 1,
            };
            (
                2,
                0,
                [*samples as i64, *withheld as i64, *missed as i64, reason],
            )
        }
        EventKind::Stopped => (3, 0, [0; 4]),
        EventKind::Paused => (4, 0, [0; 4]),
        EventKind::Resumed => (5, 0, [0; 4]),
        EventKind::OperationRejected { operation, error } => {
            let operation = match operation {
                Operation::Start => 0,
                Operation::Stop => 1,
                Operation::Pause => 2,
                Operation::Resume => 3,
            };
            (6, error_number(error), [operation, 0, 0, 0])
        }
        EventKind::RunFailed { error } => (7, error_number(error), [0; 4]),
        EventKind::RecoveryScheduled { attempt, at } => {
            (8, 0, [i64::from(*attempt), at.as_nanos(), 0, 0])
        }
        EventKind::RecoveryAttempted { attempt } => (9, 0, [i64::from(*attempt), 0, 0, 0]),
        EventKind::RecoveryStartFailed { attempt, error } => {
            (10, error_number(error), [i64::from(*attempt), 0, 0, 0])
        }
        EventKind::Recovered { attempts } => (11, 0, [i64::from(*attempts), 0, 0, 0]),
        EventKind::Failed { reason } => {
            let reason = match reason {
                FailReason::AttemptsExhausted => 0,
                FailReason::Permanent => 1,
                FailReason::RetryTimeUnrepresentable => 2,
            };
            (12, 0, [reason, 0, 0, 0])
        }
        EventKind::Stalled { silent_for_ns } => (13, 0, [*silent_for_ns, 0, 0, 0]),
        EventKind::StallCleared => (14, 0, [0; 4]),
        EventKind::FallingBehind { missed, slots } => {
            (15, 0, [*missed as i64, *slots as i64, 0, 0])
        }
        EventKind::CaughtUp => (16, 0, [0; 4]),
        EventKind::FaultReported { component, detail } => {
            // Nothing a caller writes can be in an event.
            assert!(detail.is_empty());
            let code = [1, 2, 3]
                .into_iter()
                .find(|c| self::component(*c) == *component);
            (17, 0, [i64::from(code.unwrap()), 0, 0, 0])
        }
        EventKind::FaultCleared { component } => {
            let code = [1, 2, 3]
                .into_iter()
                .find(|c| self::component(*c) == *component);
            (18, 0, [i64::from(code.unwrap()), 0, 0, 0])
        }
        EventKind::HealthChanged { from, to } => (
            19,
            0,
            [
                i64::from(health_number(*from)),
                i64::from(health_number(*to)),
                0,
                0,
            ],
        ),
        EventKind::ClockWentBackwards { latest } => (20, 0, [latest.as_nanos(), 0, 0, 0]),
    };
    LocsimEvent {
        struct_size: 64,
        level: match event.level {
            Level::Error => 0,
            Level::Warn => 1,
            Level::Info => 2,
            Level::Debug => 3,
        },
        kind,
        error,
        at_ns: event.at.as_nanos(),
        run: event.run,
        arg,
    }
}

/// The report a `HealthReport` must become, field by field.
fn expected_health(report: &HealthReport) -> LocsimHealth {
    let time = |time: Option<Timestamp>| time.map_or((0, 0), |time| (1, time.as_nanos()));
    let (emission, silent_for_ns) = match report.emission {
        Emission::NotExpected => (0, 0),
        Emission::Flowing => (1, 0),
        Emission::Stalled { silent_for_ns } => (2, silent_for_ns),
    };
    let mut causes = 0;
    for cause in &report.causes {
        causes |= match cause {
            Cause::Stalled => 1,
            Cause::FallingBehind => 2,
            Cause::Probation => 4,
            Cause::ExternalFault(_) => 8,
        };
    }
    let mut active_faults = 0;
    for name in &report.active_faults {
        let code = [1, 2, 3].into_iter().find(|c| component(*c) == *name);
        active_faults |= 1 << (code.unwrap() - 1);
    }
    let (has_observed_at, observed_at_ns) = time(report.observed_at);
    let (has_retry_at, retry_at_ns) = time(report.retry_at);
    let (has_last_sample_time, last_sample_time_ns) = time(report.current.last_sample_time);
    LocsimHealth {
        struct_size: 192,
        lifecycle: state_number(report.lifecycle),
        health: health_number(report.health),
        emission,
        causes,
        active_faults,
        attempts_used: report.attempts_used,
        max_attempts: report.max_attempts,
        last_error: report.last_error.as_ref().map_or(0, error_number),
        current_state: state_number(report.current.state),
        has_observed_at,
        has_retry_at,
        has_last_sample_time,
        trajectory_complete: u8::from(report.current.trajectory_complete),
        reserved: [0; 4],
        observed_at_ns,
        retry_at_ns,
        silent_for_ns,
        last_sample_time_ns,
        run: report.run,
        current_sample_count: report.current.sample_count,
        current_failed_count: report.current.failed_count,
        current_missed_ticks: report.current.missed_ticks,
        total_runs_started: report.totals.runs_started,
        total_runs_ended: report.totals.runs_ended,
        total_samples: report.totals.samples,
        total_withheld: report.totals.withheld,
        total_missed_ticks: report.totals.missed_ticks,
        total_failures: report.totals.failures,
        total_recovery_attempts: report.totals.recovery_attempts,
        total_recoveries_completed: report.totals.recoveries_completed,
        total_terminal_failures: report.totals.terminal_failures,
        total_rejected_operations: report.totals.rejected_operations,
    }
}

/// The table of `codes.rs`, written out a second time and independently,
/// with the numbers as numbers.
fn expected_status(result: &Result<(), ProviderError>) -> LocsimStatus {
    match result {
        Ok(()) => 0,
        Err(ProviderError::InvalidScenario(_)) => 30,
        Err(ProviderError::Transition(_)) => 31,
        Err(ProviderError::Movement(_)) => 33,
        Err(ProviderError::Noise(_)) => 34,
        Err(ProviderError::Derivation(_)) => 35,
        Err(ProviderError::Validation(_)) => 36,
        Err(ProviderError::Schedule(ScheduleError::InvalidInterval)) => 40,
        Err(ProviderError::Schedule(ScheduleError::Overflow)) => 41,
        Err(ProviderError::Schedule(ScheduleError::AlreadyPaused)) => 42,
        Err(ProviderError::Schedule(ScheduleError::NotPaused)) => 43,
        Err(ProviderError::Schedule(ScheduleError::ClockWentBackwards)) => 44,
    }
}

fn assert_same_sample(got: &LocsimSample, expected: &SyntheticLocation, run: u64, at: &str) {
    let bits = |value: f64| value.to_bits();
    assert_eq!(got.struct_size, 88, "{at}");
    assert_eq!(got.reserved, [0, 0], "{at}");
    assert_eq!(got.timestamp_ns, expected.timestamp.as_nanos(), "{at}");
    assert_eq!(got.run, run, "{at}");
    assert_eq!(
        bits(got.latitude_deg),
        bits(expected.coordinate.latitude()),
        "{at}"
    );
    assert_eq!(
        bits(got.longitude_deg),
        bits(expected.coordinate.longitude()),
        "{at}"
    );
    assert_eq!(bits(got.altitude_m), bits(expected.altitude_m), "{at}");
    assert_eq!(
        bits(got.horizontal_accuracy_m),
        bits(expected.horizontal_accuracy_m),
        "{at}"
    );
    assert_eq!(
        bits(got.vertical_accuracy_m),
        bits(expected.vertical_accuracy_m),
        "{at}"
    );
    for (flag, value, source) in [
        (got.has_speed, got.speed_mps, expected.speed_mps),
        (got.has_course, got.course_deg, expected.course_deg),
    ] {
        match source {
            Some(known) => {
                assert_eq!(flag, 1, "{at}");
                assert_eq!(bits(value), bits(known), "{at}");
            }
            None => {
                assert_eq!(flag, 0, "{at}");
                assert!(value.is_nan(), "{at}");
            }
        }
    }
    let source = match expected.source {
        LocationSource::Simulation => 0,
        LocationSource::Replay => 1,
        LocationSource::Test => 2,
    };
    assert_eq!(got.source, source, "{at}");
    let state = match expected.simulation_state {
        SimulationState::Idle => 0,
        SimulationState::Starting => 1,
        SimulationState::Running => 2,
        SimulationState::Paused => 3,
        SimulationState::Stopping => 4,
        SimulationState::Error => 5,
        SimulationState::Recovering => 6,
    };
    assert_eq!(got.simulation_state, state, "{at}");
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Start(i64),
    Stop,
    Pause(i64),
    Resume(i64),
    Poll(i64),
    Check(i64),
    /// Report a fault of the component at the time.
    Report(i64, u32),
    /// Clear it.
    Clear(i64, u32),
}

/// One session's worth of calls from `origin` on: calls that are refused,
/// polls with and without a sample due, a late poll, a time that goes
/// backwards, a pause, a resume earlier than its pause, and a second run.
fn script(origin: i64) -> Vec<Step> {
    let at = |offset: i64| origin.saturating_add(offset);
    let mut steps = vec![
        Step::Poll(at(0)),
        Step::Stop,
        Step::Pause(at(0)),
        Step::Resume(at(0)),
        Step::Start(at(0)),
        Step::Start(at(0)),
    ];
    steps.extend((0..=24).map(|k| Step::Poll(at(k * 250 * MS))));
    steps.extend([
        // Late: ticks were skipped.
        Step::Poll(at(9_900 * MS)),
        // Earlier than the call before it.
        Step::Poll(at(9_000 * MS)),
        Step::Poll(at(10_200 * MS)),
        Step::Pause(at(10_300 * MS)),
        Step::Pause(at(10_400 * MS)),
        Step::Poll(at(11 * SECOND)),
        // Earlier than the pause.
        Step::Resume(at(10 * SECOND)),
        Step::Resume(at(12_500 * MS)),
        Step::Resume(at(12_600 * MS)),
    ]);
    steps.extend((0..=12).map(|k| Step::Poll(at(12_500 * MS + k * 250 * MS))));
    steps.extend([
        Step::Stop,
        Step::Stop,
        Step::Poll(at(16 * SECOND)),
        Step::Start(at(20 * SECOND)),
    ]);
    steps.extend((0..=8).map(|k| Step::Poll(at(20 * SECOND + k * 500 * MS))));
    steps.push(Step::Stop);
    steps
}

/// What a run of the script produced, for the tests to say what they
/// expect to have been exercised.
#[derive(Default)]
struct Seen {
    samples: Vec<LocsimSample>,
    runs: Vec<u64>,
    statuses: Vec<LocsimStatus>,
    /// The deadline after each step.
    deadlines: Vec<Option<i64>>,
    /// The step and the supervisor's error of every poll that failed.
    failed_polls: Vec<(usize, ProviderError)>,
    /// Every event, in the order it left the queue.
    events: Vec<LocsimEvent>,
    /// The health after each step.
    healths: Vec<LocsimHealth>,
}

/// Makes every call of the script on a session made from `json` and on a
/// supervisor made from it, and requires the same answer from both.
fn run_side_by_side(json: &str, origin: i64) -> Seen {
    let label = format!("from {origin}");
    let (direct, events) = reference(json);
    compare(open_with(json), direct, events, script(origin), &label)
}

/// Makes every step on `session` and on `direct`, which must have been
/// built alike, and requires the same answer from both after each one.
fn compare(
    session: u64,
    mut direct: Supervisor<SimulationProvider>,
    direct_events: MemorySink,
    steps: Vec<Step>,
    label: &str,
) -> Seen {
    let mut seen = Seen::default();

    for (n, step) in steps.into_iter().enumerate() {
        let at = format!("step {n} {step:?} {label}");
        let status = match step {
            Step::Start(now) => {
                let expected = direct.start(Timestamp::from_nanos(now));
                let status = locsim_session_start(session, now);
                assert_eq!(status, expected_status(&expected), "{at}");
                status
            }
            Step::Stop => {
                let expected = direct.stop();
                let status = locsim_session_stop(session);
                assert_eq!(status, expected_status(&expected), "{at}");
                status
            }
            Step::Pause(now) => {
                let expected = direct.pause(Timestamp::from_nanos(now));
                let status = locsim_session_pause(session, now);
                assert_eq!(status, expected_status(&expected), "{at}");
                status
            }
            Step::Resume(now) => {
                let expected = direct.resume(Timestamp::from_nanos(now));
                let status = locsim_session_resume(session, now);
                assert_eq!(status, expected_status(&expected), "{at}");
                status
            }
            Step::Poll(now) => {
                let expected = direct.poll(Timestamp::from_nanos(now));
                let polled = poll(session, now);
                match expected {
                    Err(error) => {
                        assert_eq!(polled.status, expected_status(&Err(error.clone())), "{at}");
                        assert!(polled.untouched(), "{at}");
                        seen.failed_polls.push((n, error));
                    }
                    Ok(None) => assert!(polled.sample().is_none(), "{at}"),
                    Ok(Some(expected)) => {
                        let got = polled.sample().expect(&at);
                        let run = direct.health().run;
                        assert_same_sample(&got, &expected, run, &at);
                        seen.samples.push(got);
                        if seen.runs.last() != Some(&run) {
                            seen.runs.push(run);
                        }
                    }
                }
                polled.status
            }
            Step::Check(now) => {
                let expected = direct.check(Timestamp::from_nanos(now));
                let checked = check(session, now);
                assert_eq!(checked.report(), expected_health(&expected), "{at}");
                checked.status
            }
            Step::Report(now, code) => {
                direct.report_fault(Timestamp::from_nanos(now), component(code), String::new());
                let status = locsim_session_report_fault(session, now, code);
                assert_eq!(status, LOCSIM_OK, "{at}");
                status
            }
            Step::Clear(now, code) => {
                let expected = direct.clear_fault(Timestamp::from_nanos(now), component(code));
                let (status, was_set) = clear_fault(session, now, code);
                assert_eq!((status, was_set), (LOCSIM_OK, u8::from(expected)), "{at}");
                status
            }
        };
        seen.statuses.push(status);

        // The same events, in the same order, and none lost.
        let all = direct_events.events();
        let expected: Vec<LocsimEvent> = all[seen.events.len()..]
            .iter()
            .map(expected_event)
            .collect();
        assert_eq!(drain(session), expected, "{at}");
        assert_eq!(dropped(session), (LOCSIM_OK, 0), "{at}");
        seen.events.extend(expected);

        // The same health, and asking for it changed nothing.
        let reported = health(session).report();
        assert_eq!(reported, expected_health(&direct.health()), "{at}");
        assert_eq!(health(session).report(), reported, "{at}");
        assert!(next_event(session).event().is_none(), "{at}");
        seen.healths.push(reported);

        let tick = direct.provider().next_deadline();
        let retry = direct.next_deadline();
        let expected = match (tick, retry) {
            (Some(tick), Some(retry)) => Some(tick.min(retry)),
            (tick, retry) => tick.or(retry),
        };
        let got = deadline(session).value();
        assert_eq!(got, expected.map(|time| time.as_nanos()), "{at}");
        seen.deadlines.push(got);
    }

    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
    seen
}

// --- Against the supervisor ---------------------------------------------

#[test]
fn every_example_gives_the_same_samples_statuses_and_deadlines_as_the_supervisor() {
    let _guard = shared();
    for (n, example) in EXAMPLES.into_iter().enumerate() {
        // Zero, a time before it that runs across it, and a large one.
        for origin in [0, -5 * SECOND, 1_700_000_000 * SECOND] {
            let seen = run_side_by_side(example, origin);
            // The script did what it was written to do. At an interval
            // of a second or less, the on-time polls alone meet seven
            // ticks in the first run and five in the second.
            assert!(seen.samples.len() >= 12, "example {n}");
            // At least the two runs the script starts, numbered upwards
            // from 1. There are more where a run failed on its own and
            // the supervisor replaced it; the session did the same.
            assert_eq!(seen.runs[0], 1, "example {n}");
            assert!(seen.runs.len() >= 2, "example {n}: {:?}", seen.runs);
            assert!(seen.runs.windows(2).all(|pair| pair[0] < pair[1]));
            // A failed poll is reported by that one call.
            for (step, error) in &seen.failed_polls {
                assert_eq!(seen.statuses[step + 1], LOCSIM_OK, "example {n}: {error:?}");
            }
            let with_deadline = seen.deadlines.iter().flatten().count();
            assert!(with_deadline >= 40, "example {n}: {with_deadline}");
            for refused in [
                LOCSIM_ERR_PROVIDER_TRANSITION,
                LOCSIM_ERR_SCHEDULE_CLOCK_WENT_BACKWARDS,
            ] {
                assert!(seen.statuses.contains(&refused), "example {n}: {refused}");
            }
        }
    }
}

#[test]
fn a_run_whose_times_run_out_fails_once_with_overflow_like_the_supervisor() {
    let _guard = shared();
    // Four seconds before the last representable time.
    let seen = run_side_by_side(FIXED, i64::MAX - 4 * SECOND);
    let first = seen
        .statuses
        .iter()
        .position(|status| *status == LOCSIM_ERR_SCHEDULE_OVERFLOW)
        .expect("the run reaches a time that cannot be represented");
    // Reported by one call. This error is permanent, so no restart follows
    // and nothing reports it again until the session is started anew.
    assert_eq!(seen.statuses[first + 1], LOCSIM_OK);
    assert!(!seen.samples.is_empty());
}

// --- Runs that fail, whatever the scenario ------------------------------

/// The real model of a scenario until its `fail_at`-th sample (from 0),
/// which is an error instead.
struct FailsAt {
    real: Box<dyn MovementModel + Send>,
    calls: u64,
    fail_at: u64,
}

impl MovementModel for FailsAt {
    fn sample_at(&mut self, at: Timestamp) -> Result<MovementSample, MovementError> {
        let call = self.calls;
        self.calls += 1;
        if call == self.fail_at {
            return Err(MovementError::InvalidConfiguration("injected failure"));
        }
        self.real.sample_at(at)
    }

    fn is_complete(&self) -> bool {
        self.real.is_complete()
    }
}

/// A supervisor over the real provider of `scenario`, whose first
/// `failing_runs` runs fail at their fourth sample. Two of them built with
/// the same arguments behave alike: each has its own count of runs.
fn failing(
    scenario: &Scenario,
    failing_runs: u32,
    sink: Box<dyn EventSink + Send>,
) -> Supervisor<SimulationProvider> {
    let built = AtomicU32::new(0);
    let provider = SimulationProvider::with_model_factory(
        scenario.clone(),
        Box::new(move |scenario| {
            let run = built.fetch_add(1, Ordering::Relaxed);
            let real = model_for(scenario)?;
            Ok(if run < failing_runs {
                Box::new(FailsAt {
                    real,
                    calls: 0,
                    fail_at: 3,
                })
            } else {
                real
            })
        }),
    );
    Supervisor::for_simulation(provider, policy().to_health(), sink).unwrap()
}

/// A session and a supervisor built alike around the failing model, run
/// through `steps` side by side.
fn compare_failing(scenario: &Scenario, failing_runs: u32, steps: Vec<Step>, label: &str) -> Seen {
    let events = MemorySink::new();
    let direct = failing(scenario, failing_runs, Box::new(events.clone()));
    let session = adopt(16, |sink| failing(scenario, failing_runs, sink));
    compare(session, direct, events, steps, label)
}

/// Start, a poll every quarter of a second for 22 seconds, stop; then a
/// short second session.
fn long_script() -> Vec<Step> {
    let mut steps = vec![Step::Start(0)];
    steps.extend((0..=88).map(|k| Step::Poll(k * 250 * MS)));
    steps.extend([Step::Stop, Step::Start(30 * SECOND)]);
    steps.extend((0..=2).map(|k| Step::Poll((30 + k) * SECOND)));
    steps.push(Step::Stop);
    steps
}

#[test]
fn a_failing_run_is_reported_once_and_replaced_like_the_supervisor_does() {
    let _guard = shared();
    for (n, example) in EXAMPLES.into_iter().enumerate() {
        let scenario = locsim_scenario::import_scenario(example).unwrap();
        let label = format!("example {n}, two failing runs");
        let seen = compare_failing(&scenario, 2, long_script(), &label);
        // The policy allows three restarts, after 1 s, 2 s and 4 s. Run 1
        // fails at its fourth sample and so does run 2; run 3 is sound; the
        // second start is run 4.
        assert_eq!(seen.runs, [1, 2, 3, 4], "{label}");
        let failures: Vec<usize> = seen.failed_polls.iter().map(|(step, _)| *step).collect();
        assert_eq!(failures.len(), 2, "{label}");
        for step in failures {
            assert_eq!(seen.statuses[step], LOCSIM_ERR_PROVIDER_MOVEMENT, "{label}");
            // Reported by that one call; the next one is not an error.
            assert_eq!(seen.statuses[step + 1], LOCSIM_OK, "{label}");
            // While a restart is pending, the deadline is its time.
            assert!(seen.deadlines[step].is_some(), "{label}");
        }
        // A sample of a later run never carries the number of an earlier one.
        let numbers: Vec<u64> = seen.samples.iter().map(|sample| sample.run).collect();
        assert!(numbers.windows(2).all(|pair| pair[0] <= pair[1]), "{label}");
    }
}

#[test]
fn a_run_that_keeps_failing_ends_in_a_session_without_a_deadline() {
    let _guard = shared();
    let scenario = locsim_scenario::import_scenario(FIXED).unwrap();
    let seen = compare_failing(&scenario, 4, long_script(), "every allowed run fails");
    // The first run and three restarts all fail; then nothing is tried
    // until the session is stopped and started, which is run 5.
    assert_eq!(seen.runs, [1, 2, 3, 4, 5]);
    assert_eq!(seen.failed_polls.len(), 4);
    let (last_failure, _) = seen.failed_polls[3];
    assert_eq!(seen.statuses[last_failure], LOCSIM_ERR_PROVIDER_MOVEMENT);
    // Failed for good: no deadline, and polls are not errors.
    assert_eq!(seen.deadlines[last_failure], None);
    assert_eq!(seen.deadlines[last_failure + 1], None);
    assert_eq!(seen.statuses[last_failure + 1], LOCSIM_OK);
    // fixed.json: run 1 fails at 3 s, and the first restart is due 1 s later.
    let (first_failure, _) = seen.failed_polls[0];
    assert_eq!(seen.deadlines[first_failure], Some(4 * SECOND));

    // The health says so, and one event says why: attempts exhausted.
    let failed = seen.healths[last_failure];
    assert_eq!((failed.health, failed.lifecycle), (3, 5));
    assert_eq!((failed.attempts_used, failed.max_attempts), (3, 3));
    assert_eq!((failed.last_error, failed.has_retry_at), (33, 0));
    assert_eq!(failed.total_terminal_failures, 1);
    let gave_up: Vec<&LocsimEvent> = seen.events.iter().filter(|e| e.kind == 12).collect();
    assert_eq!(gave_up.len(), 1);
    assert_eq!((gave_up[0].level, gave_up[0].arg), (0, [0, 0, 0, 0]));
    // Every failed run left an event carrying the error's code and no more.
    let failures: Vec<&LocsimEvent> = seen.events.iter().filter(|e| e.kind == 7).collect();
    assert_eq!(failures.len(), 4);
    assert!(failures.iter().all(|e| e.error == 33 && e.arg == [0; 4]));
}

// --- Where the count of nanoseconds starts ------------------------------

#[test]
fn moving_every_time_by_a_constant_moves_the_timestamps_and_nothing_else() {
    let _guard = shared();
    for (n, example) in EXAMPLES.into_iter().enumerate() {
        let base = run_side_by_side(example, 0);
        for shift in [-5 * SECOND, 123_456_789, 1_700_000_000 * SECOND] {
            let at = format!("example {n} shifted by {shift}");
            let moved = run_side_by_side(example, shift);
            assert_eq!(moved.statuses, base.statuses, "{at}");
            assert_eq!(moved.runs, base.runs, "{at}");
            let unshifted: Vec<Option<i64>> = moved
                .deadlines
                .iter()
                .map(|deadline| deadline.map(|time| time - shift))
                .collect();
            assert_eq!(unshifted, base.deadlines, "{at}");
            assert_eq!(moved.samples.len(), base.samples.len(), "{at}");
            for (moved, base) in moved.samples.iter().zip(&base.samples) {
                assert_eq!(moved.timestamp_ns - shift, base.timestamp_ns, "{at}");
                // Everything else, bit for bit.
                let rest = |sample: &LocsimSample| {
                    (
                        sample.run,
                        sample.source,
                        sample.simulation_state,
                        (sample.has_speed, sample.has_course),
                        [
                            sample.latitude_deg,
                            sample.longitude_deg,
                            sample.altitude_m,
                            sample.horizontal_accuracy_m,
                            sample.vertical_accuracy_m,
                            sample.speed_mps,
                            sample.course_deg,
                        ]
                        .map(f64::to_bits),
                    )
                };
                assert_eq!(rest(moved), rest(base), "{at}");
            }
        }
    }
}

// --- Stated outright, without the supervisor beside it -------------------

#[test]
fn samples_carry_the_time_of_their_tick_and_the_number_of_their_run() {
    let _guard = shared();
    // fixed.json updates once a second.
    let session = open();
    assert_eq!(deadline(session).value(), None);
    assert!(poll(session, 1_000).sample().is_none());

    assert_eq!(locsim_session_start(session, 1_000), LOCSIM_OK);
    assert_eq!(deadline(session).value(), Some(1_000));
    let first = poll(session, 1_000).sample().unwrap();
    assert_eq!(first.timestamp_ns, 1_000);
    assert_eq!(first.run, 1);
    assert_eq!(first.source, 0);
    assert_eq!(first.simulation_state, 2);
    // The first sample of a run has neither speed nor course: not zero.
    assert_eq!((first.has_speed, first.has_course), (0, 0));
    assert!(first.speed_mps.is_nan() && first.course_deg.is_nan());

    // One nanosecond early there is nothing; on the tick there is.
    assert_eq!(deadline(session).value(), Some(1_000 + SECOND));
    assert!(poll(session, 999_999_999 + 1_000).sample().is_none());
    let second = poll(session, 1_000 + SECOND).sample().unwrap();
    assert_eq!(second.timestamp_ns, 1_000 + SECOND);
    assert_eq!(second.has_speed, 1);
    assert!(second.speed_mps >= 0.0);

    // A late poll gets the latest tick that is due, stamped with its own time.
    let late = poll(session, 1_000 + 3 * SECOND + 500 * MS)
        .sample()
        .unwrap();
    assert_eq!(late.timestamp_ns, 1_000 + 3 * SECOND);
    assert_eq!(deadline(session).value(), Some(1_000 + 4 * SECOND));

    // Paused and stopped sessions have no deadline and no sample.
    assert_eq!(locsim_session_pause(session, 1_000 + 4 * SECOND), LOCSIM_OK);
    assert_eq!(deadline(session).value(), None);
    assert!(poll(session, 1_000 + 9 * SECOND).sample().is_none());
    assert_eq!(locsim_session_stop(session), LOCSIM_OK);
    assert_eq!(deadline(session).value(), None);

    // A second start is a second run.
    assert_eq!(locsim_session_start(session, 50 * SECOND), LOCSIM_OK);
    let again = poll(session, 50 * SECOND).sample().unwrap();
    assert_eq!((again.timestamp_ns, again.run), (50 * SECOND, 2));
    assert_eq!(again.has_speed, 0);

    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

#[test]
fn negative_times_are_times_like_any_other() {
    let _guard = shared();
    let session = open();
    assert_eq!(locsim_session_start(session, -3 * SECOND), LOCSIM_OK);
    for expected in [-3 * SECOND, -2 * SECOND, -SECOND, 0, SECOND] {
        assert_eq!(deadline(session).value(), Some(expected));
        let sample = poll(session, expected).sample().unwrap();
        assert_eq!(sample.timestamp_ns, expected);
    }
    assert_eq!(locsim_session_start(session, i64::MIN), 31);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

#[test]
fn calls_the_lifecycle_does_not_allow_are_refused_and_change_nothing() {
    let _guard = shared();
    let session = open();
    // Stopped: only start is allowed.
    assert_eq!(locsim_session_stop(session), 31);
    assert_eq!(locsim_session_pause(session, 0), 31);
    assert_eq!(locsim_session_resume(session, 0), 31);
    assert_eq!(locsim_session_start(session, 0), LOCSIM_OK);
    // Running.
    assert_eq!(locsim_session_start(session, 0), 31);
    assert_eq!(locsim_session_resume(session, 0), 31);
    assert_eq!(poll(session, 0).sample().unwrap().timestamp_ns, 0);
    assert_eq!(locsim_session_pause(session, 2 * SECOND), LOCSIM_OK);
    // Paused.
    assert_eq!(locsim_session_pause(session, 2 * SECOND), 31);
    assert_eq!(locsim_session_start(session, 2 * SECOND), 31);
    assert_eq!(locsim_session_resume(session, SECOND), 44);
    // Still paused after the refused resume: the same call is refused again.
    assert_eq!(locsim_session_resume(session, SECOND), 44);
    assert!(poll(session, 9 * SECOND).sample().is_none());
    assert_eq!(locsim_session_resume(session, 9 * SECOND), LOCSIM_OK);
    assert!(deadline(session).value().is_some());
    assert_eq!(locsim_session_stop(session), LOCSIM_OK);
    assert_eq!(locsim_session_stop(session), 31);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

// --- Handles ------------------------------------------------------------

#[test]
fn handles_that_name_no_session_are_refused_by_every_operation() {
    let _guard = shared();
    let destroyed = open();
    assert_eq!(locsim_session_start(destroyed, 0), LOCSIM_OK);
    assert_eq!(locsim_session_destroy(destroyed), LOCSIM_OK);
    for handle in [0, u64::MAX, 1 << 32, destroyed] {
        assert_all_refused(handle, LOCSIM_ERR_INVALID_HANDLE);
    }
}

#[test]
fn a_stale_handle_does_not_reach_the_session_that_reuses_its_slot() {
    let _guard = alone();
    let old = open();
    assert_eq!(locsim_session_destroy(old), LOCSIM_OK);
    let new = open();
    assert_eq!(new as u32, old as u32);
    assert_all_refused(old, LOCSIM_ERR_INVALID_HANDLE);
    // The new session was not started by the call on the old handle.
    assert_eq!(locsim_session_start(new, 0), LOCSIM_OK);
    assert_eq!(locsim_session_destroy(new), LOCSIM_OK);
}

// --- Pointers, sizes and the order of checks ----------------------------

#[test]
fn poll_checks_pointers_then_the_struct_size_then_the_handle() {
    let _guard = shared();
    let session = open();
    assert_eq!(locsim_session_start(session, 0), LOCSIM_OK);

    // A null pointer, with a wrong size and a handle that names nothing.
    let mut has = BYTE_BEFORE;
    let mut room = SampleRoom::new(7);
    // SAFETY: the pointers that are not null are to live locals.
    let status = unsafe { locsim_session_poll(0, 0, null_mut(), &mut has) };
    assert_eq!((status, has), (LOCSIM_ERR_NULL_POINTER, BYTE_BEFORE));
    // SAFETY: as above.
    let status = unsafe { locsim_session_poll(0, 0, room.0.as_mut_ptr().cast(), null_mut()) };
    assert_eq!(status, LOCSIM_ERR_NULL_POINTER);
    assert!(room.0[4..].iter().all(|byte| *byte == BYTE_BEFORE));

    // A wrong size, with a handle that names nothing; then with a good one.
    for handle in [0, session] {
        for struct_size in [0, 87, 89, u32::MAX] {
            let polled = poll_sized(handle, 0, struct_size);
            assert_eq!(polled.status, LOCSIM_ERR_STRUCT_SIZE);
            assert!(polled.untouched());
        }
    }
    // None of the refused calls took the sample that was due.
    assert_eq!(poll(session, 0).sample().unwrap().timestamp_ns, 0);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

#[test]
fn next_deadline_checks_pointers_before_the_handle() {
    let _guard = shared();
    let session = open();
    assert_eq!(locsim_session_start(session, 0), LOCSIM_OK);
    for handle in [0, session] {
        let mut at = DEADLINE_BEFORE;
        let mut has = BYTE_BEFORE;
        // SAFETY: the pointers that are not null are to live locals.
        let status = unsafe { locsim_session_next_deadline(handle, null_mut(), &mut has) };
        assert_eq!((status, has), (LOCSIM_ERR_NULL_POINTER, BYTE_BEFORE));
        // SAFETY: as above.
        let status = unsafe { locsim_session_next_deadline(handle, &mut at, null_mut()) };
        assert_eq!((status, at), (LOCSIM_ERR_NULL_POINTER, DEADLINE_BEFORE));
    }
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

#[test]
fn a_failed_poll_writes_nothing() {
    let _guard = shared();
    let session = open();
    let start = i64::MAX - SECOND / 2;
    assert_eq!(locsim_session_start(session, start), LOCSIM_OK);
    assert_eq!(poll(session, start).sample().unwrap().timestamp_ns, start);
    // The next tick lies beyond the last representable time.
    assert_eq!(deadline(session).value(), None);
    let failed = poll(session, i64::MAX);
    assert_eq!(failed.status, LOCSIM_ERR_SCHEDULE_OVERFLOW);
    assert!(failed.untouched());
    // Reported once.
    assert!(poll(session, i64::MAX).sample().is_none());
    assert_eq!(deadline(session).value(), None);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

// --- Two callers at once ------------------------------------------------

#[test]
fn a_session_in_use_is_busy_for_every_other_call_and_none_of_them_waits() {
    let _guard = shared();
    let session = open();
    assert_eq!(locsim_session_start(session, 0), LOCSIM_OK);

    let (gate, latch) = gate();
    let holder = std::thread::spawn(move || {
        latch.stop_at(Stop::HoldingLock);
        poll(session, 0)
    });
    // From here until the gate opens, the other thread holds the session.
    gate.wait_until_reached();
    assert_all_refused(session, LOCSIM_ERR_BUSY);
    assert_eq!(locsim_session_destroy(session), LOCSIM_ERR_BUSY);
    gate.open();

    // The call that held the session was not disturbed, and the refused
    // ones did nothing: the sample due at 0 went to the holder, the session
    // is still running, and the next sample is the one due a second later.
    let held = holder.join().unwrap();
    assert_eq!(held.sample().unwrap().timestamp_ns, 0);
    assert_eq!(deadline(session).value(), Some(SECOND));
    assert_eq!(poll(session, SECOND).sample().unwrap().run, 1);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

#[test]
fn a_call_that_found_its_session_before_it_was_destroyed_does_nothing() {
    let _guard = shared();
    let session = open();
    assert_eq!(locsim_session_start(session, 0), LOCSIM_OK);

    let (gate, latch) = gate();
    let late = std::thread::spawn(move || {
        latch.stop_at(Stop::BeforeLock);
        poll(session, 0)
    });
    // The other thread has the session's cell and has not locked it.
    gate.wait_until_reached();
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
    gate.open();

    let polled = late.join().unwrap();
    assert_eq!(polled.status, LOCSIM_ERR_INVALID_HANDLE);
    assert!(polled.untouched());
}

// --- Panics -------------------------------------------------------------

#[test]
fn a_panic_in_an_operation_poisons_the_session_and_releases_it_at_once() {
    let _guard = alone();
    let session = open();
    let other = open();
    assert_eq!(locsim_session_start(session, 0), LOCSIM_OK);
    assert_eq!(locsim_session_start(other, 0), LOCSIM_OK);
    take_session_drop();

    arm(Point::Operation);
    let panicked = poll(session, 0);
    assert_eq!(panicked.status, LOCSIM_ERR_PANIC);
    assert!(panicked.untouched());
    // Released by that call, after the lock, with no panic unwinding; and
    // the lock itself was let go in the ordinary way.
    assert_eq!(take_session_drop(), Some(RELEASED_ALONE));
    assert!(!registry().get(session).unwrap().is_poisoned());

    assert_all_refused(session, LOCSIM_ERR_POISONED);
    assert_all_refused(session, LOCSIM_ERR_POISONED);
    // Nothing was left to release, and the other session is as it was.
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
    assert_eq!(take_session_drop(), None);
    assert_all_refused(session, LOCSIM_ERR_INVALID_HANDLE);
    assert_eq!(poll(other, 0).sample().unwrap().timestamp_ns, 0);
    assert_eq!(locsim_session_destroy(other), LOCSIM_OK);
}

#[test]
fn a_panic_outside_the_operation_poisons_the_lock_and_the_next_call_deals_with_it() {
    let _guard = alone();
    let session = open();
    assert_eq!(locsim_session_start(session, 0), LOCSIM_OK);
    take_session_drop();

    arm(Point::HoldingLock);
    assert_eq!(locsim_session_pause(session, 0), LOCSIM_ERR_PANIC);
    // Caught by the catch around the whole call: the guard was dropped
    // while unwinding, so the lock is poisoned and the session still there.
    assert!(registry().get(session).unwrap().is_poisoned());
    assert_eq!(take_session_drop(), None);

    // The next call finds the poisoned lock, releases the session after
    // the lock and outside any panic, and says what happened.
    let polled = poll(session, 0);
    assert_eq!(polled.status, LOCSIM_ERR_POISONED);
    assert!(polled.untouched());
    assert_eq!(take_session_drop(), Some(RELEASED_ALONE));
    assert_all_refused(session, LOCSIM_ERR_POISONED);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
    assert_eq!(take_session_drop(), None);
}

#[test]
fn the_sample_struct_has_the_documented_layout() {
    let sample = LocsimSample {
        struct_size: 88,
        source: 0,
        simulation_state: 0,
        has_speed: 0,
        has_course: 0,
        reserved: [0; 2],
        timestamp_ns: 0,
        run: 0,
        latitude_deg: 0.0,
        longitude_deg: 0.0,
        altitude_m: 0.0,
        horizontal_accuracy_m: 0.0,
        vertical_accuracy_m: 0.0,
        speed_mps: 0.0,
        course_deg: 0.0,
    };
    let base = std::ptr::addr_of!(sample) as usize;
    let offsets = [
        (std::ptr::addr_of!(sample.struct_size) as usize, 0),
        (std::ptr::addr_of!(sample.source) as usize, 4),
        (std::ptr::addr_of!(sample.simulation_state) as usize, 8),
        (std::ptr::addr_of!(sample.has_speed) as usize, 12),
        (std::ptr::addr_of!(sample.has_course) as usize, 13),
        (std::ptr::addr_of!(sample.reserved) as usize, 14),
        (std::ptr::addr_of!(sample.timestamp_ns) as usize, 16),
        (std::ptr::addr_of!(sample.run) as usize, 24),
        (std::ptr::addr_of!(sample.latitude_deg) as usize, 32),
        (std::ptr::addr_of!(sample.longitude_deg) as usize, 40),
        (std::ptr::addr_of!(sample.altitude_m) as usize, 48),
        (
            std::ptr::addr_of!(sample.horizontal_accuracy_m) as usize,
            56,
        ),
        (std::ptr::addr_of!(sample.vertical_accuracy_m) as usize, 64),
        (std::ptr::addr_of!(sample.speed_mps) as usize, 72),
        (std::ptr::addr_of!(sample.course_deg) as usize, 80),
    ];
    for (address, offset) in offsets {
        assert_eq!(address - base, offset);
    }
    assert_eq!(std::mem::size_of::<LocsimSample>(), 88);
    assert_eq!(std::mem::align_of::<LocsimSample>(), 8);
}

// --- Health, faults and events -------------------------------------------

/// A session's worth of everything the supervisor reports on: faults
/// reported and cleared, a refused call, a failing run and its restart by
/// `check`, a time that goes backwards, a stall, a late poll, a pause.
fn health_script() -> Vec<Step> {
    let half = |k: i64| k * 500 * MS;
    let mut steps = vec![
        Step::Check(0),
        Step::Report(0, 1),
        Step::Start(0),
        Step::Start(0),
    ];
    steps.extend((0..=5).map(|k| Step::Poll(half(k))));
    steps.extend([
        Step::Report(2_500 * MS, 2),
        Step::Report(2_600 * MS, 2),
        Step::Clear(2_700 * MS, 1),
        Step::Clear(2_800 * MS, 1),
        Step::Poll(3 * SECOND),
        Step::Check(3_500 * MS),
        Step::Check(4 * SECOND),
    ]);
    steps.extend((8..=16).map(|k| Step::Poll(half(k))));
    steps.extend([
        // Earlier than the call before it.
        Step::Check(7 * SECOND),
        // Nothing for seven seconds.
        Step::Check(15 * SECOND),
        Step::Poll(15 * SECOND),
    ]);
    steps.extend((31..=80).map(|k| Step::Poll(half(k))));
    steps.extend([
        Step::Clear(40 * SECOND, 2),
        Step::Pause(40_200 * MS),
        Step::Check(41 * SECOND),
        Step::Resume(42 * SECOND),
        Step::Poll(43 * SECOND),
        Step::Stop,
        Step::Check(45 * SECOND),
    ]);
    steps
}

#[test]
fn health_and_events_follow_the_supervisor_through_faults_a_failure_and_a_stall() {
    let _guard = shared();
    for (n, example) in EXAMPLES.into_iter().enumerate() {
        let scenario = locsim_scenario::import_scenario(example).unwrap();
        let label = format!("example {n}, health script");
        compare_failing(&scenario, 1, health_script(), &label);
    }

    // On fixed.json, where nothing but the injected failure goes wrong,
    // the script reaches every state it was written to reach.
    let scenario = locsim_scenario::import_scenario(FIXED).unwrap();
    let seen = compare_failing(&scenario, 1, health_script(), "fixed.json");
    let kinds: BTreeSet<u32> = seen.events.iter().map(|event| event.kind).collect();
    // All but a restart that cannot start (10) and giving up (12).
    let expected: BTreeSet<u32> = (1..=20).filter(|kind| ![10, 12].contains(kind)).collect();
    assert_eq!(kinds, expected);
    let healths: BTreeSet<u32> = seen.healths.iter().map(|report| report.health).collect();
    assert_eq!(healths, BTreeSet::from([0, 1, 2, 4]));
    let causes = seen
        .healths
        .iter()
        .fold(0, |bits, report| bits | report.causes);
    assert_eq!(causes, 1 | 2 | 4 | 8);
    let emissions: BTreeSet<u32> = seen.healths.iter().map(|report| report.emission).collect();
    assert_eq!(emissions, BTreeSet::from([0, 1, 2]));
    assert_eq!(seen.runs, [1, 2]);
}

/// A provider whose model cannot be built the second time it is asked.
fn cannot_restart(
    scenario: &Scenario,
    sink: Box<dyn EventSink + Send>,
) -> Supervisor<SimulationProvider> {
    let built = AtomicU32::new(0);
    let provider = SimulationProvider::with_model_factory(
        scenario.clone(),
        Box::new(
            move |scenario| match built.fetch_add(1, Ordering::Relaxed) {
                0 => Ok(Box::new(FailsAt {
                    real: model_for(scenario)?,
                    calls: 0,
                    fail_at: 3,
                })),
                1 => Err(MovementError::InvalidConfiguration("injected failure")),
                _ => model_for(scenario),
            },
        ),
    );
    Supervisor::for_simulation(provider, policy().to_health(), sink).unwrap()
}

#[test]
fn a_restart_that_cannot_start_is_an_event_with_the_error_code() {
    let _guard = shared();
    let scenario = locsim_scenario::import_scenario(FIXED).unwrap();
    let events = MemorySink::new();
    let direct = cannot_restart(&scenario, Box::new(events.clone()));
    let session = adopt(16, |sink| cannot_restart(&scenario, sink));
    let seen = compare(session, direct, events, long_script(), "cannot restart");
    let refused: Vec<&LocsimEvent> = seen.events.iter().filter(|e| e.kind == 10).collect();
    assert_eq!(refused.len(), 1);
    // The first attempt, an error, the movement error's code.
    assert_eq!(refused[0].arg, [1, 0, 0, 0]);
    assert_eq!((refused[0].level, refused[0].error), (0, 33));
}

#[test]
fn a_fault_is_reported_by_its_code_and_shows_in_health_and_events() {
    let _guard = shared();
    let session = open();
    // Stopped: a fault is listed, and the health is still "stopped".
    assert_eq!(locsim_session_report_fault(session, 10, 3), LOCSIM_OK);
    let stopped = health(session).report();
    assert_eq!((stopped.health, stopped.causes), (4, 0));
    assert_eq!(stopped.active_faults, 0b100);
    assert_eq!((stopped.has_observed_at, stopped.observed_at_ns), (1, 10));

    // With a run, it degrades the health and does nothing else.
    assert_eq!(locsim_session_start(session, 20), LOCSIM_OK);
    assert_eq!(poll(session, 20).sample().unwrap().timestamp_ns, 20);
    let degraded = health(session).report();
    assert_eq!((degraded.health, degraded.lifecycle), (1, 2));
    assert_eq!((degraded.causes, degraded.emission), (8, 1));
    assert_eq!(locsim_session_report_fault(session, 30, 1), LOCSIM_OK);
    assert_eq!(health(session).report().active_faults, 0b101);

    // Clearing says whether there was one.
    assert_eq!(clear_fault(session, 40, 3), (LOCSIM_OK, 1));
    assert_eq!(clear_fault(session, 50, 3), (LOCSIM_OK, 0));
    assert_eq!(clear_fault(session, 60, 2), (LOCSIM_OK, 0));
    assert_eq!(health(session).report().active_faults, 0b001);
    assert_eq!(clear_fault(session, 70, 1), (LOCSIM_OK, 1));
    let healthy = health(session).report();
    assert_eq!(
        (healthy.health, healthy.causes, healthy.active_faults),
        (0, 0, 0)
    );

    // The events: the component's code, and nothing a caller wrote.
    let faults: Vec<(u32, i64, [i64; 4])> = drain(session)
        .into_iter()
        .filter(|event| [17, 18].contains(&event.kind))
        .map(|event| (event.kind, event.at_ns, event.arg))
        .collect();
    assert_eq!(
        faults,
        [
            (17, 10, [3, 0, 0, 0]),
            (17, 30, [1, 0, 0, 0]),
            (18, 40, [3, 0, 0, 0]),
            (18, 70, [1, 0, 0, 0]),
        ]
    );
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

#[test]
fn a_component_that_does_not_exist_is_refused_before_the_handle_is_looked_at() {
    let _guard = shared();
    let session = open();
    for handle in [0, session] {
        for code in [0, 4, 255, u32::MAX] {
            assert_eq!(
                locsim_session_report_fault(handle, 0, code),
                LOCSIM_ERR_INVALID_ARGUMENT
            );
            assert_eq!(
                clear_fault(handle, 0, code),
                (LOCSIM_ERR_INVALID_ARGUMENT, BYTE_BEFORE)
            );
        }
        // A null pointer comes before the component.
        // SAFETY: the pointer is null and must not be written.
        let status = unsafe { locsim_session_clear_fault(handle, 0, 99, null_mut()) };
        assert_eq!(status, LOCSIM_ERR_NULL_POINTER);
    }
    // Nothing was reported and no event was made.
    assert_eq!(health(session).report().active_faults, 0);
    assert!(drain(session).is_empty());
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

// --- The queue ----------------------------------------------------------

fn open_sized(event_capacity: u32) -> u64 {
    let policy = policy();
    let mut session = 0;
    let mut err_len = 0;
    // SAFETY: every pointer is to a live local or to the document.
    let status = unsafe {
        crate::session::locsim_session_create(
            FIXED.as_ptr(),
            FIXED.len(),
            &policy,
            event_capacity,
            &mut session,
            null_mut(),
            0,
            &mut err_len,
        )
    };
    assert_eq!(status, LOCSIM_OK);
    session
}

#[test]
fn a_full_queue_keeps_the_newest_events_in_order_and_counts_what_it_dropped() {
    let _guard = shared();
    // What a short session reports, from a supervisor of its own.
    let drive = |start: &mut dyn FnMut(i64), poll: &mut dyn FnMut(i64), stop: &mut dyn FnMut()| {
        for round in 0..3 {
            let origin = round * 10 * SECOND;
            start(origin);
            (0..3).for_each(|k| poll(origin + k * SECOND));
            stop();
        }
    };
    let (mut direct, events) = reference(FIXED);
    {
        let direct = std::cell::RefCell::new(&mut direct);
        drive(
            &mut |now| {
                direct
                    .borrow_mut()
                    .start(Timestamp::from_nanos(now))
                    .unwrap()
            },
            &mut |now| {
                let _sample = direct
                    .borrow_mut()
                    .poll(Timestamp::from_nanos(now))
                    .unwrap();
            },
            &mut || direct.borrow_mut().stop().unwrap(),
        );
    }
    let all: Vec<LocsimEvent> = events.events().iter().map(expected_event).collect();
    assert!(all.len() >= 12, "{}", all.len());

    for capacity in [1, 2, 5, all.len() as u32 - 1, all.len() as u32, 4096] {
        let session = open_sized(capacity);
        assert_eq!(dropped(session), (LOCSIM_OK, 0));
        drive(
            &mut |now| assert_eq!(locsim_session_start(session, now), LOCSIM_OK),
            &mut |now| assert!(poll(session, now).sample().is_some()),
            &mut || assert_eq!(locsim_session_stop(session), LOCSIM_OK),
        );
        let kept = all.len().min(capacity as usize);
        let lost = (all.len() - kept) as u64;
        assert_eq!(dropped(session), (LOCSIM_OK, lost), "capacity {capacity}");
        assert_eq!(
            drain(session),
            all[all.len() - kept..],
            "capacity {capacity}"
        );
        // Empty now: nothing is written, and the count stays.
        let empty = next_event(session);
        assert_eq!((empty.status, empty.has), (LOCSIM_OK, 0));
        assert!(empty.room.untouched(64));
        assert_eq!(dropped(session), (LOCSIM_OK, lost), "capacity {capacity}");
        // The health's totals never depended on the queue.
        let report = health(session).report();
        assert_eq!((report.total_runs_started, report.total_samples), (3, 9));
        assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
    }
}

#[test]
fn events_leave_the_queue_one_at_a_time_and_room_is_reused() {
    let _guard = shared();
    let session = open_sized(2);
    // A refused stop is one event.
    let refuse = || assert_eq!(locsim_session_stop(session), 31);
    refuse();
    refuse();
    let first = next_event(session).event().unwrap();
    assert_eq!((first.kind, first.error, first.arg), (6, 31, [1, 0, 0, 0]));
    assert_eq!((first.level, first.run, first.struct_size), (1, 0, 64));
    refuse();
    // Two waiting, room for two: nothing was dropped.
    assert_eq!(dropped(session), (LOCSIM_OK, 0));
    refuse();
    assert_eq!(dropped(session), (LOCSIM_OK, 1));
    assert_eq!(drain(session).len(), 2);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

#[test]
fn no_event_and_no_report_holds_a_coordinate_that_was_emitted() {
    let _guard = shared();
    for (n, example) in EXAMPLES.into_iter().enumerate() {
        let seen = run_side_by_side(example, 0);
        assert!(!seen.samples.is_empty() && !seen.events.is_empty());
        // Every number a sample carries, as the integer with the same bits.
        let emitted: BTreeSet<i64> = seen
            .samples
            .iter()
            .flat_map(|sample| [sample.latitude_deg, sample.longitude_deg, sample.altitude_m])
            .map(|value| value.to_bits() as i64)
            .collect();
        for event in &seen.events {
            let numbers = [event.at_ns, event.run as i64].into_iter().chain(event.arg);
            for number in numbers {
                assert!(!emitted.contains(&number), "example {n}: {event:?}");
            }
        }
        for report in &seen.healths {
            // SAFETY: a `LocsimHealth` is 192 bytes without padding.
            let words: [u64; 24] = unsafe { std::mem::transmute(*report) };
            for word in words {
                assert!(!emitted.contains(&(word as i64)), "example {n}");
            }
        }
    }
}

// --- Pointers, sizes and the order of checks, again ----------------------

#[test]
fn health_and_check_test_the_pointer_then_the_struct_size_then_the_handle() {
    let _guard = shared();
    let session = open();
    for handle in [0, session] {
        // SAFETY: the pointer is null and must not be written.
        let status = unsafe { locsim_session_health(handle, null_mut()) };
        assert_eq!(status, LOCSIM_ERR_NULL_POINTER);
        // SAFETY: as above.
        let status = unsafe { locsim_session_check(handle, 5, null_mut()) };
        assert_eq!(status, LOCSIM_ERR_NULL_POINTER);
        for struct_size in [0, 191, 193, 88, u32::MAX] {
            let reported = health_sized(handle, struct_size);
            assert_eq!(reported.status, LOCSIM_ERR_STRUCT_SIZE);
            assert!(reported.untouched());
            let checked = check_sized(handle, 5, struct_size);
            assert_eq!(checked.status, LOCSIM_ERR_STRUCT_SIZE);
            assert!(checked.untouched());
        }
    }
    // The refused checks did not reach the session: it has seen no time.
    assert_eq!(health(session).report().has_observed_at, 0);
    let checked = check(session, 5).report();
    assert_eq!((checked.has_observed_at, checked.observed_at_ns), (1, 5));
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

#[test]
fn the_event_functions_test_pointers_then_the_struct_size_then_the_handle() {
    let _guard = shared();
    let session = open();
    assert_eq!(locsim_session_stop(session), 31);
    for handle in [0, session] {
        let mut has = BYTE_BEFORE;
        let mut room = Room::<64>::new(3);
        // SAFETY: the pointers that are not null are to live locals.
        let status = unsafe { locsim_session_next_event(handle, null_mut(), &mut has) };
        assert_eq!((status, has), (LOCSIM_ERR_NULL_POINTER, BYTE_BEFORE));
        // SAFETY: as above.
        let status =
            unsafe { locsim_session_next_event(handle, room.0.as_mut_ptr().cast(), null_mut()) };
        assert_eq!(status, LOCSIM_ERR_NULL_POINTER);
        assert!(room.untouched(3));
        // SAFETY: the pointer is null and must not be written.
        let status = unsafe { locsim_session_events_dropped(handle, null_mut()) };
        assert_eq!(status, LOCSIM_ERR_NULL_POINTER);
        for struct_size in [0, 63, 65, u32::MAX] {
            let taken = next_event_sized(handle, struct_size);
            assert_eq!(taken.status, LOCSIM_ERR_STRUCT_SIZE);
            assert!(taken.untouched());
        }
    }
    // None of the refused calls took the event that was waiting.
    assert_eq!(drain(session).len(), 1);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

// --- A panic while an event is being recorded -----------------------------

#[test]
fn a_panic_inside_the_event_sink_poisons_the_session_and_holds_no_lock() {
    let _guard = alone();
    let session = open();
    let other = open();
    take_session_drop();

    // Starting reports events; the first of them panics with the queue
    // locked, deep inside the supervisor.
    arm(Point::EventRecord);
    assert_eq!(locsim_session_start(session, 0), LOCSIM_ERR_PANIC);
    // The session was released by that call, after its lock, with the
    // registry free and no panic unwinding.
    assert_eq!(take_session_drop(), Some(RELEASED_ALONE));
    assert!(!registry().get(session).unwrap().is_poisoned());
    // Not busy: no lock was left held. Poisoned: nothing can be read from it.
    assert_all_refused(session, LOCSIM_ERR_POISONED);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
    assert_all_refused(session, LOCSIM_ERR_INVALID_HANDLE);

    // Another session is as it was, and its events arrive.
    assert_eq!(locsim_session_start(other, 0), LOCSIM_OK);
    assert!(!drain(other).is_empty());
    assert_eq!(locsim_session_destroy(other), LOCSIM_OK);
}

#[test]
fn the_health_struct_has_the_documented_layout() {
    let (direct, _) = reference(FIXED);
    let report = expected_health(&direct.health());
    let base = std::ptr::addr_of!(report) as usize;
    let offsets = [
        (std::ptr::addr_of!(report.struct_size) as usize, 0),
        (std::ptr::addr_of!(report.lifecycle) as usize, 4),
        (std::ptr::addr_of!(report.health) as usize, 8),
        (std::ptr::addr_of!(report.emission) as usize, 12),
        (std::ptr::addr_of!(report.causes) as usize, 16),
        (std::ptr::addr_of!(report.active_faults) as usize, 20),
        (std::ptr::addr_of!(report.attempts_used) as usize, 24),
        (std::ptr::addr_of!(report.max_attempts) as usize, 28),
        (std::ptr::addr_of!(report.last_error) as usize, 32),
        (std::ptr::addr_of!(report.current_state) as usize, 36),
        (std::ptr::addr_of!(report.has_observed_at) as usize, 40),
        (std::ptr::addr_of!(report.has_retry_at) as usize, 41),
        (std::ptr::addr_of!(report.has_last_sample_time) as usize, 42),
        (std::ptr::addr_of!(report.trajectory_complete) as usize, 43),
        (std::ptr::addr_of!(report.reserved) as usize, 44),
        (std::ptr::addr_of!(report.observed_at_ns) as usize, 48),
        (std::ptr::addr_of!(report.retry_at_ns) as usize, 56),
        (std::ptr::addr_of!(report.silent_for_ns) as usize, 64),
        (std::ptr::addr_of!(report.last_sample_time_ns) as usize, 72),
        (std::ptr::addr_of!(report.run) as usize, 80),
        (std::ptr::addr_of!(report.current_sample_count) as usize, 88),
        (std::ptr::addr_of!(report.current_failed_count) as usize, 96),
        (
            std::ptr::addr_of!(report.current_missed_ticks) as usize,
            104,
        ),
        (std::ptr::addr_of!(report.total_runs_started) as usize, 112),
        (std::ptr::addr_of!(report.total_runs_ended) as usize, 120),
        (std::ptr::addr_of!(report.total_samples) as usize, 128),
        (std::ptr::addr_of!(report.total_withheld) as usize, 136),
        (std::ptr::addr_of!(report.total_missed_ticks) as usize, 144),
        (std::ptr::addr_of!(report.total_failures) as usize, 152),
        (
            std::ptr::addr_of!(report.total_recovery_attempts) as usize,
            160,
        ),
        (
            std::ptr::addr_of!(report.total_recoveries_completed) as usize,
            168,
        ),
        (
            std::ptr::addr_of!(report.total_terminal_failures) as usize,
            176,
        ),
        (
            std::ptr::addr_of!(report.total_rejected_operations) as usize,
            184,
        ),
    ];
    for (address, offset) in offsets {
        assert_eq!(address - base, offset);
    }
    assert_eq!(std::mem::size_of::<LocsimHealth>(), 192);
    assert_eq!(std::mem::align_of::<LocsimHealth>(), 8);
}
