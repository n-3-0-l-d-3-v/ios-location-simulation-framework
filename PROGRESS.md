# Progress

## Current Ticket
T07 — Consistency Engine (not started)

## Status
T00–T06 complete and tested on Windows. No iOS code exists yet.
Repository: https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework

## Completed
- T00 Repository bootstrap: Cargo workspace, README, LICENSE, Docs, Scripts.
- T01 Domain model: SyntheticLocation, SimulationState (transition table),
  HealthState, Scenario, MovementMode/Parameters, NoiseParameters,
  PlaybackParameters, Timestamp; validation with field-level errors.
- T02 Geographic engine: validated Coordinate, Vincenty inverse/direct,
  interpolation, ENU frames, seeded PRNG (xoshiro256**).
- T03 Fixed location engine: Clock, drift-free TickSchedule, SampleValidator,
  MovementModel + FixedModel, LocationProvider + SimulationProvider.
- T04 Noise engine: seeded Gauss–Markov jitter, drift, clipped metadata noise;
  hard offset/displacement/boundary limits.
- T05 Movement engine: CircularModel, SteeredModel (random walk, walking,
  driving), Kinematics, gate stages for acceleration/deceleration/heading rate,
  simulated time.
- T06 Route engine:
  - `domain::Route` redesigned as validated input data: `RoutePoint` with
    elapsed nanoseconds from route start, optional altitude; optional route
    name; structured `RouteError`; `from_absolute`, `is_closed`, `legs()`.
  - `route::RoutePlan`: cubic Hermite spline in ECEF through the recorded
    points, velocity shared at every point, rest at open ends, waits, loops
    over closed routes, reverse and playback speed; `state_at(elapsed)` for any
    instant; per-segment `SegmentKinematics` with certified peaks.
  - `route::admit` / `RoutePlan::violations`: structured `RouteViolation`
    (segment, constraint, observed, limit, time) for max speed, displacement
    per sample, acceleration, deceleration, heading rate, turn at a stop,
    boundary.
  - `movement::RouteModel`: route replay as one more `MovementModel`.
  - `MovementModel::is_complete`, `ProviderStatus::trajectory_complete`.
  - Geographic: public `geodetic_to_ecef`, `ecef_to_geodetic`, `local_axes`.

## Current Work
- None in flight.

## Tests
Last full run (2026-10-09, Windows 11, rustc 1.98.1): **227 passed, 0 failed.**
`cargo fmt --check` clean; `cargo clippy --all-targets --all-features -D warnings` clean.

| Suite | Tests |
|---|---|
| Unit (in `src/`) | 170 |
| `geographic_props` | 6 |
| `fixed_pipeline` | 4 |
| `noise_props` | 8 |
| `noisy_pipeline` | 5 |
| `movement_props` | 8 |
| `moving_pipeline` | 12 |
| `route_props` | 6 |
| `route_pipeline` | 8 |

T06 property-test statistics:
- 400 random routes (open with waits, closed circuits, reverse, 0.3–3× playback,
  near the poles and on the date line), each admitted under limits only 3e-6
  above its own peaks: 400 admitted, 0 unbounded; 266 041 sampled instants,
  including 1 ns either side of every recorded point; closest approach to the
  speed / acceleration / deceleration / turn / radius limits 0.999997 of each;
  violations 0.
- The independent gate on the same kind of routes at three unrelated cadences
  each (0.3 ms–10 s, regular and jittered): 1 754 106 consecutive pairs over
  900 streams, 0 rejections.
- Any single limit set 1 % below the route (speed, displacement, acceleration,
  deceleration, heading rate) or 2 % (boundary): detected in 200 of 200 routes
  each, reporting nothing else.
- Every recorded point reproduced bit-exactly at its time, forwards and
  reversed, for 200 random routes.
- Provider, independently re-checked consecutive pairs: 3 194 (route without
  noise at 0.05, 0.37, 1 and 7 s), 912 (route with noise at 0.2, 1 and 3 s),
  600 (closed loop, 5 laps) — 4 706 in total, 0 rejections.
- Gate independence: route output corrupted in six ways (teleport, excessive
  speed, acceleration, deceleration, turn, boundary escape) is rejected by the
  validator alone, with no provider, noise stage or admission involved.
- Mutation checks, all caught (by unit tests, by the property tests and by the
  gate): heading-rate peak under-reported by half; velocity not shared across
  a recorded point (5 % mismatch); deceleration peak ignored on braking legs.

Numerical issues found and fixed during T06:
- Certified bound not actually an upper bound: the branch-and-bound returned
  an attained value up to the gap below the true supremum. It now returns the
  value plus the gap, so the bound is certified.
- Search budget: bounding a ratio by bounding numerator and denominator
  separately converges only linearly, so a 1e-9 gap needed ~30 000 intervals
  and exhausted the budget, returning a bound 0.4 % too high. Gap set to 1e-7
  (still ten times finer than the admission margin), budget raised.
- Coordinate resolution: a fix is stored to ~2 nm, so a route running within
  1e-6 of the speed limit can be measured over it at sub-millimetre steps.
  Admission now keeps `2 × resolution / update interval` of headroom on speed
  and `1e-9° / update interval` on turn rate.
- Cancellation at rest: one nanosecond before a stop the reported heading rate
  was −1757°/s where the true value is −0.82°/s. Kinematics near rest are now
  evaluated in factored form, with both segment fractions taken from the
  integer times.

## Known Issues / Limitations
- History: commit `7dab0a0` is titled as the provider change but also contains
  the `movement::RouteModel` commit. The route-model commit failed its isolated
  test run (an older provider test still expected route replay to be
  unsupported), the gate correctly refused it, and its staged files were then
  swept into the next commit. Both changes are tested together at that commit.
  Earlier: `7c96eb3` and `b3b0d6e` (T05) each have one failing unit test, fixed
  in `e318635`. History is not rewritten.
- A realistic raw GPS log will usually be rejected: an open route must start
  and end at rest, and receiver jitter produces accelerations and turn rates
  beyond pedestrian or vehicle limits. There is no smoothing, resampling or
  repair step; that would be an explicit normalisation contract, not yet
  designed.
- Speed and course are derived from the interpolant. A route cannot carry
  recorded speed/course, and the spline's speed between points is not the
  recording's (it overshoots leg mean speeds, by up to 1.5× on a rest-to-rest leg).
- A reversal between recorded points is never admitted; turning round needs a
  wait at a recorded point long enough to turn in.
- Legs longer than 100 km are rejected.
- Admission's guarantee is for the scenario's update interval or longer.
- Altitude is interpolated linearly and is not limit-checked.
- `min_speed` is not enforced on routes.
- The scenario origin is only the boundary centre in route mode; the route
  defines where the trajectory is.
- `route_props` takes about 27 s in a debug build (1.75 million gate checks).
- Carried over: behaviour of generated models is a seeded policy, not a model
  of people or traffic; one model step per sample for steered models; the gate
  skips heading checks across samples without a course; with position noise
  the heading check is weak near a pole; reported speed/course of generated
  models are not re-derived from noisy positions (T07); noise does not touch
  altitude; no real-time driver; provider does not self-recover from `Error`
  (T10); `inverse` fails for near-antipodal points; Scenario JSON is T08;
  Docs/TESTING.md and Docs/RELIABILITY.md come with T13–T15; Rust layout
  differs from the spec tree.

## Compatibility
- Core: Verified on Windows 11 x86_64 / rustc 1.98.1 only.
- iOS Simulator, physical devices, jailbroken devices: Untested (no adapter yet).

## Next
- T07 Consistency engine: make the *emitted* metadata consistent with the
  emitted positions and timestamps, after noise — speed vs displacement,
  course vs direction of travel, accuracy vs noise — and add the matching
  stage to the validation gate.
- Decisions T07 must make explicitly:
  - Reported speed is currently the model's instantaneous speed plus noise,
    while displacement reflects the mean speed over the step plus position
    noise. Decide which is authoritative and how far they may disagree.
  - Position noise moves fixes without moving speed/course. Either derive
    metadata from the noisy fixes (and accept noisier metadata) or define a
    quantified tolerance between them.
- Gotchas:
  - Gate is strict; engines keep `KINEMATIC_MARGIN` below limits, and
    admission adds coordinate-resolution headroom.
  - Models receive simulated time, not wall time.
  - Gate every commit on its test result, and unstage on failure before the
    next commit.
  - Shell quirk on this machine: a command containing a lone apostrophe
    anywhere (even inside a heredoc) fails to parse; put such text in a file.

## Working agreement
- One ticket at a time; the specification is the contract, not a to-do list
  to implement at once.
- One commit per logical change, each tested and pushed immediately.
