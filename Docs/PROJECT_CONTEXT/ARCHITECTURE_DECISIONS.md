# Architecture decisions

Why the project is shaped the way it is. For *how* each module works, read
[../ARCHITECTURE.md](../ARCHITECTURE.md); this file records the decisions and
their reasons, and the alternatives that were turned down.

Each decision says where its rationale comes from:

- **(recorded)** — the reason is written in the code, a commit message or
  `Docs/ARCHITECTURE.md`.
- **(session)** — the reason was given during the work but is recorded in the
  repository only here. Treat as accurate but unverifiable from other sources.
- **(uncertain)** — the reason is a reconstruction. Do not build on it without
  checking.

## Layers and dependency direction

    rng → geographic → domain → route / scheduler → movement → noise
        → consistency → validation → provider

Stated in `crates/locsim-core/src/lib.rs`. A module may use modules to its
left. `consistency` uses only `domain` and `geographic`; `validation` uses
`consistency`; `provider` wires everything. Platform code, when it exists,
sits outside this crate and contains conversion and lifecycle only.

## D1. The core is a dependency-free Rust crate (recorded, session)

- **Decision.** All simulation logic lives in `crates/locsim-core`, with no
  dependencies and `#![forbid(unsafe_code)]`.
- **Why.** The specification requires the core to be testable on macOS/Linux
  without an iPhone and independent of the delivery mechanism. The machine
  the work started on has Rust but no Swift toolchain (session). Geodesy and
  the PRNG are in-crate so seeded output never changes with a dependency
  upgrade (recorded in `Cargo.toml` and `rng.rs`).
- **Rejected.** A Swift package for the core: could not be compiled or tested
  on the development machine (session). The `rand` crate: reproducibility
  across versions is not guaranteed (recorded). `proptest`: property tests
  use the in-crate seeded PRNG instead, to stay dependency-free; the cost is
  no automatic shrinking (session).
- **Consequence.** The repository layout is a Cargo workspace, not the
  `Sources/` + `Tests/` tree of the specification. Unit tests sit beside the
  code; property and pipeline tests are in `crates/locsim-core/tests/`.
- **Open for T08.** JSON needs either a justified dependency or a hand-written
  codec. See `CURRENT_STATE.md`.

## D2. Validated types instead of validated values (recorded)

- **Decision.** `Coordinate` has private fields and can only be constructed
  valid. `Route` likewise. `Timestamp` is integer nanoseconds.
- **Why.** A value that exists is valid, so "never emit NaN or out-of-range
  coordinates" is enforced by the type system rather than by remembering to
  check. Integer time makes `start + n × interval` exact and runs
  bit-reproducible.
- **Consequence.** Parameter structs (`MovementParameters`, `NoiseParameters`)
  deliberately have no `Default`: every value is an explicit choice, and
  `validate` reports every problem at once as `ConfigError { field, reason }`.
  `walking_preset()` / `driving_preset()` are named starting points, not
  defaults.

## D3. Geodesy: Vincenty on WGS84, errors instead of approximations (recorded)

- **Decision.** Vincenty inverse/direct with a *relative* convergence
  tolerance. Nearly antipodal pairs return `GeoError::NotConverged`.
- **Why.** Sub-millimetre accuracy for every pair the simulation produces. An
  absolute tolerance silently returns the first iterate for millimetre lines
  (found by test in T02). No scenario spans half the planet in one step, so an
  error is acceptable there; a silent approximation is not.
- **Rejected.** Haversine/spherical fallback (hides the failure). Karney's
  algorithm (noted as the upgrade path if antipodal support is ever needed).
- **Later additions.** `Geodesic::convergence_deg` (T05): the change of
  bearing along a line, by a half-angle formula, because subtracting the two
  bearings of a very short line is ill-conditioned. ECEF conversions and
  `local_axes` made public (T06). `COORDINATE_RESOLUTION_M` (T06, moved here
  in T07): a stored position is only good to about 2 nm, and everything
  derived from stored positions inherits that.

## D4. Time is passed in; nothing sleeps or reads a clock (recorded)

- **Decision.** `TickSchedule` is a pure state machine. `LocationProvider`
  methods take `now`. Samples are stamped with the tick's ideal time
  `start + n × interval`, never with the poll time.
- **Why.** A run is then a pure function of (scenario, start time, poll
  times), and sample *content* does not depend on poll times at all. Tests
  never sleep. Lateness cannot accumulate into drift.
- **Deviation from the specification.** The specification sketches
  `start()`, `pause()`, … without arguments; here they take the time.
- **Backpressure.** If polling falls behind, overdue ticks are skipped and
  counted, never replayed in a burst.
- **Not built.** A real-time driver that sleeps until `next_deadline()`. The
  "six-hour" tests are simulated time.

## D5. Models run on simulated time (recorded, T05)

- **Decision.** A movement model is sampled at `start + tick index × interval`,
  not at the wall timestamp on the sample.
- **Why.** Pausing shifts wall time but must not advance the simulated world;
  otherwise every mover leaps ahead by the pause duration on resume.

## D6. Movement models (recorded)

- **Decision.** `MovementModel::sample_at(t)`; `ModelFactory` lets a different
  engine be plugged into the provider. Circular motion and route replay are
  closed-form functions of elapsed time. Random walk, walking and driving are
  one `SteeredModel`: an inertial mover with exact integration of a linear
  speed ramp, heading-rate-limited turns along geodesics, and
  brake-for-the-fence boundary handling.
- **Why closed form where possible.** No accumulated state means no drift and
  identical output whatever was sampled before.
- **Why one steered model.** The physics (inertia, turn limits, fences) is the
  same; only the behaviour policy differs. Walking and driving differ only in
  parameters. This is stated plainly in the code: behaviour is a seeded
  policy, **not** a model of people or traffic.
- **Why brake for the fence rather than clip.** Clipping a position at a
  boundary is an instantaneous stop, which violates the deceleration limit.
  The mover keeps a braking distance in hand and steers back in.
- **Rejected.** Minimum turning radius / lateral-acceleration limit for
  driving: considered and left out as scope growth; a "driver" can turn on the
  spot at low speed (session). Sub-stepping within a sample: one step per
  sample was kept for simplicity, at the cost that the same seed at a
  different rate is a different trajectory (recorded as a limitation).
- **Margins, not tolerances.** Models stay `KINEMATIC_MARGIN` (1e-6 relative)
  below every limit, because the gate is strict and uses different arithmetic.

## D7. The displacement cap is a speed cap (recorded, T05)

- **Decision.** `max_displacement_per_sample_m` is defined per nominal update
  interval and acts as an extra speed limit `cap / update_interval`.
- **Why.** A hard per-sample cap conflicts with the deceleration limit when a
  tick is skipped: the mover would have to slow down instantly.

## D8. Noise is measurement error on top of true motion (recorded)

- **Decision.** The noise engine perturbs position (time-correlated jitter,
  constant-rate drift) and guarantees, by exact geodesic checks: offset from
  truth ≤ `max_position_offset_m`; emitted step ≤ true step +
  `max_offset_rate × dt`; inside the scenario boundary. If no position
  satisfies them it returns an error. With everything off it is an exact
  identity. Each component draws from its own forked random stream.
- **Why `max_offset_rate`.** A noisy fix moves even when the truth does not.
  Without a rate limit, jitter at high sample rates would imply impossible
  speeds.
- **Changed in T07.** The offset itself is now slew-limited to
  `max_offset_rate × dt` (previously only the emitted step was, which let the
  emitted track double back on a straight mover). The fence is the one
  exception; see D11.
- **No masking.** A base position outside the fence is an error, not
  something noise "repairs".
- **Leftover.** The engine still computes its own noisy speed and course.
  Since T07 they are not emitted. Removing them was avoided to keep T07 from
  redesigning the engine (session). A candidate for cleanup, with care for
  `noise_props`.

## D9. A route is input data, not an authority (recorded, T06)

- **Decision.** Three separate stages: structural validation
  (`domain::Route::new`), admission against the movement limits
  (`route::admit`), and the same final per-sample gate as everything else.
  Routes carry elapsed nanoseconds from their start, not absolute timestamps.
  Nothing is repaired.
- **Interpolation.** A cubic Hermite spline in ECEF through the recorded
  points, dropped onto the ellipsoid; velocity at each point shared by both
  adjoining segments; open routes start and end at rest.
- **Why a spline and not straight legs.** Straight legs have an instantaneous
  change of speed and heading at every point, which the strict gate rejects
  at any small sampling interval. Continuity of velocity is required for an
  admitted route to pass at *any* sampling interval.
- **Why ECEF.** One global Cartesian frame: no per-segment frames to stitch,
  no special cases at the antimeridian or poles. A per-segment tangent-plane
  spline was considered and dropped for the stitching discontinuities
  (session).
- **Why rest at the ends.** After the end the route holds its final point at
  rest; arriving at speed would be an instantaneous stop.
- **Why admission bounds suprema.** Sampling a segment can miss a peak. Speed,
  tangential acceleration and turn rate are polynomials or ratios of them, so
  they are bounded with Bernstein coefficients and bisection. This is what
  lets admission promise the gate will pass between samples too.
- **Rejected.** A path-following controller (output would depend on the
  sampling history, breaking "function of time alone"). Looping an open route
  (a teleport). Treating "nearly closed" as closed (silent repair).
- **Known cost.** A raw GPS log is usually not admissible: it does not start
  and end at rest and its jitter exceeds realistic limits. A smoothing or
  resampling step would be an explicit normalisation contract; none exists.

## D10. Position and timestamp are the source of truth (recorded, T07)

- **Decision.** Emitted speed and course are derived from consecutive emitted
  positions by backward difference: distance over elapsed time, and the
  arrival bearing of the geodesic. The model's own speed and course never
  reach the output. Position cannot be changed after noise.
- **Why backward difference.** It is causal; a central difference needs the
  next fix and would delay every sample by one interval.
- **Consequences that surprise.** Speed is an interval mean and depends on the
  sampling rate. The first sample of a run has no speed or course. A noisy
  fixed position reports the speed and direction of its jitter. Below 0.1 mm
  of movement there is a speed but no course; below 4 nm speed is exactly 0.
- **Accuracy.** Configured value plus clipped noise; never derived; never
  widened to explain a disagreement.
- **Observation noise.** `speed_noise_mps` and `heading_noise_deg` are kept as
  an observation model on the derived values (±3 σ) rather than dropped,
  because existing scenarios configure them (session).

## D11. The final gate is independent and strict (recorded)

- **Decision.** `SampleValidator` re-derives everything from the emitted
  samples with exact geodesics and shares no state with the engines. A
  rejected sample is never emitted and never becomes a reference.
  Comparisons are strict; the only amounts added to limits are physical or
  stated resolutions.
- **Why.** Defence in depth: a bug in a model, the noise engine or the
  consistency engine must not be able to leak out. Mutation checks in every
  ticket since T04 confirm the gate catches deliberately broken engines.
- **Restated in T07.** When speed became an interval mean and course a chord
  direction, the acceleration stage moved to the time between interval
  midpoints `(dt + dt′)/2` and the heading stage to both intervals
  `dt + dt′`. The first equals the old `a·dt` for even sampling; the second
  is up to twice as permissive as the old `ω·dt`. This was a deliberate
  restatement for the new meaning of the quantities, recorded in commit
  `afeb26c`, and is the one place where a later ticket made a gate stage more
  permissive than it had been. Do not relax it further without the same level
  of justification.
- **Fence exception.** A fix that position noise had to hold at the fence has
  an unbounded noise step; the speed-change and heading stages skip it.
- **Turning is measured on the surface.** The previous course is carried
  along the geodesic before comparing, so straight travel near a pole is not
  mistaken for turning.

## D12. Explicit state machine (recorded, T01/T03)

- **Decision.** `SimulationState::transition` is the only way to change state
  and rejects illegal moves. Every active state can reach `Stopping`.
  A configuration that fails validation leaves the provider `Idle`. A failed
  sample moves it to `Error` and it stays there.
- **Not built.** Automatic recovery (`Recovering`), health states, watchdog:
  T10. The enum variants exist; nothing drives them yet.

## D13. Future platform adapter (planned; uncertain where noted)

- **Decision (planned).** A C ABI crate (`locsim-ffi`) exposes the core; a
  Swift adapter converts `SyntheticLocation` to `CLLocation` (`None` speed or
  course become Core Location's `-1`). No simulation mathematics on the
  platform side.
- **Uncertain.** Whether a C ABI is the best boundary versus, for example,
  UniFFI or swift-bridge has not been evaluated. Whether the core builds for
  `aarch64-apple-ios` has not been tried. How delivery on a jailbroken
  research device would work has deliberately not been designed, because no
  hardware has been available to verify anything.
- **Fixed regardless.** The scope boundaries in `PROJECT_BRIEF.md`.

## Decisions a future ticket will have to make

- T08: dependency policy for JSON; where the codec lives; field naming versus
  the specification's example; what a schema migration is when there is only
  version 1.
- T09: what "last known simulation state" means for a provider that is a
  pure function of time.
- T10: whether recovery re-creates the run or resumes it, and what the
  deriver and validator state should be afterwards.
- T11: everything in D13.
