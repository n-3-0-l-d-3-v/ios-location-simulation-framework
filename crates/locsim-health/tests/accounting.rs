//! T10: faults reported by the caller, and the checks that nothing the
//! supervisor says can be misleading: every report is consistent, every
//! total can be recounted from the events, and no event reveals a position.

mod support;

use locsim_core::domain::{HealthState, SimulationState};
use locsim_core::provider::{LocationProvider, ProviderError};
use locsim_core::rng::Rng;
use locsim_core::scheduler::ScheduleError;
use locsim_health::{Cause, Emission, EventKind, HealthPolicy, MemorySink, Supervisor};
use locsim_scenario::import_scenario;
use support::scenario::{example, EXAMPLES};
use support::*;

// --- Faults of other components ------------------------------------------------------

#[test]
fn a_reported_fault_degrades_a_run_and_does_nothing_else() {
    let (mut s, provider, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    s.poll(t(T0)).unwrap().expect("due");

    s.report_fault(t(T0 + SEC), "persistence", "disk full".into());
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Degraded);
    assert_eq!(report.causes, [Cause::ExternalFault("persistence")]);
    assert_eq!(report.active_faults, ["persistence"]);
    // The simulation is untouched: running, emitting, nothing restarted.
    assert_eq!(report.lifecycle, SimulationState::Running);
    assert_eq!(report.emission, Emission::Flowing);
    let calls = provider.calls();
    for tick in 1..20 {
        s.poll(t(T0 + tick * SEC))
            .unwrap()
            .expect("samples keep coming");
    }
    let report = check(&s, &events);
    assert_eq!((report.run, report.totals.failures), (1, 0));
    assert_eq!(report.totals.recovery_attempts, 0);
    assert_eq!(report.retry_at, None);
    assert_eq!(provider.calls().len(), calls.len() + 19, "only the polls");

    assert!(s.clear_fault(t(T0 + 30 * SEC), "persistence"));
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Healthy);
    assert!(report.active_faults.is_empty());
    assert_eq!(
        kinds(&events),
        [
            EventKind::RunStarted {
                recovery_attempt: None
            },
            EventKind::FaultReported {
                component: "persistence",
                detail: "disk full".into()
            },
            EventKind::FaultCleared {
                component: "persistence"
            },
        ]
    );
}

#[test]
fn faults_are_kept_per_component_and_cleared_one_by_one() {
    let (mut s, _, events) = scripted(policy());
    s.start(t(T0)).unwrap();
    s.report_fault(t(T0), "persistence", "disk full".into());
    s.report_fault(t(T0), "delivery", "no listener".into());
    // Reporting again replaces the detail; the component is listed once.
    s.report_fault(t(T0 + SEC), "persistence", "still full".into());
    let report = check(&s, &events);
    assert_eq!(report.active_faults, ["persistence", "delivery"]);
    assert_eq!(
        report.causes,
        [
            Cause::ExternalFault("persistence"),
            Cause::ExternalFault("delivery")
        ]
    );
    assert_eq!(
        kinds(&events).last(),
        Some(&EventKind::FaultReported {
            component: "persistence",
            detail: "still full".into()
        })
    );

    assert!(s.clear_fault(t(T0 + 2 * SEC), "persistence"));
    assert_eq!(
        check(&s, &events).health,
        HealthState::Degraded,
        "one remains"
    );
    // Clearing what is not there is nothing: no event, no change.
    let recorded = events.len();
    assert!(!s.clear_fault(t(T0 + 3 * SEC), "persistence"));
    assert!(!s.clear_fault(t(T0 + 3 * SEC), "never reported"));
    assert_eq!(events.len(), recorded);
    assert!(s.clear_fault(t(T0 + 4 * SEC), "delivery"));
    assert_eq!(check(&s, &events).health, HealthState::Healthy);
}

#[test]
fn a_fault_is_listed_whatever_the_lifecycle_but_only_a_run_can_be_degraded_by_it() {
    let (mut s, provider, events) = scripted(HealthPolicy {
        max_recovery_attempts: 1,
        ..policy()
    });
    // Stopped: listed, and still stopped.
    s.report_fault(t(T0), "persistence", "read-only".into());
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Stopped);
    assert_eq!(report.active_faults, ["persistence"]);
    assert!(report.causes.is_empty());

    // A run begins degraded, because the fault stands.
    s.start(t(T0 + SEC)).unwrap();
    assert_eq!(check(&s, &events).health, HealthState::Degraded);

    // Recovering: the fault does not hide that there is no run.
    provider.fail_next(pipeline_error());
    s.poll(t(T0 + SEC)).unwrap_err();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Recovering);
    assert_eq!(report.active_faults, ["persistence"]);
    assert!(report.causes.is_empty());

    // Restarted: on probation and with the fault, both named.
    s.poll(t(T0 + 2 * SEC)).unwrap().expect("restarted");
    assert_eq!(
        check(&s, &events).causes,
        [Cause::Probation, Cause::ExternalFault("persistence")]
    );

    // Failed for good: failed, not degraded.
    provider.fail_next(pipeline_error());
    s.poll(t(T0 + 3 * SEC)).unwrap_err();
    let report = check(&s, &events);
    assert_eq!(report.health, HealthState::Failed);
    assert_eq!(report.active_faults, ["persistence"]);

    // It outlives the session and is cleared only by the caller.
    s.stop().unwrap();
    assert_eq!(check(&s, &events).active_faults, ["persistence"]);
    assert!(s.clear_fault(t(T0 + 9 * SEC), "persistence"));
    s.start(t(T0 + 10 * SEC)).unwrap();
    assert_eq!(check(&s, &events).health, HealthState::Healthy);
}

#[test]
fn every_cause_is_reported_at_once() {
    let (mut s, provider, events) = scripted(HealthPolicy {
        backoff_initial_s: 0.0,
        ..policy()
    });
    s.start(t(T0)).unwrap();
    provider.fail_next(pipeline_error());
    s.poll(t(T0)).unwrap_err();
    // Restarted (probation), far behind, with a fault, and then silent.
    provider.push(Step::Emit {
        at: None,
        missed: 9,
    });
    s.poll(t(T0 + SEC)).unwrap().expect("restarted");
    s.report_fault(t(T0 + SEC), "persistence", "disk full".into());
    let report = s.check(t(T0 + 7 * SEC));
    assert_consistent(&report);
    assert_eq!(
        report.causes,
        [
            Cause::Stalled,
            Cause::FallingBehind,
            Cause::Probation,
            Cause::ExternalFault("persistence")
        ]
    );
    assert_eq!(report.health, HealthState::Degraded);
    check(&s, &events);
}

// --- Nothing misleading, over long random histories ------------------------------------

/// A long random history on a scripted provider: everything that can
/// happen, in any order. After every single step the report must be
/// consistent and every total must be recountable from the events.
#[test]
fn reports_stay_consistent_and_totals_recountable_through_random_histories() {
    let mut rng = Rng::from_seed(0xACC0);
    let (mut steps, mut failures, mut rejected, mut withheld) = (0u64, 0u64, 0u64, 0u64);
    let mut seen = [false; 5];
    for case in 0..250 {
        let (mut s, provider, events) = scripted(HealthPolicy {
            max_recovery_attempts: (rng.next_u64() % 4) as u32,
            backoff_initial_s: [0.0, 0.5, 2.0][(rng.next_u64() % 3) as usize],
            backoff_multiplier: 2.0,
            backoff_max_s: 8.0,
            stable_after_s: 3.0,
            stall_after_s: 4.0,
            missed_window_ticks: 5,
            max_missed_in_window: 1,
        });
        let mut now = T0 + (case as i64) * SEC;
        for _ in 0..160 {
            now += (rng.uniform(0.0, 2.5) * 1e9) as i64;
            match rng.next_u64() % 40 {
                0 | 1 => {
                    let _ = s.start(t(now));
                }
                2 => {
                    let _ = s.stop();
                }
                3 | 4 => {
                    let _ = s.pause(t(now));
                }
                5 | 6 => {
                    let _ = s.resume(t(now));
                }
                7..=9 => {
                    s.check(t(now));
                }
                10 => s.report_fault(t(now), ["persistence", "delivery"][case % 2], "x".into()),
                11 => {
                    s.clear_fault(t(now), ["persistence", "delivery"][case % 2]);
                }
                12 => provider.fail_next(pipeline_error()),
                13 => provider.fail_next(ProviderError::Schedule(ScheduleError::Overflow)),
                14 => provider
                    .script()
                    .start_results
                    .push_back(Some(pipeline_error())),
                15 => provider.push(Step::Emit {
                    at: Some(now - 3_600 * SEC), // a stale timestamp
                    missed: 0,
                }),
                16 | 17 => provider.push(Step::Emit {
                    at: None,
                    missed: rng.next_u64() % 4,
                }),
                18 => provider.push(Step::Fail {
                    error: pipeline_error(),
                    missed: rng.next_u64() % 6,
                }),
                19 => provider.push(Step::Nothing),
                // Time running backwards now and then.
                20 => {
                    provider.push(Step::Nothing);
                    let _ = s.poll(t(now - 5 * SEC));
                }
                _ => {
                    let _ = s.poll(t(now));
                }
            }
            let report = check(&s, &events);
            seen[match report.health {
                HealthState::Stopped => 0,
                HealthState::Healthy => 1,
                HealthState::Degraded => 2,
                HealthState::Recovering => 3,
                HealthState::Failed => 4,
            }] = true;
            // What the supervisor says of the provider is true of it.
            if matches!(
                report.lifecycle,
                SimulationState::Running | SimulationState::Paused
            ) {
                assert_eq!(provider.state(), report.lifecycle, "case {case}");
            }
            steps += 1;
        }
        let totals = check(&s, &events).totals;
        failures += totals.failures;
        rejected += totals.rejected_operations;
        withheld += totals.withheld;
    }
    println!(
        "histories: {steps} steps checked; {failures} failed runs, {rejected} rejected calls, \
         {withheld} withheld samples"
    );
    // The histories really went everywhere.
    assert_eq!(seen, [true; 5]);
    assert!(failures > 300 && rejected > 300 && withheld > 20);
}

#[test]
fn the_real_provider_through_failures_restarts_and_late_polls_reconciles() {
    let mut rng = Rng::from_seed(0xACC1);
    for (name, _) in EXAMPLES {
        let scenario = import_scenario(example(name)).unwrap();
        let interval = scenario.update_interval_s;
        let (provider, _) = failing_provider(&scenario, 2, 25);
        let events = MemorySink::new();
        let mut s = Supervisor::for_simulation(
            provider,
            HealthPolicy {
                backoff_initial_s: interval,
                backoff_max_s: 4.0 * interval,
                stable_after_s: 10.0 * interval,
                stall_after_s: 6.0 * interval,
                ..policy()
            },
            Box::new(events.clone()),
        )
        .unwrap();
        s.start(t(T0)).unwrap();
        let mut now = T0;
        let mut samples = 0u64;
        for _ in 0..300 {
            if let Ok(Some(_)) = s.poll(t(now)) {
                samples += 1;
            }
            let report = check(&s, &events);
            if report.lifecycle == SimulationState::Running {
                // Nothing was withheld, so the two counts are the same.
                assert_eq!(
                    s.status().sample_count,
                    s.provider().status().sample_count,
                    "{name}"
                );
                assert_eq!(
                    s.status().missed_ticks,
                    s.provider().status().missed_ticks,
                    "{name}"
                );
            }
            now += (rng.uniform(0.4, 2.2) * interval * 1e9) as i64;
        }
        let report = check(&s, &events);
        assert_eq!(report.totals.samples, samples, "{name}");
        assert_eq!(report.totals.withheld, 0, "{name}");
        assert_eq!((report.totals.failures, report.run), (2, 3), "{name}");
        // Run 2 emits 25 samples before it fails, far longer than the
        // stable time of ten intervals, so its episode had ended: two
        // episodes, each completed by the run that followed.
        assert_eq!(report.totals.recoveries_completed, 2, "{name}");
        assert_eq!(report.lifecycle, SimulationState::Running, "{name}");
    }
}

// --- Privacy --------------------------------------------------------------------------

#[test]
fn no_event_reveals_a_position() {
    let mut searched = 0;
    for (name, _) in EXAMPLES {
        let scenario = import_scenario(example(name)).unwrap();
        let interval = scenario.update_interval_s;
        let (provider, _) = failing_provider(&scenario, 2, 12);
        let events = MemorySink::new();
        let mut s = Supervisor::new(
            provider,
            HealthPolicy {
                backoff_initial_s: 0.0,
                backoff_max_s: 0.0,
                stall_after_s: 3.0 * interval,
                ..policy()
            },
            Box::new(events.clone()),
        )
        .unwrap();
        s.start(t(T0)).unwrap();
        let mut now = T0;
        let mut positions = vec![
            scenario.origin.latitude().to_string(),
            scenario.origin.longitude().to_string(),
        ];
        for step in 0..120 {
            if let Ok(Some(sample)) = s.poll(t(now)) {
                positions.push(sample.coordinate.latitude().to_string());
                positions.push(sample.coordinate.longitude().to_string());
            }
            match step {
                30 => s.report_fault(t(now), "persistence", "could not replace the record".into()),
                // A long gap: a stall, then a late poll.
                60 => {
                    now += (20.0 * interval * 1e9) as i64;
                    s.check(t(now));
                }
                90 => {
                    let _ = s.pause(t(now));
                    let _ = s.pause(t(now)); // refused
                    let _ = s.resume(t(now));
                }
                _ => {}
            }
            now += (interval * 1e9) as i64;
        }
        s.stop().unwrap();

        let recorded = events.events();
        assert!(recorded.len() > 15, "{name}: {}", recorded.len());
        // Everything an event says, in both of its printed forms.
        let text: String = recorded.iter().map(|e| format!("{e}\n{e:?}\n")).collect();
        for position in &positions {
            // Skip values too short to be told from a count or a state.
            if position.trim_start_matches('-').len() < 5 {
                continue;
            }
            assert!(
                !text.contains(position.as_str()),
                "{name}: an event contains the coordinate {position}"
            );
            searched += 1;
        }
        // And the report: its error values and counts carry none either.
        let report = format!("{:?}", s.health());
        for position in positions.iter().filter(|p| p.len() >= 7) {
            assert!(!report.contains(position.as_str()), "{name}: {position}");
        }
    }
    println!("privacy: {searched} coordinate values searched for in the events of eight runs");
    assert!(searched > 1_000);
}
