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
`consistency`; `provider` wires everything. `locsim-scenario` is a second
crate on top of the core (D14), `locsim-store` a third on top of both
(D16), and `locsim-health` a fourth on top of the core alone (D17). Platform code, when it exists,
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
- **Settled in T08.** JSON lives in a separate crate with justified
  dependencies; the core is untouched. See D14.

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
- **Why.** A run is then a deterministic function of the scenario and the
  sequence of `start` / `poll` / `pause` / `resume` times. Tests never
  sleep. Lateness cannot accumulate into drift.
- **Corrected in T09.** This entry used to say that sample content "does not
  depend on poll times at all". That holds only while no tick is skipped,
  which is what `tests/fixed_pipeline.rs::punctual_polling_makes_content_independent_of_poll_jitter`
  asserts. When a late poll skips ticks, steered trajectories, noise and
  derived speed all change (one longer step, a longer decorrelation gap, a
  longer interval). And a run is not a function of scenario and time alone:
  it carries state. See `Docs/ARCHITECTURE.md`, `provider`, "State".
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
- **Since T10.** The provider still never leaves `Error` by itself. The
  supervisor in `locsim-health` reports `Recovering` while a restart is
  pending and uses `HealthState`; see D17.

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

## D14. Scenario JSON is a separate crate on `serde_json` (recorded, T08)

- **Decision (project owner, 2026-10-10).** A new crate `locsim-scenario`
  depends on `locsim-core`, `serde` and `serde_json` with `float_roundtrip`.
  `locsim-core` keeps no dependencies.
- **Why a separate crate.** D1 stays true where it matters: nothing that
  produces the sample stream depends on third-party code, and the core that
  T11 will expose over an FFI boundary carries no text parsing.
- **Why `serde_json` rather than a hand-written reader.** Reading JSON is
  correctness- and safety-sensitive text handling (escapes, surrogates, the
  number grammar, recursion limits). A widely used, fuzzed implementation
  was preferred. A hand-written codec in the new crate was the alternative
  offered; the owner chose the dependency.
- **Why `float_roundtrip`.** Without it `serde_json` does not always parse a
  float to the nearest double. Verified: with the feature removed, the
  random-float round-trip tests fail.
- **Why no derive, and our own tree.** `serde` is used only for the traits
  `serde_json` drives. The schema is decoded by hand from the crate's own
  ordered `Json` tree because (a) `serde_json::Value` silently keeps the last
  of two duplicate keys, (b) a derived decoder stops at the first error
  while C5 asks for all of them with paths, (c) the default serializer
  writes NaN as `null`. Without derive no proc-macro crate is compiled:
  the build adds `serde`, `serde_core`, `serde_json`, `itoa`, `memchr`,
  `zmij`.
- **Cost.** The project now has a `Cargo.lock` that matters. A dependency
  upgrade is a reviewed change, checked by the float and round-trip tests.

## D15. The scenario document: shape, strictness, seed, versions (recorded, T08)

- **Shape.** The document mirrors the domain model, with the Rust field names
  and their units as member names. Paths in decode errors are then the paths
  `Scenario::validate` already uses. The specification's short example
  (`speed.min`, `jitter.radius`, `updateInterval`) is *not* the schema: the
  domain model has many more required values and, by D2, no defaults. It is
  refused, and a test pins the refusal.
- **Everything required, explicit `null`.** An optional value that is absent
  is written `null`. A missing member is an error. This keeps "nothing was
  defaulted" checkable by reading the file.
- **Seed as a canonical decimal string (project owner).** A `u64` above 2^53
  is damaged by any tool that reads JSON numbers as doubles; a string is
  safe everywhere. Only the canonical spelling is accepted so that one seed
  has one text. **Rejected:** a JSON number (the specification's example
  writes one; exact in our reader, lossy elsewhere).
- **Integers are integers.** `schema_version` and `elapsed_ns` refuse `1.0`
  and `1e9`. Accepting them would be rounding by another name.
- **Canonical output.** Fixed member order, shortest round-trip floats, two
  spaces, trailing newline. Input may use any order and any valid number
  spelling. So files can be compared byte for byte, and the examples are
  kept identical to their own export by a test.
- **Stages stop early.** A later stage is not run on a document an earlier
  stage refused: a structurally wrong document has no `Scenario` to validate.
  Within a stage everything is reported.
- **Migration on the tree, before typing.** A step is a function on the
  document tree from one version to the next; the chain advances the version
  and the strict decoder reads the result, so a step that leaves something
  behind is caught. **Rejected:** keeping old Rust types per version (heavy,
  and nothing to keep yet); a tolerant decoder that accepts several shapes
  (silent repair). **Not done on purpose:** no historical schema was invented
  to give the production chain a step. The mechanism is tested with chains
  that exist only in tests, including a pretend "version 0" that production
  refuses.
- **Import does not admit routes.** Admission stays where D9 put it, at the
  start of a run, against the limits in force then.
- **Uncertain.** Whether errors from `Scenario::validate` should carry line
  and column. They do not; only syntax errors do.

## D16. Persistence: what is stored, where, and how (recorded, T09)

Decided by the project owner on 2026-10-10 after a design review; the two
technical details (integrity, replacement) were resolved before coding.

- **Scope: A and B, not C.** (A) the scenario and (B) the last emitted
  sample with the provider's counters are stored. (C) checkpoint/resume is
  deferred to a separate ticket nobody owns. **Why.** A run holds state the
  core does not expose (D4, corrected): random streams, mover state, noise
  filters, the deriver's and the gate's history. A scenario and a last
  sample are not enough to continue any mode but a noise-free fixed one.
  Exposing that state is a change to the core with its own contract.
  "Configuration" and "preferences" from the ticket text are not stored:
  no such type exists; the application ticket can use the same store.
- **Two ways C could be done later** (not chosen): snapshot/restore in the
  engines; or logging the call sequence and replaying it (about 1.2 µs per
  sample).
- **Layout.** A new crate `locsim-store` owns files and has no third-party
  dependency. The text form of the last-known record lives in
  `locsim-scenario` beside the other codec. **Rejected:** file I/O inside
  `locsim-scenario` (C19 says it opens no file); a second JSON decoder in
  the store (duplicates or loses the strict tree); publishing the `json`
  module (internals as API).
- **Integrity: an envelope with SHA-256 over the exact payload.** T08's
  damage test had shown that a changed character can decode as a different
  valid scenario; the owner asked for this to be closed, not accepted.
  A line header, then the payload unescaped, so the canonical document is
  the payload byte for byte. **Rejected:** JSON inside JSON (the document
  as an escaped string is unreadable; as a nested object the "exact bytes"
  depend on re-serialisation); CRC-32 (misses one random corruption in 2³²
  and loses its burst guarantee on megabyte routes); a `sha2` dependency
  (six crates for ninety lines). The digest is for accidental damage only.
- **Fingerprint is a different thing.** FNV-1a 64 of the exported text
  associates a record with its scenario. Not cryptographic, not integrity.
- **Replacement: standard library only.** Temporary file, sync, read back,
  rename, directory sync on Unix. **What was verified:** Rust 1.98.1 renames
  on Windows with `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`, falling back to
  a POSIX-semantics rename on access denied; Microsoft's documentation does
  not call the replacement atomic; the standard library neither passes
  `MOVEFILE_WRITE_THROUGH` nor can sync a directory. So on Windows the
  rename is not confirmed durable, and that is stated, not hidden.
  **Not chosen:** an FFI call with write-through (`unsafe`, which every
  crate here forbids) or a `windows-sys` dependency for one flag. Either is
  a contained follow-up if the guarantee is wanted.
- **No backup generation, no lock file, no automatic cleanup.** A failed
  save already keeps the old record; a backup would only guard against a
  successful save of something unwanted. A lock goes stale after a crash.
  Temporary files may belong to a live writer, so they are removed only on
  request.
- **Every save is synced.** The caller chooses how often to save.
- **Loading never writes.** Missing is `None`; corrupt is an error; nothing
  becomes a default.

## D17. Health is a supervisor outside the core; recovery is a new run (recorded, T10)

Decided by the project owner on 2026-10-10 after a proposal, a revision and
an addendum, all before any code.

- **A new crate, depending on the core only.** `locsim-health` wraps a
  `LocationProvider`. **Why.** The engines cannot be altered by something
  that cannot reach inside them; the pattern is that of D14 and D16; and it
  works for any future provider. **Rejected:** a module in the core (the
  core has been unchanged since T07, and health needs nothing private).
- **The supervisor is a `LocationProvider`.** It drops in wherever a
  provider goes. A failing `poll` returns the provider's error once, then
  `Ok(None)`; the reason is in the report. **Rejected:** a separate result
  type for `poll` (every consumer would need two code paths).
- **Recovery is a new run, bounded, with nothing altered.** T09 established
  that a run cannot be resumed (D16; D4 as corrected). So the only possible
  recovery is to start the scenario again. Deterministic failures are
  retried up to the bound and fail each time. **Rejected:** reseeding or
  adjusting the scenario so that a retry differs (rule 11: no component
  silently modifies another's configuration); a detector for "the same
  failure at the same tick" (more machinery than the bound it would save).
- **Three fields, not one.** Lifecycle, observed emission and a derived
  health word. The first draft let "degraded" cover a silent provider
  without saying it was silent, and let "stopped" and "failed" blur; the
  review asked for the distinction.
- **A refused start is not a failure.** It leaves the supervisor stopped,
  mirroring contract C4. An earlier draft made it `Failed`.
- **Only one error is permanent.** `ScheduleError::Overflow`, because a
  later start provably cannot cure it. An earlier draft also listed
  `Transition`; the real provider never returns it from `poll`, and there
  was no evidence either way for another provider, so it is retried.
- **Timestamps are enforced, not assumed.** The supervisor is generic, so
  the scheduler argument that covers `SimulationProvider` is backed by a
  check that covers everything. An earlier draft claimed the first sample
  after a restart carries the restart time; it carries its tick's time,
  which is later if the first poll is late.
- **Backoff as a recurrence with constant cost.** One multiplication per
  attempt, held per episode. **Rejected:** a power function (not guaranteed
  identical across platforms); a loop from the first attempt (cost grows
  with the attempt number and, with a multiplier just above 1, without
  practical bound); jitter (needs randomness and serves contention between
  many clients, of which there are none).
- **Watchdog by call, not by timer.** Consistent with D4. The cost is
  stated everywhere it matters: nothing is noticed unless something calls.
- **Windows of tick slots that do not overlap**, tripping at once and
  clearing only at a clean close: simple to state exactly and to test at its
  boundaries. **Rejected:** a sliding window (needs a history buffer, and
  its boundary cases are harder to state).
- **Other components report; health does not look.** A generic fault hook.
  **Rejected:** a dependency on `locsim-store` (wrong direction), or a typed
  list of components (the crate would have to know them).
- **Events are structured, to a sink, with no positions.** No logging
  dependency, no destination chosen here. Totals are updated from events in
  one function so that the two cannot drift.
- **Call legality from the supervisor's own lifecycle.** The design said it
  would call the core's transition function; the implementation does not. A
  test checks that the two agree in every state.

## Decisions a future ticket will have to make

- T11: everything in D13; whether to split the ticket so that the Rust side
  can be done without a Mac; where the one exception to `forbid(unsafe_code)`
  lives; who calls `poll` and `check` and with which clock; how the run
  number reaches the platform as a discontinuity.
- Unowned: checkpoint/resume (D16); `MOVEFILE_WRITE_THROUGH` on Windows; a
  timer or driver; a bridge from `StoreError` to fault reports.
