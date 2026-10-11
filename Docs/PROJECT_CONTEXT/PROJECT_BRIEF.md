# Project brief

## What this is

A **synthetic location framework** for authorised privacy testing, application
development and controlled research. It produces a stable, configurable,
geographically realistic stream of synthetic locations and is intended to
deliver it to iOS through a replaceable platform adapter.

It is meant to be an engineering project, not a proof of concept: strong
abstractions, validation, deterministic behaviour, recovery mechanisms,
honest compatibility documentation and extensive testing.

Priority order when goals conflict (Decision, from the original specification):

    correctness → stability → predictable behaviour → realistic simulation
                → performance → extensibility

The system is engineered for fault tolerance and observable failure. It is
never described as "fail-proof".

## Intended use cases

- Testing how an app you are developing behaves at a fixed place, along a
  route, while walking or driving, at the antimeridian, near a pole.
- Reproducing a location-dependent bug exactly, from a scenario and a seed.
- Privacy testing on devices you are authorised to test: presenting a
  synthetic location instead of a real one.
- Research in controlled environments that needs a known, repeatable
  location stream.

## Non-goals and safety boundaries

These are fixed. They are not "not yet"; they are "no".

**Do not implement, and do not accept contributions that implement:**

- mechanisms whose purpose is to conceal the framework or a jailbreak from
  security systems;
- bypasses of application integrity checks;
- evasion of third-party anti-spoofing or anti-fraud mechanisms;
- any claim, in code or documentation, that the framework is undetectable or
  untraceable.

Where the specification asks for internally consistent metadata (speed that
matches displacement, course that matches direction), the reason is
**simulation correctness and testability**, not evading detection. Keep the
documentation saying so.

Other non-goals:

- Reading the device's real location. The core never does.
- Modelling pedestrians, traffic or road networks. Movement "behaviour" is a
  seeded policy; see `ARCHITECTURE_DECISIONS.md`.
- Supporting a platform on the basis that it should work. See "Honesty rules".

## Target architecture and platform assumptions

    Control / Configuration        app, CLI                      planned (T12)
    Simulation Manager             owns state machine, wiring     partly: SimulationProvider
    Scenario Engine                strict versioned JSON           implemented (locsim-scenario)
    Movement Engine                fixed/walk/drive/orbit/replay  implemented
    Route Engine                   route -> trajectory, admission implemented
    Noise / Realism Engine         seeded jitter, drift           implemented
    Consistency Engine             speed/course from emitted fixes implemented
    Geographic Engine              WGS84 geodesics, ENU, ECEF     implemented
    Location Abstraction           LocationProvider trait         implemented (one provider)
    Platform Adapter               C ABI + Swift -> CLLocation    C ABI implemented (locsim-ffi, T11A); Swift planned (T11B)
    Health / Recovery              supervisor, watchdog, restart   implemented (locsim-health)
    Persistence / Storage          digest envelope, atomic replace implemented (locsim-store)

Assumptions (Decisions unless marked):

- The simulation core is a Rust crate with no dependencies, testable on any
  desktop OS with no Apple tooling. **Verified** on Windows only.
- Anything that needs a dependency lives in its own crate on top of the
  core. So far that is `locsim-scenario` (`serde`, `serde_json`).
  **Verified** on Windows only.
- Files are touched by one crate only, `locsim-store`, which has no
  third-party dependency. **Verified** on Windows only; iOS is a Unix-like
  system and that path has never run.
- iOS will be reached through a C ABI crate and a Swift adapter. The C ABI
  crate, `locsim-ffi`, exists and exposes one supervised session;
  **Verified** on Windows only, from Rust and from C. The Swift adapter is
  **Planned** (T11B). Nothing has been cross-compiled for
  `aarch64-apple-ios`. **Unverified** that any crate builds for it.
- A timestamp is a count of nanoseconds on a time scale the caller chooses,
  with an arbitrary origin; samples carry times on the same scale.
  **Decision** (2026-10-11). Unix-epoch times are not required.
- Delivery mechanisms considered in scope (all **Planned**, none designed in
  detail): `xcrun simctl location` and GPX export for the iOS Simulator; GPX
  for Xcode's device location simulation; in-app injection through the
  framework's own `LocationProvider` for an app under development; a
  mechanism for jailbroken research devices only once hardware is available
  to verify it.
- The development machine so far is Windows with Rust 1.98.1 and no Swift
  toolchain. **Verified** 2026-10-09. T11B and T12 will need a Mac.

## Implemented, verified, planned

| Capability | Status |
|---|---|
| Domain model, scenario validation | Implemented, tested on desktop |
| WGS84 geodesics, ENU/ECEF | Implemented, tested on desktop |
| Seeded PRNG | Implemented, tested on desktop |
| Drift-free scheduler, clocks | Implemented, tested on desktop |
| Movement: fixed, bounded random walk, walking, driving, circular | Implemented, tested on desktop |
| Route replay with admission | Implemented, tested on desktop |
| Noise engine | Implemented, tested on desktop |
| Consistency engine (derived speed/course) | Implemented, tested on desktop |
| Final validation gate | Implemented, tested on desktop |
| Provider with lifecycle, pause/resume, completion | Implemented, tested on desktop. **Open defect:** for some scenarios a sample after a pause and resume is rejected by the gate (see `PROGRESS.md`) |
| Real-time driver (something that sleeps until the next tick) | Not implemented |
| Scenario JSON import/export, schema versioning | Implemented, tested on desktop |
| Migration between schema versions | Mechanism implemented and tested with test-only steps; no real migration exists (only version 1) |
| GPX import/export | Not implemented. GPX for delivery belongs to T11B; import has no ticket |
| Persistence of a scenario and of the last emitted sample (digest, atomic replacement) | Implemented, tested on Windows only; Unix path never run; power loss never tested |
| Checkpoint / resume of a running simulation | **Not implemented; no ticket owns it.** A stored simulation can only be started again |
| Stored configuration and preferences | Not implemented: no such types exist; expected with the application (T12) |
| Health state, watchdog (stall, missed ticks), bounded restart, structured events | Implemented, tested on Windows only. Recovery is a new run, not a resume. Nothing calls the watchdog: there is no timer |
| Logging to a file or system log | Not implemented: events go to a sink the caller supplies, and through the C interface they wait in a queue the caller empties (platform work, T11B/T12) |
| CPU, memory and battery metrics | Not implemented (T14) |
| C ABI to a supervised session: header, handles, lifecycle, samples, health, faults, events | Implemented (T11A, awaiting closing review). Tested on Windows x86_64 only, from Rust and from a C program built with MSVC and with MinGW gcc |
| Swift adapter, `CLLocation` conversion, driver and clock, GPX for delivery | Planned (T11B). Needs a Mac with Xcode |
| Demo application | Planned (T12) |
| Wall-clock soak tests, memory measurements | Planned (T14) |
| Anything on iOS Simulator | **Never run** |
| Anything on a physical iPhone | **Never run** |
| Anything on a jailbroken device | **Never run, not designed** |
| Build with the declared minimum Rust (1.75) | **Never run** |
| Build on macOS or Linux | **Never run** |

## The ticket plan

The original specification was a prompt in the first conversation; it is not
in the repository. Its ticket plan is reproduced here so that it is.

| Ticket | Title | Scope | State |
|---|---|---|---|
| T00 | Repository bootstrap | Structure, README, PROGRESS, gitignore, initial architecture doc | Done |
| T01 | Domain model | SyntheticLocation, Scenario, MovementParameters, NoiseParameters, SimulationState, HealthState; unit tests | Done |
| T02 | Geographic engine | Distance, bearing, destination, interpolation, coordinate validation, local coordinate conversion; extensive tests | Done |
| T03 | Fixed location engine | Fixed-position simulation; scheduler, lifecycle, deterministic tests, validation | Done |
| T04 | Jitter engine | Bounded noise, seeded randomness, drift, configurable radius; statistical verification | Done |
| T05 | Movement engine | Walking, driving, bounded random walk, circular | Done |
| T06 | Route engine | Route loading, interpolation, playback, pause/resume, looping, playback speed | Done |
| T07 | Consistency engine | Consistency of speed, course, timestamps, accuracy, position | Done |
| T08 | Scenario system | Serialisation, validation, import/export, schema versioning (JSON; migration between versions) | Done |
| T09 | Persistence | Reliable configuration persistence and recovery: active scenario, configuration, last known state, route, preferences, schema version; atomic writes (write temporary, validate, replace) | Done for the scenario (with its route and schema version) and the last-known record. Configuration and preferences: nothing to store yet. Resume: deferred, see below |
| T10 | Health system | Health state, metrics, watchdog, bounded recovery (e.g. three retries with backoff, then FAILED), structured logging with levels | Done: supervisor, policy, events. "Metrics" are the counters and totals of the report; resource metrics are T14. "Logging" is structured events to a caller-supplied sink |
| **T11** | **Platform adapter** | **The authorised iOS delivery/test adapter; document platform limitations** | **Open. Split by the project owner on 2026-10-11 into T11A and T11B (below). Complete only when T11B is** |
| T11A | C interface | A Rust crate with a C ABI over one supervised session; a public header; a C harness; tested on Windows | Implementation stages done and reviewed; documentation stage awaiting review. Not closed |
| T11B | Apple integration | Build and link for Apple targets; Swift adapter; conversion to `CLLocation`; driver, clock and timer; GPX for `simctl` and Xcode; runs on the Simulator | Not started. Needs a Mac with Xcode |
| T12 | Test application | UI and visualisation: current location, speed, course, accuracy, mode, state, health, sample count; start/pause/resume/stop/reset/load/save; map where practical | Planned |
| T13 | Integration testing | Whole pipeline: scenario → movement → noise → validation → consistency → scheduler → adapter → app | Planned |
| T14 | Reliability testing | Long-duration runs, repeated start/stop, pause/resume, failure injection, corrupted configuration, memory | Planned |
| T15 | Documentation | Architecture, compatibility, setup, testing, limitations, troubleshooting | Planned |

### Acceptance criteria of T11A

T11A is accepted when all of these hold. Whether they do is for the project
owner to say; this list does not close the ticket.

1. **The Rust gates pass** on the tree to be accepted: `cargo test`,
   `cargo fmt --all -- --check`,
   `cargo clippy --all-targets --all-features -- -D warnings`,
   `cargo clippy --workspace --all-targets -q -- -D warnings`,
   `cargo build -p locsim-ffi`. The tests of the four older crates are
   intact: none removed, none weakened.
2. **The C harness passes**, `crates/locsim-ffi/harness/run.sh`, exit 0, in
   the configurations `Docs/COMPATIBILITY.md` reports: a C caller compiled
   by MSVC against the DLL and the static library, and by MinGW gcc against
   the DLL, each for the debug and the release library. It is a real C
   program linked to the built library; `cargo test` is not a substitute.
3. **The header is checked against the Rust ABI**: the size, alignment and
   every field offset of each public struct, asserted in the header and
   accepted by both C compilers; the DLL's exports equal to the header's
   declarations; every constant and function name equal on both sides
   (`tests/header.rs`); and the behaviour the header describes, by the
   harness. (Argument types are checked by use, not mechanically.)
4. **No runtime behaviour of the four older crates changed.** The one edit
   to them is the doc comment on `Timestamp` in `locsim-core`, made in the
   documentation stage with the owner's permission: documentation only.
5. **Platform claims go no further than what was run**: Windows 11 x86_64,
   rustc 1.98.1 for `x86_64-pc-windows-msvc`, the six harness
   configurations. Everything else is Untested and says so.
6. **The project documents agree with each other and with the code** on the
   split into T11A and T11B, on the time contract, and on the known issues,
   the open pause/resume defect among them.

### Acceptance criteria of T11B

Written on 2026-10-11 at the owner's request, before any design of T11B.
**Nothing here has been done; every item is Untested.** A criterion passes
only on the evidence named, recorded with the versions it was produced on.
Where a criterion depends on something not yet decided, that is listed as a
prerequisite: T11B's design must settle it, with the owner, before the
criterion can be worked on. None of these decisions is made here.

Prerequisites for the whole ticket: a Mac with Xcode; the Rust toolchain
and the Apple target or targets to build for; how the library and header
are packaged for Xcode; what the "integration target" is that the library
is linked into and the adapter is tested in (T12 is the demo application);
the minimum iOS and Xcode versions; whether the Apple build unwinds or
aborts on panic (the C interface's panic contract holds only when it
unwinds).

| # | Criterion | Evidence required for a pass | Prerequisite decisions |
|---|---|---|---|
| 1 | **Apple build.** The Rust library builds for each intended Apple target | The exact command, the `rustc -vV` output, the target triple or triples, the profile and the panic strategy, with a build that completes; every target-specific warning, error or change needed, reported. A compile check alone is recorded as a compile check | Which targets (device, Simulator on Apple silicon, Simulator on Intel) |
| 2 | **Linking.** The library links into the integration target | A link of the real target that completes, with the linker and Xcode versions and the link command or build log kept; the list of system libraries it needed | Static library, XCFramework or another packaging; the integration target |
| 3 | **Swift integration.** The actual Swift adapter compiles against `include/locsim.h` | A build of the adapter's own sources (not a sketch) importing the header unmodified; a Swift test that reads `locsim_abi_version()` and compares it with `LOCSIM_ABI_VERSION`; Swift tests that the size, stride and alignment Swift sees for each of the four structs equal the header's, and that each field converts to the Swift type the adapter uses | How the header is exposed to Swift (module map or bridging header) |
| 4 | **Location conversion.** A sample becomes a `CLLocation` correctly | Unit tests, run on an Apple platform, for: coordinates passed through unchanged and valid for `CLLocationCoordinate2D`; altitude and both accuracies; `has_speed == 0` and `has_course == 0` becoming Core Location's negative "invalid" value and never 0 (C12); known speed and course passed through; the timestamp converted by the documented rule; a change of `run` surfaced as a discontinuity (C21). No value computed on the Swift side that the core computed (D13) | How a sample's time, on the caller's scale, becomes a `CLLocation.timestamp` |
| 5 | **Driver and clock.** Something calls `poll` and `check` at the right times and owns the session | Tests with a controllable clock for: polling by `locsim_session_next_deadline`; `check` being called so that a stall is noticed; pause and resume; stop and start; destroy on teardown and after a failed create; one polling caller per session. A statement of the clock used and of what happens when the app is suspended, each with a test. Times passed to the interface never decrease within a session and are on one scale (C2); the interface itself is not changed to suit the driver | Which clock; which timer; behaviour on suspension and on return to the foreground |
| 6 | **Error handling.** Failures stay failures | Tests for: a refused scenario and a refused policy (the status and the text reach the caller); every non-OK status the adapter can meet, `LOCSIM_ERR_BUSY`, `LOCSIM_ERR_POISONED` and `LOCSIM_ERR_INVALID_HANDLE` among them; a run that fails and is restarted; a session that fails for good. In each, the adapter delivers no location that the session did not emit: no repeated, interpolated or default sample stands in for a missing one | How the adapter reports errors and health to its app |
| 7 | **GPX.** The intended GPX workflow is validated | For whatever is built: files produced from known samples compared with expected files, and checked against the GPX schema; malformed and edge-case inputs to any parser the project writes refused with an error. **Kept apart:** that a file is well formed is one result; that the Simulator or Xcode accepts it and moves the simulated location as the file says is another, and needs criterion 8's kind of evidence | Whether GPX is in scope as export only or also import; whether it is written in Rust or Swift; which tool consumes it |
| 8 | **Simulator.** One documented end-to-end run | A written procedure that someone else can repeat; the macOS, Xcode, Simulator runtime and device-type versions; an app built with the adapter receiving samples from a named scenario in an iOS Simulator; the received locations compared with the reference for that scenario (`examples/reference.rs` or its successor); the logs or recorded output kept in the repository or referenced from it. Then, and only for the combination that was run, the Simulator row of `Docs/COMPATIBILITY.md` may change | The test app; where evidence is kept |
| 9 | **Regression.** Nothing that worked stops working | The Rust gates of T11A on the tree to be accepted; the adapter's own tests; where the machine allows it, the C harness. **And a test at the adapter's level for the open pause/resume defect**: a moving scenario paused and resumed, asserting what the adapter does when the session then reports a failed run (the status, the new run number, no invented sample). That test documents the defect; it does not resolve it. The defect is resolved only by its own core fix passing its own criteria (pauses of several lengths; a real discontinuity across a pause still rejected) | None; the defect has its own ticket |
| 10 | **Scope and claims.** What is said matches what was run | A reading of every document changed by T11B against the evidence above. No claim of physical-device support, of replacing the system's location for other apps, or of compatibility with or evasion of any anti-spoofing or integrity check: each would need separately authorised work and its own evidence, and the last is outside the project's scope altogether (C18). Rows of `Docs/COMPATIBILITY.md` change only for combinations that were run; the rest stay Untested | None |

A physical device is not part of T11B as agreed. If it is added, it needs
its own criterion and its own recorded runs.

**Unowned work, recorded so it is not lost.** Checkpoint/resume of a running
simulation (deferred from T09 by decision; it needs engine state the core
does not expose, see decision D16). Write-through rename on Windows.
GPX import. A timer or driver that calls `poll` and `check` (now part of
T11B). A bridge from `StoreError` to the supervisor's fault reports
(expected with the application). **The pause/resume gate defect in the
core** (found in T11A; see `PROGRESS.md`).

Things the specification asks for that no ticket has delivered yet and that a
later ticket must pick up: `TestProvider`, `DevelopmentAdapter` and
`CoreLocationAdapter` implementations of the provider interface;
`Docs/TESTING.md` and `Docs/RELIABILITY.md`; structured logging; CPU, memory
and battery measurements; a `Sources/` + `Tests/` directory layout (replaced
here by the Rust crate layout — see `ARCHITECTURE_DECISIONS.md`).

## Engineering rules (from the specification)

1. Do not build a toy where a proper abstraction is required.
2. Do not hide failures; every failure must be observable and diagnosable.
3. Do not claim compatibility without testing.
4. Prefer deterministic behaviour.
5. Keep platform-specific code isolated.
6. Every non-trivial algorithm needs tests.
7. Do not optimise prematurely; measure first.
8. No arbitrary constants where a configuration parameter is appropriate.
9. No dependency without justification.
10. Every generated coordinate must pass validation before emission.
11. No component silently modifies another component's configuration.
12. Fail safely: if a valid sample cannot be produced, report it; never invent one.

## Honesty rules

- Never record a test, push or device run as successful unless its output
  showed it.
- Compatibility statuses are `Verified`, `Partially Verified`, `Untested`,
  `Unsupported`. A row is never upgraded because it should work.
- When blocked, say: blocked by what, why, what was tested, possible solutions.
- Known mistakes stay in the history and are written down
  (`TICKET_HISTORY.md`), not rewritten away.

## Working agreement with the project owner

- One ticket at a time. The specification is a contract, not a to-do list to
  implement at once. Stop at the end of the requested ticket.
- One commit per logical change, each tested before it is made, each pushed
  immediately to the public repository. Do not batch a ticket into one commit.
- Never rewrite or force-push history.
- No "Co-Authored-By" or "Generated with" attribution lines in commits or
  pull requests.
- Update `PROGRESS.md` at the end of every ticket; it is the ticket-level
  source of truth.
- End each ticket with a report: what changed, tests and results, files
  changed, known limitations, next ticket.
