//! T10: a supervisor that has nothing to do changes nothing.
//!
//! Every example scenario is run twice, bare and supervised, with the same
//! polling. The two streams must be identical bit for bit, and the
//! supervised one is re-checked independently of the gate.

mod support;

use locsim_core::domain::{HealthState, SimulationState};
use locsim_core::provider::{LocationProvider, SimulationProvider};
use locsim_health::{MemorySink, Supervisor};
use locsim_scenario::import_scenario;
use support::scenario::common::recheck_stream;
use support::scenario::{example, fingerprint, EXAMPLES};
use support::*;

#[test]
fn a_supervised_stream_is_the_bare_stream_bit_for_bit() {
    let mut pairs = 0;
    for (name, _) in EXAMPLES {
        let scenario = import_scenario(example(name)).unwrap();
        for jitter in [None, Some(11)] {
            let mut bare = SimulationProvider::new(scenario.clone());
            bare.start(t(T0)).unwrap();
            let expected = drive(&mut bare, scenario.update_interval_s, 500, jitter);

            let events = MemorySink::new();
            let mut supervised = Supervisor::new(
                SimulationProvider::new(scenario.clone()),
                policy(),
                Box::new(events.clone()),
            )
            .unwrap();
            supervised.start(t(T0)).unwrap();
            let stream = drive(&mut supervised, scenario.update_interval_s, 500, jitter);

            assert_eq!(fingerprint(&stream), fingerprint(&expected), "{name}");
            assert!(stream.len() >= 300, "{name}");
            pairs += recheck_stream(&scenario, &stream, name).pairs;

            // The supervisor's account of the run is the provider's.
            let report = check(&supervised, &events);
            let inner = supervised.provider().status();
            assert_eq!(supervised.status(), inner, "{name}");
            assert_eq!(bare.status(), inner, "{name}");
            assert_eq!(supervised.current_location(), bare.current_location());
            assert_eq!(report.lifecycle, SimulationState::Running);
            assert_eq!(report.totals.samples, stream.len() as u64);
            assert_eq!(report.totals.withheld, 0);
            assert_eq!(report.totals.missed_ticks, inner.missed_ticks);
            assert_eq!(report.totals.failures, 0);
            assert_eq!(report.run, 1);
            assert!(matches!(
                report.health,
                HealthState::Healthy | HealthState::Degraded
            ));
        }
    }
    println!("pass-through: {pairs} sample pairs re-checked");
    assert!(pairs > 5_000);
}

#[test]
fn pausing_through_the_supervisor_is_pausing_the_provider() {
    let scenario = import_scenario(example("driving")).unwrap();
    let interval = (scenario.update_interval_s * 1e9) as i64;
    let mut bare = SimulationProvider::new(scenario.clone());
    let mut supervised = Supervisor::new(
        SimulationProvider::new(scenario),
        policy(),
        Box::new(MemorySink::new()),
    )
    .unwrap();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    bare.start(t(T0)).unwrap();
    supervised.start(t(T0)).unwrap();
    for tick in 0..40 {
        let now = t(T0 + tick * interval);
        a.extend(bare.poll(now).unwrap());
        b.extend(supervised.poll(now).unwrap());
    }
    let (paused, resumed) = (t(T0 + 40 * interval), t(T0 + 1_000 * interval));
    bare.pause(paused).unwrap();
    supervised.pause(paused).unwrap();
    bare.resume(resumed).unwrap();
    supervised.resume(resumed).unwrap();
    for tick in 0..40 {
        let now = t(T0 + (1_000 + tick) * interval);
        a.extend(bare.poll(now).unwrap());
        b.extend(supervised.poll(now).unwrap());
    }
    assert_eq!(a.len(), 80);
    assert_eq!(fingerprint(&a), fingerprint(&b));
}
