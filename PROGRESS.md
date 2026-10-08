# Progress

## Current Ticket
T03 — Fixed Location Engine (not started)

## Status
T00–T02 complete and tested on Windows. No iOS code exists yet.

## Completed
- T00 Repository bootstrap: Cargo workspace, README, LICENSE, Docs (architecture,
  compatibility, development), Scripts (test/lint/validate).
- T01 Domain model: SyntheticLocation, LocationSource, SimulationState (transition
  table), HealthState, Scenario, MovementMode/Parameters, NoiseParameters, Route,
  PlaybackParameters, Timestamp; validation with field-level errors.
- T02 Geographic engine: Coordinate validation, longitude/bearing normalisation,
  Vincenty inverse/direct, interpolation, radius check, velocity between fixes,
  ECEF/ENU frames. Plus seeded PRNG (xoshiro256**) pulled forward for property tests.

## Current Work
- None in flight.

## Tests
- 49 unit tests + 6 property tests (20 000 seeded cases each; interpolation 5 000). All pass.
- `cargo fmt --check` and `cargo clippy -D warnings` clean.
- Reference checks: Vincenty's Flinders Peak–Buninyong line (to 1 mm / 1e-5°),
  meridian quadrant 10 001 965.729 m, equatorial degree.
- Mutation spot-checks (ENU sign flip; dropping the Δσ term in `inverse`) are
  caught by the unit tests. The property tests were not separately mutation-checked.

## Known Issues
- `inverse` returns NotConverged for nearly antipodal points (documented, by design).
- JSON (de)serialisation of Scenario is not implemented (T08); Examples/Scenarios is empty.
- Docs/TESTING.md and Docs/RELIABILITY.md are not written yet (T13–T15).
- Layout deviates from the spec's `Sources/`/`Tests/` tree: Rust modules live in
  `crates/locsim-core/src/<layer>/`, unit tests beside the code, property and
  integration tests in `crates/locsim-core/tests/`.

## Compatibility
- Core: Verified on Windows 11 x86_64 / rustc 1.98.1 only.
- iOS Simulator, physical devices, jailbroken devices: Untested (no adapter yet).

## Next
- T03: `LocationProvider` trait, drift-free scheduler (`start + n × interval`,
  injectable clock), fixed-position engine, lifecycle via SimulationState,
  per-sample validation, deterministic tests.
- Gotcha for T03+: use `Timestamp` nanos for tick maths; `from_secs_f64` only
  resolves ~240 ns at present-day epoch values.
