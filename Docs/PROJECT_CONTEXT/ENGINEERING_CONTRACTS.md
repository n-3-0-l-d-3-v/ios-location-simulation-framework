# Engineering contracts

The invariants future tickets must preserve. Each names where it is enforced
and which tests would fail if it broke. Paths are relative to
`crates/locsim-core/`. Test references are `file::test_function`.

Paths in C19 are relative to `crates/locsim-scenario/`, in C20 to
`crates/locsim-store/`, in C21 to `crates/locsim-health/`, in C22 to
`crates/locsim-ffi/`.

References in C1–C18 were checked to exist at commit `1be8852` (2026-10-09);
T08, T09 and T10 changed no file of `locsim-core`. References in C19 and C20
were checked at `29c8342`, those in C21 at `fcda25c` (2026-10-10), those in
C22 at `e7fad44` (2026-10-11). T11A changed no behaviour of the four older
crates; its documentation stage reworded one doc comment in `locsim-core`
(`src/domain/time.rs`). If a reference no longer resolves, the code moved:
find the new home before concluding the contract is gone.

**If a change makes one of these tests fail, the default assumption is that
the change is wrong, not the test.** A contract may be changed deliberately —
T07 did so for the gate's kinematic stages — but only with the reason written
in the commit message, the architecture document and `PROGRESS.md`.

---

## C1. Determinism

Same scenario, seed, start time and poll times ⇒ bit-identical output. No
wall-clock randomness anywhere. Each random consumer has its own forked
stream, so enabling one never changes another's sequence.

- Enforced in: `src/rng.rs`; stream forks in `src/noise/mod.rs` and
  `src/movement/steered.rs`.
- Tests: `src/rng.rs::same_seed_same_sequence`,
  `src/noise/mod.rs::components_use_independent_streams`,
  `tests/fixed_pipeline.rs::identical_inputs_reproduce_the_identical_stream`,
  `tests/moving_pipeline.rs::identical_inputs_reproduce_and_seeds_diverge`,
  `tests/consistency_pipeline.rs::identical_inputs_produce_identical_metadata`.
- Do not: add a dependency that supplies randomness; call a clock inside the
  core; iterate a `HashMap` where order reaches the output.

## C2. Time is passed in; scheduling does not drift

Nothing in the core sleeps or reads a clock. Tick `n` is due at
`start + n × interval`, computed from the origin every time. Samples carry
the tick's ideal time, not the poll time. Overdue ticks are skipped and
counted, never replayed. Timestamps are integer nanoseconds.

**The time scale is the caller's** (decided 2026-10-11). A timestamp is a
count of nanoseconds whose origin is arbitrary: the code compares
timestamps and subtracts them and never reads their absolute value.
Timestamps that come out (samples, deadlines, reports, events) are on the
same scale as those that went in. A caller should not give a session a time
earlier than one it has already given it; a time that cannot be represented
is never produced. Nobody is required to use the Unix epoch. `SystemClock`
does, by reading it once and advancing it with a monotonic timer; that is
its implementation, not a rule for callers.

- Enforced in: `src/scheduler/schedule.rs`, `src/domain/time.rs`,
  `src/provider/simulation.rs`.
- Tests: `src/scheduler/schedule.rs::late_polls_do_not_accumulate_drift`,
  `src/scheduler/schedule.rs::overdue_slots_are_skipped_and_counted_not_replayed`,
  `src/scheduler/schedule.rs::pause_preserves_phase_and_counts_nothing_as_missed`,
  `tests/fixed_pipeline.rs::six_simulated_hours_at_10_hz_stay_exact`. For
  the time scale: `crates/locsim-ffi/src/ops/tests.rs::moving_every_time_by_a_constant_moves_the_timestamps_and_nothing_else`.

## C3. Models run on simulated time

A movement model is sampled at `start + tick index × interval`. Pausing does
not advance it.

- Enforced in: `src/provider/simulation.rs` (the `simulated` computation in
  `generate`).
- Tests: `src/provider/simulation.rs::simulated_time_stands_still_while_paused`,
  `tests/moving_pipeline.rs::a_long_pause_does_not_make_a_walker_leap`.

## C4. Explicit lifecycle; fail safe

State changes only through `SimulationState::transition`. An invalid
configuration is rejected at `start` and leaves the provider `Idle`. A sample
that cannot be produced or fails validation is not emitted; the provider
enters `Error` and emits nothing further. `stop` works from every active
state. The provider never leaves `Error` by itself; restarting is done from
outside it (C21) and is a new run.

- Enforced in: `src/domain/state.rs`, `src/provider/simulation.rs`.
- Tests: `src/domain/state.rs::transition_table_is_exactly_as_specified`,
  `src/provider/simulation.rs::invalid_scenario_is_rejected_and_provider_stays_idle`,
  `src/provider/simulation.rs::invalid_sample_is_never_emitted_and_enters_error`,
  `src/provider/simulation.rs::illegal_lifecycle_calls_are_rejected_without_side_effects`.

## C5. Structured errors, never silent repair

Invalid input is rejected with an error that says what and where. Nothing is
clamped, sorted, defaulted or dropped to make input acceptable, unless that
is a documented normalisation (`Route::from_absolute` subtracts the first
timestamp, and that is all). Scenario validation reports every problem, not
just the first. There are no `Default` impls on parameter structs.

- Enforced in: `src/domain/scenario.rs`, `src/domain/params.rs`,
  `src/domain/route.rs`, `src/route/admission.rs`.
- Tests: `src/domain/scenario.rs::reports_all_top_level_errors_together`,
  `src/domain/route.rs::rejects_out_of_order_and_zero_duration_segments_with_the_index`,
  `src/domain/route.rs::absolute_timestamps_are_shifted_to_the_first_and_nothing_else`,
  `src/route/tests.rs::each_kinematic_limit_is_detected_independently`.
- Scenario documents apply this directly; see C19.

## C6. Geography goes through the geographic engine

No arithmetic on latitude/longitude outside `src/geographic/`. Distances,
bearings and destinations are geodesic on WGS84. A `Coordinate` that exists
is valid. A geodesic that cannot be computed is an error, not an
approximation.

- Enforced in: `src/geographic/` (private fields on `Coordinate`).
- Tests: `src/geographic/geodesic.rs::inverse_matches_published_line`,
  `src/geographic/geodesic.rs::antipodal_reports_non_convergence`,
  `src/geographic/geodesic.rs::convergence_stays_accurate_for_sub_millimetre_lines_at_high_latitude`,
  `tests/geographic_props.rs::direct_then_inverse_round_trips`.
- Every new feature needs antimeridian and high-latitude cases in its tests.

## C7. Movement and noise are separate

A movement model produces the true trajectory and knows nothing of noise.
The noise engine perturbs it and knows nothing of scheduling or delivery.
Noise never "repairs" a bad base sample. With noise disabled the engine is an
exact identity.

- Enforced in: `src/movement/`, `src/noise/mod.rs`.
- Tests: `src/noise/mod.rs::disabled_noise_is_an_exact_identity`,
  `src/noise/mod.rs::out_of_limit_base_speed_is_not_masked`,
  `src/noise/mod.rs::boundary_is_enforced_and_base_violations_are_reported`,
  `tests/consistency_pipeline.rs::deriving_metadata_never_changes_a_position`.

## C8. Noise is bounded

For every emitted position: offset from truth ≤ `max_position_offset_m`;
step from the previous emitted position ≤ true step + `max_offset_rate × dt`;
inside the scenario boundary. The offset itself changes by at most
`max_offset_rate × dt`, except where the boundary forces more. If no position
satisfies the hard limits, an error — not a position.

- Enforced in: `src/noise/mod.rs` (`noisy_position`, its `satisfies` check).
- Tests: `tests/noise_props.rs::limits_hold_for_random_configurations_motions_and_time_steps`,
  `tests/noise_props.rs::extreme_configurations_stay_within_limits`,
  `src/noise/mod.rs::the_offset_itself_changes_no_faster_than_its_rate`.

## C9. Movement respects its limits with margin

Generated trajectories keep `KINEMATIC_MARGIN` below max speed, acceleration,
deceleration and heading rate, stay inside any fence, start from rest, and
never jump. Bounded modes do not wander off.

- Enforced in: `src/movement/steered.rs`, `src/movement/circular.rs`,
  `src/domain/params.rs` (`KINEMATIC_MARGIN`).
- Tests: `tests/movement_props.rs::steered_models_respect_every_limit_for_random_configurations`,
  `tests/movement_props.rs::bounded_random_walk_never_wanders_off_and_uses_its_area`,
  `tests/movement_props.rs::circular_orbits_are_exact_for_random_configurations`.

## C10. A route is admitted before it is replayed

A recording is structurally validated, then admitted against the scenario's
movement limits over the *whole* interpolated trajectory (suprema, not
samples), then replayed. The trajectory is a function of played time alone:
it passes exactly through each recorded point at its time; sampling at a
different interval reads the same curve. Open routes start and end at rest
and hold their final point; only closed routes loop. Violations are reported
as segment, constraint, observed, limit, time.

- Enforced in: `src/domain/route.rs`, `src/route/plan.rs`,
  `src/route/admission.rs`, `src/movement/route.rs`.
- Tests: `src/route/tests.rs::passes_exactly_through_every_recorded_point_at_its_time`,
  `src/route/tests.rs::before_start_at_end_and_after_end`,
  `src/route/tests.rs::closed_route_loops_seamlessly_and_never_completes`,
  `src/route/tests.rs::structural_rejections`,
  `tests/route_props.rs::admitted_routes_pass_the_gate_at_any_sampling_interval`,
  `tests/route_props.rs::any_single_limit_set_below_the_route_is_detected`.
- Route replay is not a privileged path. It goes through noise and the gate
  like every other model.

## C11. Position and timestamp are the source of truth

Emitted speed and course are derived from consecutive emitted positions and
timestamps: speed = geodesic distance / elapsed time; course = arrival
bearing of the geodesic. Nothing may emit a speed or course from a model.
Nothing may change a position after noise.

- Enforced in: `src/consistency/mod.rs` (`KinematicsDeriver`),
  `src/provider/simulation.rs` (`generate`).
- Tests: `src/consistency/tests.rs::speed_is_distance_over_elapsed_time_and_course_the_arrival_bearing`,
  `src/consistency/tests.rs::derive_does_not_advance_until_the_fix_is_accepted`,
  `tests/consistency_pipeline.rs::every_movement_source_emits_consistent_kinematics`,
  `tests/consistency_pipeline.rs::consistency_holds_from_one_millisecond_to_several_seconds`,
  `tests/moving_pipeline.rs::a_well_behaved_script_is_accepted_and_its_claimed_metadata_ignored`.

## C12. Stationary and first-sample semantics

- The first sample of a run has no speed and no course (`None`, not zero).
- Displacement below `STATIONARY_DISPLACEMENT_M` (4 nm): speed exactly 0, no
  course.
- Displacement below `MIN_COURSE_DISPLACEMENT_M` (0.1 mm): speed `d/dt`, no
  course.
- A course is never carried over from an earlier sample.

- Enforced in: `src/consistency/mod.rs`.
- Tests: `src/consistency/tests.rs::first_fix_has_unknown_kinematics`,
  `src/consistency/tests.rs::stationary_and_short_movement_thresholds`,
  `src/consistency/tests.rs::no_course_is_manufactured_from_rounding_noise`,
  `src/consistency/tests.rs::a_stale_course_is_detected`,
  `tests/consistency_pipeline.rs::stationary_periods_report_zero_speed_and_no_course`.
- A platform adapter must map `None` to the platform's "unknown" value
  (Core Location: `-1`), never to 0.

## C13. Accuracy semantics

Reported accuracy is the scenario's configured value plus clipped accuracy
noise. It is not derived from anything and is never widened to explain an
inconsistency.

- Enforced in: `src/noise/mod.rs`, `src/consistency/mod.rs` (`AccuracyBand`),
  `src/domain/scenario.rs` (accuracy noise must leave accuracy positive).
- Tests: `src/consistency/tests.rs::accuracy_must_stay_within_its_configured_band`,
  `src/domain/scenario.rs::accuracy_noise_must_leave_accuracy_positive`.

## C14. Independent validation before every emission

Every sample passes `SampleValidator` immediately before it is emitted, and
nothing touches it afterwards. The gate recomputes from the samples alone. A
rejected sample is not emitted and does not become the reference for the next
check. Comparisons are strict; allowances are physical or stated resolutions,
derived from configuration.

Stages: field validity; timestamp order; boundary; displacement; consistency;
speed; acceleration and deceleration; heading rate.

- Enforced in: `src/validation/mod.rs`, `src/provider/simulation.rs`.
- Tests: `src/validation/tests.rs::rejects_invalid_fields_without_advancing_state`,
  `src/validation/tests.rs::acceleration_is_judged_between_interval_midpoints`,
  `src/validation/tests.rs::straight_travel_near_a_pole_is_not_mistaken_for_turning`,
  `src/validation/tests.rs::a_teleport_with_honest_metadata_is_still_a_teleport`,
  `src/validation/tests.rs::a_fix_held_at_the_fence_has_an_unbounded_noise_step`,
  `tests/moving_pipeline.rs::gate_rejects_teleportation`,
  `tests/route_pipeline.rs::the_gate_alone_rejects_each_kind_of_corrupted_route_output`,
  `tests/consistency_pipeline.rs::corrupted_metadata_is_detected_by_the_gate`.
- **Never make a failing test pass by loosening the gate.** If a correct
  engine is rejected, either the engine lacks margin or the gate's stated
  model is wrong; find out which and write down why.

## C15. Tests are reproducible and independent

Property tests use fixed seeds and print the failing case. Pipeline tests
re-check emitted streams with `tests/common/recheck.rs`, which restates the
rules without calling the gate or the deriver. New engines get mutation
checks: break the engine on purpose, confirm tests fail, restore.

- Enforced in: `tests/common/recheck.rs`, the `*_props.rs` and
  `*_pipeline.rs` suites.

## C16. The quality gate

Before a ticket is called complete, and before each commit where practical:

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

All three must succeed, with no test failing. `Scripts/validate.sh` runs the
equivalent. `route_props` takes about 25 s in a debug build; that is normal.

## C17. Workflow

- One ticket at a time; stop at its end.
- One commit per logical change, tested first, pushed immediately.
- Test the staged change, not the working tree, when a commit is a subset of
  what is on disk (stash the rest). Two T05 commits and one T06 commit show
  what happens otherwise; see `TICKET_HISTORY.md`.
- Never rewrite or force-push history. No AI attribution lines.
- Write the commit message, read it back, then commit, as separate steps.
  Commit `019ad0f` carries another commit's message because the message file
  had not been rewritten when the commit ran.
- Update `PROGRESS.md`, `Docs/ARCHITECTURE.md` and, when long-lived context
  changes, the documents in this directory.

## C18. Scope

No concealment of the framework or a jailbreak, no integrity-check bypass, no
anti-spoofing evasion, no claims of undetectability. No compatibility claim
without a run on that exact combination. See `PROJECT_BRIEF.md`.

## C19. Scenario documents are strict, exact and versioned

Added by T08. The crate is `locsim-scenario`; `locsim-core` stays without
dependencies and without knowledge of JSON.

- **Stages.** Import is parse → version → migrate → strict decode →
  `Scenario::validate`, in that order; a stage that reports anything ends
  the import and returns everything it found. Export validates first and
  writes only a valid scenario. Nothing invalid comes out of either.
- **No repair.** Every member is required (an absent optional value is
  `null`); unknown and duplicate members are errors; types are exact (an
  integer member refuses `1.0`); enum names are exact; coordinates are built
  with `Coordinate::new` and routes with `Route::new`. Nothing is defaulted,
  wrapped, clamped, sorted, trimmed or rounded on the way in.
- **Exact.** `import(export(s))` equals `s` bit for bit, negative zero
  included. `export(import(text))` equals `text` for exported text. A
  scenario that went through export and import drives the provider to a
  bit-identical stream (C1 across serialisation). Seeds cover the whole
  `u64` range as canonical decimal strings.
- **Versioned.** A document states its `schema_version`; a version this build
  cannot read is refused before anything else is looked at. A change to the
  document shape is a new version plus a migration step in the chain, never
  an edit to version 1 and never a tolerant decoder. No historical schema is
  invented: a migration step exists only for a version that was released.
- **Strings only.** The crate opens no file.
- **Two document types (since T09).** The last-known record
  (`export_last_known` / `import_last_known`, `record_version`) follows
  every rule above: required members, no unknown or duplicate members,
  exact types, its own version on the same chain mechanism, no invented
  history. `null` speed or course means unknown and is never read or
  written as zero. A record must hold a sample that passes
  `SyntheticLocation::validate`. `scenario_fingerprint` is FNV-1a 64 of the
  exported text: an association, not a security or integrity measure, and
  it must stay documented as such. Tests:
  `src/record/tests.rs::arbitrary_records_round_trip_bit_exactly_and_canonically`,
  `src/record/tests.rs::unknown_speed_and_course_stay_unknown_and_zero_stays_zero`,
  `src/record/tests.rs::every_member_refuses_every_json_type_it_does_not_take`,
  `src/record/tests.rs::a_record_must_hold_a_sample_that_was_fit_to_emit`,
  `src/record/tests.rs::fnv_1a_64_matches_the_published_vectors`,
  `tests/last_known.rs::fingerprints_are_those_of_the_exported_text_and_tell_the_examples_apart`.
- Enforced in: `src/lib.rs` (`import_with`, `export_scenario`),
  `src/json.rs` (`parse`, `find_duplicates`, the `Serialize` impl),
  `src/migrate.rs` (`MigrationChain`), `src/schema.rs`.
- Tests: `src/json.rs::every_finite_float_round_trips_bit_exactly`,
  `src/json.rs::every_duplicate_key_is_reported_with_its_path`,
  `src/json.rs::writer_refuses_non_finite_numbers_instead_of_writing_null`,
  `src/schema/tests.rs::arbitrary_finite_scenarios_round_trip_bit_exactly_through_text`,
  `src/schema/tests.rs::each_missing_member_is_reported_by_its_path`,
  `src/schema/tests.rs::an_unknown_member_is_rejected_in_every_object`,
  `src/schema/tests.rs::every_member_refuses_every_json_type_it_does_not_take`,
  `src/schema/tests.rs::seed_is_a_canonical_decimal_string_over_the_whole_u64_range`,
  `src/schema/tests.rs::the_sample_document_is_exactly_the_reference_text`,
  `src/migrate.rs::steps_run_in_order_from_the_documents_version_to_the_newest`,
  `src/migrate.rs::versions_outside_the_chain_are_refused_without_running_anything`,
  `src/lib.rs::an_old_document_is_migrated_then_strictly_decoded_and_validated`,
  `tests/malformed.rs::each_stage_reports_alone_and_before_the_next`,
  `tests/malformed.rs::an_invalid_scenario_is_reported_exactly_as_scenario_validate_reports_it`,
  `tests/malformed.rs::export_refuses_an_invalid_scenario_and_says_why`,
  `tests/malformed.rs::random_damage_never_panics_and_never_lets_an_invalid_scenario_through`,
  `tests/round_trip.rs::valid_scenarios_and_their_streams_survive_export_and_import_exactly`,
  `tests/examples.rs::every_example_imports_and_is_already_in_exported_form`,
  `tests/examples.rs::every_example_runs_through_the_provider_and_the_final_gate`.
- Do not: switch the document tree to `serde_json::Value` (it drops duplicate
  keys silently); drop `float_roundtrip` (floats stop reading back exactly);
  derive `Deserialize` with defaults; accept a JSON number as a seed; upgrade
  `serde_json` without running the float and round-trip tests.

## C20. Stored records: verified before use, replaced or left alone

Added by T09. The crate is `locsim-store`. It is the only crate that touches
files, and it has no third-party dependency.

- **Stored is not resumable.** What is stored is a scenario and the last
  emitted sample. Neither, nor both, can continue a run; nothing may claim
  or imply otherwise. Checkpoint/resume is a separate, unowned ticket and
  would need engine state the core does not expose.
- **Digest before meaning.** A stored file is an envelope whose payload is
  the exported document byte for byte. Kind of file, envelope version,
  header, length and SHA-256 are checked, in that order, before the payload
  is decoded. A payload that fails the digest never reaches a codec.
- **Loading never writes.** No file is `Ok(None)`: absence is not an error
  and not corruption. A file that cannot be loaded is an error that says
  why; it is never repaired, moved, deleted, or turned into a default. A
  read error is an I/O error, neither missing nor corrupt.
- **Saving never damages.** A value is validated and encoded before the
  disk is touched; an invalid value is not written. The record is not
  opened or written before the rename. A failure at any earlier step leaves
  the previous record byte-identical and removes the temporary file, or
  reports that it could not. A directory-sync failure after the rename is
  reported as "replaced, not confirmed durable", never as a plain failure
  and never as success.
- **Claims match evidence.** Visibility and durability are stated
  separately and per platform. Atomic visibility is claimed for POSIX
  `rename` only; for Windows it is "observed, not documented". Durability is
  "requested", and on Windows not requested for the change of name
  (`DIRECTORY_SYNC`). Nothing is claimed about power loss. Unix behaviour is
  marked untested until it is run.
- **One writer.** Saves through one `Store` are serialised; several writers
  per directory are unsupported. No lock file, no backup generation, no
  automatic removal of temporary files.
- **Two different numbers.** The SHA-256 digest says a file is the bytes
  that were written; it is unkeyed and detects accidents only. The FNV
  fingerprint says which scenario a record belongs to. Neither does the
  other's job and neither is a security measure.
- Enforced in: `src/store.rs` (`Store`, `interpret`), `src/envelope.rs`
  (`seal`, `open`), `src/atomic.rs` (`replace`), `src/sha256.rs`,
  `src/error.rs`.
- Tests: `src/atomic.rs::a_failure_at_any_step_before_the_replacement_keeps_the_old_record`,
  `src/atomic.rs::a_failure_at_any_step_before_the_replacement_creates_no_record`,
  `src/atomic.rs::a_directory_sync_failure_says_the_record_was_replaced`,
  `src/atomic.rs::a_temporary_file_that_cannot_be_removed_is_reported_with_the_first_error`,
  `src/atomic.rs::running_out_of_temporary_names_is_an_error_not_an_overwrite`,
  `src/envelope.rs::every_single_bit_flip_anywhere_in_the_file_is_detected`,
  `src/envelope.rs::every_truncation_is_detected`,
  `src/envelope.rs::the_header_has_exactly_one_spelling`,
  `src/sha256.rs::matches_the_fips_180_4_vectors`,
  `src/sha256.rs::matches_another_implementation_at_every_length_up_to_300`,
  `src/store/tests.rs::a_save_that_fails_at_any_step_leaves_the_stored_scenario_loadable`,
  `src/store/tests.rs::damage_that_the_document_codec_accepts_is_caught_by_the_digest`,
  `src/store/tests.rs::the_digest_is_checked_before_the_document`,
  `src/store/tests.rs::a_read_error_is_an_io_error_not_a_missing_or_corrupt_record`,
  `tests/scenario_store.rs::nothing_stored_is_none_and_loading_writes_nothing`,
  `tests/scenario_store.rs::an_empty_file_is_corrupt_not_missing`,
  `tests/scenario_store.rs::a_corrupt_record_is_left_alone_and_can_be_replaced_by_a_save`,
  `tests/scenario_store.rs::an_invalid_scenario_is_refused_and_the_stored_one_is_kept`,
  `tests/scenario_store.rs::saves_through_one_store_are_serialised_and_readers_see_whole_records`,
  `tests/last_known_store.rs::the_stored_record_cannot_continue_a_run`,
  `tests/last_known_store.rs::a_record_from_another_scenario_loads_and_its_fingerprint_does_not_match`,
  `tests/crash.rs::a_writer_killed_at_any_moment_leaves_a_whole_valid_record`,
  `tests/platform.rs` (Windows module run; Unix module **never run**).
- Do not: write a record in place; load a payload before its digest is
  checked; treat a missing file as corrupt or a corrupt file as missing;
  delete or overwrite a corrupt record on load; add a default for a missing
  record inside the store; describe the record as a checkpoint; describe
  the digest as protection against tampering; upgrade a platform row in the
  documentation without a run on that platform.

## C21. Health: derived, bounded, honest about what a restart is

Added by T10. The crate is `locsim-health`; it depends on `locsim-core`
only, and no file of the other crates was changed for it. Paths below are
relative to `crates/locsim-health/`.

- **A supervisor changes no sample.** With nothing to do, the supervised
  stream is the bare stream bit for bit. It decides only whether a sample is
  handed on and what follows a failure. The gate is untouched and is never
  bypassed, relaxed or re-run by the supervisor.
- **Health is derived, never stored, and cannot contradict the lifecycle.**
  `Stopped` if and only if no run exists and none is scheduled; `Failed` if
  and only if the run failed and nothing will be tried; `Recovering` if and
  only if a retry time exists; `Healthy` only with no active cause. Whether
  samples are arriving is a separate field: a stalled provider is reported
  as running *and* as not emitting.
- **A refused call is not a failed run.** A lifecycle call that returns an
  error changes nothing and is reported as `OperationRejected`. A run fails
  only when `poll` errs, a sample is withheld, or the provider's state
  disagrees with the lifecycle. A refused initial start leaves the
  supervisor `Stopped`.
- **Recovery is a new run and is described as one.** It starts the scenario
  from its beginning. No document, name or message may suggest that a run
  is resumed, continued or restored, or that the last-known record plays any
  part. The run number increases and marks a discontinuity. Nothing is
  reseeded or altered to make a retry differ.
- **Recovery is bounded.** `max_recovery_attempts` counts restart attempts
  per failure episode; an attempt is counted when made; a restart that
  cannot start a run uses its attempt; the count returns to zero only when a
  restarted run has emitted for `stable_after_s`, or at `stop`. After the
  bound: `Failed`, until `stop` and `start`.
- **Errors are classified by variant, on evidence.** Only
  `ScheduleError::Overflow` is final at once. Nothing is read from an
  error's text. Retrying is not a claim of recoverability.
- **Timestamps handed on strictly increase within a session**, across
  restarts. A stale one is withheld and ends the run. For the simulation
  provider this follows from the scheduler; the check is what guarantees it
  for any provider.
- **Time is passed in.** No clock, sleep, thread or randomness. Seconds
  become nanoseconds once, by the core's rounding rule; a duration that does
  not fit is refused at validation. The backoff costs one multiplication per
  attempt whatever the attempt number. No power function, no loop over
  attempts, no arithmetic that can overflow silently.
- **One place for events and totals.** Every event passes through one
  function, which updates the totals from it. Every total can be recounted
  from the event stream.
- **No position in any event**, at any level, and no event per sample.
- **Other components are reported, not known.** Their faults enter through
  `report_fault`. A fault degrades a run and does nothing else. The crate
  must not depend on `locsim-store` or `locsim-scenario` (tests excepted).
- Enforced in: `src/supervisor.rs` (`record`, `fail_run`, `after_failure`,
  `attempt_restart`, `note_slots`, `watch_for_stall`, `derive_health`),
  `src/policy.rs` (`HealthPolicy::limits`, `seconds_to_nanos`, `Backoff`),
  `src/event.rs`, `src/report.rs`.
- Tests: `tests/pass_through.rs::a_supervised_stream_is_the_bare_stream_bit_for_bit`,
  `tests/lifecycle.rs::a_call_the_lifecycle_does_not_allow_is_refused_and_changes_nothing`,
  `tests/lifecycle.rs::a_refused_start_leaves_the_supervisor_stopped_not_failed`,
  `tests/lifecycle.rs::a_sample_with_a_stale_timestamp_is_withheld_and_ends_the_run`,
  `tests/lifecycle.rs::ticks_skipped_by_the_failing_poll_are_counted_in_the_run_and_the_totals`,
  `tests/recovery.rs::the_bound_counts_restart_attempts_not_runs`,
  `tests/recovery.rs::an_attempt_is_made_at_its_time_and_not_a_nanosecond_before`,
  `tests/recovery.rs::a_restart_that_cannot_start_uses_its_attempt_and_the_next_is_scheduled`,
  `tests/recovery.rs::an_episode_ends_when_the_restarted_run_has_emitted_for_the_stable_time`,
  `tests/recovery.rs::every_error_is_retried_except_unrepresentable_tick_times`,
  `tests/recovery.rs::a_restarted_run_is_a_fresh_run_of_the_scenario_not_a_continuation`,
  `tests/recovery.rs::a_failure_that_does_not_depend_on_timing_happens_again_in_every_restart`,
  `tests/recovery.rs::the_first_sample_after_a_restart_is_stamped_by_the_schedule_not_by_the_restart`,
  `tests/recovery.rs::random_sequences_of_failures_delays_and_pauses_keep_timestamps_increasing`,
  `tests/recovery.rs::a_provider_that_restarts_with_an_old_timestamp_is_caught_by_the_check`,
  `tests/recovery.rs::which_calls_are_allowed_agrees_with_the_core_transition_table`,
  `tests/watchdog.rs::a_run_is_stalled_one_nanosecond_after_the_threshold_and_not_at_it`,
  `tests/watchdog.rs::nothing_is_expected_of_a_paused_run`,
  `tests/watchdog.rs::exactly_the_limit_is_not_falling_behind_and_one_more_is`,
  `tests/watchdog.rs::it_ends_only_when_a_whole_window_closes_within_the_limit`,
  `tests/accounting.rs::a_reported_fault_degrades_a_run_and_does_nothing_else`,
  `tests/accounting.rs::reports_stay_consistent_and_totals_recountable_through_random_histories`,
  `tests/accounting.rs::no_event_reveals_a_position`,
  `src/policy.rs::a_million_attempts_with_a_multiplier_just_above_one_stay_cheap_and_sound`,
  `src/policy.rs::a_duration_must_fit_in_nanoseconds`.
- In every test, after every step, `tests/support/mod.rs::assert_consistent`
  and `assert_reconciles` are applied through `check`. A new test that
  skips them is not testing what this contract says.
- Do not: store health; give mutable access to the wrapped provider; add a
  clock, sleep or thread to the supervisor; make a fault stop or restart a
  run; put a coordinate in an event; call a restart a resume; retry without
  a bound; treat an error as permanent or recoverable because of its text;
  weaken the gate so that a restarted run passes.

## C22. The C interface: nothing undefined that can be defined, nothing changed on the way

Added by T11A. The crate is `locsim-ffi`; the header is
`include/locsim.h`. It is the only crate that contains `unsafe`.

- **It exposes a supervised session and adds nothing to it.** A session is
  a `Supervisor<SimulationProvider>`. Every status, sample, deadline, health
  report and event that comes out is the supervisor's, made on the same
  calls. No simulation mathematics, no clock, no file, no conversion of
  times.
- **The header is the contract and the library must match it.** Fifteen
  functions; four structs whose size, alignment and field offsets the
  header asserts; every constant. Any change is a new `LOCSIM_ABI_VERSION`.
- **A handle is checked.** Zero, a handle never issued and a handle already
  destroyed are `LOCSIM_ERR_INVALID_HANDLE`, from every function. No handle
  is issued twice.
- **No call waits for a session.** One in use is `LOCSIM_ERR_BUSY`, and the
  refused call did nothing. That includes destroy, which then leaves the
  handle valid. A call that found its session before it was destroyed does
  nothing and says the handle is invalid. Lock order: registry, session,
  event queue.
- **Checks have one order**: null pointers, struct size, the other
  arguments in order, the handle, the session's state, the operation.
- **A call that does not return `LOCSIM_OK` writes nothing**, except
  `create`, by the table in the header. With a flag at 0, the value beside
  it is not written. An unknown speed or course is flag 0 and NaN.
- **What can be checked is checked; what cannot is the caller's.** Null,
  lengths, sizes, codes, UTF-8. A pointer that is not null and not valid is
  undefined behaviour that the library does not detect; the header says so.
  No reference with an alignment is made from a caller's pointer.
- **A number from C is never taken for a value it does not name.** An
  unknown component is refused. Numbers going out come from matches without
  a wildcard, so a new variant in another crate does not compile until it
  has a number.
- **Only the kind of an error crosses.** Text exists only for a refused
  scenario or policy, at most 4 096 bytes, returned to the caller.
- **An event holds no text and no position; a fault has no text.** A record
  is integers. The queue is bounded; a full one discards its oldest record
  and counts it. Totals in the health report do not depend on the queue.
- **A panic does not unwind into the caller**, and leaves no lock held. The
  session is released with no lock held and no other panic unwinding; its
  handle answers `LOCSIM_ERR_POISONED` until destroyed, and destroy
  succeeds. This is not promised of a build that aborts on panic.
- **Time is the caller's** (C2). The interface does no arithmetic on it.
- **What was run is all that is claimed.** Windows x86_64; see
  `Docs/COMPATIBILITY.md`. The C harness is `harness/run.sh`, and
  `cargo test` does not run it.
- Enforced in: `src/registry.rs` (`insert`, `close`, `get`),
  `src/session.rs` (`create`, `locsim_session_create`,
  `locsim_session_destroy`, `with_session`, `contain`, `release`),
  `src/ops.rs`, `src/health.rs`, `src/events.rs` (`EventQueue`,
  `LocsimEvent::of`), `src/codes.rs`, `include/locsim.h` (the assertions).
- Tests (unit tests, in the files named):
  `src/registry.rs::a_reused_slot_gets_a_new_handle_and_the_old_one_stays_invalid`,
  `src/registry.rs::a_held_value_cannot_be_closed_and_its_handle_stays_valid`,
  `src/registry.rs::a_slot_whose_generation_ran_out_is_retired`,
  `src/session/tests.rs::a_null_pointer_is_refused_and_nothing_at_all_is_written`,
  `src/session/tests.rs::several_faults_at_once_are_reported_in_the_documented_order`,
  `src/session/tests.rs::the_text_is_copied_whole_or_not_at_all_and_its_length_is_always_reported`,
  `src/session/tests.rs::destroy_is_busy_while_another_thread_holds_the_session_and_changes_nothing`,
  `src/session/tests.rs::a_session_is_released_after_the_locks_and_outside_any_panic`,
  `src/session/tests.rs::a_session_whose_release_panics_is_reported_and_its_handle_is_invalid`,
  `src/ops/tests.rs::every_example_gives_the_same_samples_statuses_and_deadlines_as_the_supervisor`,
  `src/ops/tests.rs::a_failing_run_is_reported_once_and_replaced_like_the_supervisor_does`,
  `src/ops/tests.rs::health_and_events_follow_the_supervisor_through_faults_a_failure_and_a_stall`,
  `src/ops/tests.rs::a_session_in_use_is_busy_for_every_other_call_and_none_of_them_waits`,
  `src/ops/tests.rs::a_call_that_found_its_session_before_it_was_destroyed_does_nothing`,
  `src/ops/tests.rs::a_panic_in_an_operation_poisons_the_session_and_releases_it_at_once`,
  `src/ops/tests.rs::a_panic_outside_the_operation_poisons_the_lock_and_the_next_call_deals_with_it`,
  `src/ops/tests.rs::a_panic_inside_the_event_sink_poisons_the_session_and_holds_no_lock`,
  `src/ops/tests.rs::a_full_queue_keeps_the_newest_events_in_order_and_counts_what_it_dropped`,
  `src/ops/tests.rs::no_event_and_no_report_holds_a_coordinate_that_was_emitted`,
  `src/ops/tests.rs::a_component_that_does_not_exist_is_refused_before_the_handle_is_looked_at`,
  `src/ops/tests.rs::a_failed_poll_writes_nothing`,
  `src/codes.rs::every_provider_error_has_its_documented_number`,
  `tests/header.rs::every_constant_has_the_same_number_on_both_sides_and_none_is_missing`.
  From C: `harness/harness.c`, by `harness/run.sh`.
- In the comparison tests, after every call, the session's status, sample,
  deadline, health and events are required to equal the supervisor's. A new
  operation that is not put through that comparison is not tested as this
  contract says.
- Do not: wait for a session's lock; hand out a pointer to a session; add a
  callback into the caller; put text or a floating-point value in an event;
  accept text with a fault; pass on an error's message; write an output of
  a call that failed; change the header without changing the version; add
  `unsafe` to another crate; add a clock, a file or a computation of
  position to this crate; count a passing `cargo test` as a run of the C
  harness; claim a platform the harness has not run on.
