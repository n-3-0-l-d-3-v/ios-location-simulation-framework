# Current state — fresh-session entry point

> **Re-check the repository before relying on anything here.** This file was
> written on 2026-10-09 and describes commit `1be8852` plus the documentation
> commits that added this directory. Run the checklist at the bottom first.
> Whenever this file and the repository disagree, the repository is right and
> this file needs fixing.

## Snapshot (verified 2026-10-09)

| Item | Value | How it was verified |
|---|---|---|
| Repository | https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework (public) | `git remote -v` |
| Branch | `main`, tracking `origin/main`, in sync | `git fetch && git status -sb` |
| Last code commit | `1be8852` — "progress: close T07 with test statistics, issues found and limitations" | `git log` |
| Commits on `main` at that point | 48 | `git log --oneline \| wc -l` |
| Later commits | Documentation only: this directory, links to it, corrections of stale statements | `git log 1be8852..HEAD --stat` should show only `.md` files |
| Toolchain | rustc 1.98.1, Windows 11 x86_64 | `rustc --version` |
| `cargo test` | 273 passed, 0 failed | run at `1be8852`, 2026-10-09 02:32 UTC |
| `cargo fmt --check` | clean | same run |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean | same run |

Test breakdown at `1be8852`:

| Suite | Tests |
|---|---|
| Unit tests in `src/` | 202 |
| `tests/consistency_pipeline.rs` | 14 |
| `tests/fixed_pipeline.rs` | 4 |
| `tests/geographic_props.rs` | 6 |
| `tests/movement_props.rs` | 8 |
| `tests/moving_pipeline.rs` | 12 |
| `tests/noise_props.rs` | 8 |
| `tests/noisy_pipeline.rs` | 5 |
| `tests/route_pipeline.rs` | 8 |
| `tests/route_props.rs` | 6 |

`route_props` takes about 25 s in a debug build. That is expected.

## Last completed ticket, next ticket

- **Last completed: T07 — Consistency engine.**
- **Next: T08 — Scenario system** (JSON import/export, scenario validation,
  schema versioning, migration between schema versions).
- **T08 has not started.** Evidence at `1be8852`: no serialisation code
  anywhere in the crate (no `serde`, no JSON), `crates/locsim-core/Cargo.toml`
  has an empty `[dependencies]`, `Examples/Scenarios/` contains only
  `.gitkeep`, and `PROGRESS.md` says "T08 — Scenario System (not started)".

To confirm which ticket is next in a later session, do not trust this
section: read the "Current Ticket" heading of `PROGRESS.md`, then check that
the code agrees (the checklist below).

## What exists

Implemented and tested on a desktop host — one crate, `crates/locsim-core`:

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

## What does not exist

- Any iOS code: no C ABI crate, no Swift, no `CLLocation` conversion, no app.
- Serialisation of any kind. A `Scenario` can only be built in Rust code.
- Persistence, health monitoring, watchdog, automatic recovery, logging.
- A real-time driver. `SimulationProvider` is polled; nothing sleeps.
- `TestProvider`, `DevelopmentAdapter`, `CoreLocationAdapter`.
- `Docs/TESTING.md`, `Docs/RELIABILITY.md`.
- CI. Tests have only ever been run by hand on one machine.

## What has never been verified

- Anything on an iOS Simulator, an iPhone, or a jailbroken device.
- A build on macOS or Linux.
- A cross-compile for `aarch64-apple-ios`.
- A build with the declared minimum Rust version (1.75); only 1.98.1 was used.
- Wall-clock long runs. The "six hours" in the tests is simulated time.
- Memory, CPU or battery behaviour. One timing measurement exists (T07).

## Known limitations

The living list is the "Known Issues / Limitations" section of
[../../PROGRESS.md](../../PROGRESS.md). The ones most likely to matter next:

- The first sample of a run has no speed or course.
- Speed is a mean over the sampling interval, not an instantaneous value.
- Behaviour of generated models is a seeded policy, not a model of people or
  traffic; a "driver" can turn on the spot.
- A raw GPS log will usually be rejected as a route.
- A provider in `Error` stays there.
- The noise engine computes a noisy speed and course that nothing uses.

## Outstanding risks

- **The core has only ever met a desktop.** T11 may force interface changes
  nobody has foreseen (threading, callbacks, time sources, `no_std`-style
  constraints of an FFI boundary).
- **No CI.** A regression is caught only if someone runs the gate.
- **MSRV is a claim.** `rust-version = "1.75"` is unverified.
- **The gate is strict by design.** New engines will be rejected for rounding
  unless they keep margin. That is working as intended; the temptation to
  loosen the gate instead is the risk.
- **Single maintainer context.** Until now all context lived in one
  conversation. This directory is the mitigation; keep it current.

## Before starting T08

Prerequisites, all satisfied at `1be8852`:

- The scenario types are stable: `Scenario`, `MovementParameters`,
  `NoiseParameters`, `PlaybackParameters`, `Route`, `RoutePoint`
  (`crates/locsim-core/src/domain/`).
- `Scenario::validate` exists and reports every problem.
- `CURRENT_SCHEMA_VERSION` is `1` and `Scenario::validate` already rejects
  any other version.

Decisions T08 has to make explicitly (none has been made):

1. **Dependency policy.** The core has no dependencies on purpose (decision
   D1). Options: a hand-written JSON reader/writer in the core; `serde` +
   `serde_json` in the core with a written justification in `Cargo.toml`; or
   a separate crate (for example `locsim-scenario`) that depends on the core
   and on a JSON library, leaving the core dependency-free. The last keeps D1
   intact and is what this document suggests — a suggestion written during
   the handoff, not a decision, and not something the project owner has
   agreed to.
2. **Field naming and shape.** The original specification's example is

       { "name": "Walking Test",
         "origin": { "latitude": 12.9352, "longitude": 77.6245 },
         "mode": "walking",
         "speed": { "min": 0.8, "max": 1.8 },
         "jitter": { "radius": 3 },
         "updateInterval": 1,
         "seed": 12345 }

   The domain model has many more required fields and no defaults. Decide the
   real schema; do not invent defaults to make the short example load
   (contract C5).
3. **Numbers.** Coordinates and limits must round-trip bit-exactly through
   JSON, or determinism (C1) breaks between an exported and a re-imported
   scenario. Route times are integer nanoseconds; `u64` seeds exceed what a
   JSON double holds exactly.
4. **Versioning.** There is only version 1. Decide what the migration
   mechanism is and how it is tested when no older version exists.
5. **Errors.** Malformed JSON, unknown fields, unknown version, wrong types:
   all structured errors, none repaired (C5).
6. **Examples.** `Examples/Scenarios/` is empty and waiting for real files,
   which should be loaded by a test.

T08 is not allowed to: add persistence (T09), touch the platform (T11), or
change engine behaviour.

## Discrepancies found in the audit of 2026-10-09

Found by comparing documents, history and the reported state. Corrected in
the documentation commits unless noted.

| # | Discrepancy | Resolution |
|---|---|---|
| 1 | `README.md` said only the domain model and geographic engine were implemented | Corrected |
| 2 | `Docs/ARCHITECTURE.md`, noise section: said reported speed and course are the model's plus noise and "not re-derived"; false since T07. A neighbouring sentence said the speed field never exceeds `max_speed`; since T07 the bound includes the noise rate | Corrected, marked as superseded |
| 3 | `Docs/ARCHITECTURE.md`, provider section: a leftover "(Since T04: …)" pipeline line contradicting the T07 one above it | Removed |
| 4 | `Docs/COMPATIBILITY.md` dated the verified run 2026-10-08, before most of the code existed | Updated to the `1be8852` run |
| 5 | `rust-version = "1.75"` in `Cargo.toml` and "Rust ≥ 1.75" in `Docs/DEVELOPMENT.md` have never been tested | Recorded as Untested in `COMPATIBILITY.md`; not changed |
| 6 | The original specification and its ticket plan existed only in the first conversation | Ticket plan and rules now in `PROJECT_BRIEF.md` |
| 7 | Commit `bca95b5` contains T00, T01 and T02 together | Recorded; history unchanged |
| 8 | Commits `7c96eb3` and `b3b0d6e` each have one failing unit test | Recorded; fixed forward in `e318635` |
| 9 | Commit `7dab0a0` is titled as a provider change but also contains the route model | Recorded; history unchanged |
| 10 | Per-ticket test counts and statistics in `PROGRESS.md` history were not re-run | Labelled "reported" in `TICKET_HISTORY.md` |
| 11 | The state reported at the end of T07 (273 tests, six commit hashes, T08 not started) | **Confirmed** against the repository |

Also corrected: the `Docs/DEVELOPMENT.md` workflow line, which omitted
"push". Not corrected, on purpose: the noise engine's unused speed/course
output (that is code, not documentation).

## What goes stale, and how to check it

| Fact | Check |
|---|---|
| Branch, head, sync with remote | `git fetch origin && git status -sb && git log --oneline -5` |
| Next ticket | `PROGRESS.md` → "Current Ticket"; then confirm in code |
| Test count and gate status | Run the three gate commands |
| What is implemented | `ls crates/*/src`, `crates/locsim-core/src/lib.rs` |
| Dependencies | `crates/*/Cargo.toml` |
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
   it needs from the project owner, and your proposed first step.
8. Work on that one ticket only. Commit per logical change, test what is
   staged, push each commit.
9. Finish with the gate, then update `PROGRESS.md`, `Docs/ARCHITECTURE.md`,
   and this file (snapshot, next ticket, prerequisites, discrepancies), and
   `TICKET_HISTORY.md` with the ticket just closed.
10. Report what was done, what was verified and how, what was not, and what
    comes next.
