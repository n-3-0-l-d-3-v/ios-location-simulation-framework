# Progress

Ticket-level log and the source of truth for which ticket is current. For the
long-lived project context, the invariants to preserve and how to resume in a
new session, start at [Docs/PROJECT_CONTEXT/README.md](Docs/PROJECT_CONTEXT/README.md).

## Current Ticket
T11 — Platform adapter (open), split into two parts:
- **T11A — the C interface** (`crates/locsim-ffi`). **Closed by the project
  owner on 2026-10-11**, after review of its five implementation stages and
  its documentation stage. Its scope is what was run: Windows x86_64, in
  the configurations of `Docs/COMPATIBILITY.md`, with the limitations
  listed under "Known Issues".
- **T11B — everything Apple** (building for Apple targets, Swift,
  `CLLocation`, a driver and its clock, GPX, the Simulator). **Not started
  and not authorised.** Its acceptance criteria are approved as a planning
  baseline; the decisions they list as prerequisites have not been made,
  and it needs a Mac with Xcode.

T11 is complete only when T11B has been implemented and run with Apple
tooling.

## Status
T00–T10 complete and tested on Windows. T11A is implemented and tested on
Windows, from Rust and from a C program. No Swift adapter and no app
exist. On 2026-10-12 two readiness experiments ran on a hosted macOS
runner (below); they are experiments, and T11B is not started.
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
- T11A The C interface (`crates/locsim-ffi`, `include/locsim.h`, a C
  harness): the first part of T11, closed 2026-10-11. Validated on Windows
  x86_64 only. What it delivered is summarised under "Latest closed work"
  below; its history is in `Docs/PROJECT_CONTEXT/TICKET_HISTORY.md`.
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
- None in flight. T11B has not been started or authorised.

## Readiness experiments for T11B (2026-10-12)

Two experiments the project owner authorised, and a proposal. **T11B is
still not started and not authorised; none of this is acceptance evidence
for it, and no row of `Docs/COMPATIBILITY.md` was changed.**

- **Long-gap recording** (`Docs/Diagnostics/long_gap`,
  `crates/locsim-health/examples/gap_diagnostic.rs`). 1056 cases: eight
  scenarios, bare and supervised, a late poll or a pause and resume of 0 s
  to 86400 s, at three places. Nothing in the engine changed.
  - A late poll of any length gave a sample every time.
  - The pause/resume defect reproduces in 17 cases on each pipeline:
    `antimeridian_loop` for pauses of 1 to 30 s, `driving` mid-run for 1
    and 2 s.
  - New: in every moving scenario, with or without an error, the first
    sample after a resume is one interval of movement stamped one interval
    plus the pause later, so its reported speed collapses (`driving`, 5 s
    pause: 24.4, then 2.3, then 26.9 m/s, all accepted).
  - New: on macOS arm64 every outcome is the same as on Windows and the
    numbers differ in their last digits. The engine is repeatable on a
    host and not bit-for-bit the same across these two. Cause unknown.
  - `tests/gap_recording.rs` pins the recording (outcomes on every host,
    numbers on Windows x86_64).
- **Apple packaging** (`Docs/Diagnostics/apple_packaging`,
  `Scripts/apple/build-xcframework.sh`, `Apple/PackagingProbe`, workflow
  job `packaging`). On a hosted macOS runner, Xcode 16.4, Rust 1.98.1,
  iPhone 16 Pro Simulator on iOS 18.6: the static library for
  `aarch64-apple-ios-sim` was packaged as an XCFramework with the
  unmodified header, a Swift test target compiled against it, linked with
  no linker diagnostic, and ran four tests in the Simulator (ABI version;
  create and destroy a session; a refused document). A missing or damaged
  artifact stops the build. The consumer is a fixture, not the adapter. No
  sample was polled on iOS, no app exists, Core Location and GPX are
  untouched.
- **`nm`** exits 1 on the library: Xcode's LLVM cannot read the bitcode in
  Rust's standard-library objects. With the bitcode reader off it reads
  everything. A consumer built with link-time optimisation is untested.
- **Proposal, not adopted:** `Docs/Proposals/IOS_COMPATIBILITY_POLICY.md`.
  iPhone 16 on iOS 18.7.8 is in it as Untested.

## Latest closed work: T11A
Closed on 2026-10-11. It delivered a fifth crate,
`crates/locsim-ffi`, depending on the core, health and scenario crates and
on nothing outside the workspace:
- **One thing is exposed: a supervised simulation session**, built from a
  scenario document and a health policy. Fifteen functions: version, create,
  destroy, start, stop, pause, resume, poll, next deadline, check, health,
  report fault, clear fault, next event, events dropped. The store is not
  exposed, and there is no simulation mathematics in the crate.
- **The header**, `crates/locsim-ffi/include/locsim.h`, with the contract
  for pointers, outputs, time, threads and panics. The compiler checks every
  struct's size, alignment and field offsets.
- **A session is an integer handle**, never a pointer. A handle that was
  destroyed or never issued is a reported error. No handle is issued twice.
- **No call waits for a session.** One in use is `LOCSIM_ERR_BUSY`, and
  nothing happened. Destroying a session another thread is using is allowed
  and is refused the same way until that call returns.
- **A panic does not reach the caller.** The call returns
  `LOCSIM_ERR_PANIC`, the session answers `LOCSIM_ERR_POISONED` until it is
  destroyed, and other sessions are unaffected.
- **Events** wait in a bounded queue of fixed-size records: times, counts
  and codes, no text and no position. A full queue discards its oldest
  record and counts it.
- **A fault** is reported by the code of one of three components. There is
  no text parameter.
- **This is the only crate with `unsafe`**; the other four still forbid it.
- **The C harness**, `crates/locsim-ffi/harness/harness.c`, built and run by
  `crates/locsim-ffi/harness/run.sh`. **`cargo test` does not run it.**
- The four older crates: no behaviour changed. One doc comment was reworded
  (`Timestamp`, see "Time" below).

T11A decisions are D18; its contract is C22; its history is in
`Docs/PROJECT_CONTEXT/TICKET_HISTORY.md`.

### Time
A timestamp is nanoseconds on a time scale the caller chooses, with an
arbitrary origin. Samples, deadlines, reports and events are on the same
scale. Callers should not pass a time earlier than one already given to the
same session. Unix-epoch times are not required; the core's `SystemClock`
produces them by its own choice. (Contract C2.)

## Tests
Last full run (2026-10-12, Windows 11, rustc 1.98.1, commit `6bcf2c2`):
**595 passed, 0 failed** (the 594 of `e7fad44` and
`locsim-health`'s `gap_recording`). `cargo fmt --all -- --check` clean;
`cargo clippy --all-targets --all-features -- -D warnings` clean. The
same 595 passed on macOS 15.7 arm64, rustc 1.98.1, in workflow run
38239633331 at `72d4c7b`; that run is a probe, and the breakdown below is
that of `e7fad44`.

The C harness is a separate command, `crates/locsim-ffi/harness/run.sh`.
Last run at `e7fad44`: exit 0 in all six configurations (MSVC against the
DLL and against the static library, MinGW gcc against the DLL; debug and
release), about 385 000 checks each.

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
| `locsim-ffi` | Unit: `registry` 8, `session` 28, `ops` 29, `events` 5, `codes` 5 | 75 |
| `locsim-ffi` | `header` | 3 |

T11A figures:
- Every call of every test script is made on a session and on a supervisor
  built alike, and after each call the status, the sample, the next
  deadline, the health report and the events must be equal: all eight
  examples, three time origins, a failing run and its restarts, a stall,
  faults, a pause. All twenty kinds of event are produced.
- Shifting every time of a script by a constant shifts the timestamps and
  deadlines by it and changes nothing else, bit for bit.
- From C: about 385 000 checks per run; the samples of all eight examples
  equal the Rust supervisor's byte for byte; 3 000 create, use and destroy
  cycles; the limit of 1 024 sessions; two threads on one session (about
  180 000 calls refused as busy per run, none that did anything else); 300
  rounds of destroying a session in use; eight threads with sessions of
  their own.
- Mutation checks: 29 deliberate breaks (6 in stage 1, 8 in stage 2, 1 in
  its follow-up, 10 in stage 3, 4 against the C harness: two in the header,
  two in the library), all caught and restored.

Findings during T11A:
- **A natural pipeline failure exists.** Pausing and resuming two of the
  example scenarios makes the gate reject a sample. It is a defect of the
  core, open, and described under "Known Issues" below.
- `CoordinateError` carries the out-of-range value it refused, so the text
  of a provider's error can contain a latitude or longitude that was never
  emitted. The C interface passes an error's code only.
- A test that failed while holding a session left it registered, and the
  tests that count sessions then failed too. The test guards now destroy
  what a test created.
- Wrong expectations of mine, corrected by derivation: how many samples a
  script yields; that every example runs through a pause; in the C harness,
  how many calls a lifecycle walk refuses, and that a session two threads
  poll stays "healthy" (a poll refused as busy did not happen, so ticks are
  missed).

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
  public `ModelFactory`, or a scripted provider). *(True of the suite as it
  was at T10. T11A found one: the pause/resume defect under "Known
  Issues".)*
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
  - Not verified: any build but Windows. A supervisor is reached from
    several threads only through `locsim-ffi`, one call at a time.
- **Persistence (T09):** nothing run on a Unix-like system; nothing tested
  under power loss; on Windows the rename is not confirmed durable; one
  writer per directory; no checkpoint/resume (unowned).
- **History:** commit `019ad0f` (T08 examples) carries the message of
  `7547565` by mistake. History is left as it is.
- Scenario documents (T08): no defaults; seeds are strings; import does not
  admit a route; only schema version 1 exists.
- `rust-version = "1.75"` is unverified for all five crates.
- Carried over: speed is an interval mean; the first sample of a run has no
  speed or course; the noise engine computes a speed and course nothing
  uses; raw GPS logs are usually not admissible as routes; no real-time
  driver; nothing has run on iOS.
- **The C interface (T11A):**
  - Built and run on Windows x86_64 only, with the library built by the
    MSVC Rust toolchain. See `Docs/COMPATIBILITY.md` for exactly what was
    run and what was not.
  - Nothing calls a session: there is no timer or driver. A stall is
    noticed only by a call.
  - Two calls on one session do not wait: one is refused as busy and did
    not happen. A poll that was refused can miss its tick; one thread
    should poll a session.
  - The C harness cannot force two calls to collide, and cannot reach the
    panic paths. Those are forced only in the Rust tests.
  - Health reports list causes and faults as bit sets: their order is not
    kept. An error crosses as its code; what it carried does not.
  - The events of a session that panicked are lost with it.
  - Argument types in the header are checked by use, not mechanically. The
    header has not been compiled as C++.
  - `unsafe` is checked by tests and by reading; Miri has not been run.
  - A change to the header of any kind needs a new `LOCSIM_ABI_VERSION`.
- **Open defect in the core: a sample after a pause and a resume can be
  rejected by the gate. Unresolved.** Found during T11A; it is in
  `locsim-core`, it is not caused by `locsim-ffi`, and T11A did not change
  it.
  - *Observed.* On `SimulationProvider` alone, for `antimeridian_loop.json`
    and `driving.json`: after `pause` and `resume`, the first sample is
    refused with `ValidationError::DecelerationExceeded`, or it is emitted
    and the next one is refused with `AccelerationExceeded`. Seen for
    pauses from about 10 ms to about 10 s. Pauses of 1 ms and of a minute
    or more passed, and the other six examples passed at every length
    tried. The supervisor then replaces the run, as for any failed run.
  - *Suspected cause, not proven.* How four things meet: a resume shifts
    the schedule, so the next timestamp is one interval plus the pause
    after the last; the movement model's time does not advance during a
    pause; speed is derived as distance over the time between timestamps;
    and the validator limits how much that speed may change between
    samples. One traced case agrees with this arithmetic. No fix has been
    written or tried, so the explanation is untested.
  - *Why the existing tests pass.* The two pause tests in the core pause for
    an hour and for a day, which are among the lengths that pass.
  - *What a fix needs.* Its own ticket. Tests over a range of pause lengths
    on a fast mover, and a test that a real discontinuity (a teleport)
    across a pause is still rejected, **before** any exemption from
    validation is considered. Nothing has been exempted or loosened.

## Compatibility
- All five crates: Verified on Windows 11 x86_64 / rustc 1.98.1
  (`x86_64-pc-windows-msvc`) only.
- The C interface: Verified from a C caller compiled by MSVC (DLL and
  static library) and by MinGW gcc (DLL), debug and release, on that same
  machine. Not built with the Rust GNU toolchain, not compiled as C++, not
  run on any other system.
- `locsim-health` is logic only (no files, clock, threads or platform
  calls), but it has still been built and run on Windows alone.
- macOS, Linux, iOS, the iOS Simulator, physical devices, jailbroken
  devices, Rust 1.75: Untested. No adapter exists.
- The detail is `Docs/COMPATIBILITY.md`.

## Next
- **Nothing is authorised to start.** T11A is closed; T11 stays open.
- **Decisions waiting for the project owner** after the experiments of
  2026-10-12: the ones listed at the end of
  `Docs/Proposals/IOS_COMPATIBILITY_POLICY.md`, and what the engine
  should do across a pause (`Docs/Diagnostics/long_gap/README.md`).
- **T11B**, when the project owner authorises it and a Mac with Xcode is
  available. Before any implementation, the prerequisites listed with its
  acceptance criteria need explicit decisions. Agreed areas: build and
  link the crates for Apple targets; consume `locsim.h` from Swift; convert
  a sample to a `CLLocation`; a driver that calls `poll` and `check` on a
  real clock, with a decision about a suspended app; GPX that `simctl` and
  Xcode accept; runs on the Simulator. Its acceptance criteria are written
  and approved as a planning baseline
  (`Docs/PROJECT_CONTEXT/PROJECT_BRIEF.md`, "Acceptance criteria of T11B");
  its design is not written, and the decisions those criteria wait for
  have not been made. It cannot be done, or partly done, on the Windows machine used
  so far; a hosted macOS runner has run the two experiments above and
  nothing interactive.
- **The pause/resume defect** needs a ticket of its own (see "Known
  Issues").
- What T11B can build on: the header is the whole interface; a session is
  driven by `locsim_session_next_deadline`, `locsim_session_poll` and
  `locsim_session_check`; a change of `run` on a sample marks a
  discontinuity; unknown speed and course are flagged, and must become Core
  Location's `-1`, never `0`.
- What T11B must not do: put simulation mathematics on the Swift side;
  claim to replace the system's location (the scope is an app built with
  the adapter); conceal anything or evade any check (C18); mark anything as
  working on iOS that was not run there.
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
  - `cargo test` does not run the C harness. After any change to
    `locsim-ffi`, run `crates/locsim-ffi/harness/run.sh` as well.
  - A test in `locsim-ffi` that creates a session takes the `shared()` or
    `alone()` guard first.

## Working agreement
- One ticket at a time; the specification is the contract.
- For a ticket with design decisions: investigate, propose, get approval,
  then implement in agreed stages, each with its own gate.
- One commit per logical change, each tested and pushed immediately.
