# Current state — fresh-session entry point

> **Re-check the repository before relying on anything here.** This file was
> last rewritten on 2026-10-10 at the close of T10 and describes commit
> `fcda25c` plus the documentation commit that closed the ticket. Run the
> checklist at the bottom first. Whenever this file and the repository
> disagree, the repository is right and this file needs fixing.

## Snapshot (verified 2026-10-10)

| Item | Value | How it was verified |
|---|---|---|
| Repository | https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework (public) | `git remote -v` |
| Branch | `main`, tracking `origin/main`, in sync | `git fetch && git status -sb` |
| Last code commit | `fcda25c` — "tests: supervisor call legality agrees with the core transition table (T10)" | `git log` |
| Commits on `main` at that point | 68 | `git log --oneline \| wc -l` |
| Later commits | Documentation only: the close of T10 | `git log fcda25c..HEAD --stat` should show only `.md` files |
| Toolchain | rustc 1.98.1, Windows 11 x86_64 | `rustc --version` |
| `cargo test` | 516 passed, 0 failed | run on the staged content of `fcda25c`, 2026-10-10 |
| `cargo fmt --check` | clean | same run |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean | same run |

Test breakdown at `fcda25c`:

| Crate | Suites | Tests |
|---|---|---|
| `locsim-core` | unit 202; `consistency_pipeline` 14, `fixed_pipeline` 4, `geographic_props` 6, `movement_props` 8, `moving_pipeline` 12, `noise_props` 8, `noisy_pipeline` 5, `route_pipeline` 8, `route_props` 6 | 273 |
| `locsim-scenario` | unit 56; `examples` 11, `last_known` 5, `malformed` 15, `round_trip` 2 | 89 |
| `locsim-store` | unit 38; `scenario_store` 15, `last_known_store` 10, `platform` 4, `crash` 2 | 69 |
| `locsim-health` | unit 18; `lifecycle` 15, `pass_through` 2, `recovery` 25, `watchdog` 18, `accounting` 7 | 85 |

`route_props` takes 25–40 s in a debug build and `crash` about 14 s. Both
are expected. `locsim-store`'s `platform` has three more tests under
`cfg(unix)` that have never been compiled.

## Last completed ticket, next ticket

- **Last completed: T10 — Health system.**
- **Next: T11 — Platform adapter** (the authorised iOS delivery/test adapter;
  document platform limitations).
- **T11 has not started.** Evidence at `fcda25c`: no crate exposes a C ABI
  (`grep -rn "extern \"C\"\|no_mangle" crates` finds nothing), every crate
  has `#![forbid(unsafe_code)]`, there is no Swift file, no Xcode project
  and no `CLLocation` anywhere; `PROGRESS.md` says "T11 — Platform adapter
  (not started)".
- **T11 cannot be finished on the machine all work has been done on.** That
  machine is Windows with no Swift toolchain. See "Before starting T11".

To confirm which ticket is next in a later session, do not trust this
section: read the "Current Ticket" heading of `PROGRESS.md`, then check that
the code agrees (the checklist below).

## What exists

Implemented and tested on a desktop host — four crates.

`crates/locsim-core` (no dependencies; unchanged since T07): `rng`,
`geographic`, `domain`, `scheduler`, `movement`, `route`, `noise`,
`consistency`, `validation`, `provider`. The simulation, its final gate and
`SimulationProvider`.

`crates/locsim-scenario` (depends on the core, `serde`, `serde_json`):
strict, versioned JSON for a scenario and for a last-known record;
`scenario_fingerprint`.

`crates/locsim-store` (depends on the two above; standard library only):
`Store` for the scenario and the last-known record; SHA-256 envelope; atomic
replacement.

`crates/locsim-health` (depends on the core only; the scenario crate in its
tests):

| Module | What it does |
|---|---|
| `policy` | `HealthPolicy`, validation, seconds-to-nanoseconds conversion, the backoff |
| `event` | `Event`, `EventKind`, `Level`, `EventSink`, `NullSink`, `MemorySink` |
| `report` | `HealthReport`, `Emission`, `Cause`, `Totals` |
| `supervisor` | `Supervisor<P: LocationProvider>`: lifecycle, pass-through, bounded restart, watchdog, fault reports |

`Examples/Scenarios/`: eight documents, loaded and run by tests of three
crates.

## What does not exist

- Any iOS code: no C ABI crate, no Swift, no `CLLocation` conversion, no app.
- **A timer, thread or real-time driver.** The provider and the supervisor
  are polled. A stall is noticed only when something calls the supervisor.
- **Checkpoint/resume.** A stored or failed simulation can only be started
  again from its beginning. No ticket owns this.
- A bridge from `StoreError` to the supervisor's fault reports.
- Logging to a file, console or operating-system log: events go to a sink
  the caller supplies.
- Stored configuration or preferences (no such types exist).
- `TestProvider`, `DevelopmentAdapter`, `CoreLocationAdapter`.
- GPX import or export.
- `Docs/TESTING.md`, `Docs/RELIABILITY.md`.
- CI. Tests have only ever been run by hand on one machine.

## What has never been verified

- Anything on an iOS Simulator, an iPhone, or a jailbroken device.
- A build of any crate on macOS or Linux, including the Unix-only code and
  tests of `locsim-store`.
- A cross-compile for `aarch64-apple-ios`.
- Use of any type from more than one thread.
- Real-time behaviour: nothing sleeps, so nothing has been timed against a
  wall clock.
- Persistence under power loss or an operating-system crash.
- A build with the declared minimum Rust version (1.75).
- Whether bounded recovery helps with a real failure: none has ever
  occurred in the suite without being injected.
- Memory, CPU or battery behaviour beyond three timing measurements.

## Known limitations

The living list is the "Known Issues / Limitations" section of
[../../PROGRESS.md](../../PROGRESS.md). The ones most likely to matter next:

- Recovery is a new run from the scenario's beginning: a position jump, a
  first sample without speed, a new run number.
- Nothing calls `check`; whoever integrates the supervisor must.
- On Windows a successful save is not confirmed durable against power loss;
  the Unix path of the store has never run.
- Scenario documents have no defaults; seeds are decimal strings.

## Outstanding risks

- **The core has only ever met a desktop.** T11 may force interface changes
  nobody has foreseen (threading, callbacks, time sources, the constraints
  of an FFI boundary).
- **Everything has only ever met Windows.** iOS is a Unix-like system: the
  store's replacement path that matters there is the one that has never run.
- **Every crate forbids `unsafe`.** A C ABI cannot be written without it.
  T11 has to decide where the exception lives and how small it is.
- **No CI.** A regression is caught only if someone runs the gate.
- **MSRV is a claim.** `rust-version = "1.75"` is unverified for all crates.
- **The gate is strict by design.** The temptation to loosen it is the risk.
- **Single maintainer context.** This directory is the mitigation.

## Before starting T11

What T11 can build on, all present at `fcda25c`:

- One object to drive: `Supervisor<SimulationProvider>` is a
  `LocationProvider`. `poll(now)` yields the sample due; `next_deadline()`
  on the provider gives the next tick and on the supervisor the next retry.
- `SyntheticLocation` is the canonical sample. `speed_mps` and `course_deg`
  are `Option`s: `None` must become Core Location's `-1`, never `0`
  (contract C12).
- `HealthReport.run` changes when a run is replaced; the adapter must treat
  that as a discontinuity (contract C21).
- `Store` takes a directory from the caller; nothing has a built-in path.
- `EventSink` is a trait: an adapter can forward events to `os_log`.

**What the current machine can and cannot do.** It is Windows with Rust and
no Swift toolchain.

| Part of T11 | On Windows |
|---|---|
| A C ABI crate (`locsim-ffi`) and tests that call it from Rust | Possible |
| A C header for it | Possible to write; not possible to compile against Swift |
| `cargo check --target aarch64-apple-ios` for the Rust crates | Possibly, after `rustup target add`; a check is not a link and not a run. Not yet tried |
| Any Swift code compiled | **Not possible** |
| Any run on the iOS Simulator or a device | **Not possible** |
| Any row of `Docs/COMPATIBILITY.md` under "Delivery on Apple platforms" other than Untested | **Not possible** |

So on this machine T11 can at most reach "the Rust side exists and is tested
on a desktop". Nothing may be reported as working on iOS from here.

Decisions T11 has to make explicitly (none has been made):

1. **Split the ticket or wait for a Mac.** Do the Rust half now and leave the
   Swift half and all verification for a Mac, or do nothing until one is
   available. The project owner's call.
2. **The boundary.** A hand-written C ABI (decision D13, marked uncertain),
   or a generator such as UniFFI or swift-bridge (new dependencies, which
   need a written justification).
3. **`unsafe`.** Every crate forbids it. An FFI crate needs it. Decide that
   it lives in one new crate only, and what its tests must cover (null
   pointers, double free, use after free, strings that are not UTF-8).
4. **Ownership and threads across the boundary.** Who owns the supervisor,
   who may call it from which thread, how errors and events cross.
5. **Who calls `poll` and `check`, and with which clock.** The core takes
   time as an argument. The adapter must supply a monotonic time and must
   decide what a paused or suspended app means for it.
6. **Which delivery mechanisms.** Simulator (`simctl location`, GPX),
   development device (Xcode GPX, in-app injection through the provider).
   A jailbroken research device remains undesigned until hardware exists.
7. **What "tested" will mean** for each mechanism, and which rows of the
   compatibility table each test is allowed to change.

T11 is not allowed to: put simulation mathematics on the platform side;
conceal the framework or a jailbreak, bypass integrity checks, evade
anti-spoofing, or claim undetectability (contract C18); claim anything on
iOS that was not run on iOS; change engine or gate behaviour.

## Discrepancies found

At the start of the T10 design (2026-10-10), documents versus code: none.
`HealthState` and `SimulationState::Recovering` existed and were unused, as
the documents said.

Introduced during T10 and recorded rather than hidden:

| # | Discrepancy | Resolution |
|---|---|---|
| 1 | The approved design said the supervisor would use the core's transition table to decide which calls are allowed; the implementation decides from its own lifecycle | A test (`recovery.rs::which_calls_are_allowed_agrees_with_the_core_transition_table`) checks they agree in every state; commit `fcda25c` |
| 2 | The revised design said a restarted run's first sample is stamped with the restart time | Wrong for a late first poll; corrected in the design addendum before coding, and tested |
| 3 | The revised design listed `Transition` as a permanent error | Not grounded in the provider's behaviour; corrected in the addendum: only `ScheduleError::Overflow` is permanent |

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
7. Find out what machine you are on (`rustc -vV`, whether `swift` and
   `xcodebuild` exist). For T11 this decides what can be done at all.
8. State, before writing code: the last completed ticket, the next ticket,
   your understanding of its scope, the contracts it touches, the decisions
   it needs from the project owner, and your proposed first step. For a
   ticket with design decisions, propose the design and wait for approval
   (T09 and T10 were done this way, with the design revised twice in T10).
9. Work on that one ticket only, in the agreed stages. Commit per logical
   change, test what is staged, push each commit. Write the commit message,
   read it, then commit.
10. Finish with the gate, then update `PROGRESS.md`, `Docs/ARCHITECTURE.md`,
    `Docs/COMPATIBILITY.md`, and this file (snapshot, next ticket,
    prerequisites, discrepancies), and `TICKET_HISTORY.md` with the ticket
    just closed.
11. Report what was done, what was verified and how, what was not, and what
    comes next.
