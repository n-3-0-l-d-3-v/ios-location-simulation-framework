# Current state — fresh-session entry point

> **Re-check the repository before relying on anything here.** This file was
> last rewritten on 2026-10-11, after the implementation stages of T11A and
> as part of its documentation stage. It describes code commit `e7fad44`
> plus documentation commits. Run the checklist at the bottom first.
> Whenever this file and the repository disagree, the repository is right
> and this file needs fixing.

## Snapshot (verified 2026-10-11)

| Item | Value | How it was verified |
|---|---|---|
| Repository | https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework (public) | `git remote -v` |
| Branch | `main`, tracking `origin/main`, in sync | `git fetch && git status -sb` |
| Last commit that changed behaviour | `fc5760a` — "ffi: health, fault reports and the event queue (T11A, stage 3)" | `git log` |
| Last commit that added code | `e7fad44` — the C header, the C harness, a reference example and a header test; no library source changed | `git show --stat e7fad44` |
| Later commits | Documentation; one of them also rewords two doc comments (`Timestamp` in the core, the module doc of `locsim-ffi`) | `git log e7fad44..HEAD --stat` |
| Toolchain | rustc 1.98.1, Windows 11 x86_64, target `x86_64-pc-windows-msvc` | `rustc -vV` |
| `cargo test` | 594 passed, 0 failed | run on 2026-10-11 |
| `cargo fmt --all -- --check` | clean | same run |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean | same run |
| C harness, `crates/locsim-ffi/harness/run.sh` | exit 0: six configurations, about 385 000 checks each, none failed | run at `e7fad44`; **not part of `cargo test`** |

Test breakdown:

| Crate | Suites | Tests |
|---|---|---|
| `locsim-core` | unit 202; `consistency_pipeline` 14, `fixed_pipeline` 4, `geographic_props` 6, `movement_props` 8, `moving_pipeline` 12, `noise_props` 8, `noisy_pipeline` 5, `route_pipeline` 8, `route_props` 6 | 273 |
| `locsim-scenario` | unit 56; `examples` 11, `last_known` 5, `malformed` 15, `round_trip` 2 | 89 |
| `locsim-store` | unit 38; `scenario_store` 15, `last_known_store` 10, `platform` 4, `crash` 2 | 69 |
| `locsim-health` | unit 18; `lifecycle` 15, `pass_through` 2, `recovery` 25, `watchdog` 18, `accounting` 7 | 85 |
| `locsim-ffi` | unit 75 (`registry` 8, `session` 28, `ops` 29, `events` 5, `codes` 5); `header` 3 | 78 |

`route_props` takes 25–40 s in a debug build and `crash` about 14 s. Both
are expected. `locsim-store`'s `platform` has three more tests under
`cfg(unix)` that have never been compiled.

## Last completed ticket, current ticket

- **Last completed: T10 — Health system.**
- **Current: T11 — Platform adapter. Open.** It was split, by decision of
  the project owner (D18), into:
  - **T11A — the C interface.** A Rust crate, `locsim-ffi`, a public C
    header and a C harness, tested on Windows. **Its five implementation
    stages are done and were each reviewed. Its documentation stage is this
    rewrite, which is awaiting review. T11A is not closed until that review
    says so.**
  - **T11B — everything Apple.** Building and linking for Apple targets,
    the Swift adapter, conversion to `CLLocation`, the driver and its
    clock, GPX, and runs on the Simulator. **Not started. Blocked: it needs
    a Mac with Xcode.**
- **T11 is complete only when T11B is implemented and has been run with
  Apple tooling.** Nothing done so far is evidence about iOS.

To confirm which ticket is current in a later session, do not trust this
section: read the "Current Ticket" heading of `PROGRESS.md`, then check that
the code agrees (the checklist below).

## What exists

Implemented and tested on a desktop host — five crates.

`crates/locsim-core` (no dependencies; behaviour unchanged since T07): `rng`,
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
tests): `Supervisor<P: LocationProvider>` with lifecycle, pass-through,
bounded restart, watchdog and fault reports; `HealthPolicy`; `HealthReport`;
events to a sink.

`crates/locsim-ffi` (depends on the core, health and scenario crates;
nothing outside the workspace). The only crate that contains `unsafe`.

| Part | What it is |
|---|---|
| `include/locsim.h` | The public C header: 15 functions, four structs, every constant, and the contract for pointers, outputs, time, threads and panics. The compiler checks each struct's size, alignment and every field offset |
| `src/registry.rs`, `src/session.rs` | Sessions as generation-checked integer handles; create and destroy; the one place every call takes a session (never waiting) and contains a panic |
| `src/ops.rs` | `start`, `stop`, `pause`, `resume`, `poll`, `next_deadline`; the sample struct |
| `src/health.rs` | `check`, `health`, `report_fault`, `clear_fault`; the health struct |
| `src/events.rs` | The bounded event queue; `next_event`, `events_dropped`; the event struct |
| `src/codes.rs`, `src/status.rs` | Every number that crosses the boundary |
| `harness/harness.c`, `harness/run.sh` | A C program that links the built library and checks it; the script that builds it with MSVC and MinGW gcc and runs it |
| `examples/reference.rs` | The same run made on the supervisor without the C interface, for the script to compare with |
| `tests/header.rs` | Header constants and function names against the Rust ones, as text |

The interface exposes one thing: a **supervised simulation session**, a
`Supervisor<SimulationProvider>` built from a scenario document and a
health policy. It does not expose the store, and it contains no simulation
mathematics.

`Examples/Scenarios/`: eight documents, loaded and run by tests of four
crates and by the C harness.

## What does not exist

- Any Apple code: no Swift, no `CLLocation` conversion, no Xcode project, no
  app. Nothing has been built for an Apple target.
- **A timer, thread or real-time driver.** A session is polled.
  `locsim_session_next_deadline` says when to call next; nothing calls. A
  stall is noticed only when something calls `check` or `poll`.
- **Checkpoint/resume.** A stored or failed simulation can only be started
  again from its beginning. No ticket owns this.
- The store behind the C interface, and a bridge from `StoreError` to fault
  reports. A caller reports a fault of the store by its component code.
- Logging to a file, console or operating-system log: a session's events
  wait in a queue the caller empties.
- Stored configuration or preferences (no such types exist).
- `TestProvider`, `DevelopmentAdapter`, `CoreLocationAdapter`.
- GPX import or export.
- `Docs/TESTING.md`, `Docs/RELIABILITY.md`.
- CI. Tests and the C harness have only ever been run by hand on one machine.

## What has never been verified

- Anything on an iOS Simulator, an iPhone, or a jailbroken device.
- A build of any crate on macOS or Linux, including the Unix-only code and
  tests of `locsim-store`.
- A cross-compile for `aarch64-apple-ios`, even a compile check.
- The header compiled as C++, or by any compiler other than MSVC 19.44 and
  MinGW-w64 gcc 14.2; any 32-bit build.
- `locsim-ffi` built by the Rust GNU toolchain. (A C caller compiled by
  MinGW gcc was linked to the library built by the MSVC Rust toolchain;
  that is a different thing.)
- The panic and poisoned-session paths from C: they are reachable only
  through hooks that exist in the Rust unit tests.
- Real-time behaviour: nothing sleeps, so nothing has been timed against a
  wall clock.
- Persistence under power loss or an operating-system crash.
- A build with the declared minimum Rust version (1.75), for any crate.
- Memory, CPU or battery behaviour beyond three timing measurements.
- Miri or any other check of the `unsafe` code beyond the tests.

## Known limitations

The living list is the "Known Issues / Limitations" section of
[../../PROGRESS.md](../../PROGRESS.md). The ones most likely to matter next:

- **Open core defect: after a pause and a resume, the next sample of some
  scenarios is rejected by the gate.** Not fixed, and outside T11A. See
  `PROGRESS.md`.
- Recovery is a new run from the scenario's beginning: a position jump, a
  first sample without speed, a new run number.
- Nothing calls `check`; whoever integrates a session must.
- Two calls on one session never wait for each other: one of them is
  refused as busy and did not happen. A poll that was refused can miss its
  tick, so one thread should poll a session.
- On Windows a successful save is not confirmed durable against power loss;
  the Unix path of the store has never run.
- Scenario documents have no defaults; seeds are decimal strings.

## Outstanding risks

- **Everything has only ever met Windows.** iOS is a Unix-like system: the
  store's replacement path that matters there is the one that has never
  run, and no crate has been compiled for it.
- **The interface has only ever met C.** Swift will be its first real
  consumer and may show that something is awkward or missing. Any change to
  the header is a new `LOCSIM_ABI_VERSION`.
- **`unsafe` exists now**, in `locsim-ffi` only. It is checked by tests and
  by reading, not by a tool.
- **No CI.** A regression is caught only if someone runs the gate, and the
  C harness is a second command that `cargo test` does not run.
- **MSRV is a claim.** `rust-version = "1.75"` is unverified for all crates.
- **The gate is strict by design.** The temptation to loosen it is the risk;
  the open pause/resume defect is where that temptation now sits.
- **Single maintainer context.** This directory is the mitigation.

## The decisions T11 had to make, and where they stand

All seven were listed here before T11 began. Recorded in D18 and C22.

| # | Decision | Outcome |
|---|---|---|
| 1 | Split the ticket or wait for a Mac | Split: T11A and T11B |
| 2 | The boundary | A hand-written C interface and header; no generator, no dependency |
| 3 | `unsafe` | One crate only, `locsim-ffi`; the other four still forbid it. Handles are integers, so a destroyed or unknown handle is a reported error |
| 4 | Ownership and threads | The library owns a session; the caller holds a handle. Any thread may call; a session in use is `LOCSIM_ERR_BUSY`, never a wait. Events are taken from a queue, not called back |
| 5 | Who calls `poll` and `check`, with which clock | **Half decided.** The interface takes the time from the caller and never reads a clock (see "Time" below). Who calls, on what timer, with which clock, and what a suspended app means: T11B |
| 6 | Which delivery mechanisms | In-app injection through the C interface is what T11A serves. GPX for `simctl` and Xcode: T11B. A jailbroken research device: still not designed |
| 7 | What "tested" means, and which compatibility rows may change | T11A added one section to `Docs/COMPATIBILITY.md`, for the C interface on Windows. Every Apple row is still Untested and may change only on a run with Apple tooling |

### Time

A timestamp is a count of nanoseconds on **a time scale the caller
chooses**. Where the count starts is arbitrary: the code only compares
times and subtracts them. The timestamps of samples, deadlines, reports and
events are on the same scale as the times passed in. The caller should not
pass a time earlier than one already given to the same session, and a time
that cannot be represented is never produced (a run that would need one
fails). Nobody is required to pass Unix-epoch times; the core's
`SystemClock` does (an anchor read once, advanced by a monotonic timer), and
that is its choice, not a rule. Contract C2.

## What T11B needs, and what it can build on

T11B is not designed. What is fixed for it:

- **The boundary.** Swift calls the C interface. The Swift layer converts a
  `locsim_sample_t` to a `CLLocation` and owns the lifecycle of a session;
  **no simulation mathematics on the platform side** (D13).
- **Unknown speed and course.** The interface flags them (`has_speed`,
  `has_course` are 0, the numbers are NaN). The adapter must turn them into
  Core Location's `-1`, never `0` (C12).
- **Run numbers.** `locsim_sample_t.run` changes when a run is replaced; the
  adapter must treat that as a discontinuity (C21).
- **Scope.** In-app injection for an app built with the adapter. Not a
  replacement of the system's location, and nothing that conceals the
  framework or a jailbreak, bypasses an integrity check or evades
  anti-spoofing (C18).

What it has to produce before any Apple row changes, each with a recorded
run: the Rust crates built and linked for Apple targets; the header
consumed by Swift; the conversion to `CLLocation`; a driver that calls
`poll` and `check` on a real clock, with a decision about suspension; GPX
that `simctl` and Xcode accept; and runs on the Simulator. These are the
areas agreed for T11B; **its acceptance criteria in detail have not been
written or approved.**

**What the current machine can and cannot do.** It is Windows with Rust,
MSVC and MinGW gcc, and no Swift toolchain. No part of T11B can be run on
it. `cargo check --target aarch64-apple-ios` might be possible after
`rustup target add`; it has not been tried, and a check is not a link and
not a run.

T11 is not allowed to: put simulation mathematics on the platform side;
conceal the framework or a jailbreak, bypass integrity checks, evade
anti-spoofing, or claim undetectability (contract C18); claim anything on
iOS that was not run on iOS; change engine or gate behaviour.

## Discrepancies found

Found during T11A, documents and code:

| # | Discrepancy | Resolution |
|---|---|---|
| 1 | `Timestamp` was documented in the core as "nanoseconds since the Unix epoch"; `locsim-scenario` said "whatever epoch the caller's clock uses"; this file said the adapter must supply "a monotonic time". The code uses only differences | The caller-chosen scale was adopted (above; C2); the doc comment was reworded. No behaviour changed |
| 2 | The T10 documents say no pipeline failure had ever occurred naturally. Pausing and resuming two of the examples produces one | Recorded as an open core defect in `PROGRESS.md`; not fixed |
| 3 | The T10 documents say events carry "the provider's error values, none of which carries a position". `CoordinateError` carries the out-of-range latitude or longitude it refused | No emitted position is involved, and T10's test searches for emitted ones. The C interface passes an error's code only. `locsim-health` is unchanged |
| 4 | D13 said the C interface "exposes the core" | It exposes the supervised session; D13 corrected |
| 5 | The approved T11A plan had a separate stage for panic handling and thread tests | Each operation needed them to meet its contract, so they were built in stages 1–3; reported at the time |

Introduced during T10 and recorded rather than hidden:

| # | Discrepancy | Resolution |
|---|---|---|
| 6 | The approved design said the supervisor would use the core's transition table to decide which calls are allowed; the implementation decides from its own lifecycle | A test (`recovery.rs::which_calls_are_allowed_agrees_with_the_core_transition_table`) checks they agree in every state; commit `fcda25c` |
| 7 | The revised design said a restarted run's first sample is stamped with the restart time | Wrong for a late first poll; corrected in the design addendum before coding, and tested |
| 8 | The revised design listed `Transition` as a permanent error | Not grounded in the provider's behaviour; corrected in the addendum: only `ScheduleError::Overflow` is permanent |

Carried over and still true:

| # | Discrepancy | Resolution |
|---|---|---|
| 9 | Commit `019ad0f` (T08 examples and their tests) carries the message of `7547565` | History unchanged; recorded |
| 10 | `rust-version = "1.75"` has never been built | Recorded as Untested |

## What goes stale, and how to check it

| Fact | Check |
|---|---|
| Branch, head, sync with remote | `git fetch origin && git status -sb && git log --oneline -5` |
| Current ticket | `PROGRESS.md` → "Current Ticket"; then confirm in code |
| Test count and gate status | Run the three gate commands |
| Whether the C interface still does what the header says | `crates/locsim-ffi/harness/run.sh` |
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
       cargo fmt --all -- --check
       cargo clippy --all-targets --all-features -- -D warnings

   and, on a Windows machine with MSVC and MinGW gcc, the C harness:

       crates/locsim-ffi/harness/run.sh

   A failing gate at the start of a session is a finding to report, not
   something to fix silently.
6. Confirm the next piece of work has really not been started (look for its
   code: for T11B, any Swift file or Xcode project).
7. Find out what machine you are on (`rustc -vV`, whether `swift` and
   `xcodebuild` exist). For T11B this decides what can be done at all.
8. State, before writing code: the last completed ticket, the current one,
   your understanding of its scope, the contracts it touches, the decisions
   it needs from the project owner, and your proposed first step. For a
   ticket with design decisions, propose the design and wait for approval
   (T09, T10 and T11A were done this way, each with its design revised
   before any code).
9. Work on that one ticket only, in the agreed stages, stopping for review
   where the owner asked for it. Commit per logical change, test what is
   staged, push each commit. Write the commit message, read it, then commit.
10. Finish with the gate, then update `PROGRESS.md`, `Docs/ARCHITECTURE.md`,
    `Docs/COMPATIBILITY.md`, and this file (snapshot, current ticket,
    prerequisites, discrepancies), and `TICKET_HISTORY.md` with the ticket
    just closed.
11. Report what was done, what was verified and how, what was not, and what
    comes next.
