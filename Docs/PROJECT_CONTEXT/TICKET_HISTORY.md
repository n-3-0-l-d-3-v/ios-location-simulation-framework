# Ticket history: T00–T09

What each completed ticket was for, what it built, what it decided, how it
was tested, what it left open, and which commits it consists of.

How to read the evidence:

- **Commits** are verified: every hash below is in `git log` on `main` as of
  2026-10-09. `git show <hash>` is the authority on what a commit contains.
- **Test counts at the close of each ticket** are *reported*: taken from
  `PROGRESS.md` as written at the time, not re-run at those commits. Only the
  final count (273 at `1be8852`) was re-run when this file was written. The
  T08 and T09 counts were observed in the session that did the work.
- **Statistics** (pairs re-checked, closest approach to a limit, and so on)
  are *reported* from test output. They are printed by the tests themselves
  and can be reproduced with `cargo test -- --nocapture`.

The detailed mechanics of each module are in
[../ARCHITECTURE.md](../ARCHITECTURE.md); the reasons are in
[ARCHITECTURE_DECISIONS.md](ARCHITECTURE_DECISIONS.md). This file does not
repeat them.

---

## T00 — Repository bootstrap · T01 — Domain model · T02 — Geographic engine

Delivered together in one commit. That was before the "one commit per logical
change" agreement, which the project owner asked for immediately afterwards.

- **Purpose.** Establish the repository, the canonical types, and correct
  WGS84 geometry.
- **Built.** Cargo workspace; README, LICENSE (MIT), `Docs/ARCHITECTURE.md`,
  `Docs/COMPATIBILITY.md`, `Docs/DEVELOPMENT.md`, `Scripts/`. Domain:
  `SyntheticLocation`, `Scenario`, `MovementMode`, `MovementParameters`,
  `NoiseParameters`, `Route`, `PlaybackParameters`, `SimulationState` with a
  transition table, `HealthState`, `Timestamp`, field-level `ConfigError`.
  Geographic: validated `Coordinate`, Vincenty inverse/direct, interpolation,
  radius and velocity helpers, ENU frames. A seeded PRNG (xoshiro256**).
- **Key decisions.** Rust core with no dependencies (D1). Validated types
  (D2). Errors for non-convergent geodesics (D3). Integer-nanosecond time.
- **Found by tests.** Vincenty with an absolute convergence tolerance returns
  the uncorrected first iterate for millimetre lines; made relative. A test
  wrongly expected `f64` epoch seconds to resolve nanoseconds.
- **Tests at close (reported).** 49 unit + 6 property = 55.
- **Left open.** Antipodal pairs unsupported. `HealthState`,
  `SimulationState::Recovering` and `Route` existed as types with nothing
  using them yet.
- **Commits.** `bca95b5` (T00–T02), `3cfbb3b` (LF line endings).

## T03 — Fixed location engine

- **Purpose.** The first complete pipeline: a stationary position through a
  scheduler, a lifecycle and a validation gate.
- **Built.** `Clock`, `SystemClock`, `ManualClock`; drift-free `TickSchedule`
  with missed-tick counting and pause/resume; `SampleValidator` (field
  validity, timestamp order); `MovementModel` and `FixedModel`;
  `LocationProvider` and `SimulationProvider` with `ModelFactory`.
- **Key decisions.** Time is passed in, nothing sleeps (D4). Samples are
  stamped with the tick's ideal time. A rejected configuration leaves the
  provider idle; a failed sample puts it in `Error`.
- **Tests at close (reported).** 86 (76 unit, 6 geographic, 4 pipeline).
  Mutation: stamping ticks with poll time is caught.
- **Left open.** No real-time driver. No recovery from `Error`.
- **Commits.** `c3db4d5`, `7717f3d`, `06e8563`, `c5ce12b`, `ec1d186`,
  `7fbf771`, `2f61e2f`, `430fa66`, `f239411`.

## T04 — Noise engine

- **Purpose.** Bounded, seeded, temporally correlated measurement noise, kept
  separate from movement and from the gate.
- **Built.** `NoiseEngine`: Gauss–Markov jitter, constant-rate waypoint drift,
  clipped speed/heading/accuracy channels, independent random streams, exact
  geodesic verification of offset, step and boundary, a fallback chain, an
  error rather than an out-of-limit position. New parameters
  `position_correlation_time_s`, `max_offset_rate_mps`. `Scenario::boundary`.
  Gate stages for speed, boundary and displacement. Pipeline became
  movement → noise → validate → emit.
- **Key decisions.** Noise is measurement error on true motion (D8). The gate
  is independent of the engines (D11).
- **Found by tests.** A 12 µm mismatch between projecting onto a tangent plane
  and dropping back onto the ellipsoid forced the fallback on about 1.4 % of
  moving samples. Projecting onto two discs in turn is not a projection onto
  their intersection. A purely relative margin vanishes for tiny limits.
- **Tests at close (reported).** 120.
- **Left open.** Noise does not touch altitude. Speed and course were the
  model's plus noise, not derived from positions — superseded in T07. Noise
  on a moving base was tested only at engine level until T05.
- **Latent defect, found in T07.** Only the emitted step was rate-limited, not
  the offset, although the parameter's documentation promised the latter.
- **Commits.** `8c34d85`, `ca6940a`, `1ecb706`, `82e6589`, `70b22f5`,
  `bd28af1`, `51b53a9`, `db8a675`, `1c96820`, `a4f2a13`.

## T05 — Movement engine

- **Purpose.** Coherent trajectories: bounded random walk, walking, driving,
  circular; with speed, acceleration, deceleration, heading-rate and
  displacement constraints.
- **Built.** `CircularModel` (closed form). `SteeredModel` (inertial mover
  behind three modes). `Kinematics`. Parameters
  `max_displacement_per_sample_m`, `speed_change_interval_s`,
  `start_phase_deg`; `walking_preset`, `driving_preset`; `KINEMATIC_MARGIN`.
  `Geodesic::convergence_deg`. Gate stages for acceleration, deceleration and
  heading rate, with turning measured on the surface. Models driven on
  simulated time.
- **Key decisions.** One steered model, behaviour as a seeded policy (D6).
  Displacement cap as a speed cap (D7). Simulated time (D5). Margins instead
  of tolerances.
- **Found by tests.** Bearings of very short lines are too ill-conditioned for
  a strict heading check; hence `convergence_deg`. Steering at the exact
  centre of a tight fence made every seed trace the same path.
- **Tests at close (reported).** 176. Steered models reached 0.999999 of each
  limit with zero violations over 150 000 random steps; 600 random scenarios
  through the provider with zero rejections.
- **Left open.** No road network, lateral-acceleration limit or minimum
  turning radius. One model step per sample. `min_speed` bounds the cruising
  target only.
- **Mistake.** Commits `7c96eb3` and `b3b0d6e` each have one failing unit
  test (`unimplemented_modes_are_reported_not_faked` still expected circular
  mode to be unsupported). They were committed with the working tree tested
  instead of the staged subset. Fixed by `e318635`. History was not rewritten.
- **Commits.** `d9a8394`, `ae372f2`, `1368171`, `cfa3ec5`, `7c96eb3`,
  `b3b0d6e`, `e318635`, `b424426`, `a0f2330`, `99163bd`, `aa025fa`,
  `f93fb14`.

## T06 — Route engine

- **Purpose.** Replay of recorded routes as a first-class trajectory source
  that obeys the same physical contract as generated motion.
- **Built.** `domain::Route` redesigned: elapsed-nanosecond points, optional
  altitude and name, structured `RouteError`, `from_absolute`, `is_closed`,
  `legs`. `route::RoutePlan`: ECEF cubic Hermite spline, shared velocities,
  rest at open ends, waits, loops over closed routes, reverse and playback
  speed, `state_at`. `route::admit` with `RouteViolation`. `RouteModel`.
  `is_complete` / `trajectory_complete`. Public ECEF conversions.
- **Key decisions.** A route is input, not authority; three stages (D9).
  Admission bounds suprema, not samples. Only closed routes loop.
- **Found by tests.** The certified bound was not an upper bound. The peak
  search exhausted its budget at a 1e-9 target. Coordinates are stored to
  about 2 nm, so admission needs headroom tied to the update interval.
  Cancellation one nanosecond before a stop gave a heading rate of
  −1757°/s for a true −0.82°/s.
- **Tests at close (reported).** 227. 400 random routes admitted at limits
  3e-6 above their own peaks; the gate accepted 1 754 106 consecutive pairs.
- **Left open.** Raw GPS logs are usually not admissible. Legs over 100 km are
  rejected. Altitude is not limit-checked. Admission's guarantee is for the
  scenario's update interval or longer.
- **Mistake.** Commit `7dab0a0` is titled as the provider change but also
  contains `movement/route.rs`. The route-model commit failed its isolated
  test run and was correctly refused, but its files stayed staged and were
  swept into the next commit. Both changes are tested together there.
- **Commits.** `fb9cef2`, `5f5923e`, `87d8f86`, `7dab0a0`, `5ba2511`,
  `e4376a3`, `af88ebb`, `00f376a`, `f5c9959`.

## T07 — Consistency engine

- **Purpose.** Make the emitted speed, course, timestamp, accuracy and
  position describe one observation.
- **Built.** `consistency::KinematicsDeriver` (backward-difference speed and
  course from emitted positions), `check_first` / `check_pair`,
  `ConsistencyTolerance`. Provider pipeline became movement → noise → final
  position → derive → validate → emit. Gate: a consistency stage, and the
  kinematic stages restated for interval means and chord directions.
  `tests/common/recheck.rs`. `COORDINATE_RESOLUTION_M` moved to `geographic`.
- **Key decisions.** Position and timestamp are the source of truth (D10).
  First sample has no kinematics; two stationary thresholds; accuracy is
  never derived.
- **Changed existing behaviour.** (1) The gate's acceleration stage is judged
  between interval midpoints and its heading stage over both intervals; the
  latter is up to twice as permissive as before for even sampling. (2) The
  noise engine's offset is now slew-limited as its parameter documented. (3)
  Existing tests were updated to the new contract: a first sample has no
  speed; scripted gate tests corrupt positions because model metadata is
  ignored.
- **Found by tests.** The noise-engine slew defect above. Rounding slack must
  scale with the magnitude being rounded. `1 − cos(90°)` is not exactly 1.
- **Tests at close (verified 2026-10-09).** 273. Reported statistics: 11 484
  pairs across all six sources; 40 560 across 1 ms–5 s intervals; 51 694
  across 180 random scenarios; without observation noise the reported speed
  and course equal the geodesic computation exactly.
- **Performance (reported, release build).** About 1.2 µs per sample for the
  whole pipeline, about 270 ns of it for derivation.
- **Left open.** Speed is an interval mean, so it depends on the sampling
  rate. The noise engine still computes speed and course nobody uses.
  `afeb26c` is a large commit: the noise fix, gate restatement, provider
  change and test updates had to land together to keep every commit passing.
- **Commits.** `9c0c8a0`, `c31854b`, `afeb26c`, `2a472fb`, `bdac304`,
  `1be8852`.

## After T07 — context documentation

Not a ticket. This directory was added, and stale statements found during
the audit were corrected (commits `bb2a354`, `0b603cd`).

## T08 — Scenario system

- **Purpose.** Give a `Scenario` a text form: import, export, validation,
  schema versioning and migration.
- **Decided by the project owner before work began.** A separate crate on
  `serde` + `serde_json` (`float_roundtrip`), core kept dependency-free;
  seeds as canonical decimal strings; duplicate and unknown keys, missing
  members, wrong types, unsupported versions and invalid scenarios all
  refused with no defaults or repair; parse → version → migrate → decode →
  validate, and export validates first; migration infrastructure without an
  invented historical schema; eight example scenarios.
- **Built.** Crate `locsim-scenario`: `json` (ordered document tree over
  `serde_json`, duplicate keys reported, exact integers, non-finite numbers
  refused on writing), `schema` (version 1, hand-written strict `encode` /
  `decode` that collects every error with its path), `migrate`
  (`MigrationChain`, production chain empty), `error` (`ScenarioError`),
  and the two entry points `import_scenario` / `export_scenario`.
  `Examples/Scenarios/` with eight documents. `locsim-core`: no file changed.
- **Key decisions.** D14 (separate crate, dependencies, no derive, own
  tree) and D15 (document shape, strictness, seed, canonical output,
  migration on the tree). Contract C19.
- **Changed existing behaviour.** None. No engine, gate or domain code was
  touched; the 273 core tests are the same tests.
- **Found by tests.** `float_roundtrip` is necessary (removing it fails the
  float tests). `serde_json::Value` drops duplicate keys and the default
  serializer writes NaN as `null` — both avoided by the crate's own tree.
  Two wrong expectations in new tests (hand-miscounted refusal totals; six
  date-line crossings where three laps give five) were corrected against a
  derivation, not against the output.
- **Tests at close (observed 2026-10-10).** 343 in the workspace: 273 core,
  70 scenario (42 unit, 11 examples, 15 malformed, 2 round-trip). Reported
  statistics: 200 000 random floats and 4 000 arbitrary scenarios bit-exact
  through text; 210 random valid scenarios with bit-identical provider
  streams after export → import, 40 097 pairs re-checked; examples 13 608
  pairs; 32 000 random damages, none panicking, none letting an invalid
  scenario through; ten mutation checks, all caught.
- **Mistake.** Commit `019ad0f` contains the example documents and
  `tests/examples.rs` but has the message of the commit before it
  (`7547565`), because the commit ran before the message file was rewritten.
  The content was tested first (326 passing at that point). Intended title:
  "examples: eight scenario documents, each run through the provider and
  gate". History was not rewritten; the next commit message records it.
- **Left open.** No real migration exists to prove the mechanism on a
  released schema. `rust-version` 1.75 unverified for the new crate too.
  Validation errors carry a path but no line or column. Import does not
  admit routes (by design). GPX is not handled by any ticket so far. The
  `json` layer is private; T09 may want it.
- **Commits.** `5014030` (document layer), `31753c9` (schema v1),
  `7547565` (version check, migration, import/export), `019ad0f` (examples
  and their tests; mislabelled, see above), `e6bcbf2` (malformed-input and
  round-trip tests), then the documentation commit that closed the ticket.

---

## T09 — Persistence

- **Purpose.** Keep a scenario, and what a simulation last emitted, across
  a restart, without a failed or interrupted write destroying what was there.
- **How it was run.** Design first: an investigation of the code, a written
  proposal, the decisions of the project owner, two technical details
  resolved in an addendum (file integrity, atomic replacement), then six
  stages.
- **Decided by the project owner.** Store (A) the scenario and (B) the
  last-known output; defer (C) checkpoint/resume to a separate, unowned
  ticket; leave configuration and preferences out; `locsim-store` for files
  with the record codec in `locsim-scenario`; no backup generation; FNV-1a
  64 fingerprint to associate a record with its scenario; an envelope with a
  digest over the exact payload, because T08 had shown that damaged text
  can decode as a different valid scenario; correct the stale statements.
- **Found by reading the code before designing.** A run is a deterministic
  function of the scenario and the whole sequence of calls, with state in
  the schedule, the model, the noise engine, the deriver and the gate; it
  is not a function of scenario and time. Two documents said otherwise
  (D4; the provider section of the architecture) and so did the T08
  hand-off in `CURRENT_STATE.md`. Exact resume is impossible from a
  scenario and a last sample for every mode but a noise-free fixed one.
- **Built.** In `locsim-scenario`: `record` (`LastKnown`,
  `export_last_known`, `import_last_known`, `scenario_fingerprint`);
  `MigrationChain` generalised with a version key; `ScenarioError` gained
  `InvalidSample`. New crate `locsim-store`: `sha256`, `envelope`, `atomic`
  (with a private file-operations seam), `store`, `error`. `locsim-core`:
  no file changed.
- **Key decisions.** D16; D4 corrected. Contract C20 added, C19 extended.
- **Changed existing behaviour.** None in any engine or gate. In
  `locsim-scenario`, the text of `ScenarioError::UnsupportedVersion` no
  longer names `schema_version` (it now serves two document types).
- **Verified against sources.** The Windows `rename` of Rust 1.98.1 read in
  its source (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`, POSIX-rename
  fallback); the Microsoft documentation of `MoveFileExW` read for what it
  does and does not promise.
- **Found by tests.** A read-only record cannot be replaced on Windows (the
  expectation written first was the opposite). The killed-writer test passes
  even with a deliberately non-atomic replacement, so it is weak evidence
  alone; the concurrent-reader test catches that defect. Two defects in new
  tests, corrected by derivation: a payload length miscounted by one, and a
  check made through the faulty file system it was meant to be independent
  of.
- **Tests at close (observed 2026-10-10).** 431 in the workspace: 273 core,
  89 scenario, 69 store. Reported: 31 786 single-character changes to the
  examples, 1 357 a different valid scenario as bare documents, none through
  the envelope; a save failed at each of seven steps with the old record
  intact; 40 writer processes killed with the record whole every time; 240
  replacements under about 2 200 concurrent loads on Windows with no missing
  or partial record; about 3.6 ms per synced save; twelve mutation checks,
  all caught.
- **Left open.** Nothing run on a Unix-like system, including code that only
  compiles there. Nothing tested under power loss. On Windows the rename is
  not confirmed durable. Checkpoint/resume unowned. One writer per
  directory, unenforced. `rust-version` 1.75 unverified for a third crate.
- **Commits.** `1c51a32` (record codec and fingerprint), `a7de906`
  (envelope, SHA-256, atomic replacement), `d8ae8da` (`Store`, scenario),
  `e93662d` (last-known in the store; not a checkpoint), `29c8342`
  (killed-writer and platform tests), then the documentation commit that
  closed the ticket.

## Recurring lessons

- Strict comparisons plus independent arithmetic expose every rounding
  assumption. Each ticket from T04 on found at least one numerical issue this
  way. Expect the same; quantify it, do not widen a tolerance.
- Property tests with declared-tight limits (a few parts per million above
  what the engine needs) are what found the real defects.
- Test what is staged, not what is on disk, when committing a subset.
- Write the commit message, read it, then commit. Never in one step with
  anything that can fail (`019ad0f`).
- Read the code before writing the design. The scope of T09 turned on a
  fact (a run holds unexposed state) that two documents had stated wrongly.
- A test is itself tested: break the thing it guards and see whether it
  notices. The killed-writer test of T09 did not notice a non-atomic rename.
- A platform row says what was run. Code that has never been compiled for a
  platform is not "expected to work" there; it is untested.
- An expected number in a test is derived, not copied from the first run.
  When a new test fails, first ask whether the expectation or the code is
  wrong, and say which it was.
- A parameter's documentation is a contract too: T04's `max_offset_rate_mps`
  said one thing and did a weaker one for three tickets.
