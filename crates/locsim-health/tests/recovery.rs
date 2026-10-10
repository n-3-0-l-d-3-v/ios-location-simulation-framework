//! T10: bounded restarts. What an attempt is, when it is made, when the
//! supervisor gives up, and what a restarted run is and is not.

mod support;

use locsim_core::consistency::DeriveError;
use locsim_core::domain::{HealthState, InvalidTransition, SimulationState, SyntheticLocation};
use locsim_core::noise::NoiseError;
use locsim_core::provider::{LocationProvider, ProviderError, SimulationProvider};
use locsim_core::rng::Rng;
use locsim_core::scheduler::ScheduleError;
use locsim_core::validation::ValidationError;
use locsim_health::{
    Cause, EventKind, FailReason, HealthPolicy, MemorySink, Operation, RunEndReason, Supervisor,
};
use locsim_scenario::import_scenario;
use support::scenario::{example, fingerprint};
use support::*;

fn failing(runs: usize) -> impl Iterator<Item = Step> {
    (0..runs).map(|_| Step::Fail {
        error: pipeline_error(),
        missed: 0,
    })
}

fn scheduled(events: &MemorySink) -> Vec<(u32, i64)> {
    events
        .events()
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::RecoveryScheduled { attempt, at } => Some((attempt, at.as_nanos() - T0)),
            _ => None,
        })
        .collect()
}

// --- The bound -----------------------------------------------------------------------

#[test]
fn the_bound_counts_restart_attempts_not_runs() {
    for max in [0u32, 1, 3, 7] {
        let (mut s, provider, events) = scripted(HealthPolicy {
            max_recovery_attempts: max,
            backoff_initial_s: 0.0,
            backoff_max_s: 0.0,
            ..policy()
        });
        // Every run fails at its first poll, for more runs than can exist.
        provider.script().steps.extend(failing(max as usize + 5));
        s.start(t(T0)).unwrap();
        let mut errors = 0;
        for call in 0..(max as i64 + 10) {
            if s.poll(t(T0 + call)).is_err() {
                errors += 1;
            }
            check(&s, &events);
        }
        let report = check(&s, &events);
        // The original run and one per attempt, each failing once.
        assert_eq!(errors, max + 1, "max {max}");
        assert_eq!(report.totals.runs_started, u64::from(max) + 1);
        assert_eq!(report.totals.failures, u64::from(max) + 1);
        assert_eq!(report.totals.recovery_attempts, u64::from(max));
        assert_eq!(report.attempts_used, max);
        assert_eq!(report.health, HealthState::Failed);
        assert_eq!(report.totals.terminal_failures, 1);
        assert_eq!(report.totals.recoveries_completed, 0);
        assert_eq!(report.run, u64::from(max) + 1);
        assert_eq!(
            kinds(&events).last(),
            Some(&EventKind::Failed {
                reason: FailReason::AttemptsExhausted
            })
        );
        // Unused failures are still in the script: nobody polled for them.
        assert_eq!(provider.script().steps.len(), 4);
    }
}

#[test]
fn one_failure_one_restart_in_events() {
    let (mut s, provider, events) = scripted(HealthPolicy {
        max_recovery_attempts: 1,
        ..policy()
    });
    s.start(t(T0)).unwrap();
    s.poll(t(T0)).unwrap().expect("due");
    provider.fail_next(pipeline_error());
    assert_eq!(s.poll(t(T0 + SEC)), Err(pipeline_error()));

    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Recovering);
    assert_eq!(report.lifecycle, SimulationState::Recovering);
    assert_eq!(report.retry_at, Some(t(T0 + 2 * SEC)));
    assert_eq!(s.next_deadline(), Some(t(T0 + 2 * SEC)));
    // The attempt has been scheduled, not made.
    assert_eq!((report.attempts_used, report.run), (0, 1));
    assert_eq!(s.current_location(), None);

    let first = s.poll(t(T0 + 2 * SEC)).unwrap().expect("the new run");
    let report = check(&s, &events);
    assert_eq!(first.timestamp, t(T0 + 2 * SEC));
    assert_eq!((report.attempts_used, report.run), (1, 2));
    assert_eq!(report.health, HealthState::Degraded);
    assert_eq!(report.causes, [Cause::Probation]);
    assert_eq!(report.retry_at, None);
    assert_eq!(report.last_error, Some(pipeline_error()));

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
            EventKind::RecoveryScheduled {
                attempt: 1,
                at: t(T0 + 2 * SEC)
            },
            EventKind::RecoveryAttempted { attempt: 1 },
            EventKind::RunStarted {
                recovery_attempt: Some(1)
            },
        ]
    );
    // The provider was stopped out of its error state, then started.
    let calls = provider.calls();
    assert_eq!(
        &calls[calls.len() - 3..],
        ["stop", "start@2000000000", "poll@2000000000"]
    );
}

// --- When an attempt is made -------------------------------------------------------

#[test]
fn an_attempt_is_made_at_its_time_and_not_a_nanosecond_before() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    let due = T0 + SEC;
    let calls = provider.calls().len();
    for early in [T0, T0 + 1, due - 1] {
        assert_eq!(s.poll(t(early)), Ok(None));
        assert_eq!(check(&s, &events).health, HealthState::Recovering);
        assert_eq!(s.check(t(early)).health, HealthState::Recovering);
    }
    assert_eq!(provider.calls().len(), calls, "the provider was left alone");
    assert!(s.poll(t(due)).unwrap().is_some());
    assert_eq!(check(&s, &events).run, 2);
}

#[test]
fn delays_double_up_to_the_cap_each_measured_from_the_failure_before_it() {
    let (mut s, provider, events) = scripted(HealthPolicy {
        max_recovery_attempts: 8,
        ..policy() // 1 s, doubling, at most 60 s
    });
    provider.script().steps.extend(failing(20));
    s.start(t(T0)).unwrap();
    let mut now = T0;
    s.poll(t(now)).unwrap_err();
    // Each restart is made exactly when due and fails at once, so the next
    // delay starts from that same instant.
    let mut expected = Vec::new();
    for (attempt, delay) in [1, 2, 4, 8, 16, 32, 60, 60].into_iter().enumerate() {
        now += delay * SEC;
        expected.push((attempt as u32 + 1, now - T0));
        assert_eq!(s.next_deadline(), Some(t(now)));
        s.poll(t(now)).unwrap_err();
        check(&s, &events);
    }
    assert_eq!(scheduled(&events), expected);
    assert_eq!(check(&s, &events).health, HealthState::Failed);
}

#[test]
fn a_zero_delay_retries_at_the_next_call_even_at_the_same_time() {
    let (mut s, provider, events) = scripted(HealthPolicy {
        backoff_initial_s: 0.0,
        ..policy()
    });
    s.start(t(T0)).unwrap();
    let before = s.poll(t(T0)).unwrap().unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0 + SEC)).unwrap_err();
    assert_eq!(s.next_deadline(), Some(t(T0 + SEC)));
    let after = s
        .poll(t(T0 + SEC))
        .unwrap()
        .expect("restarted in the same instant");
    assert!(after.timestamp > before.timestamp);
    assert_eq!(check(&s, &events).run, 2);
}

// --- A restart that does not start ---------------------------------------------------

#[test]
fn a_restart_that_cannot_start_uses_its_attempt_and_the_next_is_scheduled() {
    let (mut s, provider, events) = scripted(policy()); // three attempts
    s.start(t(T0)).unwrap();
    provider.script().start_results.extend([
        Some(ProviderError::InvalidScenario(vec![])),
        Some(pipeline_error()),
        None,
    ]);
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();

    // Attempt 1 at +1 s: the start is refused. No error is returned by the
    // poll: no run failed, there was none.
    assert_eq!(s.poll(t(T0 + SEC)), Ok(None));
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Recovering);
    assert_eq!(report.attempts_used, 1);
    assert_eq!(
        report.retry_at,
        Some(t(T0 + 3 * SEC)),
        "2 s after the failed restart"
    );
    assert_eq!(
        report.last_error,
        Some(ProviderError::InvalidScenario(vec![]))
    );
    assert_eq!(report.run, 1, "no new run");

    // Attempt 2 at +3 s fails too; attempt 3 at +7 s succeeds.
    assert_eq!(s.poll(t(T0 + 3 * SEC)), Ok(None));
    assert_eq!(check(&s, &events).retry_at, Some(t(T0 + 7 * SEC)));
    assert!(s.poll(t(T0 + 7 * SEC)).unwrap().is_some());
    let report = check(&s, &events);
    assert_eq!((report.attempts_used, report.run), (3, 2));
    assert_eq!(report.totals.recovery_attempts, 3);
    assert_eq!(
        report.totals.failures, 1,
        "one run failed; two starts were refused"
    );

    let failed_starts: Vec<_> = kinds(&events)
        .into_iter()
        .filter(|k| matches!(k, EventKind::RecoveryStartFailed { .. }))
        .collect();
    assert_eq!(
        failed_starts,
        [
            EventKind::RecoveryStartFailed {
                attempt: 1,
                error: ProviderError::InvalidScenario(vec![])
            },
            EventKind::RecoveryStartFailed {
                attempt: 2,
                error: pipeline_error()
            },
        ]
    );
    // The provider was stopped once, out of its error state. After a start
    // that failed it was idle already and was not stopped again.
    let calls = provider.calls();
    assert_eq!(calls.iter().filter(|c| *c == "stop").count(), 1);
    assert_eq!(calls.iter().filter(|c| c.starts_with("start@")).count(), 4);
}

#[test]
fn restarts_that_never_start_end_in_failure_after_the_bound() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider
        .script()
        .start_results
        .extend((0..10).map(|_| Some(pipeline_error())));
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    for (attempt, at) in [(1, 1), (2, 3), (3, 7)] {
        assert_eq!(check(&s, &events).attempts_used, attempt - 1);
        assert_eq!(s.check(t(T0 + at * SEC)).attempts_used, attempt);
    }
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Failed);
    assert_eq!(report.totals.recovery_attempts, 3);
    assert_eq!(provider.script().start_results.len(), 7);
    // A provider that will not stop is a failed restart as well.
    let (mut s, provider, events) = scripted(HealthPolicy {
        max_recovery_attempts: 1,
        ..policy()
    });
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    provider.script().stop_error = Some(pipeline_error());
    assert_eq!(s.check(t(T0 + SEC)).health, HealthState::Failed);
    assert!(kinds(&events).contains(&EventKind::RecoveryStartFailed {
        attempt: 1,
        error: pipeline_error()
    }));
}

// --- Probation and the end of an episode --------------------------------------------

#[test]
fn an_episode_ends_when_the_restarted_run_has_emitted_for_the_stable_time() {
    let (mut s, provider, events) = scripted(policy()); // stable after 10 s
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    let first = T0 + SEC;
    s.poll(t(first)).unwrap().expect("restarted");

    // One nanosecond short of ten seconds after its first sample.
    s.poll(t(first + 10 * SEC - 1)).unwrap().expect("due");
    let report = check(&s, &events);
    assert_eq!(
        (report.health, report.attempts_used),
        (HealthState::Degraded, 1)
    );
    assert_eq!(report.causes, [Cause::Probation]);

    // Exactly ten seconds.
    s.poll(t(first + 10 * SEC)).unwrap().expect("due");
    let report = check(&s, &events);
    assert_eq!(
        (report.health, report.attempts_used),
        (HealthState::Healthy, 0)
    );
    assert!(report.causes.is_empty());
    assert_eq!(report.totals.recoveries_completed, 1);
    assert_eq!(
        kinds(&events).last(),
        Some(&EventKind::Recovered { attempts: 1 })
    );

    // A failure now is a new episode: attempt 1 again, the initial delay.
    provider.fail_next(pipeline_error());
    let failed_at = first + 20 * SEC;
    s.poll(t(failed_at)).unwrap_err();
    assert_eq!(check(&s, &events).retry_at, Some(t(failed_at + SEC)));
    assert_eq!(scheduled(&events).last(), Some(&(1, failed_at + SEC - T0)));
}

#[test]
fn a_failure_on_probation_continues_the_episode() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    s.poll(t(T0 + SEC)).unwrap().expect("restarted");
    s.poll(t(T0 + 5 * SEC)).unwrap().expect("due");
    // Still on probation: 4 s of 10.
    provider.fail_next(pipeline_error());
    s.poll(t(T0 + 6 * SEC)).unwrap_err();
    let report = check(&s, &events);
    assert_eq!(report.attempts_used, 1, "not reset");
    assert_eq!(
        report.retry_at,
        Some(t(T0 + 8 * SEC)),
        "the second delay, 2 s"
    );
    assert_eq!(scheduled(&events), [(1, SEC), (2, 8 * SEC)]);

    s.poll(t(T0 + 8 * SEC)).unwrap().expect("restarted again");
    provider.fail_next(pipeline_error());
    s.poll(t(T0 + 9 * SEC)).unwrap_err();
    s.poll(t(T0 + 13 * SEC)).unwrap().expect("third restart");
    provider.fail_next(pipeline_error());
    s.poll(t(T0 + 14 * SEC)).unwrap_err();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Failed);
    assert_eq!(report.attempts_used, 3);
    assert_eq!(report.totals.recoveries_completed, 0);
}

#[test]
fn with_no_stable_time_the_episode_ends_at_the_first_sample() {
    let (mut s, provider, events) = scripted(HealthPolicy {
        stable_after_s: 0.0,
        ..policy()
    });
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    // Started by a check: on probation until something is emitted.
    let report = s.check(t(T0 + SEC));
    assert_eq!(
        (report.health, report.attempts_used),
        (HealthState::Degraded, 1)
    );
    s.poll(t(T0 + SEC)).unwrap().expect("due");
    let report = check(&s, &events);
    assert_eq!(
        (report.health, report.attempts_used),
        (HealthState::Healthy, 0)
    );
}

// --- Stopping, and calls that do not fit ----------------------------------------------

#[test]
fn stop_cancels_a_pending_restart_and_ends_the_episode() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    assert_eq!(
        s.pause(t(T0)),
        Err(ProviderError::Transition(InvalidTransition {
            from: SimulationState::Recovering,
            to: SimulationState::Paused
        }))
    );
    assert!(s.start(t(T0)).is_err());
    assert_eq!(check(&s, &events).health, HealthState::Recovering);

    s.stop().unwrap();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Stopped);
    assert_eq!((report.attempts_used, report.retry_at), (0, None));
    assert_eq!(provider.state(), SimulationState::Idle);
    // Long after the restart would have been due, nothing happens.
    let calls = provider.calls().len();
    assert_eq!(s.poll(t(T0 + 100 * SEC)), Ok(None));
    assert_eq!(s.check(t(T0 + 200 * SEC)).health, HealthState::Stopped);
    assert_eq!(provider.calls().len(), calls);
    assert!(!kinds(&events)
        .iter()
        .any(|k| matches!(k, EventKind::RecoveryAttempted { .. })));
    assert_eq!(
        kinds(&events)
            .iter()
            .filter(
                |k| matches!(k, EventKind::OperationRejected { operation, .. }
                if matches!(operation, Operation::Pause | Operation::Start))
            )
            .count(),
        2
    );

    // A new session starts a new episode from nothing.
    s.start(t(T0 + 300 * SEC)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0 + 300 * SEC)).unwrap_err();
    assert_eq!(check(&s, &events).retry_at, Some(t(T0 + 301 * SEC)));
}

#[test]
fn stop_during_probation_ends_the_episode_too() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    s.poll(t(T0 + SEC)).unwrap().expect("restarted");
    assert_eq!(check(&s, &events).attempts_used, 1);
    s.stop().unwrap();
    assert_eq!(check(&s, &events).attempts_used, 0);
    // The next session's first failure waits the initial delay again, and
    // has all three attempts.
    s.start(t(T0 + 10 * SEC)).unwrap();
    assert_eq!(
        check(&s, &events).health,
        HealthState::Healthy,
        "no probation"
    );
    provider.fail_next(pipeline_error());
    s.poll(t(T0 + 10 * SEC)).unwrap_err();
    assert_eq!(check(&s, &events).retry_at, Some(t(T0 + 11 * SEC)));
    assert_eq!(scheduled(&events).last(), Some(&(1, 11 * SEC)));
}

#[test]
fn stop_after_restarts_that_failed_to_start_does_not_stop_an_idle_provider() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider
        .script()
        .start_results
        .push_back(Some(pipeline_error()));
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    s.check(t(T0 + SEC)); // the restart fails to start; the provider is idle
    assert_eq!(provider.state(), SimulationState::Idle);
    let stops = provider.calls().iter().filter(|c| *c == "stop").count();
    s.stop().unwrap();
    assert_eq!(
        provider.calls().iter().filter(|c| *c == "stop").count(),
        stops
    );
    assert_eq!(check(&s, &events).health, HealthState::Stopped);
}

// --- Which errors are retried -------------------------------------------------------

#[test]
fn every_error_is_retried_except_unrepresentable_tick_times() {
    let transition = ProviderError::Transition(InvalidTransition {
        from: SimulationState::Idle,
        to: SimulationState::Running,
    });
    let stamp = |n: i64| t(T0 + n);
    let retried = [
        pipeline_error(),
        transition,
        ProviderError::InvalidScenario(vec![]),
        ProviderError::Schedule(ScheduleError::InvalidInterval),
        ProviderError::Schedule(ScheduleError::ClockWentBackwards),
        ProviderError::Schedule(ScheduleError::AlreadyPaused),
        ProviderError::Schedule(ScheduleError::NotPaused),
        ProviderError::Noise(NoiseError::NonIncreasingTime {
            previous: stamp(2),
            current: stamp(1),
        }),
        ProviderError::Derivation(DeriveError::NonIncreasingTime {
            previous: stamp(2),
            current: stamp(1),
        }),
        ProviderError::Validation(ValidationError::NonMonotonicTimestamp {
            previous: stamp(2),
            current: stamp(1),
        }),
    ];
    for error in retried {
        // From a poll.
        let (mut s, provider, events) = scripted(policy());
        s.start(t(T0)).unwrap();
        provider.fail_next(error.clone());
        assert_eq!(s.poll(t(T0)), Err(error.clone()));
        let report = check(&s, &events);
        assert_eq!(report.health, HealthState::Recovering, "{error:?}");
        assert_eq!(report.retry_at, Some(t(T0 + SEC)), "{error:?}");
        // From a restart.
        provider
            .script()
            .start_results
            .push_back(Some(error.clone()));
        assert_eq!(
            s.check(t(T0 + SEC)).health,
            HealthState::Recovering,
            "{error:?}"
        );
    }

    let overflow = ProviderError::Schedule(ScheduleError::Overflow);
    // From a poll: final at once, no attempt made or scheduled.
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider.fail_next(overflow.clone());
    assert_eq!(s.poll(t(T0)), Err(overflow.clone()));
    let report = check(&s, &events);
    assert_eq!(
        (report.health, report.attempts_used),
        (HealthState::Failed, 0)
    );
    assert_eq!(
        kinds(&events).last(),
        Some(&EventKind::Failed {
            reason: FailReason::Permanent
        })
    );
    assert!(scheduled(&events).is_empty());
    // From a restart: final, with two attempts unused.
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    provider.script().start_results.push_back(Some(overflow));
    let report = s.check(t(T0 + SEC));
    assert_eq!(
        (report.health, report.attempts_used),
        (HealthState::Failed, 1)
    );
    assert_eq!(
        kinds(&events).last(),
        Some(&EventKind::Failed {
            reason: FailReason::Permanent
        })
    );
}

#[test]
fn a_retry_time_that_cannot_be_represented_is_a_failure_not_a_panic() {
    // The failure happens one second before the end of time; the retry
    // would be due one second after it... and one nanosecond more.
    for (failed_at, representable) in [(i64::MAX - SEC, true), (i64::MAX - SEC + 1, false)] {
        let (mut s, provider, events) = scripted(policy()); // first delay 1 s
        s.start(t(T0)).unwrap();
        provider.fail_next(pipeline_error());
        s.poll(t(failed_at)).unwrap_err();
        let report = check(&s, &events);
        if representable {
            assert_eq!(report.health, HealthState::Recovering);
            assert_eq!(report.retry_at, Some(t(i64::MAX)));
        } else {
            assert_eq!(report.health, HealthState::Failed);
            assert_eq!(
                kinds(&events).last(),
                Some(&EventKind::Failed {
                    reason: FailReason::RetryTimeUnrepresentable
                })
            );
        }
    }
}

// --- The real provider: what a restarted run is ---------------------------------------

fn walking() -> locsim_core::domain::Scenario {
    import_scenario(example("walking")).unwrap()
}

fn supervised_failing(
    policy: HealthPolicy,
    failing_runs: u32,
    fail_at: u64,
) -> (Supervisor<SimulationProvider>, MemorySink) {
    let (provider, _) = failing_provider(&walking(), failing_runs, fail_at);
    let events = MemorySink::new();
    let supervisor = Supervisor::new(provider, policy, Box::new(events.clone())).unwrap();
    (supervisor, events)
}

/// Polls every second from `from` for `ticks` ticks; returns the samples.
fn poll_each_second(
    s: &mut Supervisor<SimulationProvider>,
    from: i64,
    ticks: i64,
) -> Vec<SyntheticLocation> {
    (0..ticks)
        .filter_map(|tick| s.poll(t(from + tick * SEC)).ok().flatten())
        .collect()
}

#[test]
fn a_restarted_run_is_a_fresh_run_of_the_scenario_not_a_continuation() {
    // The first run fails at its 30th sample; later runs are sound.
    let (mut s, events) = supervised_failing(policy(), 1, 30);
    s.start(t(T0)).unwrap();
    let before = poll_each_second(&mut s, T0, 30);
    assert_eq!(before.len(), 30);
    assert!(s.poll(t(T0 + 30 * SEC)).is_err());
    assert_eq!(check(&s, &events).totals.samples, 30);

    let restart = T0 + 31 * SEC;
    let after = poll_each_second(&mut s, restart, 40);
    assert_eq!(after.len(), 40);
    let report = check(&s, &events);
    assert_eq!(report.run, 2);
    assert_eq!(report.totals.samples, 70);
    assert_eq!(s.status().sample_count, 40, "per-run counts start again");
    assert_eq!(report.health, HealthState::Healthy, "stable after 10 s");

    // A first sample: no speed, no course.
    assert_eq!((after[0].speed_mps, after[0].course_deg), (None, None));
    assert!(before[29].speed_mps.is_some());

    // It is exactly what a provider started at that instant produces...
    let mut fresh = SimulationProvider::new(walking());
    fresh.start(t(restart)).unwrap();
    let expected: Vec<_> = (0..40)
        .map(|tick| fresh.poll(t(restart + tick * SEC)).unwrap().unwrap())
        .collect();
    assert_eq!(fingerprint(&after), fingerprint(&expected));

    // ...and not what the interrupted run would have gone on to produce.
    let mut uninterrupted = SimulationProvider::new(walking());
    uninterrupted.start(t(T0)).unwrap();
    let whole: Vec<_> = (0..71)
        .map(|tick| uninterrupted.poll(t(T0 + tick * SEC)).unwrap().unwrap())
        .collect();
    assert_eq!(fingerprint(&before), fingerprint(&whole[..30].to_vec()));
    assert_ne!(after[0].coordinate, whole[31].coordinate);
    // The positions start over: the run begins where the first one began,
    // which is not where the failed run was.
    assert_eq!(
        fingerprint(&after[0].coordinate),
        fingerprint(&whole[0].coordinate)
    );
    assert_ne!(after[0].coordinate, before[29].coordinate);
}

#[test]
fn a_failure_that_does_not_depend_on_timing_happens_again_in_every_restart() {
    // Every run fails at its 6th sample.
    let (provider, built) = failing_provider(&walking(), u32::MAX, 5);
    let events = MemorySink::new();
    let mut s = Supervisor::new(
        provider,
        HealthPolicy {
            backoff_initial_s: 0.0,
            backoff_max_s: 0.0,
            ..policy()
        },
        Box::new(events.clone()),
    )
    .unwrap();
    s.start(t(T0)).unwrap();
    let mut samples = Vec::new();
    for tick in 0..200 {
        samples.extend(s.poll(t(T0 + tick * SEC)).ok().flatten());
        check(&s, &events);
    }
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Failed);
    // Four runs, five samples each, each failing at the same point.
    assert_eq!(built.load(std::sync::atomic::Ordering::Relaxed), 4);
    assert_eq!(samples.len(), 20);
    assert_eq!(report.totals.failures, 4);
    let ended: Vec<_> = kinds(&events)
        .into_iter()
        .filter_map(|k| match k {
            EventKind::RunEnded { samples, .. } => Some(samples),
            _ => None,
        })
        .collect();
    assert_eq!(ended, [5, 5, 5, 5]);
    // Each run retraced the same positions.
    for run in 1..4 {
        for i in 0..5 {
            assert_eq!(
                fingerprint(&samples[run * 5 + i].coordinate),
                fingerprint(&samples[i].coordinate)
            );
        }
    }
}

// --- Timestamps across runs -----------------------------------------------------------

fn assert_strictly_increasing(samples: &[SyntheticLocation]) {
    for pair in samples.windows(2) {
        assert!(
            pair[1].timestamp > pair[0].timestamp,
            "{} then {}",
            pair[0].timestamp.as_nanos(),
            pair[1].timestamp.as_nanos()
        );
    }
}

#[test]
fn the_first_sample_after_a_restart_is_stamped_by_the_schedule_not_by_the_restart() {
    // The restart is made by `check`; the first poll comes later.
    for (late_by_ms, ticks_skipped) in [(0, 0), (500, 0), (3_700, 3), (1_000_000, 1_000)] {
        let (mut s, events) = supervised_failing(policy(), 1, 10);
        s.start(t(T0)).unwrap();
        let mut samples = poll_each_second(&mut s, T0, 10);
        assert!(s.poll(t(T0 + 10 * SEC)).is_err());
        let restart = T0 + 11 * SEC;
        assert_eq!(s.check(t(restart)).run, 2);
        assert_eq!(s.current_location(), None, "started, not polled");

        let first_poll = restart + late_by_ms * 1_000_000;
        let first = s.poll(t(first_poll)).unwrap().expect("due");
        // The ideal time of the latest tick that was due: the start of the
        // run plus a whole number of intervals, never before the start.
        assert_eq!(
            first.timestamp,
            t(restart + ticks_skipped * SEC),
            "late by {late_by_ms} ms"
        );
        assert_eq!(s.status().missed_ticks, ticks_skipped as u64);
        samples.push(first);
        samples.extend(poll_each_second(&mut s, first_poll + SEC, 5));
        assert_strictly_increasing(&samples);
        let report = check(&s, &events);
        assert_eq!(report.totals.withheld, 0);
        assert_eq!(report.totals.missed_ticks, ticks_skipped as u64);
    }
}

#[test]
fn timestamps_increase_through_repeated_failures_with_and_without_backoff() {
    for (initial, max) in [(0.0, 0.0), (1.0, 60.0), (0.25, 0.25)] {
        // Three runs fail, each at its 4th sample; the fourth is sound.
        let (mut s, events) = supervised_failing(
            HealthPolicy {
                backoff_initial_s: initial,
                backoff_max_s: max,
                ..policy()
            },
            3,
            3,
        );
        s.start(t(T0)).unwrap();
        let mut samples = Vec::new();
        let mut now = T0;
        // Polled twice at every instant, so a zero delay is retried at an
        // unchanged time.
        for _ in 0..80 {
            for _ in 0..2 {
                samples.extend(s.poll(t(now)).ok().flatten());
                check(&s, &events);
            }
            now += SEC / 2;
        }
        let report = check(&s, &events);
        assert_eq!(report.run, 4, "backoff {initial}");
        assert_eq!(report.totals.failures, 3);
        assert_eq!(report.totals.withheld, 0);
        assert_eq!(report.totals.samples, samples.len() as u64);
        assert!(samples.len() > 20);
        assert_strictly_increasing(&samples);
        // The supervisor's count of what it handed on is the provider's.
        assert_eq!(s.status().sample_count, s.provider().status().sample_count);
    }
}

#[test]
fn a_restarted_run_that_fails_at_its_first_tick_can_be_restarted_at_the_same_instant() {
    // Run 1 emits two samples and fails; runs 2 and 3 fail at tick 0.
    let scenario = walking();
    let built = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counter = built.clone();
    let provider = SimulationProvider::with_model_factory(
        scenario.clone(),
        Box::new(move |scenario| {
            let run = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let fail_at = match run {
                0 => 2,
                1 | 2 => 0,
                _ => u64::MAX,
            };
            Ok(Box::new(FailAfter {
                real: locsim_core::movement::model_for(scenario)?,
                calls: 0,
                fail_at,
            }))
        }),
    );
    let events = MemorySink::new();
    let mut s = Supervisor::new(
        provider,
        HealthPolicy {
            backoff_initial_s: 0.0,
            backoff_max_s: 0.0,
            ..policy()
        },
        Box::new(events.clone()),
    )
    .unwrap();
    s.start(t(T0)).unwrap();
    let mut samples = poll_each_second(&mut s, T0, 2);
    let instant = T0 + 2 * SEC;
    assert!(s.poll(t(instant)).is_err(), "run 1 fails");
    assert!(
        s.poll(t(instant)).is_err(),
        "run 2 starts and fails at its tick 0"
    );
    assert!(s.poll(t(instant)).is_err(), "run 3 likewise");
    let first = s.poll(t(instant)).unwrap().expect("run 4 emits");
    assert_eq!(first.timestamp, t(instant));
    samples.push(first);
    assert_strictly_increasing(&samples);
    let report = check(&s, &events);
    assert_eq!((report.run, report.attempts_used), (4, 3));
    assert_eq!(report.totals.withheld, 0);
}

struct FailAfter {
    real: Box<dyn locsim_core::movement::MovementModel + Send>,
    calls: u64,
    fail_at: u64,
}

impl locsim_core::movement::MovementModel for FailAfter {
    fn sample_at(
        &mut self,
        at: locsim_core::domain::Timestamp,
    ) -> Result<locsim_core::movement::MovementSample, locsim_core::movement::MovementError> {
        let call = self.calls;
        self.calls += 1;
        if call == self.fail_at {
            return Err(locsim_core::movement::MovementError::InvalidConfiguration(
                "injected failure",
            ));
        }
        self.real.sample_at(at)
    }
}

#[test]
fn a_pause_before_the_failure_does_not_disturb_the_order() {
    let (mut s, events) = supervised_failing(policy(), 1, 8);
    s.start(t(T0)).unwrap();
    let mut samples = poll_each_second(&mut s, T0, 5);
    s.pause(t(T0 + 5 * SEC)).unwrap();
    s.resume(t(T0 + 500 * SEC)).unwrap();
    samples.extend(poll_each_second(&mut s, T0 + 500 * SEC, 3));
    assert!(s.poll(t(T0 + 503 * SEC)).is_err());
    samples.extend(poll_each_second(&mut s, T0 + 504 * SEC, 5));
    assert_eq!(samples.len(), 13);
    assert_strictly_increasing(&samples);
    assert_eq!(check(&s, &events).totals.withheld, 0);
}

#[test]
fn random_sequences_of_failures_delays_and_pauses_keep_timestamps_increasing() {
    let mut rng = Rng::from_seed(0x7_10);
    let mut restarts = 0;
    for case in 0..60 {
        let fail_at = rng.next_u64() % 12;
        let failing_runs = (rng.next_u64() % 4) as u32;
        let (mut s, events) = supervised_failing(
            HealthPolicy {
                max_recovery_attempts: 3,
                backoff_initial_s: [0.0, 0.3, 2.0][(rng.next_u64() % 3) as usize],
                backoff_multiplier: 1.5,
                backoff_max_s: 5.0,
                stable_after_s: 4.0,
                ..policy()
            },
            failing_runs,
            fail_at,
        );
        s.start(t(T0)).unwrap();
        let mut now = T0;
        let mut samples = Vec::new();
        for _ in 0..150 {
            match rng.next_u64() % 12 {
                0 => {
                    if s.pause(t(now)).is_ok() {
                        now += (rng.uniform(0.0, 20.0) * 1e9) as i64;
                        s.resume(t(now)).unwrap();
                    }
                }
                1 => {
                    s.check(t(now));
                }
                _ => samples.extend(s.poll(t(now)).ok().flatten()),
            }
            check(&s, &events);
            // From a fraction of an interval to several.
            now += (rng.uniform(0.0, 3.5) * 1e9) as i64;
        }
        assert_strictly_increasing(&samples);
        let report = check(&s, &events);
        assert_eq!(report.totals.withheld, 0, "case {case}");
        assert_eq!(report.totals.samples, samples.len() as u64, "case {case}");
        restarts += report.totals.recovery_attempts;
    }
    assert!(restarts > 40, "{restarts}");
}

#[test]
fn a_provider_that_restarts_with_an_old_timestamp_is_caught_by_the_check() {
    // What the argument about the real provider cannot cover: a provider
    // whose new run stamps a sample at or before the last one handed on.
    for stale_by in [0, 1, 5 * SEC] {
        let (mut s, provider, events) = scripted(policy());
        s.start(t(T0)).unwrap();
        let last = s.poll(t(T0 + 5 * SEC)).unwrap().unwrap();
        provider.fail_next(pipeline_error());
        s.poll(t(T0 + 6 * SEC)).unwrap_err();
        // The restarted run's first sample.
        provider.push(Step::Emit {
            at: Some(last.timestamp.as_nanos() - stale_by),
            missed: 0,
        });
        let result = s.poll(t(T0 + 7 * SEC));
        assert_eq!(
            result,
            Err(ProviderError::Validation(
                ValidationError::NonMonotonicTimestamp {
                    previous: last.timestamp,
                    current: t(last.timestamp.as_nanos() - stale_by),
                }
            ))
        );
        let report = check(&s, &events);
        assert_eq!(report.totals.withheld, 1);
        assert_eq!(report.totals.samples, 1);
        // An ordinary run failure: the second attempt is scheduled, and the
        // provider, which thinks it is running, is stopped before it.
        assert_eq!(report.health, HealthState::Recovering);
        assert_eq!(provider.state(), SimulationState::Running);
        assert!(s.poll(t(T0 + 9 * SEC)).unwrap().is_some());
        let calls = provider.calls();
        assert_eq!(
            &calls[calls.len() - 3..],
            ["stop", "start@9000000000", "poll@9000000000"]
        );
    }
}

// --- Determinism -------------------------------------------------------------------

#[test]
fn the_same_calls_give_the_same_samples_reports_and_events() {
    let run = || {
        let (mut s, events) = supervised_failing(policy(), 2, 7);
        s.start(t(T0)).unwrap();
        let mut rng = Rng::from_seed(99);
        let mut now = T0;
        let mut out = Vec::new();
        for _ in 0..200 {
            out.push(format!("{:?}", s.poll(t(now))));
            now += (rng.uniform(0.2, 2.2) * 1e9) as i64;
        }
        (
            out,
            format!("{:?}", s.health()),
            format!("{:?}", events.events()),
        )
    };
    assert_eq!(run(), run());
}
