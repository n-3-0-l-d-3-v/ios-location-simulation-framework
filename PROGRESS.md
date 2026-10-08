# Progress

## Current Ticket
T06 — Route Engine (not started)

## Status
T00–T05 complete and tested on Windows. No iOS code exists yet.
Repository: https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework

## Completed
- T00 Repository bootstrap: Cargo workspace, README, LICENSE, Docs, Scripts.
- T01 Domain model: SyntheticLocation, SimulationState (transition table),
  HealthState, Scenario, MovementMode/Parameters, NoiseParameters, Route,
  PlaybackParameters, Timestamp; validation with field-level errors.
- T02 Geographic engine: validated Coordinate, Vincenty inverse/direct,
  interpolation, ENU frames, seeded PRNG (xoshiro256**).
- T03 Fixed location engine: Clock, drift-free TickSchedule, SampleValidator,
  MovementModel + FixedModel, LocationProvider + SimulationProvider.
- T04 Noise engine: seeded Gauss–Markov jitter, drift, clipped metadata
  noise; hard offset/displacement/boundary limits; pipeline
  movement → noise → assemble → validate → emit.
- T05 Movement engine:
  - `CircularModel` (Mode E): closed form in elapsed time, geodesic circle,
    both directions, starting phase; no accumulated state.
  - `SteeredModel` (Modes B, C, D — bounded random walk, walking, driving):
    inertial mover with linear speed ramps integrated exactly, heading-rate
    limited turns along geodesics, brake-for-the-fence boundary handling,
    seeded behaviour policy (cruise speed, pauses, wander).
  - `Kinematics` (position, speed, heading, acceleration, heading rate).
  - `FixedModel` unchanged.
  - New parameters: `max_displacement_per_sample_m`, `speed_change_interval_s`,
    `start_phase_deg`; `walking_preset()` / `driving_preset()`;
    `effective_max_speed_mps`; `KINEMATIC_MARGIN`.
  - New validation rules: random walk needs non-zero dynamics; orbit must fit
    the heading-rate limit and the capped speed; displacement cap must allow
    min speed; random-walk step must be a reachable speed.
  - Gate: acceleration, deceleration and heading-rate stages; turning measured
    on the surface via `Geodesic::convergence_deg` (new, well-conditioned).
  - Provider: models run on simulated time (frozen while paused).

## Current Work
- None in flight.

## Tests
Last full run (2026-10-09, Windows 11, rustc 1.98.1): **176 passed, 0 failed.**
`cargo fmt --check` clean; `cargo clippy --all-targets --all-features -D warnings` clean.

| Suite | Tests |
|---|---|
| Unit (in `src/`) | 133 |
| `geographic_props` | 6 |
| `fixed_pipeline` | 4 |
| `noise_props` | 8 |
| `noisy_pipeline` | 5 |
| `movement_props` | 8 |
| `moving_pipeline` | 12 |

T05 property-test statistics:
- Steered models, 500 random configurations × 300 steps (150 000 steps,
  irregular 1 ms–100 s, half fenced, poles/date line included): 128 763 steps
  moved; closest approach to each limit as a fraction of it — speed 0.99996,
  acceleration 0.999999, deceleration 0.999999, turn 0.999999, fence radius
  0.999999; violations 0; exact re-check shortened a step 0 times.
- 12 extreme configurations (1 µs to 1 day steps, 20 cm and 3 m pens, 500 m/s²
  and 0.001 m/s² dynamics, fence enclosing the pole): 0 violations, 0 clamps,
  none deadlocked.
- Bounded walk, 200 000 steps in a 25 m disc: > 200 km travelled, mean radius
  between 8 and 20 m, every quadrant ≥ 15 % of samples, first and last tenth
  within 2 m of each other in mean radius.
- 300 random orbits × 200 samples: radius within 1 µm, bearing within 1 µm of
  arc, turn equal to ω·dt within 1e-6.
- Pipeline: 600 random valid scenarios (150 per moving mode, 300 with noise),
  0 rejected samples, 141 924 consecutive pairs independently re-checked,
  all 600 streams moved.
- Orbit, 6 simulated hours at 10 Hz through the provider: radius error < 1 µm,
  final bearing as predicted to 1e-6°.
- Gate rejects scripted teleportation, instantaneous acceleration,
  instantaneous stop and an about-turn; a fence escape is stopped before
  emission; a legitimate script passes.
- Mutation checks, all caught by the model tests and independently by the gate
  in the pipeline tests: acceleration limit ignored; turn cap doubled; fence
  ignored; convergence dropped from the stored heading. (Earlier ones still
  hold: ENU sign, Δσ term, tick stamping, loosened noise limits.)

Numerical issues found and fixed during T05:
- Bearings from `inverse` are ill-conditioned for very short lines (~1e-3° for
  0.1 mm near a pole), which would make a strict heading-rate check reject
  correct motion. Fixed at the source with `convergence_deg` (1e-9°).
- Comparing raw course values treats meridian convergence as turning; near a
  pole straight travel swings tens of degrees per km. The gate and the tests
  carry the previous course along the geodesic first.
- A purely relative margin on the turn cap vanishes at microsecond steps; an
  absolute 1e-9° is subtracted as well.
- Rounding of `v0 + a·dt − v0` can exceed `a·dt` at tiny `dt`; the model holds
  speed for that step instead (never observed above 1e-6 relative).
- Behavioural, not numerical: steering at the exact centre made tightly fenced
  walks identical for every seed; the aim is now random within the inward
  half-plane.

## Known Issues / Limitations
- Commits `7c96eb3` and `b3b0d6e` (circular model, steered model) each have one
  failing unit test, `unimplemented_modes_are_reported_not_faked`: it still
  expected circular mode to be unsupported until the provider commit `e318635`
  updated it. I committed without gating on the test result. History is not
  rewritten; every other commit, and the head, pass.
- Route replay is not implemented (T06); it fails at `start` with `UnsupportedMode`.
- Behaviour is a seeded policy, not a model of pedestrians or traffic. No road
  network, lateral-acceleration limit, or minimum turning radius: a "driver"
  can turn on the spot at low speed.
- One model step per sample: the same seed at a different sample rate is a
  different trajectory. After skipped ticks one long step is taken.
- `min_speed` bounds the cruising target only.
- The reported course is the model heading at the sample; reported speed is
  the instantaneous speed, while displacement reflects the mean speed over the
  step. Full metadata consistency (and re-derivation from noisy fixes) is T07.
- The gate does not check a heading change across samples where a course is
  absent (stopped, or noisy speed clipped to zero).
- With position noise the heading check is weakened by the convergence
  allowance and effectively off within a few noise-reaches of a pole.
- A trajectory that passes within about a metre of a pole has an ill-defined
  bearing there; not handled specially and not observed in tests.
- Fence planning is planar with the radius reduced by `(r/R)²`; fine for the
  tested range (≤ 3 km), untested for fences of hundreds of kilometres.
- Circular reported speed is nominal `r·ω` (true ground speed lower by ~r²/6R²).
- Carried over: noise does not touch altitude; no real-time driver (poll-based,
  long runs are simulated time); provider does not self-recover from `Error`
  (T10); `inverse` fails for near-antipodal points; Scenario JSON is T08;
  Docs/TESTING.md and Docs/RELIABILITY.md come with T13–T15; Rust layout
  differs from the spec tree.

## Compatibility
- Core: Verified on Windows 11 x86_64 / rustc 1.98.1 only.
- iOS Simulator, physical devices, jailbroken devices: Untested (no adapter yet).

## Next
- T06 Route engine: `RouteModel` behind `MovementModel` — load points,
  interpolate along geodesics in simulated time, playback speed, looping,
  reverse; pause/resume already works through simulated time.
- Decisions T06 must make explicitly:
  - How replay relates to the gate: recorded tracks can contain accelerations
    and turns the movement limits forbid. Either validate the route against
    the limits up front (as is done for speed) or define replay-specific
    limits — do not loosen the gate.
  - Looping from the last point back to the first is a teleport unless the
    route is closed; reject, or require an explicit closing segment.
- Gotchas:
  - Gate is strict; models keep `KINEMATIC_MARGIN` below limits.
  - Models receive simulated time, not wall time.
  - Gate every commit on the test result (`&&`), including intermediate ones.
  - Shell quirk on this machine: a command containing a lone apostrophe
    anywhere (even inside a heredoc) fails to parse; put such text in a file.

## Working agreement
- One ticket at a time; the specification is the contract, not a to-do list
  to implement at once.
- One commit per logical change, each tested and pushed immediately.
