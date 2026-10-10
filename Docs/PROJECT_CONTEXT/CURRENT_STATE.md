# Current state — fresh-session entry point

> **Re-check the repository before relying on anything here.** This file was
> last rewritten on 2026-10-10 at the close of T08 and describes commit
> `e6bcbf2` plus the documentation commit that closed the ticket. Run the
> checklist at the bottom first. Whenever this file and the repository
> disagree, the repository is right and this file needs fixing.

## Snapshot (verified 2026-10-10)

| Item | Value | How it was verified |
|---|---|---|
| Repository | https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework (public) | `git remote -v` |
| Branch | `main`, tracking `origin/main`, in sync | `git fetch && git status -sb` |
| Last code commit | `e6bcbf2` — "tests: T08 malformed input, staged refusal and exact round trips" | `git log` |
| Commits on `main` at that point | 55 | `git log --oneline \| wc -l` |
| Later commits | Documentation only: the close of T08 | `git log e6bcbf2..HEAD --stat` should show only `.md` files |
| Toolchain | rustc 1.98.1, Windows 11 x86_64 | `rustc --version` |
| `cargo test` | 343 passed, 0 failed | run on the staged content of `e6bcbf2`, 2026-10-10 |
| `cargo fmt --check` | clean | same run |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean | same run |

Test breakdown at `e6bcbf2`:

| Crate | Suite | Tests |
|---|---|---|
| `locsim-core` | Unit tests in `src/` | 202 |
| `locsim-core` | `tests/consistency_pipeline.rs` | 14 |
| `locsim-core` | `tests/fixed_pipeline.rs` | 4 |
| `locsim-core` | `tests/geographic_props.rs` | 6 |
| `locsim-core` | `tests/movement_props.rs` | 8 |
| `locsim-core` | `tests/moving_pipeline.rs` | 12 |
| `locsim-core` | `tests/noise_props.rs` | 8 |
| `locsim-core` | `tests/noisy_pipeline.rs` | 5 |
| `locsim-core` | `tests/route_pipeline.rs` | 8 |
| `locsim-core` | `tests/route_props.rs` | 6 |
| `locsim-scenario` | Unit tests in `src/` | 42 |
| `locsim-scenario` | `tests/examples.rs` | 11 |
| `locsim-scenario` | `tests/malformed.rs` | 15 |
| `locsim-scenario` | `tests/round_trip.rs` | 2 |

`route_props` takes 25–40 s in a debug build. That is expected.

## Last completed ticket, next ticket

- **Last completed: T08 — Scenario system.**
- **Next: T09 — Persistence** (reliable configuration persistence and
  recovery: active scenario, configuration, last known state, route,
  preferences, schema version; atomic writes — write temporary, validate,
  replace).
- **T09 has not started.** Evidence at `e6bcbf2`: no crate or module opens a
  file outside tests (`grep -rn "std::fs" crates/*/src` finds nothing; the
  only use is one test listing `Examples/Scenarios`), and `PROGRESS.md` says
  "T09 — Persistence (not started)".

To confirm which ticket is next in a later session, do not trust this
section: read the "Current Ticket" heading of `PROGRESS.md`, then check that
the code agrees (the checklist below).

## What exists

Implemented and tested on a desktop host — two crates.

`crates/locsim-core` (no dependencies):

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
| `migrate` | Version check and the ordered migration chain (production chain: version 1 only, no steps) |
| `schema` | Schema version 1: strict `encode` / `decode` of a `Scenario` |
| `error` | `ScenarioError` |
| (crate root) | `import_scenario`, `export_scenario` |

`Examples/Scenarios/`: eight documents, all loaded and run by tests.

## What does not exist

- Any iOS code: no C ABI crate, no Swift, no `CLLocation` conversion, no app.
- Persistence: nothing reads or writes a file. Scenario import and export
  work on strings.
- Health monitoring, watchdog, automatic recovery, logging.
- A real-time driver. `SimulationProvider` is polled; nothing sleeps.
- `TestProvider`, `DevelopmentAdapter`, `CoreLocationAdapter`.
- GPX import or export.
- `Docs/TESTING.md`, `Docs/RELIABILITY.md`.
- CI. Tests have only ever been run by hand on one machine.

## What has never been verified

- Anything on an iOS Simulator, an iPhone, or a jailbroken device.
- A build on macOS or Linux.
- A cross-compile for `aarch64-apple-ios` (now including `serde_json`).
- A build with the declared minimum Rust version (1.75); only 1.98.1 was used.
- Wall-clock long runs. The "six hours" in the tests is simulated time.
- Memory, CPU or battery behaviour. One timing measurement exists (T07).
- A migration of a real document: only schema version 1 has ever existed.

## Known limitations

The living list is the "Known Issues / Limitations" section of
[../../PROGRESS.md](../../PROGRESS.md). The ones most likely to matter next:

- Scenario documents have no defaults; the specification's short example
  does not load.
- Seeds are decimal strings in JSON.
- Import does not admit a route; the provider does, at `start`.
- The first sample of a run has no speed or course.
- A provider in `Error` stays there.
- The noise engine computes a noisy speed and course that nothing uses.

## Outstanding risks

- **The core has only ever met a desktop.** T11 may force interface changes
  nobody has foreseen (threading, callbacks, time sources, `no_std`-style
  constraints of an FFI boundary).
- **No CI.** A regression is caught only if someone runs the gate.
- **MSRV is a claim.** `rust-version = "1.75"` is unverified for both crates.
- **Dependencies exist now.** `serde` and `serde_json` are pinned by
  `Cargo.lock`; an upgrade is a change to review and test (the float and
  round-trip tests are the check), not a routine bump.
- **The gate is strict by design.** New engines will be rejected for rounding
  unless they keep margin. The temptation to loosen the gate is the risk.
- **Single maintainer context.** This directory is the mitigation; keep it
  current.

## Before starting T09

Prerequisites, all satisfied at `e6bcbf2`:

- A validated, canonical, versioned text form of a scenario exists
  (`export_scenario` / `import_scenario`), with a route inside it.
- Import reports every problem with a path; a corrupted or truncated
  document is refused, never partly loaded (tested: every truncation of two
  documents, 32 000 random damages).
- `export(import(text)) == text` for exported text, so a stored file can be
  compared byte for byte with what was meant to be written.

Decisions T09 has to make explicitly (none has been made):

1. **Where it lives.** A new crate (the pattern T08 set) or inside
   `locsim-scenario`. The core must not open files (it has no I/O at all).
2. **What "last known simulation state" means.** A provider is a pure
   function of scenario, start time and poll times. Is the stored state
   "scenario + start time + pause bookkeeping" (enough to recompute), or a
   snapshot of the last emitted sample? The first keeps determinism; the
   second cannot resume the deriver or the gate honestly.
3. **Formats for the things that are not scenarios** (configuration,
   preferences, state). The strict document layer (`json` module) is private
   to `locsim-scenario`; reuse means exposing it or moving it.
4. **A version for the store itself**, separate from the scenario schema
   version, and what migration means there.
5. **Atomic replace.** Write temporary, flush, validate by importing it
   back, then rename over the old file. Rename-over-existing behaves
   differently on Windows and POSIX; only Windows can be verified on the
   current machine. Say which platforms are verified.
6. **Corruption policy.** A stored file that fails to import is reported and
   the previous good one is kept; nothing is reset to defaults silently
   (contract C5). Decide whether a backup generation is kept.
7. **Where files go.** The caller passes the directory; no path is hardcoded
   (iOS sandbox locations are a T11 concern).
8. **Dependencies.** Atomic file replacement can be done with `std` alone.
   Any crate needs a written justification.

T09 is not allowed to: change the scenario schema without a version bump and
a migration step, touch the platform (T11), add health or recovery logic
(T10), or change engine behaviour.

## Discrepancies found

At the start of the T08 session (2026-10-10), documents versus repository:

| # | Discrepancy | Resolution |
|---|---|---|
| 1 | `route_props` took 39 s; documents said "about 25 s" | Documents now say 25–40 s |
| 2 | Nothing else: test counts, dependencies, empty `Examples/Scenarios/` and "T08 not started" all matched | — |

Introduced during T08 and recorded rather than hidden:

| # | Discrepancy | Resolution |
|---|---|---|
| 3 | Commit `019ad0f` (examples and `tests/examples.rs`) carries the message of `7547565` | History unchanged; noted in `e6bcbf2`, `PROGRESS.md`, `TICKET_HISTORY.md` |

From the audit of 2026-10-09 (all resolved then): stale statements in
`README.md`, `Docs/ARCHITECTURE.md` and `Docs/COMPATIBILITY.md`; the
untested `rust-version`; history notes for `bca95b5`, `7c96eb3`, `b3b0d6e`,
`7dab0a0`. See `TICKET_HISTORY.md`.

## What goes stale, and how to check it

| Fact | Check |
|---|---|
| Branch, head, sync with remote | `git fetch origin && git status -sb && git log --oneline -5` |
| Next ticket | `PROGRESS.md` → "Current Ticket"; then confirm in code |
| Test count and gate status | Run the three gate commands |
| What is implemented | `ls crates/*/src`, each crate's `src/lib.rs` |
| Dependencies | `crates/*/Cargo.toml`, `cargo tree -p locsim-scenario --edges normal` |
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
   staged, push each commit. Write the commit message, read it, then commit.
9. Finish with the gate, then update `PROGRESS.md`, `Docs/ARCHITECTURE.md`,
   and this file (snapshot, next ticket, prerequisites, discrepancies), and
   `TICKET_HISTORY.md` with the ticket just closed.
10. Report what was done, what was verified and how, what was not, and what
    comes next.
