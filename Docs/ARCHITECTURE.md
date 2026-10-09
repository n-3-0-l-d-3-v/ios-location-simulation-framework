# Architecture

How each implemented module works. Why it was built this way is in
[PROJECT_CONTEXT/ARCHITECTURE_DECISIONS.md](PROJECT_CONTEXT/ARCHITECTURE_DECISIONS.md);
what must not be broken is in
[PROJECT_CONTEXT/ENGINEERING_CONTRACTS.md](PROJECT_CONTEXT/ENGINEERING_CONTRACTS.md);
the index of all context documents is
[PROJECT_CONTEXT/README.md](PROJECT_CONTEXT/README.md).

## Layers

    Control / Configuration        (T12 app, CLI)
    Simulation Manager             state machine owner, wires everything
    Scenario Engine                serialisable scenarios, versioning
    Movement Engine                fixed, random walk, walking, driving, orbit, route replay [implemented]
    Route Engine                   route -> trajectory, admission  [implemented]
    Noise / Realism Engine         seeded jitter + drift, bounded  [implemented]
    Consistency Engine             speed/course from emitted fixes [implemented]
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

### Pipeline order

    movement model          simulated time -> true position
        |
    noise engine            -> final emitted position (+ accuracies)
        |                      nothing changes a position after this point
    consistency engine      emitted position + timestamp, previous emitted fix
        |                      -> speed, course
    final validation gate   independent re-derivation; reject or pass
        |
    emit

The **emitted position and its timestamp are the authoritative observation**.
Speed and course are derived from them; the movement model's own speed and
course and the noise engine's noisy copies of them never reach the output.

### `consistency`
Makes the reported kinematics describe the emitted trajectory.

**Scheme.** One finite difference everywhere: the backward difference over the
interval ending at the sample, using the geodesic inverse.

| Quantity | Definition |
|---|---|
| speed | geodesic distance(previous emitted fix, this fix) / (this timestamp - previous timestamp) |
| course | bearing, at this fix, of the geodesic from the previous emitted fix (direction of travel on arrival) |

It is causal (a central difference would delay every sample by one interval),
has no free parameter, and uses the provider's actual timestamps: no sampling
rate is assumed. Speed is therefore the *mean ground speed over the interval*,
not an instantaneous value; the same trajectory sampled at different rates
reports different interval means (an orbit reports `2 r sin(w dt/2)/dt`).
Timestamps are strictly increasing integer nanoseconds, so the divisor is never
zero.

**First sample.** No predecessor: speed and course are `None`, not zero.

**Stationary semantics**, two fixed thresholds on the displacement `d`:

| Displacement | Speed | Course |
|---|---|---|
| `d < 4 nm` (`STATIONARY_DISPLACEMENT_M`, twice coordinate resolution) | exactly 0 | none |
| `4 nm <= d < 0.1 mm` (`MIN_COURSE_DISPLACEMENT_M`) | `d / dt` | none: the direction of so short a line is mostly rounding (0.0023 deg at the threshold, growing as `1/d`) |
| `d >= 0.1 mm` | `d / dt` | geodesic arrival bearing |

A course is never carried over from an earlier sample; after a pause the first
course is the new direction of travel.

**Resolution.** A coordinate is stored to about 2 nm
(`geographic::COORDINATE_RESOLUTION_M`), so a distance between two fixes is
uncertain by up to 4 nm. Derived speed has a resolution of `4 nm / dt` (4e-9
m/s at 1 Hz, 4e-6 m/s at 1 kHz, 4 mm/s at 1 MHz) and derived course of
`4 nm / d` radians. These are properties of the observation; the gate adds
exactly these amounts where it compares derived quantities with physical
limits.

**Noise.** Position noise is applied before derivation, so speed and course
include it: a fixed position with jitter reports the speed and direction of
the jitter, because that is what the emitted fix is doing. `speed_noise_mps`
and `heading_noise_deg` are an observation model on top:
`reported = derived + n`, with `n` clipped at 3 sigma. The contract is then
`|reported - derived| <= 3 sigma`. A stationary sample gets no speed noise and
a sample without a course gets no heading noise.

**Accuracy.** Not kinematic and not derived. Reported horizontal and vertical
accuracy are the scenario's configured values plus the noise engine's clipped
accuracy noise, nothing else; the consistency check rejects anything outside
that band. Accuracy is never widened to cover a disagreement between position
and speed. There is no model here from which a statistically meaningful
accuracy could be derived, so none is invented.

**`KinematicsDeriver`.** State is one previous fix (timestamp and coordinate).
`derive` is pure: one geodesic inverse, no allocation. `accept` is called by
the provider only after a sample has passed the gate, so a rejected sample
never becomes the reference. Nothing else is cached, so nothing can go stale.

**Independent check.** `check_first` / `check_pair` recompute the expected
values from two samples and reject wrong, missing or non-finite speed; wrong,
stale or missing course; a course without movement; speed on a stationary fix;
kinematics on a first sample; a timestamp that does not match the reported
speed; accuracy outside its band. It trusts no stored result of the deriver.

Cost, release build, per sample: derivation about 270 ns of a 1.2 us pipeline
(23 %); the gate, which recomputes the same geodesic on purpose, about 310 ns.

### `validation`
`SampleValidator` is the last step before emission; nothing modifies a sample
after it. It shares no state with the movement, noise or consistency engines
and recomputes every quantity from the emitted samples with exact geodesics.

| Stage | Rejects |
|---|---|
| Field validity | NaN/infinite values, out-of-range coordinates, negative accuracy or speed, course outside `[0, 360)`, course without motion |
| Timestamp | not strictly after the previous accepted sample |
| Boundary | position outside `movement.radius_m` of the origin (not in circular mode) |
| Displacement | moved more than `(effective max speed + noise.max_offset_rate) x dt` — teleportation |
| Consistency | speed or course that contradict the positions and timestamps (see above) |
| Speed | reported speed above `effective max speed + noise.max_offset_rate` (+ 3 sigma of speed noise) |
| Acceleration / deceleration | interval-mean speed changed by more than the limit allows |
| Heading rate | chord direction turned by more than the limit allows |

The kinematic stages are stated for what speed and course are — an interval
mean and a chord direction. With `dt` the interval ending at the sample and
`dt'` the one before:

- Two consecutive interval means of a speed whose rate of change is at most `a`
  differ by at most `a (dt + dt')/2`, the time between the interval midpoints.
  For evenly spaced samples that is `a dt`; for uneven ones `a dt` would be
  wrong in both directions.
- A chord's direction lies within the directions travelled during its
  interval, so consecutive chords differ by at most the turning possible over
  both intervals, `w (dt + dt')`. (For even sampling this bound is up to twice
  the old `w dt`; the tighter `w (dt + dt')/2` holds only at constant speed.)

Comparisons are strict. Every amount added to a limit is physical or a stated
resolution, derived from configuration:

| Allowance | Amount | Why |
|---|---|---|
| Chord shortfall | `v_max (1 - cos(w dt / 2))` on speed change | a path that turns within an interval has a shorter chord without braking |
| Position noise | `2 r` on speed change; `asin(r dt / (d - r dt))` per chord on heading, unbounded once the noise step is half the chord | noise moving at up to `r` shifts each end of a chord |
| Observation noise | `6 sigma` between two readings | each is within 3 sigma of the derived value |
| Coordinate resolution | `4 nm / dt` per speed, `4 nm / d` rad per course | fixes are stored to ~2 nm |

A fix that position noise had to hold at the scenario fence has a noise step
that is not bounded by `r dt` (see `noise`); the speed-change and heading
stages are skipped for the two comparisons it takes part in.

**Turning is measured on the surface.** The previous course is carried along
the geodesic to the new fix (`Geodesic::convergence_deg` added) and only the
remainder counts as a turn, so straight travel near a pole is not mistaken for
turning.

Mutation evidence that the gate is independent: with the noise engine's limits
loosened; with the steered model's acceleration limit, turn cap, fence or
convergence handling broken; with route peaks under-reported; and with the
deriver reporting 0.1 % too much speed, the departure bearing, or a carried-over
course — the pipeline tests fail because the gate rejects the stream.

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
| RouteReplay | `RouteModel` | closed form in time over an admitted route (see `route`) |

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

### `route`
A recorded route is **input data, not an authority**. Three separate things
happen to it, in three places, and none stands in for another:

| | Where | Question | Failure |
|---|---|---|---|
| 1. Structural validation | `domain::Route::new` | Is this a recording at all? | `RouteError` naming the point |
| 2. Admission | `route::admit`, `RoutePlan::violations` | Does the trajectory it describes respect the movement limits everywhere? | `RouteRejection` with one `RouteViolation` per broken limit per segment |
| 3. Final per-sample validation | `validation::SampleValidator` | Is this emitted sample (after noise) valid? | `ValidationError`, provider enters `Error` |

Stage 3 is the same gate every movement model passes through. It knows nothing
about routes or admission and is not weakened for them; corrupted route output
is rejected by the gate alone (tested without provider, noise or admission in
the loop).

**Route data model** (`domain::Route`). Ordered `RoutePoint`s, each with a
coordinate, an optional altitude and `elapsed_ns` — *elapsed time from the
start of the route in integer nanoseconds*, first point at zero. Not absolute
timestamps: a route has no date, replay places it wherever the run starts, and
that is what makes replay reproducible. `Route::from_absolute` is the one
documented normalisation (subtract the first timestamp). Rejected, never
repaired: fewer than two points, first point not at zero, any point not
strictly later than its predecessor (out of order or zero duration),
non-finite altitude, altitude on some points but not others. Invalid
coordinates cannot be expressed (`Coordinate` is validated). A route may carry
a name. `Route::legs()` derives distance, duration, mean speed and bearings of
each leg from the geographic engine.

**Interpolation.** Recorded points are joined by a cubic Hermite spline in
Earth-centred Cartesian (ECEF) coordinates, evaluated at the requested time and
dropped onto the ellipsoid with `ecef_to_geodetic`. Latitude and longitude are
never interpolated, so the antimeridian and the poles need no special cases.
- The trajectory passes exactly through every recorded coordinate at exactly
  its recorded time.
- The velocity at a recorded point is the three-point estimate from its
  neighbours (each neighbouring leg's mean velocity weighted by the other
  leg's duration), kept tangent to the ground, and shared by both adjoining
  segments — so position, speed and course are continuous everywhere.
- An open route starts from rest and comes to rest. Its first and last legs
  are an acceleration and a braking leg; a recording that begins or ends at
  speed will normally break the acceleration limits there and be rejected.
- Two consecutive points with the same coordinate are a wait: at rest for
  that leg, arriving and leaving at rest.
- **Speed and course are derived, not recorded**: the magnitude and true
  bearing of the interpolant's velocity at the interpolated point. A route has
  no recorded speed or course fields.
- Altitude is interpolated linearly in time, or is the scenario altitude.

The trajectory is a function of played time alone. A different sample interval
reads different instants of the same curve (the 20 Hz and 1 Hz streams of one
route agree exactly wherever both sample).

**Playback.** Playback speed divides every recorded time (rounded to the
nanosecond — that rounded timeline *is* the played route); reverse playback
reads the route from last point to first. Both are applied before
interpolation, so the result is admitted like any other route. Violations
always name the route's own leg index.

**Time boundaries.**

| Instant | Result |
|---|---|
| before the start | first point, at rest |
| exactly at the start | first point, at rest (open route) |
| between recorded points | the interpolant |
| exactly at a recorded time | exactly the recorded coordinate |
| exactly at the end | final point, at rest, `complete` |
| after the end | final point held, at rest, `complete` — no extrapolation |

On completion the provider keeps running and keeps emitting the held point;
`ProviderStatus::trajectory_complete` reports it.

**Looping.** Only a closed route (last coordinate exactly equal to the first)
may loop. The velocity is then carried across the seam like at any other point
and the seam is admitted like any other point. An open route with looping is
rejected (`playback.looping` in scenario validation, `OpenRouteCannotLoop` in
the engine): it would jump back to its start. A closed route without looping is
an ordinary open route. "Nearly closed" is open; closing it would be repairing
the input.

**Admission.** For each segment the engine derives peak speed, peak
acceleration, peak deceleration, peak heading rate, entry/exit bearings and the
farthest distance from the boundary centre, then compares:

| Constraint | Observed | Limit |
|---|---|---|
| `MaxSpeed` | peak speed | `max_speed` |
| `DisplacementPerSample` | peak speed x update interval | `max_displacement_per_sample` |
| `Acceleration` / `Deceleration` | peak rate of change of speed | `max_acceleration` / `max_deceleration` |
| `HeadingRate` | peak turn rate while moving | `max_heading_rate` |
| `TurnAtStop` | change of direction across a stop | `max_heading_rate` x time stopped |
| `Boundary` | farthest point of the curve | `movement.radius_m` |

`min_speed` is not enforced on a route: it bounds the cruising target of the
generated models, and a recording may legitimately be slow or stopped.

These are suprema over each whole segment, not samples. Along a segment speed
squared, `V.A` and `|V x A|^2` are polynomials in the segment parameter, so
speed, tangential acceleration `V.A/|V|` and turn rate `|V x A|/|V|^2` are
bounded rigorously: a polynomial lies within the hull of its Bernstein
coefficients, and branch-and-bound bisection closes the gap to an attained
value to 1e-7. Where the trajectory starts or ends at rest the vanishing
factor is cancelled analytically first. An admitted route therefore stays
within its limits between samples at any sampling interval, which is what lets
it pass the strict gate. A reversal through zero speed *between* recorded
points has an unbounded turn rate and is never admitted; turning round needs a
wait long enough to turn in.

Margins, all quantified:
- `KINEMATIC_MARGIN` (1e-6 relative) below every limit, as for generated models.
- Ground speed can exceed the spline's speed because the curve runs slightly
  below the surface: factor `1 + (L/R)^2` per segment (2.5e-8 at 1 km). Legs
  longer than 100 km are rejected.
- A fix is stored to about 2 nm (`COORDINATE_RESOLUTION_M`), so the gate may
  measure up to 4 nm more distance between two fixes than was travelled. A
  generated model shortens its step to compensate; a route cannot move its
  points. Admission keeps speed `2 x resolution / update interval` below the
  limit (4e-7 m/s at 100 Hz) and turn rate `1e-9 deg / update interval` below
  its limit. The guarantee holds at the scenario interval or any longer one.

### `noise`
Position of the engine in the pipeline:

    base movement → noise → assemble metadata → final validation → emit
    (a replayed route is one more base movement)

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

Since T07 the engine's own noisy speed and course are not emitted; its speed
and heading draws are passed on as observation noise on the kinematics derived
from the emitted positions.

Hard guarantees, each verified with exact geodesic distances before a
position is returned:
1. offset from the true position ≤ `max_position_offset_m`;
2. displacement from the previous output ≤ true displacement +
   `max_offset_rate_mps × dt`;
3. inside the scenario boundary (`movement.radius_m` around the origin).

Beyond these the offset itself is slew-limited: it changes by at most
`max_offset_rate_mps x dt` between samples, so the emitted track keeps the
direction and speed of the true one to within the noise step. (Until T07 only
the emitted step was limited, which let the offset swing by twice the true
displacement and the emitted track double back on a mover going straight.)
Only the boundary overrides the slew limit: an output that would leave the
fence is pulled back onto it however far the offset must change.

How they are met: the candidate is projected exactly onto the intersection of
the offset disc and the slew disc in a local tangent plane (re-anchored every
1 km), then pulled inside the boundary along a geodesic. If the exact check still
fails, fallbacks are tried in order — a point on the geodesic from the previous
output to the true position (feasible by the triangle inequality), holding the
previous output, the true position — and if none passes the engine returns
`ConstraintUnsatisfiable` rather than a position. `fallback_count()` exposes how
often this happens (0 in 150 000 random samples).

Design decisions worth knowing:
- **What "maximum speed" means with noise.** A noisy fix necessarily moves
  between samples even when the true position does not. The distance between
  consecutive outputs is bounded by `(max_speed + max_offset_rate) × dt`, and
  since T07 the reported speed is that distance over the elapsed time, so it
  shares the bound. `max_offset_rate_mps` is therefore the knob that makes
  jitter physically plausible at any sample rate.
- **Metadata (superseded in T07).** From T04 to T06 the reported speed and
  course were those of the underlying motion plus bounded measurement noise,
  not re-derived from the jittered fixes. Since T07 they are derived from the
  emitted (noisy) positions — see `consistency` — and the engine's own noisy
  speed and course are computed but no longer emitted. Its speed and heading
  draws survive as observation noise on the derived values.
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
- `SimulationProvider`: per due tick, model → noise → derive speed and course
  from emitted positions → validate → emit (see "Pipeline order").
  Models are sampled at *simulated* time `start + tick index × interval`,
  which stands still while paused, so a mover resumes where it was instead
  of leaping ahead; samples are stamped with wall time.
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
