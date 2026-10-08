# Progress

## Current Ticket
T05 — Movement Engine (not started)

## Status
T00–T04 complete and tested on Windows. No iOS code exists yet.
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
- T03 Fixed location engine: Clock, drift-free TickSchedule, SampleValidator,
  MovementModel + FixedModel, LocationProvider + SimulationProvider.
- T04 Jitter / noise engine:
  - `noise::NoiseEngine`: seeded, independent of scheduler and platform.
    Gauss–Markov jitter (time-correlated), constant-rate waypoint drift,
    clipped speed/heading/accuracy channels, independent random streams.
  - Hard limits verified with exact geodesics on every sample: offset bound,
    per-sample displacement, scenario boundary. Fallback chain, then an error
    — never an out-of-limit position.
  - New parameters: `position_correlation_time_s`, `max_offset_rate_mps`;
    `Scenario::boundary()`; validation rules for all of them.
  - Final validation gate extended: speed ≤ max, inside boundary,
    displacement ≤ (max_speed + max_offset_rate) × dt.
  - Pipeline: movement → noise → assemble → validate → emit.

## Current Work
- None in flight.

## Tests
Last full run (2026-10-08, Windows 11, rustc 1.98.1): **120 passed, 0 failed.**
- 97 unit tests.
- 6 geographic property tests (20 000 seeded cases each; interpolation 5 000).
- 4 fixed-pipeline tests (500 random scenarios, 6 simulated hours at 10 Hz).
- 8 noise property/statistical tests: 600 random configurations × 250 samples
  with strict (zero-tolerance) limit assertions; 8 extreme configurations ×
  3 000 samples; reproducibility; clipped-Gaussian statistics, axis
  independence, lag-1 correlation at two time steps, drift, offset-rate cap.
- 5 noisy-pipeline tests: 300 random noisy scenarios through the provider with
  zero rejected samples; reproducibility; restart replay; identity when disabled.
- `cargo fmt --check` clean; `cargo clippy --all-targets --all-features -D warnings` clean.
- Mutation checks, all caught: ENU sign flip; dropped Δσ term; ticks stamped
  with poll time; noise limits loosened 5 % with the engine's self-check
  disabled (caught by the noise tests *and* independently by the validation
  gate in the pipeline tests).
- Defects found by the T04 property tests and fixed before commit:
  plane→ellipsoid mapping mismatch (12 µm at 1 km) forcing fallbacks on ~1.4 %
  of moving samples; sequential disc projections not being a projection onto
  the intersection; relative-only margins vanishing for tiny limits
  (`ConstraintUnsatisfiable`). Fallback rate is now 0 in 150 000 random samples.

## Known Issues / Limitations
- Only `MovementMode::Fixed` can run; other modes fail at `start` with
  `UnsupportedMode` (T05/T06). Consequently noise on a *moving* base is tested
  only at the engine level (synthetic circular trajectories), not end to end.
- Noise does not touch altitude; there is no vertical position noise parameter.
- Reported speed/course are the base motion's plus measurement noise; they are
  not re-derived from noisy positions. With noise, distance between outputs is
  bounded by `(max_speed + max_offset_rate) × dt`, not by `max_speed × dt`.
  Full metadata consistency is T07.
- With limits below ~1e-7 m (e.g. an offset rate of nanometres per second) the
  output holds its previous position rather than resolving the motion; with a
  moving base and such limits `ConstraintUnsatisfiable` is possible in
  principle (reported as an error, never emitted). Not observed in tests.
- Per-step true displacement above ~5 km exceeds the tangent-plane accuracy
  margin; the geodesic fallback then takes over (correct but noise is damped).
- Statistical tests use fixed seeds: they detect regressions, they are not a
  proof of distributional correctness for every seed.
- Jitter/drift axes are tied to the local tangent plane, which is re-anchored
  every 1 km; the noise state is not rotated on re-anchoring (irrelevant for
  isotropic noise except within metres of a pole).
- No real-time driver: `SimulationProvider` is poll-based. The "6-hour" test is
  simulated time, not a wall-clock soak (T14).
- A provider in `Error` does not recover by itself (T10).
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
- T05: bounded random walk, walking, driving and circular models behind
  `MovementModel`; acceleration / heading-rate limits; extend the validation
  gate with those stages; end-to-end noise tests on moving bases.
- Gotchas:
  - Use `Timestamp` nanos for tick maths (`from_secs_f64` resolves only
    ~240 ns at present-day epochs); samples are stamped with `tick.target`.
  - The validation gate is strict (no tolerance): a model that moves at
    exactly `max_speed` must leave itself a margin, as the noise engine does.
  - Converting tangent-plane points to coordinates is not the inverse of
    `to_enu` with `up` dropped; see `from_plane` in `noise/mod.rs`.

## Working agreement
- One ticket at a time; the specification is the contract, not a to-do list
  to implement at once.
- One commit per logical change, each tested and pushed immediately.
