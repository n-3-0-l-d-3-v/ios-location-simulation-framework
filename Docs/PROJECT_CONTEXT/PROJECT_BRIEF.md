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
    Platform Adapter               C ABI + Swift -> CLLocation    planned (T11)
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
- iOS will be reached through a C ABI crate and a Swift adapter. **Planned.**
  Nothing has been cross-compiled for `aarch64-apple-ios`. **Unverified** that
  the core builds for it.
- Delivery mechanisms considered in scope (all **Planned**, none designed in
  detail): `xcrun simctl location` and GPX export for the iOS Simulator; GPX
  for Xcode's device location simulation; in-app injection through the
  framework's own `LocationProvider` for an app under development; a
  mechanism for jailbroken research devices only once hardware is available
  to verify it.
- The development machine so far is Windows with Rust 1.98.1 and no Swift
  toolchain. **Verified** 2026-10-09. T11 and T12 will need a Mac.

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
| Provider with lifecycle, pause/resume, completion | Implemented, tested on desktop |
| Real-time driver (something that sleeps until the next tick) | Not implemented |
| Scenario JSON import/export, schema versioning | Implemented, tested on desktop |
| Migration between schema versions | Mechanism implemented and tested with test-only steps; no real migration exists (only version 1) |
| GPX import/export | Not implemented; no ticket owns it yet (T11 mentions GPX for delivery) |
| Persistence of a scenario and of the last emitted sample (digest, atomic replacement) | Implemented, tested on Windows only; Unix path never run; power loss never tested |
| Checkpoint / resume of a running simulation | **Not implemented; no ticket owns it.** A stored simulation can only be started again |
| Stored configuration and preferences | Not implemented: no such types exist; expected with the application (T12) |
| Health state, watchdog (stall, missed ticks), bounded restart, structured events | Implemented, tested on Windows only. Recovery is a new run, not a resume. Nothing calls the watchdog: there is no timer |
| Logging to a file or system log | Not implemented: events go to a sink the caller supplies (platform work, T11/T12) |
| CPU, memory and battery metrics | Not implemented (T14) |
| C ABI, Swift adapter, `CLLocation` conversion | Planned (T11) |
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
| **T11** | **Platform adapter** | **The authorised iOS delivery/test adapter; document platform limitations** | **Next. Needs a Mac with Xcode; only a Rust-side C ABI can be done on the Windows machine used so far** |
| T12 | Test application | UI and visualisation: current location, speed, course, accuracy, mode, state, health, sample count; start/pause/resume/stop/reset/load/save; map where practical | Planned |
| T13 | Integration testing | Whole pipeline: scenario → movement → noise → validation → consistency → scheduler → adapter → app | Planned |
| T14 | Reliability testing | Long-duration runs, repeated start/stop, pause/resume, failure injection, corrupted configuration, memory | Planned |
| T15 | Documentation | Architecture, compatibility, setup, testing, limitations, troubleshooting | Planned |

**Unowned work, recorded so it is not lost.** Checkpoint/resume of a running
simulation (deferred from T09 by decision; it needs engine state the core
does not expose, see decision D16). Write-through rename on Windows.
GPX import/export. A timer or driver that calls `poll` and `check` (expected
with the platform adapter). A bridge from `StoreError` to the supervisor's
fault reports (expected with the application).

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
