# Progress

Ticket-level log and the source of truth for which ticket is current. For the
long-lived project context, the invariants to preserve and how to resume in a
new session, start at [Docs/PROJECT_CONTEXT/README.md](Docs/PROJECT_CONTEXT/README.md).

## Current Ticket
T11 — Platform adapter (not started)

## Status
T00–T10 complete and tested on Windows. No iOS code exists yet.
Repository: https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework

## Completed
- T00 Repository bootstrap.
- T01 Domain model.
- T02 Geographic engine (Vincenty, ENU/ECEF, seeded PRNG).
- T03 Fixed location engine (scheduler, validator, provider).
- T04 Noise engine.
- T05 Movement engine (circular, steered: random walk / walking / driving).
- T06 Route engine (ECEF spline, rigorous admission, RouteModel).
- T07 Consistency engine.
- T08 Scenario system (`crates/locsim-scenario`: strict, versioned JSON).
- T09 Persistence (`crates/locsim-store`: scenario and last-known record,
  digest, atomic replacement; no checkpoint/resume).
- T10 Health system. Details of T00–T09 are in
  `Docs/PROJECT_CONTEXT/TICKET_HISTORY.md`. T10 delivered a new crate,
  `crates/locsim-health`, depending on `locsim-core` only:
  - `Supervisor<P: LocationProvider>`, itself a `LocationProvider`. It
    hands samples on unchanged, ends a run that fails, restarts within a
    bound, watches for stalls and missed ticks, and accepts fault reports
    from the caller. It never reads a clock and never sleeps.
  - `HealthReport`: `lifecycle` (what the supervisor is doing), `emission`
    (whether samples are arriving), `health` (Stopped, Healthy, Degraded,
    Recovering, Failed; derived, never stored), every active `Cause`, the
    run number, attempts used, retry time, last error, cumulative `Totals`.
  - **Recovery is a new run**, not a resume: the scenario starts again from
    its beginning, the first sample has no speed, the position jumps, the
    run number increases. A failure that does not depend on timing recurs.
  - `HealthPolicy` (eight required fields, no default): restart bound,
    backoff, stable time, stall threshold, missed-tick window.
  - A refused call (`OperationRejected`; nothing changes) is distinct from
    a failed run (`RunFailed`). A refused initial start leaves it Stopped.
  - Within a session, timestamps handed on strictly increase, across
    restarts too; a stale one is withheld and ends the run.
  - Structured `Event`s to a caller-supplied `EventSink`; no coordinate in
    any event; no event per sample. Totals are updated from the events.
  - `locsim-core`, `locsim-scenario` and `locsim-store`: no file changed.

## Current Work
- None in flight.

## Tests
Last full run (2026-10-10, Windows 11, rustc 1.98.1, commit `fcda25c`):
**516 passed, 0 failed.** `cargo fmt --check` clean;
`cargo clippy --all-targets --all-features -- -D warnings` clean.

| Crate | Suite | Tests |
|---|---|---|
| `locsim-core` | Unit 202, nine integration suites 71 (unchanged since T07) | 273 |
| `locsim-scenario` | Unit 56, `examples` 11, `last_known` 5, `malformed` 15, `round_trip` 2 | 89 |
| `locsim-store` | Unit 38, `scenario_store` 15, `last_known_store` 10, `platform` 4, `crash` 2 | 69 |
| `locsim-health` | Unit (policy, backoff, events) | 18 |
| `locsim-health` | `lifecycle` | 15 |
| `locsim-health` | `pass_through` | 2 |
| `locsim-health` | `recovery` | 25 |
| `locsim-health` | `watchdog` | 18 |
| `locsim-health` | `accounting` | 7 |

T10 figures, printed by the tests (`cargo test -p locsim-health -- --nocapture`):
- Pass-through: all eight examples, punctual and jittered, supervised stream
  bit-identical to the bare one; 7 560 pairs re-checked independently.
- 250 random histories of 160 steps on a scripted provider: after each of
  the 40 000 steps the report is consistent and every total equals a recount
  from the events; 600 failed runs, 5 756 rejected calls, 78 withheld
  samples; all five health states occur.
- Privacy: 1 897 coordinate values searched for in the Display and Debug
  text of every event and report of eight runs; none found.
- Backoff: a million attempts with the smallest multiplier above 1 in under
  a second, in range, non-decreasing, equal to the same recurrence written
  out again.
- The edge of the representable duration range walked double by double:
  407 below, one exactly on 2^63 ns, 642 above.
- Mutation checks: 36 deliberate breaks across the five stages, all caught
  and restored. Two were missed at first and a test was added for each
  (2^63 ns accepted; a failure episode surviving a stop).

Findings during T10:
- **Recovery is often futile, and the documents say so.** A run is
  deterministic; with the same polling, a restart fails at the same sample.
  A test shows four runs failing identically. Retries help only when the
  failure came from timing.
- **No pipeline failure has ever occurred naturally** in the suite. Every
  failure in the T10 tests is injected (a failing movement model through the
  public `ModelFactory`, or a scripted provider).
- **The stale-timestamp check never acts on the real provider.** With the
  check removed, only the scripted-provider tests fail. For
  `SimulationProvider` the order follows from the scheduler.
- The first sample after a restart is stamped by the schedule, at or after
  the restart, not necessarily at it (a late first poll skips ticks). An
  earlier draft of the design said "exactly at"; corrected before coding.
- Wrong expectations of mine in new tests, corrected by derivation: a
  scripted provider emitting at an earlier time (the supervisor was right to
  withhold it); one completed recovery expected where the design gives two.
- The implementation decides call legality from its own lifecycle rather
  than by calling the core's transition function, as the design had said. A
  test now checks the two agree in every state.

## Known Issues / Limitations
- **Health supervision:**
  - A stall is noticed only when something calls `check` or `poll`. There is
    no timer, thread or driver. If nothing calls the supervisor, it reports
    what it last saw.
  - Recovery restarts the scenario from its beginning. There is a position
    jump and no continuity; consumers must watch the run number.
  - A deterministic failure is retried up to the bound and fails each time.
  - The stall threshold is checked against the update interval only by
    `Supervisor::for_simulation`. With `Supervisor::new` a threshold shorter
    than the interval reports a provider that is on time as stalled.
  - Across an explicit `stop` and `start`, timestamps may go back (the
    core's own behaviour); order is guaranteed within a session only.
  - A custom provider's errors are classified by the `ProviderError`
    variant it chooses to return.
  - `stop` takes no time argument in the trait; its events carry the latest
    time the supervisor had seen.
  - `status()` of the supervisor counts samples handed on; after a withheld
    sample the wrapped provider's own count is one higher.
  - The last-known record (T09) carries no run number.
  - Persistence faults reach health only if the caller reports them; there
    is no bridge from `StoreError`.
  - Not verified: use from several threads; any build but Windows.
- **Persistence (T09):** nothing run on a Unix-like system; nothing tested
  under power loss; on Windows the rename is not confirmed durable; one
  writer per directory; no checkpoint/resume (unowned).
- **History:** commit `019ad0f` (T08 examples) carries the message of
  `7547565` by mistake. History is left as it is.
- Scenario documents (T08): no defaults; seeds are strings; import does not
  admit a route; only schema version 1 exists.
- `rust-version = "1.75"` is unverified for all four crates.
- Carried over: speed is an interval mean; the first sample of a run has no
  speed or course; the noise engine computes a speed and course nothing
  uses; raw GPS logs are usually not admissible as routes; no real-time
  driver; nothing has run on iOS.

## Compatibility
- All four crates: Verified on Windows 11 x86_64 / rustc 1.98.1 only.
- `locsim-health` is logic only (no files, clock, threads or platform
  calls), but it has still been built and run on Windows alone.
- iOS Simulator, physical devices, jailbroken devices: Untested (no adapter yet).

## Next
- T11 Platform adapter: the authorised iOS delivery/test adapter; document
  platform limitations.
- **T11 cannot be completed on the current machine.** It needs a Mac with
  Xcode for anything Swift and for any run on the Simulator or a device.
  What can be done on Windows is limited to a C ABI crate tested from Rust
  and, possibly, a compile check for the iOS target. See
  `Docs/PROJECT_CONTEXT/CURRENT_STATE.md`.
- What T11 can build on: `Supervisor` is a `LocationProvider`, so the
  adapter wraps one object; `next_deadline()` on the provider and the
  supervisor say when to call next; `HealthReport.run` marks
  discontinuities; `None` speed and course must become Core Location's `-1`.
- Gotchas:
  - Position and timestamp are authoritative; never emit speed/course from a
    model.
  - Gate is strict; engines keep `KINEMATIC_MARGIN` below limits.
  - Models receive simulated time, not wall time.
  - Gate every commit on its test result; unstage on failure.
  - Write the commit message file, read its first line back, and only then
    commit, as separate steps (`019ad0f`).
  - A shell heredoc containing a lone apostrophe or backslashes may not
    write what was typed on this machine; write such files with a tool.
  - `cargo test -p <crate>` stops at the first failing test binary; when
    checking that a particular suite catches a mutation, run that suite.
  - Derive expected numbers by hand. When a new test fails, decide whether
    the expectation or the code is wrong, and say which.

## Working agreement
- One ticket at a time; the specification is the contract.
- For a ticket with design decisions: investigate, propose, get approval,
  then implement in agreed stages, each with its own gate.
- One commit per logical change, each tested and pushed immediately.
