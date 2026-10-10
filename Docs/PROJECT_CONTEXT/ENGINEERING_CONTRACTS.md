# Engineering contracts

The invariants future tickets must preserve. Each names where it is enforced
and which tests would fail if it broke. Paths are relative to
`crates/locsim-core/`. Test references are `file::test_function`.

Paths in C19 are relative to `crates/locsim-scenario/`.

References in C1–C18 were checked to exist at commit `1be8852` (2026-10-09);
T08 changed no file of `locsim-core`. References in C19 were checked at
`e6bcbf2` (2026-10-10). If one no longer resolves, the code moved: find the
new home before concluding the contract is gone.

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

- Enforced in: `src/scheduler/schedule.rs`, `src/domain/time.rs`,
  `src/provider/simulation.rs`.
- Tests: `src/scheduler/schedule.rs::late_polls_do_not_accumulate_drift`,
  `src/scheduler/schedule.rs::overdue_slots_are_skipped_and_counted_not_replayed`,
  `src/scheduler/schedule.rs::pause_preserves_phase_and_counts_nothing_as_missed`,
  `tests/fixed_pipeline.rs::six_simulated_hours_at_10_hz_stay_exact`.

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
state.

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
