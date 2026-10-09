# Progress

Ticket-level log and the source of truth for which ticket is current. For the
long-lived project context, the invariants to preserve and how to resume in a
new session, start at [Docs/PROJECT_CONTEXT/README.md](Docs/PROJECT_CONTEXT/README.md).

## Current Ticket
T08 — Scenario System (not started)

## Status
T00–T07 complete and tested on Windows. No iOS code exists yet.
Repository: https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework

## Completed
- T00 Repository bootstrap.
- T01 Domain model.
- T02 Geographic engine (Vincenty, ENU/ECEF, seeded PRNG).
- T03 Fixed location engine (scheduler, validator, provider).
- T04 Noise engine.
- T05 Movement engine (circular, steered: random walk / walking / driving).
- T06 Route engine (ECEF spline, rigorous admission, RouteModel).
- T07 Consistency engine:
  - `consistency::KinematicsDeriver`: speed and course derived by backward
    difference from the emitted positions and timestamps (the authoritative
    observation). First sample has neither; < 4 nm displacement is exactly
    stationary; < 0.1 mm has a speed but no course; no course is carried over.
  - `consistency::check_first` / `check_pair`: independent re-derivation that
    rejects contradictory metadata; `ConsistencyTolerance` (3 σ of configured
    observation noise, accuracy band).
  - Provider pipeline: movement → noise → final position → derive → validate
    → emit. Model/noise speed and course no longer reach the output.
  - Gate: new consistency stage; kinematic stages restated for interval means
    and chord directions (speed change over `(dt + dt′)/2`, course change over
    `dt + dt′`), with physical allowances and coordinate-resolution terms.
  - Noise engine: the offset is now slew-limited to `max_offset_rate × dt`,
    as its parameter always documented; fence pull is the one exception and
    the gate recognises a fix held at the fence.
  - `geographic::COORDINATE_RESOLUTION_M` moved from the route engine.

## Current Work
- None in flight. After T07 the project context documents were added
  (`Docs/PROJECT_CONTEXT/`) and stale statements in README, ARCHITECTURE and
  COMPATIBILITY corrected; no code changed.

## Tests
Last full run (2026-10-09, Windows 11, rustc 1.98.1): **273 passed, 0 failed.**
`cargo fmt --check` clean; `cargo clippy --all-targets --all-features -D warnings` clean.

| Suite | Tests |
|---|---|
| Unit (in `src/`) | 202 |
| `geographic_props` | 6 |
| `fixed_pipeline` | 4 |
| `noise_props` | 8 |
| `noisy_pipeline` | 5 |
| `movement_props` | 8 |
| `moving_pipeline` | 12 |
| `route_props` | 6 |
| `route_pipeline` | 8 |
| `consistency_pipeline` | 14 |

T07 independently re-checked consecutive sample pairs (shared helper that
restates the rules without using the gate or the deriver):
- every source × {no noise, noise}, regular and jittered: 11 484
- intervals 1 ms, 10 ms, 100 ms, 1 s, 5 s (each regular + jittered, all
  sources, with and without noise): 40 560
- 180 random scenarios (poles, date line, 10 ms–10 s): 51 694
  (42 440 with a course, 8 357 stationary)
- noisy fixed scenarios (existing suite, now re-checked): 95 658
- Without observation noise: max |speed − distance/elapsed| = 0 and max
  course error = 0 (bit-identical to the geodesic computation); with noise:
  within the 3 σ reach (0.3 m/s, 12°).
- Mutations of emitted metadata rejected by the gate, with and without noise:
  speed ×2, speed zero while moving, course reversed, course missing, speed
  missing, timestamp shifted, NaN/∞ speed, NaN course, accuracy inflated,
  kinematics on a first sample, speed on a stationary fix; without noise also
  stale course and 0.1 % speed error.
- Engine mutation checks, all caught: speed 0.1 % high; departure bearing
  instead of arrival bearing; carried-over course on a stationary fix;
  consistency check disabled.

Performance (release build, walking scenario, per sample): whole pipeline
≈ 1.2 µs, deriving kinematics ≈ 270 ns (≈ 23 %), final validation ≈ 310 ns.
One geodesic inverse per derivation, no allocation, no cache beyond the
previous emitted fix.

Numerical issues found and fixed during T07:
- Noise engine limited only the emitted step, not the offset's rate of change;
  the emitted track could double back on a straight mover (found when derived
  courses failed the heading stage). Offset is now slew-limited.
- Rounding slack for `derived + noise` and for accuracy comparisons must scale
  with the magnitude being rounded, not with the smaller difference.
- `1 − cos(90°)` is not exactly 1 in floating point; the chord-shortfall bound
  returns the full speed explicitly at half a turn or more.
- At 1 kHz and above, derived speed is limited by coordinate resolution
  (4 nm / dt); the gate adds exactly that term rather than a tolerance.

## Known Issues / Limitations
- Gate changes to be aware of: the heading-rate stage now allows turning over
  both intervals (`ω (dt + dt′)`), up to twice the previous `ω dt` for even
  sampling; with position noise the speed-change stage allows `2 × offset
  rate` and the heading stage is void when the noise step is half the chord;
  a fix held at the fence skips both stages.
- Speed is an interval mean, so it depends on the sampling interval; it is
  not an instantaneous (Doppler-like) speed.
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
  real-time driver; no self-recovery from `Error` (T10); Scenario JSON is T08;
  Docs/TESTING.md and Docs/RELIABILITY.md come with T13–T15; Rust layout
  differs from the spec tree; nothing has run on iOS.

## Compatibility
- Core: Verified on Windows 11 x86_64 / rustc 1.98.1 only.
- iOS Simulator, physical devices, jailbroken devices: Untested (no adapter yet).

## Next
- T08 Scenario system: JSON import/export, schema versioning and migration,
  example scenarios (including routes with elapsed-time points).
- Gotchas:
  - Position and timestamp are authoritative; never emit speed/course from a
    model.
  - Gate is strict; engines keep `KINEMATIC_MARGIN` below limits.
  - Models receive simulated time, not wall time.
  - Gate every commit on its test result; unstage on failure.
  - Shell quirk on this machine: a command containing a lone apostrophe
    anywhere (even inside a heredoc) fails to parse; put such text in a file.

## Working agreement
- One ticket at a time; the specification is the contract.
- One commit per logical change, each tested and pushed immediately.
