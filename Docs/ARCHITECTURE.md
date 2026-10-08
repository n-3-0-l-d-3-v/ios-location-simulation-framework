# Architecture

## Layers

    Control / Configuration        (T12 app, CLI)
    Simulation Manager             state machine owner, wires everything
    Scenario Engine                serialisable scenarios, versioning
    Movement Engine                fixed [implemented]; random walk / walking / driving / orbit / replay
    Noise / Realism Engine         seeded jitter + drift, bounded  [implemented]
    Geographic Engine              WGS84 geodesics, ENU            [implemented]
    Location Abstraction           LocationProvider trait          [implemented]
    Platform Adapter               C ABI + Swift → CLLocation
    Health / Recovery              metrics, watchdog, bounded retries
    Persistence / Storage          atomic writes, schema migration

Cross-cutting: `domain` (canonical types) [implemented], `rng` (seeded PRNG)
[implemented], `scheduler` [implemented], `validation` (field, timestamp, speed,
boundary and displacement stages implemented).

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
after it. Stages: (1) field validity, (2) strictly increasing timestamps,
(3) with `SampleLimits`: reported speed ≤ `max_speed`, position inside the
scenario boundary, and displacement since the previous accepted sample ≤
`(max_speed + noise.max_offset_rate) × dt`. The gate recomputes everything from
the sample with exact geodesics and shares no state with the movement or noise
engines, so it catches their bugs too (verified by mutation: with the noise
engine's limits loosened and its self-check disabled, the gate rejects the
stream). Rejected samples do not advance validator state. Acceleration and
heading-rate stages are added by T05/T07.

### `movement`
`MovementModel::sample_at(t)` returns a noise-free `MovementSample`. Only
`FixedModel` exists. `model_for(scenario)` returns
`MovementError::UnsupportedMode` for every other mode rather than faking one.

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
