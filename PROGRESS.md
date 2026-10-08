# Progress

## Current Ticket
T04 — Jitter Engine (not started)

## Status
T00–T03 complete and tested on Windows. No iOS code exists yet.
Repository: https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework

## Completed
- T00 Repository bootstrap: Cargo workspace, README, LICENSE, Docs (architecture,
  compatibility, development), Scripts (test/lint/validate).
- T01 Domain model: SyntheticLocation, LocationSource, SimulationState (transition
  table), HealthState, Scenario, MovementMode/Parameters, NoiseParameters, Route,
  PlaybackParameters, Timestamp; validation with field-level errors.
- T02 Geographic engine: Coordinate validation, longitude/bearing normalisation,
  Vincenty inverse/direct, interpolation, radius check, velocity between fixes,
  ECEF/ENU frames. Plus seeded PRNG (xoshiro256**).
- T03 Fixed location engine:
  - `scheduler`: Clock (system, manual), drift-free TickSchedule with
    missed-tick counting, pause/resume, overflow reporting.
  - `validation`: SampleValidator (field validity, timestamp monotonicity).
  - `movement`: MovementModel trait, FixedModel.
  - `provider`: LocationProvider trait, SimulationProvider with explicit
    lifecycle, failure → Error state, pluggable ModelFactory.

## Current Work
- None in flight.

## Tests
- 76 unit tests, 6 geographic property tests (20 000 seeded cases each;
  interpolation 5 000), 4 fixed-pipeline tests (500 random scenarios with
  irregular polling; reproducibility; poll-jitter independence; 6 simulated
  hours at 10 Hz with zero drift). All pass.
- `cargo fmt --check` and `cargo clippy -D warnings` clean.
- Reference checks: Vincenty's Flinders Peak–Buninyong line (to 1 mm / 1e-5°),
  meridian quadrant 10 001 965.729 m, equatorial degree.
- Mutation spot-checks, all caught: ENU sign flip; dropping the Δσ term in
  `inverse`; stamping ticks with poll time instead of the ideal time.
- Failure injection: model returning NaN altitude, model returning an error,
  factory failing at start.

## Known Issues
- Only `MovementMode::Fixed` can run; every other mode fails at `start` with
  `UnsupportedMode` (T05/T06).
- `NoiseParameters` are validated but not applied yet (T04). A fixed scenario
  with noise configured currently emits the exact origin.
- No real-time driver: `SimulationProvider` is poll-based and nothing sleeps
  until `next_deadline()` yet. The "6-hour" test is simulated time, not a
  wall-clock soak (that is T14).
- A provider in `Error` does not recover by itself; recovery policy is T10.
- `inverse` returns NotConverged for nearly antipodal points (by design).
- JSON (de)serialisation of Scenario is not implemented (T08); Examples/Scenarios is empty.
- Docs/TESTING.md and Docs/RELIABILITY.md are not written yet (T13–T15).
- Layout deviates from the spec's `Sources/`/`Tests/` tree: Rust modules live in
  `crates/locsim-core/src/<layer>/`, unit tests beside the code, property and
  integration tests in `crates/locsim-core/tests/`.

## Compatibility
- Core: Verified on Windows 11 x86_64 / rustc 1.98.1 only.
- iOS Simulator, physical devices, jailbroken devices: Untested (no adapter yet).

## Next
- T04: noise engine as its own module — Gaussian + bounded jitter,
  low-frequency drift, seeded via `Rng::fork`, applied in ENU, clamped to
  `max_position_offset_m`; statistical tests; wire into SimulationProvider
  between movement and validation.
- Gotchas: use `Timestamp` nanos for tick maths (`from_secs_f64` resolves only
  ~240 ns at present-day epochs); samples are stamped with `tick.target`, never
  with the poll time.

## Working agreement
- One ticket at a time; the specification is the contract, not a to-do list
  to implement at once.
- One commit per logical change, each tested and pushed immediately.
