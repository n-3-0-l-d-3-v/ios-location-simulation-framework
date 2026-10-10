//! T09: a writer process killed in the middle of saving.
//!
//! The parent test starts this same test binary as a child that saves two
//! scenarios alternately, as fast as it can, and kills it after a random
//! delay. Whatever instant the kill lands on, the record must afterwards be
//! one of the two scenarios, whole and valid; never a mixture, never
//! damaged. This is repeated on the same directory.
//!
//! **What this shows and what it does not.** Killing a process loses that
//! process's memory, not the operating system's: data the kernel had
//! accepted still reaches the disk. So this tests the write sequence
//! against a crash of the program. It says nothing about power loss or a
//! crash of the operating system, where the sync requests are what matter;
//! those have not been tested.

mod support;

use locsim_core::domain::{Coordinate, Route, RoutePoint, Scenario};
use locsim_core::geographic::destination;
use locsim_core::rng::Rng;
use locsim_scenario::import_scenario;
use locsim_store::Store;
use std::process::{Command, Stdio};
use std::time::Duration;
use support::scenario::example;
use support::Scratch;

const DIRECTORY: &str = "LOCSIM_STORE_CRASH_DIRECTORY";

/// Two large scenarios (a route of 3 000 points each, about a megabyte of
/// text), so that a save takes long enough for a kill to land inside it.
fn scenarios() -> [Scenario; 2] {
    let replay = import_scenario(example("route_replay")).unwrap();
    let route = |bearing: f64| {
        let start = Coordinate::new(12.9352, 77.6245).unwrap();
        let points: Vec<RoutePoint> = (0..3_000)
            .map(|j| {
                let position = destination(start, bearing, 0.01 * j as f64).unwrap();
                RoutePoint::new(j * 1_000_000_000, position)
            })
            .collect();
        Some(Route::new(points).unwrap())
    };
    [
        Scenario {
            name: "A long, slow route east".into(),
            route: route(80.0),
            ..replay.clone()
        },
        Scenario {
            name: "A long, slow route north".into(),
            route: route(10.0),
            ..replay
        },
    ]
}

/// The child. Without the environment variable it is an ordinary test that
/// does nothing; with it, it saves until it is killed.
#[test]
fn writer_child() {
    let Ok(directory) = std::env::var(DIRECTORY) else {
        return;
    };
    let store = Store::open(directory).expect("store directory");
    let scenarios = scenarios();
    loop {
        for scenario in &scenarios {
            store.save_scenario(scenario).expect("save");
        }
    }
}

#[test]
fn a_writer_killed_at_any_moment_leaves_a_whole_valid_record() {
    let scratch = Scratch::new("crash");
    let store = Store::open(scratch.path()).unwrap();
    let scenarios = scenarios();
    let mut rng = Rng::from_seed(0xC4A5);
    let (mut found, mut leftovers, mut none) = ([0u32; 2], 0usize, 0u32);
    let rounds = 40;
    for round in 0..rounds {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "writer_child", "--nocapture", "--test-threads=1"])
            .env(DIRECTORY, scratch.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the writer");
        // Long enough for the child to start and be somewhere in its loop.
        std::thread::sleep(Duration::from_millis(150 + rng.next_u64() % 250));
        child.kill().expect("kill the writer");
        child.wait().expect("reap the writer");

        match store.load_scenario() {
            Ok(Some(scenario)) => match scenarios.iter().position(|s| *s == scenario) {
                Some(which) => found[which] += 1,
                None => panic!("round {round}: a scenario that was never saved"),
            },
            // Possible only if the child was killed before its first save.
            Ok(None) => {
                assert!(found == [0, 0], "round {round}: the record disappeared");
                none += 1;
            }
            Err(e) => panic!("round {round}: {e}"),
        }
        // An interrupted save may leave its temporary file. It is never
        // mistaken for the record, and it can be removed.
        let stale = store.stale_temporaries().unwrap();
        assert!(stale.len() <= 1, "round {round}: {stale:?}");
        leftovers += stale.len();
        assert_eq!(store.remove_stale_temporaries(), Ok(stale.len()));
        assert_eq!(scratch.names().len(), usize::from(found != [0, 0]));
    }
    println!(
        "crash: {rounds} writers killed; the record was found whole as the first scenario {} times and as the second {} times, absent {none} times; {leftovers} interrupted saves left a temporary file",
        found[0], found[1]
    );
    // The writers really were writing.
    assert!(found[0] + found[1] > 0, "no writer ever completed a save");
}
