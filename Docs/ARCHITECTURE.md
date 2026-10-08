# Architecture

## Layers

    Control / Configuration        (T12 app, CLI)
    Simulation Manager             state machine owner, wires everything
    Scenario Engine                serialisable scenarios, versioning
    Movement Engine                fixed, random walk, walking, driving, orbit [implemented]; replay (T06)
    Noise / Realism Engine         seeded jitter + drift, bounded  [implemented]
    Geographic Engine              WGS84 geodesics, ENU            [implemented]
    Location Abstraction           LocationProvider trait          [implemented]
    Platform Adapter               C ABI + Swift → CLLocation
    Health / Recovery              metrics, watchdog, bounded retries
    Persistence / Storage          atomic writes, schema migration

Cross-cutting: `domain` (canonical types) [implemented], `rng` (seeded PRNG)
[implemented], `scheduler` [implemented], `validation` (field, timestamp, speed,
boundary, displacement, acceleration, deceleration and heading-rate stages
implemented).

Dependency rule: a layer may import only layers below it plus `domain`/`rng`.
The platform adapter contains conversion and lifecycle code only.

## Language and packaging

| Part | Choice | Why |
|---|---|---|
| Simulation core | Rust crate `locsim-core`, zero dependencies, `#![forbid(unsafe_code)]` | Testable on any desktop OS without an iPhone; bit-reproducible seeded output; cross-compiles to iOS |
| Platform boundary | C ABI crate (`locsim-ffi`, T11) | The only stable interface Swift/Obj-C can consume |
| iOS adapter + demo app | Swift (T11/T12) | Core Location and UI are Apple-only APIs |

## Implemented modules

### `rng`
xoshiro256** seeded via SplitMix64, with `fork(stream)` so movement and noise
draw from independent streams. In-crate so that results never change with a
dependency upgrade.

### `geographic`
- `Coordinate`: private fields, validated on construction; cannot hold NaN or
  out-of-range values. Longitude `[-180, 180]`, wrapping only via an explicit
  constructor.
- Geodesics: Vincenty inverse/direct on WGS84 with a *relative* convergence
  tolerance (an absolute one silently returns the uncorrected first iterate for
  millimetre-scale lines — found by test). Measured round-trip error is below
  1 nm for 1 mm–1 m lines; property tests bound it at 10 µm up to ~3 000 km.
- Nearly antipodal pairs do not converge in Vincenty's method; `inverse`
  returns `GeoError::NotConverged` instead of an approximation. Acceptable for
  this domain (no scenario spans half the planet in one segment); Karney's
  algorithm is the upgrade path if that changes.
- `EnuFrame`: WGS84 ⇄ ECEF ⇄ local East-North-Up. Movement models will
  integrate in metres in this frame and convert back.
- Helpers: `distance`, `bearing`, `destination`, `interpolate`,
  `within_radius`, `velocity_between`, bearing/longitude normalisation.

### `domain`
- `SyntheticLocation` — canonical sample; unknown speed/course are `None`
  (the adapter maps to Core Location's `-1`). `validate()` rejects non-finite
  values, negative accuracy/speed, course outside `[0, 360)`, and a course
  without motion.
- `SimulationState` — explicit transition table; `transition()` is the only
  way to change state. Every active state can reach `Stopping`; `Stopping`
  only leads to `Idle`.
- `Scenario`, `MovementParameters`, `NoiseParameters`, `Route`,
  `PlaybackParameters` — plain data with exhaustive validation that reports
  all errors at once, including mode-specific requirements and physically
  impossible configurations (orbit or route playback faster than `max_speed`).
- `Timestamp` — integer nanoseconds, so `start + n × interval` is exact.
- `HealthState`.

### `scheduler`
- `Clock` trait with `SystemClock` (epoch captured once, advanced by a
  monotonic timer) and `ManualClock` (tests).
- `TickSchedule`: tick `n` is due at `start + n × interval`, recomputed from
  the origin on every poll, so wake-up lateness never accumulates. It is a
  pure state machine — it neither sleeps nor reads a clock; a driver polls it.
- Backpressure: when polling falls behind, overdue slots are skipped and
  counted (`missed`), never replayed as a burst.
- Pause/resume shifts the origin by the paused duration: phase within the
  interval is preserved and paused time is not counted as missed.

### `validation`
`SampleValidator` is the last step before emission; nothing modifies a sample
after it. It shares no state with the movement or noise engines and recomputes
every quantity from the emitted samples with exact geodesics, so it catches
their bugs too.

| Stage | Rejects |
|---|---|
| Field validity | NaN/infinite values, out-of-range coordinates, negative accuracy or speed, course outside `[0, 360)`, course without motion |
| Timestamp | not strictly after the previous accepted sample |
| Speed | reported speed above `max_speed` |
| Boundary | position outside `movement.radius_m` of the origin (not in circular mode, where the radius is the orbit) |
| Displacement | moved more than `(effective max speed + noise.max_offset_rate) × dt` — teleportation |
| Acceleration | speed rose by more than `max_acceleration × dt` |
| Deceleration | speed fell by more than `max_deceleration × dt` |
| Heading rate | course turned by more than `max_heading_rate × dt` |

Comparisons are strict; there is no numerical tolerance. The only allowances
are physical and come from configured noise: speed and heading noise are
clipped at ±3 σ, so two readings may differ by 6 σ beyond the true change; and
position noise of reach `m` can alter the convergence between two fixes by up
to `2·m·tan(lat)/R` (unbounded at a pole, where the heading check is skipped).
Rejected samples do not advance validator state.

**Turning is measured on the surface.** A straight path changes bearing as it
goes — tens of degrees per kilometre near a pole — so the previous course is
first carried along the geodesic to the new fix (`Geodesic::convergence_deg`
is added) and only the remainder counts as a turn. `convergence_deg` uses a
half-angle formula because subtracting the two bearings of a very short line
is ill-conditioned (each is only good to ~1e-3° for a 0.1 mm line near a pole;
the convergence is good to 1e-9°).

Mutation evidence that the gate is independent: with the noise engine's limits
loosened, and separately with the steered model's acceleration limit, turn cap,
fence or convergence handling broken, the pipeline tests fail because the gate
rejects the stream.

### `movement`
`MovementModel::sample_at(t)` maps *simulated* time to a noise-free
`MovementSample`; `kinematics()` exposes the full trajectory state (position,
speed, heading, acceleration, heading rate). All geometry goes through the
`geographic` geodesic primitives — no model does arithmetic on latitude or
longitude.

| Mode | Model | Nature |
|---|---|---|
| Fixed | `FixedModel` | stationary, unchanged since T03 |
| Circular | `CircularModel` | closed form in elapsed time |
| RandomWalk, Walking, Driving | `SteeredModel` | inertial mover, one step per sample |
| RouteReplay | — | `UnsupportedMode` until T06 |

**`CircularModel`.** Position at time `t` is the point at exactly `radius`
from the centre on bearing `phase ± ω·(t − t₀)` (geodesic direct solution), so
nothing is integrated and no drift can accumulate: after six simulated hours at
10 Hz the radius error is below 1 µm. Course is the tangent. The orbit starts in
steady state. The reported speed is the nominal `r·ω`; true ground speed is
lower by about `r²/6R²` because a circle on a curved surface is shorter than
`2πr`.

**`SteeredModel`** — what is physically modelled:
- *Speed* ramps linearly towards a target at no more than the acceleration or
  deceleration limit, then holds. Distance covered is the exact integral of
  that profile, so the same straight run sampled at 100 Hz or every 5 s ends
  at the same point. A mover starts from rest, cannot jump to speed and cannot
  stop dead.
- *Heading* turns by at most `max_heading_rate × dt`; the mover then follows
  the geodesic leaving on that heading, and the stored heading is the bearing
  on arrival, so convergence is handled at any latitude and across the
  antimeridian.
- *Boundary.* The mover never plans to be closer to the edge than its braking
  distance `v²/2d` along its line, and once the edge is within braking distance
  plus two turning radii it steers towards a random direction in the inward
  half-plane. It therefore brakes for the fence instead of being clipped at
  it, cannot deadlock against it, and cannot wander off. The planned step is
  re-checked with exact geodesics before use; in 150 000 random steps and
  200 000 long-walk steps that re-check never had to shorten one.

What is merely configurable (a seeded policy, not a model of people or
traffic): the cruising speed is redrawn uniformly from `[min_speed, max_speed]`
after an exponentially distributed hold (`speed_change_interval_s`); pauses
start with `pause_probability` per second and last up to `max_pause_s`; the
desired turn per step is uniform within `(1 − heading_persistence) × 180°`.
Random walk aims for the constant speed `step_distance / update_interval` and
never pauses. Walking and driving are the same model with different parameters
(`MovementParameters::walking_preset()` / `driving_preset()`). There is no
road network, lateral-acceleration limit or minimum turning radius, and
`min_speed` bounds the cruising target only.

Constraints (all in `MovementParameters`, separate from noise):
`max_speed_mps`, `min_speed_mps`, `max_acceleration_mps2`,
`max_deceleration_mps2`, `max_heading_rate_dps`, and
`max_displacement_per_sample_m`. The last one is defined per nominal update
interval and acts as an extra speed limit `cap / update_interval`
(`effective_max_speed_mps`); after skipped ticks the allowance scales with the
elapsed time, which is the only definition that does not conflict with the
deceleration limit.

**Margins instead of tolerances.** The gate is strict and uses different
arithmetic from the models, so each model stays `KINEMATIC_MARGIN` (1e-6
relative) below every limit, plus 1e-9° on turns. The property tests show the
models reaching 0.999999 of each limit and never exceeding one. Configurations
that sit closer than the margin to their own limit (an orbit at exactly
`max_speed`) are rejected at validation.

**Determinism.** Randomness comes from streams forked from the scenario seed
(heading, speed, pause), separate from the noise streams, with a fixed number
of draws per step. Same scenario, seed and timestamps ⇒ identical output.
Behavioural decisions are made once per step, so the same seed sampled at a
different rate is a different (equally valid) trajectory, not a refinement.

### `noise`
Position of the engine in the pipeline:

    base movement → noise → assemble metadata → final validation → emit

It depends only on `domain`, `geographic`, `rng` and the `MovementSample`
type; it never sees the scheduler or the platform.

Model (noise = measurement error on top of the true motion):
- **Jitter**: per-axis first-order Gauss–Markov process with σ =
  `position_noise_m` and correlation time `position_correlation_time_s`
  (`ρ = exp(−dt/τ)`, so correlation depends on elapsed time, not on sample
  count). τ = 0 gives independent high-frequency noise. Clipped at 3 σ
  (`NOISE_CLIP_SIGMA`).
- **Drift**: a point travelling at exactly `drift_rate_mps` between uniformly
  drawn waypoints in a disc of radius `max_position_offset − 3 σ`.
- **Speed / heading / accuracy**: clipped scalar Gauss–Markov channels.
- Each component draws from its own forked random stream, so switching one on
  never changes another's sequence. Same `(parameters, constraints, seed,
  inputs)` ⇒ bit-identical output; `reset()` replays.

Hard guarantees, each verified with exact geodesic distances before a
position is returned:
1. offset from the true position ≤ `max_position_offset_m`;
2. displacement from the previous output ≤ true displacement +
   `max_offset_rate_mps × dt`;
3. inside the scenario boundary (`movement.radius_m` around the origin).

How they are met: the candidate is projected exactly onto the intersection of
the offset and step discs in a local tangent plane (re-anchored every 1 km),
then pulled inside the boundary along a geodesic. If the exact check still
fails, fallbacks are tried in order — a point on the geodesic from the previous
output to the true position (feasible by the triangle inequality), holding the
previous output, the true position — and if none passes the engine returns
`ConstraintUnsatisfiable` rather than a position. `fallback_count()` exposes how
often this happens (0 in 150 000 random samples).

Design decisions worth knowing:
- **What "maximum speed" means with noise.** A noisy fix necessarily moves
  between samples even when the true position does not. The *reported speed
  field* never exceeds `movement.max_speed_mps`; the *distance between
  consecutive outputs* is bounded by `(max_speed + max_offset_rate) × dt`.
  `max_offset_rate_mps` is therefore the knob that makes jitter physically
  plausible at any sample rate.
- **Metadata.** Reported speed and course are those of the underlying motion
  plus bounded measurement noise; they are not re-derived from the jittered
  fixes (which would turn position noise into fake velocity). A stationary
  sample stays exactly stationary with no course; a course is dropped when
  noisy speed reaches zero. The full consistency engine is T07.
- **No masking.** A base position outside the boundary, or a base speed above
  the maximum, is not "repaired" by noise: the former is an error, the latter
  is passed through for the validation gate to reject.
- **Identity.** With all components at zero the engine returns its input
  unchanged, bit for bit.

### `provider`
- `LocationProvider`: `start / stop / pause / resume / poll / current_location
  / status`. Unlike the sketch in the specification, lifecycle calls take the
  current time as an argument: providers never read a clock, which is what
  makes runs reproducible and testable without sleeping.
- `SimulationProvider`: per due tick, model → assemble → validate → emit.
  Models are sampled at *simulated* time `start + tick index × interval`,
  which stands still while paused, so a mover resumes where it was instead
  of leaping ahead; samples are stamped with wall time.
  (Since T04: model → noise → assemble → validate → emit.)
  Samples are stamped with the tick's ideal time, so the emitted stream does
  not depend on poll punctuality.
- Failure behaviour: an invalid scenario is rejected at `start` and the
  provider stays `Idle`. If a sample cannot be produced or fails validation,
  nothing is emitted, `failed_count` increments and the state becomes `Error`
  (no further samples; `stop` still works). Bounded automatic recovery is the
  health subsystem's job (T10).
- `ModelFactory` lets another movement engine be plugged in without modifying
  the provider.

## Platform delivery (planned, T11)

Mechanisms in scope:
1. iOS Simulator: `xcrun simctl location` and GPX export.
2. Development devices: GPX for Xcode's location simulation; in-app injection
   through the framework's own `LocationProvider` for the app under development.
3. Jailbroken research devices: to be designed only once hardware is available
   to verify it; remains Untested until then.

Explicitly out of scope: concealing the framework or jailbreak, bypassing
integrity checks, defeating third-party anti-spoofing.

## Privacy

The core never reads the device's real location. Logging (T10) will default to
not recording coordinates above DEBUG level.
