//! T10: the watchdog. A stall and falling behind, at their exact
//! boundaries, and what they must not be mistaken for.

mod support;

use locsim_core::domain::{HealthState, SimulationState};
use locsim_core::provider::{LocationProvider, SimulationProvider};
use locsim_health::{Cause, Emission, EventKind, HealthPolicy, MemorySink, NullSink, Supervisor};
use locsim_scenario::import_scenario;
use support::scenario::example;
use support::*;

fn count(events: &MemorySink, wanted: fn(&EventKind) -> bool) -> usize {
    events.events().iter().filter(|e| wanted(&e.kind)).count()
}

fn is_stalled(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Stalled { .. })
}

// --- Stall ------------------------------------------------------------------------------

#[test]
fn a_run_is_stalled_one_nanosecond_after_the_threshold_and_not_at_it() {
    let (mut s, _, events) = scripted(policy()); // stalled after 5 s
    s.start(t(T0)).unwrap();
    s.poll(t(T0 + SEC)).unwrap().expect("due");
    let progress = T0 + SEC;

    // Exactly five seconds of silence is not yet a stall.
    let report = s.check(t(progress + 5 * SEC));
    assert_consistent(&report);
    assert_eq!(report.health, HealthState::Healthy);
    assert_eq!(report.emission, Emission::Flowing);
    assert_eq!(count(&events, is_stalled), 0);

    let report = s.check(t(progress + 5 * SEC + 1));
    assert_consistent(&report);
    // The lifecycle says running. The report says nothing is arriving.
    assert_eq!(report.lifecycle, SimulationState::Running);
    assert_eq!(report.health, HealthState::Degraded);
    assert_eq!(report.causes, [Cause::Stalled]);
    assert_eq!(
        report.emission,
        Emission::Stalled {
            silent_for_ns: 5 * SEC + 1
        }
    );
    assert_eq!(
        kinds(&events).last(),
        Some(&EventKind::Stalled {
            silent_for_ns: 5 * SEC + 1
        })
    );

    // Later checks update the silence but do not announce the stall again.
    let report = s.check(t(progress + 60 * SEC));
    assert_eq!(
        report.emission,
        Emission::Stalled {
            silent_for_ns: 60 * SEC
        }
    );
    assert_eq!(count(&events, is_stalled), 1);
    // A stall is not a failure and restarts nothing.
    assert_eq!((report.totals.failures, report.run), (0, 1));
    assert_eq!(report.retry_at, None);
    check(&s, &events);
}

#[test]
fn a_sample_ends_the_stall() {
    let (mut s, _, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    assert_eq!(s.check(t(T0 + 6 * SEC)).health, HealthState::Degraded);
    s.poll(t(T0 + 7 * SEC)).unwrap().expect("due");
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Healthy);
    assert_eq!(report.emission, Emission::Flowing);
    assert_eq!(kinds(&events).last(), Some(&EventKind::StallCleared));
    // The silence is measured from that sample's call now.
    assert_eq!(s.check(t(T0 + 12 * SEC)).health, HealthState::Healthy);
    assert_eq!(s.check(t(T0 + 12 * SEC + 1)).health, HealthState::Degraded);
    assert_eq!(count(&events, is_stalled), 2);
}

#[test]
fn a_poll_that_yields_nothing_notices_the_stall_as_well() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    for second in [2, 4, 5] {
        provider.push(Step::Nothing);
        assert_eq!(s.poll(t(T0 + second * SEC)), Ok(None));
        assert_eq!(check(&s, &events).health, HealthState::Healthy, "{second}");
    }
    provider.push(Step::Nothing);
    assert_eq!(s.poll(t(T0 + 6 * SEC)), Ok(None));
    let report = check(&s, &events);
    assert_eq!(report.causes, [Cause::Stalled]);
    assert_eq!(
        report.emission,
        Emission::Stalled {
            silent_for_ns: 6 * SEC
        }
    );
}

#[test]
fn nothing_is_expected_of_a_paused_run() {
    let (mut s, _, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    s.poll(t(T0)).unwrap();
    s.pause(t(T0 + SEC)).unwrap();
    // Paused for a day.
    let report = s.check(t(T0 + 86_400 * SEC));
    assert_consistent(&report);
    assert_eq!(report.health, HealthState::Healthy);
    assert_eq!(report.emission, Emission::NotExpected);
    assert_eq!(count(&events, is_stalled), 0);

    // The measurement starts again at the resume, not at the old sample.
    let resumed = T0 + 90_000 * SEC;
    s.resume(t(resumed)).unwrap();
    assert_eq!(s.check(t(resumed + 5 * SEC)).health, HealthState::Healthy);
    assert_eq!(
        s.check(t(resumed + 5 * SEC + 1)).health,
        HealthState::Degraded
    );

    // Pausing a stalled run clears the stall: it is no longer expected to emit.
    s.pause(t(resumed + 6 * SEC)).unwrap();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Healthy);
    assert!(report.causes.is_empty());
    assert_eq!(report.emission, Emission::NotExpected);
}

#[test]
fn a_stall_is_measured_from_the_start_of_a_restarted_run() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    // While recovering or failed there is no run to be stalled.
    let report = s.check(t(T0 + SEC - 1));
    assert_eq!(report.health, HealthState::Recovering);
    assert_eq!(report.emission, Emission::NotExpected);
    // Restarted by a check at +1 s, never polled.
    assert_eq!(s.check(t(T0 + SEC)).causes, [Cause::Probation]);
    assert_eq!(s.check(t(T0 + 6 * SEC)).causes, [Cause::Probation]);
    let report = s.check(t(T0 + 6 * SEC + 1));
    assert_consistent(&report);
    assert_eq!(report.causes, [Cause::Stalled, Cause::Probation]);
    assert_eq!(report.health, HealthState::Degraded);
    check(&s, &events);
}

#[test]
fn a_stalled_run_that_is_stopped_or_fails_is_not_reported_stalled() {
    let (mut s, provider, events) = scripted(no_recovery());
    s.start(t(T0)).unwrap();
    s.check(t(T0 + 10 * SEC));
    s.stop().unwrap();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Stopped);
    assert_eq!(report.emission, Emission::NotExpected);

    s.start(t(T0 + 20 * SEC)).unwrap();
    assert_eq!(check(&s, &events).health, HealthState::Healthy, "a new run");
    s.check(t(T0 + 30 * SEC));
    provider.fail_next(pipeline_error());
    s.poll(t(T0 + 31 * SEC)).unwrap_err();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Failed);
    assert!(report.causes.is_empty());
}

#[test]
fn a_real_provider_that_nobody_polls_is_reported_stalled_and_recovers_when_polled() {
    let scenario = import_scenario(example("walking")).unwrap(); // 1 Hz
    let events = MemorySink::new();
    let mut s = Supervisor::for_simulation(
        SimulationProvider::new(scenario),
        policy(),
        Box::new(events.clone()),
    )
    .unwrap();
    s.start(t(T0)).unwrap();
    for tick in 0..10 {
        s.poll(t(T0 + tick * SEC)).unwrap().expect("due");
    }
    // The polling loop dies; only a timer still calls the supervisor.
    assert_eq!(s.check(t(T0 + 14 * SEC)).health, HealthState::Healthy);
    let report = s.check(t(T0 + 40 * SEC));
    assert_eq!(report.causes, [Cause::Stalled]);
    assert_eq!(
        report.emission,
        Emission::Stalled {
            silent_for_ns: 31 * SEC
        }
    );
    // The provider itself is none the wiser: it is running.
    assert_eq!(s.provider().status().state, SimulationState::Running);

    // Polling resumes: one sample, thirty ticks skipped, and now it is
    // "falling behind" rather than "stalled".
    let sample = s.poll(t(T0 + 40 * SEC)).unwrap().expect("due");
    assert_eq!(sample.timestamp, t(T0 + 40 * SEC));
    let report = check(&s, &events);
    assert_eq!(report.causes, [Cause::FallingBehind]);
    assert_eq!(report.emission, Emission::Flowing);
    assert_eq!(report.totals.missed_ticks, 30);
    assert_eq!(
        report.totals.missed_ticks,
        s.provider().status().missed_ticks
    );
}

// --- Falling behind -------------------------------------------------------------------

/// Polls once; the provider reports `missed` skipped ticks with the sample.
fn emit(s: &mut Supervisor<Scripted>, provider: &Scripted, now: &mut i64, missed: u64) {
    provider.push(Step::Emit { at: None, missed });
    *now += SEC;
    s.poll(t(*now)).unwrap().expect("due");
}

fn behind(s: &Supervisor<Scripted>) -> bool {
    let report = s.health();
    assert_consistent(&report);
    report.causes.contains(&Cause::FallingBehind)
}

fn windowed() -> (Supervisor<Scripted>, Scripted, MemorySink, i64) {
    // A window of ten slots, at most two missed. The stall threshold is out
    // of the way.
    let (mut s, provider, events) = scripted(HealthPolicy {
        stall_after_s: 1e6,
        ..policy()
    });
    s.start(t(T0)).unwrap();
    (s, provider, events, T0)
}

#[test]
fn exactly_the_limit_is_not_falling_behind_and_one_more_is() {
    // Two missed in a window of ten: within the limit.
    let (mut s, provider, events, mut now) = windowed();
    emit(&mut s, &provider, &mut now, 2); // slots 3, missed 2
    for _ in 0..7 {
        emit(&mut s, &provider, &mut now, 0); // slots 10: the window closes
        assert!(!behind(&s));
    }
    assert_eq!(
        count(&events, |k| matches!(k, EventKind::FallingBehind { .. })),
        0
    );

    // Three: over, and said at that very sample, before the window closes.
    let (mut s, provider, events, mut now) = windowed();
    emit(&mut s, &provider, &mut now, 2);
    assert!(!behind(&s));
    emit(&mut s, &provider, &mut now, 1);
    assert!(behind(&s));
    assert_eq!(
        kinds(&events).last(),
        Some(&EventKind::FallingBehind {
            missed: 3,
            slots: 5
        })
    );
    assert_eq!(check(&s, &events).health, HealthState::Degraded);
}

#[test]
fn it_ends_only_when_a_whole_window_closes_within_the_limit() {
    let (mut s, provider, events, mut now) = windowed();
    emit(&mut s, &provider, &mut now, 3); // slots 4, missed 3: behind
    assert!(behind(&s));
    // The rest of this window is clean, but the window as a whole is over
    // the limit: closing it changes nothing.
    for _ in 0..6 {
        emit(&mut s, &provider, &mut now, 0); // slots 10 at the last one
    }
    assert!(behind(&s), "the window that tripped closed over the limit");
    // The next window: nine clean slots are not yet a verdict...
    for _ in 0..9 {
        emit(&mut s, &provider, &mut now, 0);
        assert!(behind(&s));
    }
    // ...the tenth closes it within the limit.
    emit(&mut s, &provider, &mut now, 0);
    assert!(!behind(&s));
    assert_eq!(kinds(&events).last(), Some(&EventKind::CaughtUp));
    assert_eq!(
        count(&events, |k| matches!(k, EventKind::FallingBehind { .. })),
        1
    );
    assert_eq!(check(&s, &events).health, HealthState::Healthy);
}

#[test]
fn staying_behind_is_announced_once() {
    let (mut s, provider, events, mut now) = windowed();
    for _ in 0..12 {
        emit(&mut s, &provider, &mut now, 4); // every sample skips four
        assert!(behind(&s));
    }
    assert_eq!(
        count(&events, |k| matches!(k, EventKind::FallingBehind { .. })),
        1
    );
    assert_eq!(count(&events, |k| matches!(k, EventKind::CaughtUp)), 0);
}

#[test]
fn a_late_poll_can_overrun_a_window_and_the_excess_is_not_carried_over() {
    let (mut s, provider, events, mut now) = windowed();
    for _ in 0..9 {
        emit(&mut s, &provider, &mut now, 0); // nine slots, none missed
    }
    // One poll skips five ticks: fifteen slots, five missed. Over the
    // limit, and the window closes with it.
    emit(&mut s, &provider, &mut now, 5);
    assert!(behind(&s));
    assert_eq!(
        kinds(&events).last(),
        Some(&EventKind::FallingBehind {
            missed: 5,
            slots: 15
        })
    );
    // The next window starts empty: ten clean slots close it, not five.
    for _ in 0..9 {
        emit(&mut s, &provider, &mut now, 0);
        assert!(behind(&s));
    }
    emit(&mut s, &provider, &mut now, 0);
    assert!(!behind(&s));
    check(&s, &events);
}

#[test]
fn windows_do_not_overlap() {
    // Two missed at the end of one window and two at the start of the
    // next: four within ten consecutive slots, but never more than two in
    // a window. That is what "window" means here.
    let (mut s, provider, events, mut now) = windowed();
    for _ in 0..7 {
        emit(&mut s, &provider, &mut now, 0); // 7 slots
    }
    emit(&mut s, &provider, &mut now, 2); // 10 slots, 2 missed: closes clean
    emit(&mut s, &provider, &mut now, 2); // new window: 3 slots, 2 missed
    assert!(!behind(&s));
    assert_eq!(check(&s, &events).totals.missed_ticks, 4);
    // One more in the second window tips it.
    emit(&mut s, &provider, &mut now, 1);
    assert!(behind(&s));
}

#[test]
fn a_window_continues_across_a_pause() {
    let (mut s, provider, _, mut now) = windowed();
    emit(&mut s, &provider, &mut now, 2); // 3 slots, 2 missed
    s.pause(t(now + SEC)).unwrap();
    now += 1_000 * SEC;
    s.resume(t(now)).unwrap();
    assert!(!behind(&s));
    // The pause added no slots and forgave nothing.
    emit(&mut s, &provider, &mut now, 1);
    assert!(behind(&s));
}

#[test]
fn a_new_run_starts_with_an_empty_window_and_is_not_behind() {
    let (mut s, provider, events) = scripted(HealthPolicy {
        stall_after_s: 1e6,
        backoff_initial_s: 0.0,
        ..policy()
    });
    s.start(t(T0)).unwrap();
    let mut now = T0;
    emit(&mut s, &provider, &mut now, 5);
    assert!(behind(&s));
    provider.fail_next(pipeline_error());
    now += SEC;
    s.poll(t(now)).unwrap_err();
    assert!(!behind(&s), "no run, no cause");
    // The restarted run: two missed, which with the old five would be
    // seven. It is two.
    emit(&mut s, &provider, &mut now, 2);
    assert!(!behind(&s));
    assert_eq!(check(&s, &events).causes, [Cause::Probation]);
    assert_eq!(s.status().missed_ticks, 2);
    assert_eq!(check(&s, &events).totals.missed_ticks, 7);
}

#[test]
fn a_window_of_one_slot_judges_every_sample_on_its_own() {
    let (mut s, provider, events) = scripted(HealthPolicy {
        missed_window_ticks: 1,
        max_missed_in_window: 0,
        stall_after_s: 1e6,
        ..policy()
    });
    s.start(t(T0)).unwrap();
    let mut now = T0;
    emit(&mut s, &provider, &mut now, 0);
    assert!(!behind(&s));
    emit(&mut s, &provider, &mut now, 1);
    assert!(behind(&s));
    emit(&mut s, &provider, &mut now, 0);
    assert!(!behind(&s));
    assert_eq!(
        count(&events, |k| matches!(k, EventKind::FallingBehind { .. })),
        1
    );
    assert_eq!(count(&events, |k| matches!(k, EventKind::CaughtUp)), 1);
}

#[test]
fn counters_at_their_limit_saturate() {
    let (mut s, provider, events, mut now) = windowed();
    emit(&mut s, &provider, &mut now, u64::MAX - 1);
    assert!(behind(&s));
    // The provider's own count cannot grow further; nor can anything here.
    provider.script().missed_ticks = u64::MAX;
    emit(&mut s, &provider, &mut now, 0);
    assert_eq!(s.status().missed_ticks, u64::MAX);
    assert_eq!(check(&s, &events).totals.missed_ticks, u64::MAX);
}

// --- The threshold and the interval ---------------------------------------------------

#[test]
fn for_simulation_refuses_a_stall_threshold_that_an_on_time_provider_would_trip() {
    let scenario = import_scenario(example("walking")).unwrap(); // 1 s
    for (stall_after_s, accepted) in [(5.0, true), (1.001, true), (1.0, false), (0.5, false)] {
        let result = Supervisor::for_simulation(
            SimulationProvider::new(scenario.clone()),
            HealthPolicy {
                stall_after_s,
                ..policy()
            },
            Box::new(NullSink),
        );
        match (result, accepted) {
            (Ok(_), true) => {}
            (Err(errors), false) => {
                assert_eq!(errors.len(), 1);
                assert_eq!(errors[0].field, "stall_after_s");
            }
            (other, _) => panic!("{stall_after_s}: {:?}", other.err()),
        }
    }
    // The generic constructor cannot know the interval and accepts it; the
    // result is a provider that is on time and reported as stalled.
    let events = MemorySink::new();
    let mut s = Supervisor::new(
        SimulationProvider::new(scenario),
        HealthPolicy {
            stall_after_s: 0.5,
            ..policy()
        },
        Box::new(events.clone()),
    )
    .unwrap();
    s.start(t(T0)).unwrap();
    s.poll(t(T0)).unwrap().expect("due");
    assert_eq!(
        s.check(t(T0 + SEC - 1)).causes,
        [Cause::Stalled],
        "and yet nothing is late"
    );
    assert!(s.poll(t(T0 + SEC)).unwrap().is_some());
}

#[test]
fn for_simulation_reports_the_other_problems_of_a_policy_too() {
    let scenario = import_scenario(example("driving")).unwrap(); // 0.5 s
    let errors = Supervisor::for_simulation(
        SimulationProvider::new(scenario),
        HealthPolicy {
            backoff_multiplier: 0.0,
            stall_after_s: 0.25,
            ..policy()
        },
        Box::new(NullSink),
    )
    .err()
    .expect("accepted");
    let fields: Vec<_> = errors.iter().map(|e| e.field).collect();
    assert_eq!(fields, ["backoff_multiplier", "stall_after_s"]);
}
