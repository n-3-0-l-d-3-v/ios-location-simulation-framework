# Proposal: iOS compatibility policy and test matrix

**Status: a proposal for review, written on 2026-10-12. Not adopted.**
Nothing here changes `Docs/COMPATIBILITY.md`, the acceptance criteria of
T11B, or what is authorised. Where a row says a test does not exist, it
does not exist; describing it here is not permission to build it.

The statuses in this document are those of the day it was written. If it
is adopted, the matrix moves to `Docs/COMPATIBILITY.md` and is kept there.

## 1. What compatibility means here

The framework is a Rust engine, a C interface, and (planned) a Swift
adapter that hands synthetic locations to **an app built with the
adapter**. Compatibility with an iOS release means: that app, built with a
named toolchain, passed the named tests on that release.

It never means any of the following, and no test in this matrix is
evidence for them:

- that the system's location changes for other apps or for system
  services. Find My, in particular, does not read an app's adapter. A
  passing adapter test says nothing about what Find My reports;
- that anything is concealed from, or passes, an integrity or anti-spoofing
  check (contract C18: out of scope altogether);
- that a release works because its neighbour did.

Three kinds of evidence are kept apart and never substituted:

| Kind | What ran | What it can support |
|---|---|---|
| Host | Rust tests, C harness, on Windows or macOS | The engine and the C interface on that host |
| Simulator | A Swift consumer in a named Simulator runtime on a named Mac | That runtime, as a Simulator. The Simulator runs arm64 code of the Mac against the Simulator SDK; it is not the device OS build, has no device hardware, and does not sign or sandbox as a device does |
| Physical device | The same on a named iPhone and iOS build | That device and build only |

## 2. Support levels

A configuration is one combination of: iOS version, device or Simulator
device type, Xcode, Rust, and the commit.

| Level | Meaning | How it is earned | How it is lost |
|---|---|---|---|
| **Supported** | Every mandatory test of section 4 has status Passed on this configuration, at a named commit, with the run recorded | A recorded run. Never by building alone, and never by inference from another configuration | A mandatory test fails or can no longer be run (for instance the runtime leaves the runner image) and no other machine runs it |
| **Experimental** | It was run and something is known, but not every mandatory test has passed, or the configuration is a beta | A recorded run of at least the build, link and execution tests | Promotion, or a decision to drop it |
| **Unsupported** | Known not to work, or deliberately not maintained | A recorded failure that will not be fixed, or a written decision | A new decision and new runs |
| **Untested** | The default for everything else | — | — |

Each test on each configuration has its own status: `Passed`, `Failed`,
`Untested`, or `Not applicable`. A support level is derived from those; it
is not set by hand.

Simulator and device are separate configurations. "iOS 18.6: Supported
(Simulator)" and "iOS 18.6: Untested (device)" can both be true, and the
document must say both.

## 3. What exists today

Verified during the work of 2026-10-12 unless marked otherwise.

| Evidence | Where | Result |
|---|---|---|
| Rust gate, Windows x86_64, rustc 1.98.1 | local, commit `6bcf2c2` | 595 tests passed; fmt and clippy clean |
| Rust tests, macOS 15.7 arm64 host, rustc 1.98.1 | workflow run 38239633331, commit `72d4c7b` | 595 passed |
| Static library for `aarch64-apple-ios-sim`, symbols equal the header | same run, and 38237842696 | Passed, with diagnostics from `nm` |
| Packaging and Swift consumer in the iOS 18.6 Simulator | workflow run 38237842696, commit `e5e3100`; `Docs/Diagnostics/apple_packaging` | Passed for a fixture: compiled, linked, four tests ran and passed; a missing or damaged artifact stopped the build |
| Long-gap recording | `Docs/Diagnostics/long_gap` | Recorded on Windows; the pause/resume defect reproduces. Same outcomes on macOS arm64, numbers differ in their last digits |

Simulator runtimes seen on the `macos-15` runner image `20260907.0337.1`:
iOS 18.5, 18.6, 26.0, 26.1 and 26.2. **No 18.7 runtime was listed.**
Whether Apple publishes a Simulator runtime for 18.7.x at all has not been
checked. Until one is shown to exist, iOS 18.7.8 can only be tested on a
physical device.

## 4. The tests

`M` mandatory for the Supported level; `A` advisory (reported, does not
block). "Exists" says whether the test is in the repository today.

| ID | Test | Evidence required | Exists | Automatable on public runners | M/A |
|---|---|---|---|---|---|
| R1 | Rust gate: `cargo test`, `fmt --check`, `clippy -D warnings` | Command output with counts, `rustc -vV`, commit | Yes | Yes (any OS) | M |
| R2 | Deterministic fixtures: the C harness against `reference.rs`; the long-gap recording test | Byte-equal comparison on that host | Yes (harness: Windows only so far) | Harness needs a C compiler; recording runs under `cargo test` | M |
| R3 | The pause/resume defect: recorded outcomes unchanged, or changed by an approved fix with its own tests | `gap_recording` test result | Yes (recording only) | Yes | M (as a tripwire; it does not make the defect acceptable) |
| C1 | Header and exports agree: 15 symbols in the built library equal the declarations | `nm` listing and diff, with its diagnostics kept | Yes (workflow) | Yes (macOS) | M |
| P1 | Clean-checkout packaging: static library, header, module map, XCFramework, from the script alone | Script log, checksums of library and header | Yes (script and workflow job) | Yes (macOS) | M |
| P2 | A missing or damaged artifact stops the build | Failing build log naming the library | Yes (script and workflow job) | Yes (macOS) | M |
| S1 | Swift consumer compiles against the unmodified header | Build log | Fixture only; the adapter does not exist | Yes | M |
| S2 | It links, and the linker's warnings are recorded | Link step of the build log | Fixture only | Yes | M |
| S3 | It runs: ABI version, create, destroy, a refused document | Test log from the named runtime | Fixture only | Yes (Simulator) | M |
| S4 | Struct layout as Swift sees it equals the header (T11B criterion 3) | Swift test | No | Yes | M |
| S5 | Adapter lifecycle, concurrency, error handling, teardown (T11B criteria 5, 6) | Swift tests with a controllable clock | No. Waits for decisions not yet made | Yes | M |
| L1 | `CLLocation` conversion: coordinates, accuracies, unknown speed and course as negative values, timestamp rule, run change (T11B criterion 4) | Swift tests on an Apple platform | No | Yes (Simulator) | M |
| G1 | GPX export against expected files and the GPX schema (T11B criterion 7) | Test output | No | Yes | M if GPX is in scope |
| G2 | A Simulator or Xcode accepts the file and the simulated location moves as it says | Recorded procedure and observed locations | No. `simctl location` help was read on 2026-10-11; file replay through it is not established | Partly: `simctl location` can be scripted; an Xcode scheme or test-plan replay needs checking | A until it has been run once |
| E1 | End-to-end: a test app with the adapter receives a named scenario in the Simulator and matches the reference (T11B criterion 8) | Written procedure, versions, kept output | No | Likely, as a UI-less test host; unproven | M |
| D1 | The same consumer on a physical iPhone | Device model, iOS build, Xcode, signing identity type, test log | No | **No.** Needs a Mac, the device, Developer Mode, a signing identity and its owner's consent | M for any device claim; otherwise not applicable |
| D2 | Device-target build: `aarch64-apple-ios` static library and XCFramework slice | Build log; a build is recorded as a build | No | Yes (build only; it cannot run on a runner) | A until D1 is possible |
| T1 | Canary: R1, C1, P1–S3 with the newest Xcode on the runner and stable Rust | Same logs, not pinned | No | Yes | A |

## 5. Proposed matrix

Status as of 2026-10-12. "Fixture" means the packaging probe, not the
adapter; a Passed there is not a Passed for the adapter.

### Operating system and execution environment

| Configuration | Tests that apply | Status | Level today | M/A |
|---|---|---|---|---|
| **Baseline.** iPhone 16 Pro Simulator, iOS 18.6, Apple-silicon runner, Xcode 16.4, Rust 1.98.1 | R1–R3, C1, P1, P2, S1–S5, L1, G1, E1 | R1–R3 Passed on the macOS host (run 38239633331); C1, P1, P2 Passed, and S1–S3 Passed **for the fixture** (run 38237842696); S4, S5, L1, G1, E1 Untested: they do not exist | Experimental at most: S4, S5, L1, G1, E1 do not exist | M: this is the row a release is gated on |
| **Priority device.** iPhone 16, iOS 18.7.8 (a friend's phone) | D1 with S1–S5, L1, E1 on the device | Untested. No run of any kind. Needs D2 first, then access to the device | Untested | M before any statement about this phone |
| Other iOS 18 Simulator runtimes present on the runner (18.5 seen) | S1–S3, L1, E1 | Untested | Untested | A |
| iOS 18.7.x Simulator | S1–S3, L1, E1 | Untested. No such runtime seen | Untested | A; becomes M for the 18.7 line if a runtime exists |
| Latest stable iOS as a Simulator (26.2 was the newest runtime seen on the runner; whether it is the latest release was not checked) | S1–S3, L1, E1, with an Xcode that has its SDK | Untested | Untested | M once the baseline row is Supported |
| Next major iOS, beta runtime | S1–S3, L1, E1 | Untested. Not applicable until a beta runtime is on a runner or a Mac | Untested | A, always: a beta never blocks a release |
| Other Simulator device types (iPhone 16, 16e, 17, SE 3rd generation were listed) | S3, E1 | Untested | Untested | A. One extra type per release is enough unless a failure points at a device type |
| Physical iPhone, other models | D1 | Untested | Untested | Not applicable until a device is available |
| Intel Simulator (`x86_64-apple-ios`) | P1, S1–S3 | Untested | Untested | A. Proposed as not planned: say so, and do not build it |
| Jailbroken research device | — | Untested, not designed | Untested | Outside this proposal |

### Toolchain and integration

| Configuration | Tests | Status | M/A |
|---|---|---|---|
| Pinned baseline: Rust 1.98.1, Xcode 16.4, `macos-15` image | R1, C1, P1–S3 | Passed (run 38237842696); S1–S3 for the fixture only | M |
| Canary: stable Rust, newest Xcode on the image | T1 | Untested | A |
| Declared minimum Rust 1.75 | R1 | Untested (a claim in `Cargo.toml`) | A; proposed: verify it or raise the declared value |
| C interface on Windows, MSVC and MinGW callers | R2 harness | Passed at `e7fad44` (historical report; not rerun today, `locsim-ffi` unchanged since) | M on change to `locsim-ffi` |
| Header as C++ | compile check | Untested | A |
| `nm` reading Rust's standard-library objects with the Xcode toolchain | C1 diagnostics | Failed as a tool run: exit status 1, 73 diagnostics on the release library. The symbol listing is complete and the fixture linked with no linker diagnostic | A, tracked: see section 8 |
| Device target `aarch64-apple-ios` | D2 | Untested | A |
| Panic strategy of the Apple build | recorded in P1 | Recorded: `panic="unwind"` for the target, no panic setting in the manifest. The poisoned-session path has not been exercised on an Apple platform: Untested | M: the C interface's panic contract holds only when the library unwinds |

## 6. Proposed CI arrangement

Nothing here is set up. The repository has one workflow, run by hand.

Cost. Public repositories have, as far as GitHub's published terms go,
free use of standard hosted runners; **this account's billing status and
limits have not been verified**, and macOS minutes are the expensive kind
where they are billed. The arrangement below assumes macOS time should be
spent deliberately either way. One packaging run took about seven minutes (run 38237842696; run 38237140683 took five).

| Lane | Trigger | Runs | Blocks |
|---|---|---|---|
| 1. Rust gate | Every push and pull request | R1, R2 (recording), R3 on a Linux runner | Merge and release |
| 2. Apple baseline | A push or pull request that touches `crates/locsim-ffi/**`, `crates/locsim-core/**`, `Apple/**`, `Scripts/apple/**` or the workflow; by hand; and always before a release | C1, P1, P2, S1–S3 on the pinned baseline, later S4, S5, L1, G1, E1 | Release |
| 3. Canary | Weekly schedule and by hand | T1: same steps, newest Xcode on the image, stable Rust, newest iOS runtime already on the image | Nothing. A failure opens an issue |
| 4. Beta | By hand only | Lane 2's steps on a beta Xcode and runtime, when the runner image carries one | Nothing |
| 5. Device | By hand, on a Mac with the phone attached | D1 | Any statement about that device |

Rules:

- **No runtime is downloaded in CI.** A lane uses runtimes already on the
  runner image. A configuration whose runtime is not on the image is
  `Untested` there, and the log says which runtimes were present.
- **Pins fail, they do not fall back.** If Xcode 16.4, Rust 1.98.1 or the
  iOS 18.6 runtime is missing, lane 2 fails with that message. The
  packaging job already behaves this way.
- **Runner images move.** Hosted images drop old Xcode versions and
  runtimes. When the baseline's pieces leave the image, the baseline is
  re-established on what is available, by a recorded run, and the old
  configuration becomes `Untested (no longer runnable)` — not silently
  kept as Supported. Keeping an old version covered after that needs a Mac
  the project controls; that is a cost decision, not something CI solves.
- **No third-party actions, no secrets, read-only token**, as now.
- A red canary is information. A red baseline is a stop.

## 7. Procedures

**A new stable iOS patch or minor release.**
1. Wait until its Simulator runtime is on the runner image, or use a Mac
   that has it. Record how it was obtained.
2. Run lane 2's steps against it without changing the pins of the
   baseline. Record Xcode, runtime build, commit.
3. All mandatory tests Passed → add the row as Supported (Simulator).
   Anything else → Experimental or Failed, with the log.
4. A patch release with no Simulator runtime (18.7.x today) cannot be
   promoted on Simulator evidence for another patch. It stays Untested
   until a device run.

**A new major iOS release.**
1. During the beta: lane 4, by hand. Failures are recorded as issues
   labelled with the beta build; they block nothing.
2. At release: a new Xcode is needed for its SDK. Run lane 3 first, then
   lane 2's steps pinned to the new Xcode.
3. Check what changed in Core Location for that SDK (deprecations of the
   `CLLocation` initialisers the adapter uses, new required fields,
   concurrency annotations). This is a reading task with a recorded
   result, not a test.
4. Promote the new row only on its own runs. The old baseline stays the
   gate until the owner moves it.

**A new iPhone model.** A Simulator device type is advisory evidence. A
new CPU or ABI matters only if Apple changes what third-party apps are
compiled for; today that is `arm64`. `arm64e` is not a target for
third-party apps and is not proposed.

**A new Rust or Xcode version.** The canary meets it first. Moving a pin
is its own commit, with a lane 2 run before and after, and the `nm` and
linker diagnostics compared between the two.

**A change to the C interface.** Needs a new `LOCSIM_ABI_VERSION`, the
Windows harness, and lane 2. S3 compares the linked version with the
header, so a stale XCFramework fails instead of passing.

**A regression.** A mandatory test that was Passed and is now Failed on a
Supported configuration:
1. blocks a release;
2. is recorded with the first failing commit and the last passing one;
3. is resolved by a fix, by reverting the change, or by a written decision
   to demote the configuration to Unsupported. Loosening a test or the
   validation gate to turn it green is not one of the options.

**Rollback.** A release is a commit plus the versions in its recorded
runs. Rolling back means returning to the last commit whose mandatory rows
were Passed and rebuilding the XCFramework from it: artifacts are
generated, never stored, so there is no binary to restore. If binaries are
ever distributed, this needs rethinking.

**Unsupported environments.** They are listed, with the reason. A
configuration nobody has run is Untested, not Unsupported.

## 8. Open diagnostics that the policy must not paper over

- **`nm` and Rust's standard library.** With Xcode 16.4 and Rust 1.98.1, `nm` exits 1 on the
  library: Xcode's LLVM cannot read the bitcode that Rust's standard
  library objects carry beside their machine code. With the bitcode
  reader off it reads everything, and the fixture linked and ran. A
  consumer built with link-time optimisation has not been tried and is
  the case to test before any release build is relied on. Detail:
  `Docs/Diagnostics/apple_packaging`. Each move of a pin repeats this
  check; the gap between the two LLVM versions can widen or close.
- **The pause/resume defect** is open (`Docs/Diagnostics/long_gap`). The
  recording test stops it from changing unnoticed. No row in this document
  may be read as saying the defect is acceptable, and the adapter-level
  test of T11B criterion 9 is still required.
- **Cross-platform determinism.** The long-gap recording was made on
  Windows. On macOS arm64 with the same
  compiler every outcome is the same and 723 of 1056 records differ in
  the last digits of numbers (positions by at most 4.3e-14 degrees).
  The engine is repeatable on a host; it is **not** bit-for-bit the same
  across these two hosts, and the cause is not known. Until it is, a
  reference stream compared exactly is the reference of one platform.
  This matters for T11B criterion 8, which compares locations received
  in the Simulator with a reference: the reference must be produced on
  the same kind of host, or the comparison must say what it tolerates
  and why. That is a decision, listed below.

## 9. What needs something the project does not have

| Need | For | State |
|---|---|---|
| A Mac with Xcode under the project's control | D1; keeping old runtimes; anything interactive in Xcode (scheme GPX replay) | Not available. Only hosted runners have been used |
| The priority iPhone, attached to that Mac, with Developer Mode on and its owner's agreement | D1 on iOS 18.7.8 | Not available. Not tested |
| An Apple developer signing identity | Running anything on a device | Not established |
| Confirmation of this account's Actions billing | Scheduled lanes | Not verified |

## 10. Decisions this proposal asks for

1. Adopt, change or reject the support levels of section 2.
2. Accept iPhone 16 Pro Simulator, iOS 18.6, Xcode 16.4, Rust 1.98.1 as
   the pinned baseline.
3. XCFramework with a static library as the packaging route, or another.
4. Which lanes of section 6 to set up, and whether any may run on push.
5. Minimum iOS version (the fixture uses 16 as a placeholder only).
6. Whether a device target build (D2) is authorised before a device is
   available.
7. Whether the Intel Simulator is declared not planned.
8. Whether the matrix replaces the Apple section of
   `Docs/COMPATIBILITY.md`.
9. How a reference stream is compared across platforms, given that
   Windows and macOS differ in the last digits: a reference per
   platform, or a stated tolerance. And whether finding the cause is
   worth a ticket.
