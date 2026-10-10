//! T10: the supervisor's lifecycle, what it reports, and the difference
//! between a call that is refused and a run that fails.

mod support;

use locsim_core::domain::{HealthState, InvalidTransition, SimulationState};
use locsim_core::provider::{LocationProvider, ProviderError, SimulationProvider};
use locsim_core::scheduler::ScheduleError;
use locsim_core::validation::ValidationError;
use locsim_health::{
    Emission, EventKind, FailReason, HealthPolicy, Level, MemorySink, NullSink, Operation,
    RunEndReason, Supervisor,
};
use locsim_scenario::import_scenario;
use support::scenario::example;
use support::*;

fn transition(from: SimulationState, to: SimulationState) -> ProviderError {
    ProviderError::Transition(InvalidTransition { from, to })
}

// --- Construction -----------------------------------------------------------------

#[test]
fn an_invalid_policy_is_refused_with_every_problem() {
    let bad = HealthPolicy {
        backoff_multiplier: 0.0,
        stall_after_s: f64::NAN,
        ..policy()
    };
    let errors = Supervisor::new(Scripted::new(), bad, Box::new(NullSink))
        .err()
        .expect("accepted");
    let fields: Vec<_> = errors.iter().map(|e| e.field).collect();
    assert_eq!(fields, ["backoff_multiplier", "stall_after_s"]);
}

#[test]
fn a_new_supervisor_is_stopped_and_has_done_nothing() {
    let (supervisor, provider, events) = scripted(policy());
    let report = check(&supervisor, &events);
    assert_eq!(report.health, HealthState::Stopped);
    assert_eq!(report.lifecycle, SimulationState::Idle);
    assert_eq!(report.emission, Emission::NotExpected);
    assert_eq!((report.run, report.observed_at), (0, None));
    assert_eq!(report.totals, Default::default());
    assert_eq!(report.max_attempts, 3);
    assert!(events.is_empty() && provider.calls().is_empty());
}

// --- The ordinary path --------------------------------------------------------------

#[test]
fn start_poll_pause_resume_stop() {
    let (mut s, provider, events) = scripted(policy());

    s.start(t(T0)).unwrap();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Healthy);
    assert_eq!(report.lifecycle, SimulationState::Running);
    assert_eq!(report.emission, Emission::Flowing);
    assert_eq!(report.run, 1);

    for tick in 0..3 {
        let sample = s.poll(t(T0 + tick * SEC)).unwrap().expect("due");
        assert_eq!(sample, sample_at(T0 + tick * SEC));
        assert_eq!(s.current_location(), Some(sample));
        check(&s, &events);
    }
    assert_eq!(s.status().sample_count, 3);

    s.pause(t(T0 + 3 * SEC)).unwrap();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Healthy);
    assert_eq!(report.lifecycle, SimulationState::Paused);
    assert_eq!(report.emission, Emission::NotExpected);
    // A paused supervisor does not poll the provider.
    let calls = provider.calls().len();
    assert_eq!(s.poll(t(T0 + 4 * SEC)), Ok(None));
    assert_eq!(provider.calls().len(), calls);

    s.resume(t(T0 + 10 * SEC)).unwrap();
    assert_eq!(check(&s, &events).lifecycle, SimulationState::Running);
    s.poll(t(T0 + 10 * SEC)).unwrap().expect("due");

    s.stop().unwrap();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Stopped);
    assert_eq!(report.lifecycle, SimulationState::Idle);
    assert_eq!(report.run, 1, "the run number is that of the latest run");
    assert_eq!(s.current_location(), None);
    assert_eq!(s.poll(t(T0 + 11 * SEC)), Ok(None));
    assert_eq!(report.totals.samples, 4);
    assert_eq!(
        (report.totals.runs_started, report.totals.runs_ended),
        (1, 1)
    );

    assert_eq!(
        kinds(&events),
        [
            EventKind::RunStarted {
                recovery_attempt: None
            },
            EventKind::Paused,
            EventKind::Resumed,
            EventKind::RunEnded {
                samples: 4,
                withheld: 0,
                missed: 0,
                reason: RunEndReason::Stopped
            },
            EventKind::Stopped,
        ]
    );
    // Health was announced as it changed, and only then.
    let health: Vec<_> = events
        .events()
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::HealthChanged { from, to } => Some((from, to)),
            _ => None,
        })
        .collect();
    assert_eq!(
        health,
        [
            (HealthState::Stopped, HealthState::Healthy),
            (HealthState::Healthy, HealthState::Stopped)
        ]
    );
    assert_eq!(provider.state(), SimulationState::Idle);
}

#[test]
fn events_carry_the_time_of_their_call_and_stop_the_latest_time_seen() {
    let (mut s, _, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    s.poll(t(T0 + 7 * SEC)).unwrap();
    s.stop().unwrap();
    let recorded = events.events();
    assert_eq!(recorded[0].at, t(T0));
    assert_eq!(recorded[0].run, 1);
    for event in recorded
        .iter()
        .filter(|e| matches!(e.kind, EventKind::RunEnded { .. } | EventKind::Stopped))
    {
        assert_eq!(event.at, t(T0 + 7 * SEC), "{event}");
    }
    assert!(recorded.iter().all(|e| e.level == e.kind.level()));
}

#[test]
fn a_second_session_is_a_new_run_with_a_new_number_and_fresh_counts() {
    let (mut s, _, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    s.poll(t(T0)).unwrap();
    s.poll(t(T0 + SEC)).unwrap();
    s.stop().unwrap();
    // Like the core provider, a new session may begin earlier in time.
    s.start(t(T0 - 100 * SEC)).unwrap();
    let report = check(&s, &events);
    assert_eq!(report.run, 2);
    assert_eq!(s.status().sample_count, 0);
    let sample = s.poll(t(T0 - 100 * SEC)).unwrap().expect("due");
    assert_eq!(sample.timestamp, t(T0 - 100 * SEC));
    let report = check(&s, &events);
    assert_eq!(report.totals.samples, 3);
    assert_eq!(report.totals.runs_started, 2);
}

// --- Rejected operations ------------------------------------------------------------

/// Brings a supervisor into each state it can be in without recovery.
fn in_state(state: SimulationState) -> (Supervisor<Scripted>, Scripted, MemorySink) {
    let (mut s, provider, events) = scripted(no_recovery());
    match state {
        SimulationState::Idle => {}
        SimulationState::Running => s.start(t(T0)).unwrap(),
        SimulationState::Paused => {
            s.start(t(T0)).unwrap();
            s.pause(t(T0 + SEC)).unwrap();
        }
        SimulationState::Error => {
            s.start(t(T0)).unwrap();
            provider.fail_next(pipeline_error());
            s.poll(t(T0)).unwrap_err();
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.health().lifecycle, state);
    (s, provider, events)
}

#[test]
fn a_call_the_lifecycle_does_not_allow_is_refused_and_changes_nothing() {
    use SimulationState::*;
    type Call = fn(&mut Supervisor<Scripted>) -> Result<(), ProviderError>;
    let illegal: [(SimulationState, Operation, SimulationState, Call); 9] = [
        (Idle, Operation::Stop, Stopping, |s| s.stop()),
        (Idle, Operation::Pause, Paused, |s| {
            s.pause(t(T0 + 50 * SEC))
        }),
        (Idle, Operation::Resume, Running, |s| {
            s.resume(t(T0 + 50 * SEC))
        }),
        (Running, Operation::Start, Starting, |s| {
            s.start(t(T0 + 50 * SEC))
        }),
        (Running, Operation::Resume, Running, |s| {
            s.resume(t(T0 + 50 * SEC))
        }),
        (Paused, Operation::Start, Starting, |s| {
            s.start(t(T0 + 50 * SEC))
        }),
        (Paused, Operation::Pause, Paused, |s| {
            s.pause(t(T0 + 50 * SEC))
        }),
        (Error, Operation::Start, Starting, |s| {
            s.start(t(T0 + 50 * SEC))
        }),
        (Error, Operation::Pause, Paused, |s| {
            s.pause(t(T0 + 50 * SEC))
        }),
    ];
    for (state, operation, to, call) in illegal {
        let (mut s, provider, events) = in_state(state);
        let before = check(&s, &events);
        let (events_before, calls_before) = (events.len(), provider.calls());

        let error = call(&mut s).expect_err("allowed");
        assert_eq!(error, transition(state, to), "{state:?} {operation:?}");

        // Exactly one event, of the right kind; the provider was not asked.
        let recorded = events.events();
        assert_eq!(recorded.len(), events_before + 1, "{state:?} {operation:?}");
        let last = recorded.last().unwrap();
        assert_eq!(
            last.kind,
            EventKind::OperationRejected {
                operation,
                error: error.clone()
            }
        );
        assert_eq!(last.level, Level::Warn);
        assert_eq!(provider.calls(), calls_before, "{state:?} {operation:?}");

        // Nothing else moved: same state, health, run, error, and no run
        // ended or failed.
        let after = check(&s, &events);
        assert_eq!(after.lifecycle, before.lifecycle);
        assert_eq!(after.health, before.health);
        assert_eq!(after.run, before.run);
        assert_eq!(after.last_error, before.last_error);
        assert_eq!(after.attempts_used, before.attempts_used);
        assert_eq!(
            after.totals,
            locsim_health::Totals {
                rejected_operations: before.totals.rejected_operations + 1,
                ..before.totals
            }
        );
    }
}

#[test]
fn a_refused_start_leaves_the_supervisor_stopped_not_failed() {
    let (mut s, provider, events) = scripted(policy());
    provider
        .script()
        .start_results
        .push_back(Some(ProviderError::InvalidScenario(vec![])));
    assert_eq!(s.start(t(T0)), Err(ProviderError::InvalidScenario(vec![])));
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Stopped);
    assert_eq!(report.lifecycle, SimulationState::Idle);
    assert_eq!((report.run, report.attempts_used), (0, 0));
    assert_eq!(report.retry_at, None);
    assert_eq!(
        kinds(&events),
        [EventKind::OperationRejected {
            operation: Operation::Start,
            error: ProviderError::InvalidScenario(vec![])
        }]
    );
    // No run started, none failed, nothing to retry: a later start works.
    assert_eq!((report.totals.runs_started, report.totals.failures), (0, 0));
    s.start(t(T0 + SEC)).unwrap();
    assert_eq!(check(&s, &events).run, 1);
}

#[test]
fn a_real_scenario_that_cannot_start_is_a_refused_start() {
    let mut scenario = import_scenario(example("walking")).unwrap();
    scenario.update_interval_s = 0.0;
    let events = MemorySink::new();
    let mut s = Supervisor::new(
        SimulationProvider::new(scenario),
        policy(),
        Box::new(events.clone()),
    )
    .unwrap();
    assert!(matches!(
        s.start(t(T0)),
        Err(ProviderError::InvalidScenario(_))
    ));
    assert_eq!(check(&s, &events).health, HealthState::Stopped);
    assert_eq!(s.provider().status().state, SimulationState::Idle);
}

#[test]
fn a_refused_pause_or_resume_is_passed_on_and_the_run_goes_on() {
    // The real provider: resuming at a time before the pause is refused by
    // the schedule, and the provider stays paused.
    let scenario = import_scenario(example("walking")).unwrap();
    let events = MemorySink::new();
    let mut s = Supervisor::new(
        SimulationProvider::new(scenario),
        policy(),
        Box::new(events.clone()),
    )
    .unwrap();
    s.start(t(T0)).unwrap();
    s.poll(t(T0)).unwrap().expect("due");
    s.pause(t(T0 + 5 * SEC)).unwrap();

    let refused = s.resume(t(T0 + 4 * SEC));
    assert_eq!(
        refused,
        Err(ProviderError::Schedule(ScheduleError::ClockWentBackwards))
    );
    let report = check(&s, &events);
    assert_eq!(report.lifecycle, SimulationState::Paused);
    assert_eq!(report.health, HealthState::Healthy);
    assert_eq!(report.totals.failures, 0);
    assert_eq!(report.totals.rejected_operations, 1);
    assert!(kinds(&events).contains(&EventKind::OperationRejected {
        operation: Operation::Resume,
        error: ProviderError::Schedule(ScheduleError::ClockWentBackwards)
    }));

    // A valid resume, and the same run continues.
    s.resume(t(T0 + 9 * SEC)).unwrap();
    let sample = s.poll(t(T0 + 9 * SEC)).unwrap().expect("due");
    assert!(
        sample.speed_mps.is_some(),
        "not a first sample: the run went on"
    );
    assert_eq!(check(&s, &events).run, 1);
}

#[test]
fn a_provider_that_refuses_an_allowed_call_does_not_end_the_run() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider.script().pause_error = Some(pipeline_error());
    assert_eq!(s.pause(t(T0 + SEC)), Err(pipeline_error()));
    let report = check(&s, &events);
    assert_eq!(report.lifecycle, SimulationState::Running);
    assert_eq!(
        (report.totals.failures, report.totals.rejected_operations),
        (0, 1)
    );

    provider.script().pause_error = None;
    provider.script().stop_error = Some(pipeline_error());
    assert_eq!(s.stop(), Err(pipeline_error()));
    assert_eq!(check(&s, &events).lifecycle, SimulationState::Running);
    s.poll(t(T0 + 2 * SEC)).unwrap().expect("still running");
}

// --- A run that fails ---------------------------------------------------------------

#[test]
fn a_failed_poll_ends_the_run_and_with_no_retries_allowed_the_supervisor_fails() {
    let (mut s, provider, events) = scripted(no_recovery());
    s.start(t(T0)).unwrap();
    s.poll(t(T0)).unwrap().expect("due");
    provider.push(Step::Fail {
        error: pipeline_error(),
        missed: 0,
    });

    // The provider's error, once.
    assert_eq!(s.poll(t(T0 + SEC)), Err(pipeline_error()));
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Failed);
    assert_eq!(report.lifecycle, SimulationState::Error);
    assert_eq!(report.emission, Emission::NotExpected);
    assert_eq!(report.last_error, Some(pipeline_error()));
    assert_eq!(report.retry_at, None);
    // No run exists any more.
    assert_eq!(s.current_location(), None);
    assert_eq!(s.status().sample_count, 0);
    // Afterwards nothing comes and nothing is asked of the provider.
    let calls = provider.calls().len();
    for later in 2..5 {
        assert_eq!(s.poll(t(T0 + later * SEC)), Ok(None));
    }
    assert_eq!(provider.calls().len(), calls);

    assert_eq!(
        kinds(&events),
        [
            EventKind::RunStarted {
                recovery_attempt: None
            },
            EventKind::RunFailed {
                error: pipeline_error()
            },
            EventKind::RunEnded {
                samples: 1,
                withheld: 0,
                missed: 0,
                reason: RunEndReason::Failure
            },
            EventKind::Failed {
                reason: FailReason::AttemptsExhausted
            },
        ]
    );

    // Only an explicit stop gets it out; then a new session can start.
    s.stop().unwrap();
    assert_eq!(check(&s, &events).health, HealthState::Stopped);
    assert_eq!(provider.state(), SimulationState::Idle);
    s.start(t(T0 + 10 * SEC)).unwrap();
    let report = check(&s, &events);
    assert_eq!((report.run, report.health), (2, HealthState::Healthy));
    assert_eq!(report.last_error, None);
}

#[test]
fn a_sample_with_a_stale_timestamp_is_withheld_and_ends_the_run() {
    for stale in [T0 + SEC, T0, T0 - 5 * SEC] {
        let (mut s, provider, events) = scripted(no_recovery());
        s.start(t(T0)).unwrap();
        s.poll(t(T0)).unwrap().expect("due");
        let last = s.poll(t(T0 + SEC)).unwrap().expect("due");
        provider.push(Step::Emit {
            at: Some(stale),
            missed: 0,
        });

        let expected = ProviderError::Validation(ValidationError::NonMonotonicTimestamp {
            previous: last.timestamp,
            current: t(stale),
        });
        assert_eq!(s.poll(t(T0 + 2 * SEC)), Err(expected.clone()), "{stale}");
        // Not handed on in any form.
        assert_eq!(s.current_location(), None);
        let report = check(&s, &events);
        assert_eq!(report.health, HealthState::Failed);
        assert_eq!(report.last_error, Some(expected));
        assert_eq!((report.totals.samples, report.totals.withheld), (2, 1));
        assert!(kinds(&events).contains(&EventKind::RunEnded {
            samples: 2,
            withheld: 1,
            missed: 0,
            reason: RunEndReason::Failure
        }));
        // The provider believes it emitted three; the supervisor counts
        // what it handed on.
        assert_eq!(provider.script().sample_count, 3);
    }
}

#[test]
fn a_provider_whose_state_disagrees_with_the_lifecycle_fails_the_run() {
    // It claims to be idle while the supervisor has a run.
    let (mut s, provider, events) = scripted(no_recovery());
    s.start(t(T0)).unwrap();
    provider.push(Step::Nothing);
    provider.script().claims = Some(SimulationState::Idle);
    assert_eq!(s.poll(t(T0)), Ok(None));
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Failed);
    assert_eq!(
        report.last_error,
        Some(transition(SimulationState::Idle, SimulationState::Running))
    );

    // It returns an error but claims to carry on: the run is over anyway.
    let (mut s, provider, events) = scripted(no_recovery());
    s.start(t(T0)).unwrap();
    provider.script().keeps_running_after_failure = true;
    provider.fail_next(pipeline_error());
    assert_eq!(s.poll(t(T0)), Err(pipeline_error()));
    assert_eq!(check(&s, &events).health, HealthState::Failed);
    assert_eq!(s.poll(t(T0 + SEC)), Ok(None));
}

#[test]
fn ticks_skipped_by_the_failing_poll_are_counted_in_the_run_and_the_totals() {
    let (mut s, provider, events) = scripted(no_recovery());
    s.start(t(T0)).unwrap();
    provider.push(Step::Emit {
        at: None,
        missed: 2,
    });
    s.poll(t(T0 + 2 * SEC)).unwrap().expect("due");
    assert_eq!(s.status().missed_ticks, 2);
    assert_eq!(check(&s, &events).totals.missed_ticks, 2);
    // A late poll that skips five ticks and then fails.
    provider.push(Step::Fail {
        error: pipeline_error(),
        missed: 5,
    });
    s.poll(t(T0 + 8 * SEC)).unwrap_err();
    let report = check(&s, &events);
    assert_eq!(report.totals.missed_ticks, 7);
    assert!(kinds(&events).contains(&EventKind::RunEnded {
        samples: 1,
        withheld: 0,
        missed: 7,
        reason: RunEndReason::Failure
    }));
    // No verdict on a window from a run that is over.
    assert!(!kinds(&events)
        .iter()
        .any(|k| matches!(k, EventKind::FallingBehind { .. })));
}

#[test]
fn a_time_that_goes_backwards_is_noted_once_and_ignored_for_the_record() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0 + 10 * SEC)).unwrap();
    s.poll(t(T0 + 10 * SEC)).unwrap();
    for earlier in [9, 8, 9] {
        // As a real provider would: nothing is due at an earlier time.
        provider.push(Step::Nothing);
        s.poll(t(T0 + earlier * SEC)).unwrap();
    }
    assert_eq!(check(&s, &events).observed_at, Some(t(T0 + 10 * SEC)));
    s.poll(t(T0 + 11 * SEC)).unwrap();
    provider.push(Step::Nothing);
    s.poll(t(T0 + 10 * SEC)).unwrap();
    let regressions: Vec<_> = events
        .events()
        .into_iter()
        .filter(|e| matches!(e.kind, EventKind::ClockWentBackwards { .. }))
        .collect();
    assert_eq!(regressions.len(), 2, "one per excursion, not one per call");
    assert_eq!(regressions[0].at, t(T0 + 9 * SEC));
    assert_eq!(
        regressions[0].kind,
        EventKind::ClockWentBackwards {
            latest: t(T0 + 10 * SEC)
        }
    );
    assert_eq!(check(&s, &events).observed_at, Some(t(T0 + 11 * SEC)));
}
