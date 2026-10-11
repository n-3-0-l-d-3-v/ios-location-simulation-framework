//! What the C harness must get, produced without the C interface.
//!
//! Runs every example scenario on a `Supervisor` directly, driven by its
//! own deadlines exactly as `harness/harness.c` drives a session, and
//! prints each sample in the format the harness writes. `harness/run.sh`
//! compares the two files byte for byte.
//!
//!     cargo run -p locsim-ffi --example reference -- <scenario directory>

use locsim_core::domain::{LocationSource, SimulationState, Timestamp};
use locsim_core::provider::{LocationProvider, SimulationProvider};
use locsim_health::{HealthPolicy, NullSink, Supervisor};

const EXAMPLES: [&str; 8] = [
    "antimeridian_loop",
    "circular",
    "driving",
    "fixed",
    "high_latitude",
    "random_walk",
    "route_replay",
    "walking",
];
const SAMPLES: usize = 60;

fn main() {
    let directory = std::env::args().nth(1).expect("a scenario directory");
    // The policy of the harness's `good_policy`.
    let policy = HealthPolicy {
        max_recovery_attempts: 3,
        backoff_initial_s: 1.0,
        backoff_multiplier: 2.0,
        backoff_max_s: 60.0,
        stable_after_s: 10.0,
        stall_after_s: 5.0,
        missed_window_ticks: 10,
        max_missed_in_window: 2,
    };
    for name in EXAMPLES {
        let text = std::fs::read_to_string(format!("{directory}/{name}.json")).unwrap();
        let scenario = locsim_scenario::import_scenario(&text).unwrap();
        let mut supervisor = Supervisor::for_simulation(
            SimulationProvider::new(scenario),
            policy,
            Box::new(NullSink),
        )
        .unwrap();
        supervisor.start(Timestamp::from_nanos(0)).unwrap();
        for _ in 0..SAMPLES {
            let tick = supervisor.provider().next_deadline();
            let retry = supervisor.next_deadline();
            let deadline = match (tick, retry) {
                (Some(tick), Some(retry)) => tick.min(retry),
                (tick, retry) => tick.or(retry).expect("a deadline"),
            };
            let sample = supervisor.poll(deadline).unwrap().expect("a sample");
            let source = match sample.source {
                LocationSource::Simulation => 0,
                LocationSource::Replay => 1,
                LocationSource::Test => 2,
            };
            let state = match sample.simulation_state {
                SimulationState::Idle => 0,
                SimulationState::Starting => 1,
                SimulationState::Running => 2,
                SimulationState::Paused => 3,
                SimulationState::Stopping => 4,
                SimulationState::Error => 5,
                SimulationState::Recovering => 6,
            };
            let bits = |value: Option<f64>| value.map_or(0, f64::to_bits);
            println!(
                "{name} {} {} {source} {state} {} {} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
                sample.timestamp.as_nanos(),
                supervisor.health().run,
                u8::from(sample.speed_mps.is_some()),
                u8::from(sample.course_deg.is_some()),
                sample.coordinate.latitude().to_bits(),
                sample.coordinate.longitude().to_bits(),
                sample.altitude_m.to_bits(),
                sample.horizontal_accuracy_m.to_bits(),
                sample.vertical_accuracy_m.to_bits(),
                bits(sample.speed_mps),
                bits(sample.course_deg),
            );
        }
        let totals = supervisor.health().totals;
        println!(
            "{name} totals {} {} {}",
            totals.runs_started, totals.samples, totals.missed_ticks
        );
    }
}
