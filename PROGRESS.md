# Progress

Ticket-level log and the source of truth for which ticket is current. For the
long-lived project context, the invariants to preserve and how to resume in a
new session, start at [Docs/PROJECT_CONTEXT/README.md](Docs/PROJECT_CONTEXT/README.md).

## Current Ticket
T09 — Persistence (not started)

## Status
T00–T08 complete and tested on Windows. No iOS code exists yet.
Repository: https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework

## Completed
- T00 Repository bootstrap.
- T01 Domain model.
- T02 Geographic engine (Vincenty, ENU/ECEF, seeded PRNG).
- T03 Fixed location engine (scheduler, validator, provider).
- T04 Noise engine.
- T05 Movement engine (circular, steered: random walk / walking / driving).
- T06 Route engine (ECEF spline, rigorous admission, RouteModel).
- T07 Consistency engine (speed and course derived from emitted positions;
  independent check; gate restated). Details in
  `Docs/PROJECT_CONTEXT/TICKET_HISTORY.md`.
- T08 Scenario system — new crate `crates/locsim-scenario`:
  - `import_scenario(&str)` / `export_scenario(&Scenario)`: strings in and
    out, no files. Import runs parse → version → migrate → strict decode →
    `Scenario::validate` and stops at the first stage that reports anything,
    returning everything that stage found. Export validates first.
  - Schema version 1 mirrors the domain model: member names are the Rust
    field names (so error paths match `ConfigError`), every member required,
    optional values an explicit `null`, unknown and duplicate members
    refused, integer members refuse `1.0`, seed a canonical decimal string
    over the whole `u64` range, coordinates through `Coordinate::new`, routes
    through `Route::new`. Nothing defaulted, wrapped, clamped or sorted.
  - Exported text is canonical: fixed member order, shortest floats that
    read back to the same bits; import → export reproduces the text.
  - Migration: an ordered chain of steps over the document tree, one per
    version change. Only version 1 exists, so the production chain is empty;
    the mechanism is tested with test-only chains.
  - `ScenarioError`: syntax (line, column), duplicate key, unsupported
    version, migration, missing / unknown member, wrong type, invalid value,
    invalid coordinate, invalid route, invalid scenario, non-finite number.
  - `Examples/Scenarios/`: eight documents (six modes, an antimeridian
    circuit, a polar walk), all loaded and run by tests.
  - `locsim-core` is unchanged and still has no dependencies. The new crate
    depends on `serde` (no derive) and `serde_json` (`float_roundtrip`).

## Current Work
- None in flight.

## Tests
Last full run (2026-10-10, Windows 11, rustc 1.98.1, commit `e6bcbf2`):
**343 passed, 0 failed.** `cargo fmt --check` clean;
`cargo clippy --all-targets --all-features -- -D warnings` clean.

| Crate | Suite | Tests |
|---|---|---|
| `locsim-core` | Unit (in `src/`) | 202 |
| `locsim-core` | `geographic_props` | 6 |
| `locsim-core` | `fixed_pipeline` | 4 |
| `locsim-core` | `noise_props` | 8 |
| `locsim-core` | `noisy_pipeline` | 5 |
| `locsim-core` | `movement_props` | 8 |
| `locsim-core` | `moving_pipeline` | 12 |
| `locsim-core` | `route_props` | 6 |
| `locsim-core` | `route_pipeline` | 8 |
| `locsim-core` | `consistency_pipeline` | 14 |
| `locsim-scenario` | Unit (in `src/`) | 42 |
| `locsim-scenario` | `examples` | 11 |
| `locsim-scenario` | `malformed` | 15 |
| `locsim-scenario` | `round_trip` | 2 |

The 273 core tests are unchanged from T07. `route_props` took 39 s in this
session's debug builds (the documents used to say about 25 s).

T08 figures, printed by the tests (`cargo test -p locsim-scenario -- --nocapture`):
- 200 000 random finite `f64` bit patterns written and read back bit-exactly.
- 4 000 scenarios of arbitrary finite bit patterns (not valid simulations)
  round-trip bit-exactly through text and re-export to identical text.
- Each of the 58 members of a full document removed in turn; an unknown
  member added to each of its 12 objects; every member replaced by every
  JSON type: 319 refusals and 96 acceptances, both counts derived by hand in
  the test.
- 210 random valid scenarios (all modes, poles, date line, 1 ms–10 s, with
  and without noise): scenario bit-identical after export → import, text
  canonical, provider stream bit-identical, 40 097 sample pairs re-checked
  independently (33 924 with a course, 3 853 stationary).
- Examples: 13 608 pairs re-checked (13 505 with a course, 82 stationary).
- 32 000 random single-character damages to the examples: 28 397 refused,
  3 603 accepted (1 381 of them a different scenario); every accepted one is
  valid and round-trips. No panic.
- Every truncation of two example documents is a syntax error.
- Mutation checks, all caught and restored: seed accepting leading zeros;
  unknown members ignored; encoder writing a field under another name;
  integer member accepting a float; chain skipping its first step; newer
  versions accepted; import skipping `Scenario::validate`; export skipping
  validation; duplicate keys not reported; `serde_json` without
  `float_roundtrip`.

Findings during T08:
- `float_roundtrip` is required, not decorative: without it the random float
  test fails (some doubles do not read back to the same bits).
- `serde_json::Value` keeps the last of two duplicate keys silently, and the
  default serializer writes non-finite floats as `null`. Both would be silent
  repairs; the crate uses its own document tree and refuses both.
- An integer literal beyond `u64` is read by `serde_json` as an approximate
  double. It is therefore refused wherever an integer is required.
- Two expectations of mine in new tests were wrong and were corrected against
  a hand derivation, not against the output: the refusal/acceptance counts,
  and the number of date-line crossings in three laps (five, not six).

## Known Issues / Limitations
- **History:** commit `019ad0f` adds the eight example documents and
  `tests/examples.rs` but carries the message of `7547565` ("scenario:
  version check, ordered migration chain, import and export") by mistake.
  Its content was tested before it was made. History is left as it is.
- Scenario documents:
  - The short example in the original specification does not load: it has
    no `schema_version` and a different shape. It is refused with named
    problems; there are no defaults to complete it.
  - Seeds are strings. A tool that writes `"seed": 12345` produces a
    document that is refused.
  - Import does not admit a route. A document whose route cannot be replayed
    within its acceleration or heading limits imports and is refused when
    the provider starts.
  - A `ConfigError` from `Scenario::validate` has no line or column; only
    syntax errors do. Structural errors carry a path.
  - A member name containing a dot would make an error path ambiguous; no
    schema member has one.
  - The migration mechanism has never migrated a real document: there is no
    older schema.
  - `rust-version = "1.75"` is still unverified, now for two crates. The
    locked dependencies declare 1.71 or lower.
  - The examples were produced by exporting scenarios built in Rust, so they
    are canonical by construction; a test keeps them so.
- Gate changes to be aware of (from T07): the heading-rate stage allows
  turning over both intervals (`ω (dt + dt′)`); with position noise the
  speed-change stage allows `2 × offset rate` and the heading stage is void
  when the noise step is half the chord; a fix held at the fence skips both.
- Speed is an interval mean, so it depends on the sampling interval.
- At very high rates derived speed is coarse (4 mm/s at 1 MHz).
- Steps under 0.1 mm have no course, e.g. below 0.1 m/s at 1 kHz.
- The first sample of every run has no speed or course.
- The noise engine still computes its own noisy speed/course, now unused.
- If the noise engine falls back (not observed in tests) the slew limit may
  be exceeded away from the fence and the gate could reject the sample.
- History notes carried over: `7dab0a0` bundles two changes; `7c96eb3` and
  `b3b0d6e` each have one failing unit test fixed in `e318635`.
- Carried over: raw GPS logs are usually not admissible as routes; behaviour
  of generated models is a seeded policy; noise does not touch altitude; no
  real-time driver; no self-recovery from `Error` (T10);
  Docs/TESTING.md and Docs/RELIABILITY.md come with T13–T15; Rust layout
  differs from the spec tree; nothing has run on iOS.

## Compatibility
- Both crates: Verified on Windows 11 x86_64 / rustc 1.98.1 only.
- iOS Simulator, physical devices, jailbroken devices: Untested (no adapter yet).

## Next
- T09 Persistence: reliable storage and recovery of the active scenario,
  configuration, last known state, route, preferences and schema version,
  with atomic writes (write temporary, validate, replace).
- What T09 can build on: `export_scenario` / `import_scenario` give a
  validated, canonical, versioned text form; a stored file can be checked by
  importing it before it replaces the previous one.
- Decisions T09 needs (see `Docs/PROJECT_CONTEXT/CURRENT_STATE.md`).
- Gotchas:
  - Position and timestamp are authoritative; never emit speed/course from a
    model.
  - Gate is strict; engines keep `KINEMATIC_MARGIN` below limits.
  - Models receive simulated time, not wall time.
  - Gate every commit on its test result; unstage on failure.
  - Write the commit message file, look at it, and only then commit, as two
    separate steps. `019ad0f` is what happens otherwise.
  - Shell quirk on this machine: a command containing a lone apostrophe
    anywhere (even inside a heredoc) fails to parse; put such text in a file.

## Working agreement
- One ticket at a time; the specification is the contract.
- One commit per logical change, each tested and pushed immediately.
