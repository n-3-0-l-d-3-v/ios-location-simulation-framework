# Current state — fresh-session entry point

> **Re-check the repository before relying on anything here.** This file was
> last rewritten on 2026-10-10 at the close of T09 and describes commit
> `29c8342` plus the documentation commit that closed the ticket. Run the
> checklist at the bottom first. Whenever this file and the repository
> disagree, the repository is right and this file needs fixing.

## Snapshot (verified 2026-10-10)

| Item | Value | How it was verified |
|---|---|---|
| Repository | https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework (public) | `git remote -v` |
| Branch | `main`, tracking `origin/main`, in sync | `git fetch && git status -sb` |
| Last code commit | `29c8342` — "tests: T09 killed-writer test and platform replacement behaviour (stage 5)" | `git log` |
| Commits on `main` at that point | 61 | `git log --oneline \| wc -l` |
| Later commits | Documentation only: the close of T09 | `git log 29c8342..HEAD --stat` should show only `.md` files |
| Toolchain | rustc 1.98.1, Windows 11 x86_64 | `rustc --version` |
| `cargo test` | 431 passed, 0 failed | run on the staged content of `29c8342`, 2026-10-10 |
| `cargo fmt --check` | clean | same run |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean | same run |

Test breakdown at `29c8342`:

| Crate | Suite | Tests |
|---|---|---|
| `locsim-core` | Unit tests in `src/` | 202 |
| `locsim-core` | `consistency_pipeline` 14, `fixed_pipeline` 4, `geographic_props` 6, `movement_props` 8, `moving_pipeline` 12, `noise_props` 8, `noisy_pipeline` 5, `route_pipeline` 8, `route_props` 6 | 71 |
| `locsim-scenario` | Unit tests in `src/` | 56 |
| `locsim-scenario` | `examples` 11, `last_known` 5, `malformed` 15, `round_trip` 2 | 33 |
| `locsim-store` | Unit tests in `src/` | 38 |
| `locsim-store` | `scenario_store` 15, `last_known_store` 10, `platform` 4, `crash` 2 | 31 |

`route_props` takes 25–40 s in a debug build and `crash` about 14 s (it
starts and kills 40 processes). Both are expected. `platform` has three more
tests under `cfg(unix)` that have never been compiled.

## Last completed ticket, next ticket

- **Last completed: T09 — Persistence** (scenario and last-known record;
  checkpoint/resume explicitly deferred and unowned).
- **Next: T10 — Health system** (health state, metrics, watchdog, bounded
  recovery such as three retries with backoff then FAILED, structured
  logging with levels).
- **T10 has not started.** Evidence at `29c8342`: `HealthState` exists in
  `crates/locsim-core/src/domain/state.rs` and is used by nothing;
  `SimulationState::Recovering` exists and nothing enters it; a provider in
  `Error` stays there; there is no logging of any kind; `PROGRESS.md` says
  "T10 — Health system (not started)".

To confirm which ticket is next in a later session, do not trust this
section: read the "Current Ticket" heading of `PROGRESS.md`, then check that
the code agrees (the checklist below).

## What exists

Implemented and tested on a desktop host — three crates.

`crates/locsim-core` (no dependencies; unchanged since T07):

| Module | What it does |
|---|---|
| `rng` | Seeded xoshiro256** with forkable streams |
| `geographic` | Validated coordinates, Vincenty geodesics, ENU and ECEF, meridian convergence |
| `domain` | `SyntheticLocation`, `Scenario`, parameters, `Route`, state machine, `Timestamp`, errors |
| `scheduler` | Clocks, drift-free `TickSchedule` |
| `movement` | `FixedModel`, `CircularModel`, `SteeredModel` (random walk, walking, driving), `RouteModel` |
| `route` | Route → spline trajectory, admission against limits |
| `noise` | Bounded, seeded, time-correlated position and observation noise |
| `consistency` | Speed and course derived from emitted positions; independent check |
| `validation` | The final per-sample gate |
| `provider` | `LocationProvider`, `SimulationProvider` (lifecycle, pause/resume, completion) |

`crates/locsim-scenario` (depends on `locsim-core`, `serde`, `serde_json`):

| Module | What it does |
|---|---|
| `json` | Text ⇄ ordered document tree; duplicate keys reported; exact integers; correctly rounded floats |
| `migrate` | Version check and the ordered migration chain, for either document type |
| `schema` | Scenario document, schema version 1: strict `encode` / `decode` |
| `record` | Last-known record, record version 1; `scenario_fingerprint` (FNV-1a 64) |
| `error` | `ScenarioError` |
| (crate root) | `import_scenario`, `export_scenario`, `import_last_known`, `export_last_known` |

`crates/locsim-store` (depends on the two crates above; standard library only):

| Module | What it does |
|---|---|
| `sha256` | SHA-256, in-crate |
| `envelope` | Header + payload file format; checked before the payload is decoded |
| `atomic` | Temporary file, sync, read back, rename, directory sync; the file-operations seam used for fault injection |
| `store` | `Store`: save/load scenario and last-known record, stale temporaries |
| `error` | `StoreError`, `Corruption`, `Operation`, `Record` |

`Examples/Scenarios/`: eight documents, all loaded and run by tests.

## What does not exist

- Any iOS code: no C ABI crate, no Swift, no `CLLocation` conversion, no app.
- **Checkpoint/resume.** A stored simulation can only be started again from
  its beginning. No ticket owns this.
- Stored configuration or preferences (no such types exist).
- Health monitoring, watchdog, automatic recovery, logging.
- A real-time driver. `SimulationProvider` is polled; nothing sleeps.
- `TestProvider`, `DevelopmentAdapter`, `CoreLocationAdapter`.
- GPX import or export.
- `Docs/TESTING.md`, `Docs/RELIABILITY.md`.
- CI. Tests have only ever been run by hand on one machine.

## What has never been verified

- Anything on an iOS Simulator, an iPhone, or a jailbroken device.
- A build on macOS or Linux, **including the Unix-only code and tests of
  `locsim-store`** (directory sync, POSIX rename behaviour).
- Persistence under power loss or an operating-system crash, anywhere.
- Persistence on any file system but one NTFS volume.
- A cross-compile for `aarch64-apple-ios`.
- A build with the declared minimum Rust version (1.75); only 1.98.1 was used.
- Wall-clock long runs. The "six hours" in the tests is simulated time.
- Memory, CPU or battery behaviour beyond two timing measurements.
- A migration of a real document or record: only version 1 of each exists.

## Known limitations

The living list is the "Known Issues / Limitations" section of
[../../PROGRESS.md](../../PROGRESS.md). The ones most likely to matter next:

- A run cannot be resumed; a provider in `Error` stays there. Recovery in
  T10 can therefore only mean starting a new run.
- On Windows a successful save is not confirmed durable against power loss.
- One writer per store directory; no lock.
- Scenario documents have no defaults; seeds are decimal strings.
- The first sample of a run has no speed or course.

## Outstanding risks

- **The core has only ever met a desktop.** T11 may force interface changes
  nobody has foreseen (threading, callbacks, time sources, the constraints
  of an FFI boundary).
- **Persistence has only ever met Windows.** iOS is a Unix-like system: the
  path that will matter in production is the one that has never run.
- **No CI.** A regression is caught only if someone runs the gate.
- **MSRV is a claim.** `rust-version = "1.75"` is unverified for all crates.
- **Dependencies exist** in `locsim-scenario` (`serde`, `serde_json`); an
  upgrade is a reviewed change checked by the float and round-trip tests.
- **The gate is strict by design.** The temptation to loosen it is the risk.
- **Single maintainer context.** This directory is the mitigation.

## Before starting T10

Prerequisites, all satisfied at `29c8342`:

- `ProviderStatus` exposes state, sample count, failed count, missed ticks,
  last sample time and trajectory completion.
- Every failure in the pipeline is a structured error (`ProviderError`,
  `ScenarioError`, `StoreError`) and moves the provider to `Error` without
  emitting anything.
- `SimulationState` already has `Error → Recovering → Running | Error |
  Stopping` in its transition table; `HealthState` has `Healthy`,
  `Degraded`, `Recovering`, `Failed`, `Stopped`.
- Time is passed in everywhere (D4), so retries and backoff can be tested
  without sleeping.

Decisions T10 has to make explicitly (none has been made):

1. **What recovery is.** Established in T09: a run cannot be resumed. So
   recovery is "stop and start a new run of the same scenario" (the stream
   restarts from the scenario's beginning, with a first sample that has no
   speed), or nothing. Say which, and say it in the documentation.
2. **Where it lives.** Inside `locsim-core` (it has the state machine and
   `HealthState`) or in a supervisor around a `LocationProvider`. The
   provider's `ModelFactory` and trait suggest a wrapper is possible without
   touching the engines.
3. **Backoff without sleeping.** Retries must be driven by the `now` the
   caller passes, like everything else. Define the schedule (count, delays,
   reset condition) as configuration, not constants (engineering rule 8).
4. **What the watchdog watches.** Stalled polling (`missed_ticks`, time
   since the last sample), repeated failures, a provider stuck in `Error`.
   Thresholds are configuration.
5. **Health versus state.** How `HealthState` is derived from
   `SimulationState`, counters and thresholds, and who may change it.
6. **Logging.** A sink the caller supplies (no dependency without a written
   justification), levels, structured fields. The architecture document
   already commits to not logging coordinates above debug level.
7. **Does persistence feed health?** A `StoreError` is a failure of the
   framework but not of the simulation. Decide whether T10 observes it.

T10 is not allowed to: add checkpoint/resume by another name, weaken the
gate so that a recovering run passes, touch the platform (T11), or change
engine behaviour.

## Discrepancies found

At the start of the T09 design review (2026-10-10), documents versus code:

| # | Discrepancy | Resolution |
|---|---|---|
| 1 | `ARCHITECTURE_DECISIONS.md` D4 said sample content "does not depend on poll times at all". True only while no tick is skipped | Corrected in D4, with the test that states the real condition |
| 2 | `Docs/ARCHITECTURE.md`, provider section, said both "stamped with wall time" and "stamped with the tick's ideal time", and that the stream "does not depend on poll punctuality" | Rewritten; a "State" paragraph added listing what a run holds |
| 3 | This file (written at the close of T08) said a provider is "a pure function of scenario, start time and poll times" and that "scenario + start time + pause bookkeeping" is enough to recompute a run | Wrong on the second point: the sequence of emitted ticks is needed too. Superseded by the T09 scope decision |

Carried over and still true:

| # | Discrepancy | Resolution |
|---|---|---|
| 4 | Commit `019ad0f` (T08 examples and their tests) carries the message of `7547565` | History unchanged; recorded |
| 5 | `rust-version = "1.75"` has never been built | Recorded as Untested |

## What goes stale, and how to check it

| Fact | Check |
|---|---|
| Branch, head, sync with remote | `git fetch origin && git status -sb && git log --oneline -5` |
| Next ticket | `PROGRESS.md` → "Current Ticket"; then confirm in code |
| Test count and gate status | Run the three gate commands |
| What is implemented | `ls crates/*/src`, each crate's `src/lib.rs` |
| Dependencies | `crates/*/Cargo.toml`, `cargo tree --edges normal` |
| Source and test references in `ENGINEERING_CONTRACTS.md` | grep for the test function names |
| Compatibility | `Docs/COMPATIBILITY.md` — and distrust any row not backed by a recorded run |

Three different claims that must never be confused:

1. **Implemented** — the code is in the repository.
2. **Tested** — automated tests pass, on the host named in
   `Docs/COMPATIBILITY.md`, at the commit named.
3. **Verified on device** — run on that exact device and OS, with the result
   recorded. As of this writing: nothing.

## Fresh-session checklist

Run before changing anything. Stop and report if any step surprises you.

1. `git fetch origin && git status -sb` — on `main`, in sync, working tree
   clean. If not clean, find out why before touching it.
2. `git log --oneline -15` — does the head match what the documents say? If
   there are commits after the last documented one, read them.
3. Read `PROGRESS.md` top to bottom. Note the current ticket.
4. Read this directory in the order given in `README.md`.
5. Run the gate and compare the test count with `PROGRESS.md`:

       cargo test
       cargo fmt --check
       cargo clippy --all-targets --all-features -- -D warnings

   A failing gate at the start of a session is a finding to report, not
   something to fix silently.
6. Confirm the next ticket has really not been started (look for its code).
7. State, before writing code: the last completed ticket, the next ticket,
   your understanding of its scope, the contracts it touches, the decisions
   it needs from the project owner, and your proposed first step. For a
   ticket with design decisions, propose the design and wait for approval
   (T09 was done this way).
8. Work on that one ticket only. Commit per logical change, test what is
   staged, push each commit. Write the commit message, read it, then commit.
9. Finish with the gate, then update `PROGRESS.md`, `Docs/ARCHITECTURE.md`,
   `Docs/COMPATIBILITY.md`, and this file (snapshot, next ticket,
   prerequisites, discrepancies), and `TICKET_HISTORY.md` with the ticket
   just closed.
10. Report what was done, what was verified and how, what was not, and what
    comes next.
