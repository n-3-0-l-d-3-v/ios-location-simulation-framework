use super::*;
use crate::hooks::{arm, gate, take_session_drop, Point, Stop};
use crate::session::{adopt, locsim_session_destroy, registry};
use crate::testing::*;
use locsim_core::domain::{LocationSource, Scenario, SimulationState};
use locsim_core::movement::{model_for, MovementError, MovementModel, MovementSample};
use locsim_core::scheduler::ScheduleError;
use locsim_health::NullSink;
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

/// Every operation of this stage on `session`, each expected to be refused
/// with `status` and to have written nothing.
fn assert_all_refused(session: u64, status: LocsimStatus) {
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

fn reference(json: &str) -> Supervisor<SimulationProvider> {
    let scenario = locsim_scenario::import_scenario(json).unwrap();
    Supervisor::for_simulation(
        SimulationProvider::new(scenario),
        policy().to_health(),
        Box::new(NullSink),
    )
    .unwrap()
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
}

/// Makes every call of the script on a session made from `json` and on a
/// supervisor made from it, and requires the same answer from both.
fn run_side_by_side(json: &str, origin: i64) -> Seen {
    let label = format!("from {origin}");
    compare(open_with(json), reference(json), script(origin), &label)
}

/// Makes every step on `session` and on `direct`, which must have been
/// built alike, and requires the same answer from both after each one.
fn compare(
    session: u64,
    mut direct: Supervisor<SimulationProvider>,
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
        };
        seen.statuses.push(status);

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
fn failing(scenario: &Scenario, failing_runs: u32) -> Supervisor<SimulationProvider> {
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
    Supervisor::for_simulation(provider, policy().to_health(), Box::new(NullSink)).unwrap()
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
        let seen = compare(
            adopt(failing(&scenario, 2)),
            failing(&scenario, 2),
            long_script(),
            &label,
        );
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
    let seen = compare(
        adopt(failing(&scenario, 4)),
        failing(&scenario, 4),
        long_script(),
        "every allowed run fails",
    );
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
