//! Shared by the scenario integration tests.
#![allow(dead_code)]

/// The core's own test helpers, including the independent re-check of an
/// emitted stream (`recheck_stream`), reused rather than restated a second
/// time.
#[path = "../../../locsim-core/tests/common/mod.rs"]
pub mod common;

use common::START;
use locsim_core::domain::{Scenario, SyntheticLocation, Timestamp};
use locsim_core::provider::{LocationProvider, ProviderStatus, SimulationProvider};
use locsim_core::rng::Rng;
use std::fmt::Debug;

macro_rules! example {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!("../../../../Examples/Scenarios/", $name, ".json")),
        )
    };
}

/// Every file in `Examples/Scenarios`, by name without extension.
pub const EXAMPLES: [(&str, &str); 8] = [
    example!("fixed"),
    example!("random_walk"),
    example!("walking"),
    example!("driving"),
    example!("circular"),
    example!("route_replay"),
    example!("antimeridian_loop"),
    example!("high_latitude"),
];

pub fn example(name: &str) -> &'static str {
    EXAMPLES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no example {name}"))
        .1
}

/// `Debug` prints floats in their shortest round-trip form and keeps the
/// sign of zero: equal text means equal bits in every field.
pub fn fingerprint<T: Debug>(value: &T) -> String {
    format!("{value:?}")
}

/// Starts a provider on `scenario` and polls it `polls` times through the
/// real pipeline and final gate. With `jitter`, gaps vary between 0.3 and
/// 2.5 intervals, so ticks are skipped and timestamps are irregular. Panics
/// if any sample is rejected.
pub fn run(
    scenario: &Scenario,
    polls: usize,
    jitter: Option<u64>,
) -> (Vec<SyntheticLocation>, ProviderStatus) {
    let interval = scenario.update_interval_s * 1e9;
    let mut rng = Rng::from_seed(jitter.unwrap_or(0));
    let mut provider = SimulationProvider::new(scenario.clone());
    provider
        .start(Timestamp::from_nanos(START))
        .unwrap_or_else(|e| panic!("{}: start refused: {e}", scenario.name));
    let mut now = START;
    let mut stream = Vec::new();
    for _ in 0..polls {
        match provider.poll(Timestamp::from_nanos(now)) {
            Ok(Some(sample)) => stream.push(sample),
            Ok(None) => {}
            Err(e) => panic!("{}: sample {} rejected: {e}", scenario.name, stream.len()),
        }
        let gap = if jitter.is_some() {
            rng.uniform(0.3, 2.5)
        } else {
            1.0
        };
        now += (gap * interval) as i64;
    }
    let status = provider.status();
    assert_eq!(status.failed_count, 0, "{}", scenario.name);
    (stream, status)
}
