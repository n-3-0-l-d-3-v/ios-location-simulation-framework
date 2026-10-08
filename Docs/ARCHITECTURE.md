# Architecture

## Layers

    Control / Configuration        (T12 app, CLI)
    Simulation Manager             state machine owner, wires everything
    Scenario Engine                serialisable scenarios, versioning
    Movement Engine                fixed [implemented]; random walk / walking / driving / orbit / replay
    Noise / Realism Engine         seeded jitter + drift, bounded
    Geographic Engine              WGS84 geodesics, ENU            [implemented]
    Location Abstraction           LocationProvider trait          [implemented]
    Platform Adapter               C ABI + Swift → CLLocation
    Health / Recovery              metrics, watchdog, bounded retries
    Persistence / Storage          atomic writes, schema migration

Cross-cutting: `domain` (canonical types) [implemented], `rng` (seeded PRNG)
[implemented], `scheduler` [implemented], `validation` (first two stages
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
`SampleValidator` is the gate in front of emission. Implemented stages: field
validity (`SyntheticLocation::validate`) and strictly increasing timestamps.
Rejected samples do not advance validator state. Movement-constraint,
metadata-consistency and scenario-boundary stages are added by T05/T07.

### `movement`
`MovementModel::sample_at(t)` returns a noise-free `MovementSample`. Only
`FixedModel` exists. `model_for(scenario)` returns
`MovementError::UnsupportedMode` for every other mode rather than faking one.

### `provider`
- `LocationProvider`: `start / stop / pause / resume / poll / current_location
  / status`. Unlike the sketch in the specification, lifecycle calls take the
  current time as an argument: providers never read a clock, which is what
  makes runs reproducible and testable without sleeping.
- `SimulationProvider`: per due tick, model → assemble → validate → emit.
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
