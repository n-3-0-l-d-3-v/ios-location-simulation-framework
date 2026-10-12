# Long-gap recording

A recording of what the engine does when time passes between two polls.
It is a diagnostic. It changes nothing in the engine, asserts no outcome
and fixes nothing. The open pause/resume defect of `PROGRESS.md` is still
open.

| File | What it is |
|---|---|
| `results.jsonl` | One JSON object per case, 1056 cases. Written by the tool |
| `summary.md` | One cell per case. Written by the tool |
| this file | What was run, and what the recording shows. Written by hand |

Produced by `crates/locsim-health/examples/gap_diagnostic.rs`:

    cargo run -p locsim-health --example gap_diagnostic -- \
        Examples/Scenarios Docs/Diagnostics/long_gap

`crates/locsim-health/tests/gap_recording.rs` compares the written files
with a fresh run, so a change to the engine that changes any recorded
outcome fails `cargo test` until the files are regenerated. `summary.md` is
compared on every host; `results.jsonl` only on Windows x86_64, where it was
recorded (see "Across platforms"). That test pins the recording. It does
not say the recorded behaviour is right.

## What was run

8 scenarios × 2 pipelines × 2 kinds of gap × 3 placements × 11 gap lengths.

- **Scenarios:** the eight documents of `Examples/Scenarios`.
- **Pipelines:** `bare`, a `SimulationProvider`; `supervised`, a
  `Supervisor` around one, with the policy of the C harness.
- **Kinds of gap,** kept apart:
  - `late_poll`: no call for the length of the gap; the next poll comes
    that long after its deadline. No pause.
  - `pause_resume`: `pause` half an interval after the last sample,
    `resume` after the length of the gap, then a poll at the deadline the
    resumed run reports.
- **Placements:** `after_initial`, after the first sample; `mid_run`, after
  the sample with index 20; `near_route_end`, after the last sample before
  the route's final point.
- **Gap lengths:** 0, 1, 2, 5, 10, 30, 60, 300, 1800, 3600 and 86400 s.

Before the gap every poll is made at its deadline. After it, the first poll
and three more are recorded, each with the provider's counters and, for the
supervised pipeline, the health report. Every case is run twice in one
process and the two records are compared.

## What could not be exercised

- `near_route_end` applies to the two scenarios that have a route. The
  other six are recorded as `"applicable": false` with the reason: 264 of
  the 1056 cases.
- `antimeridian_loop` loops, so it has no end. Its `near_route_end` cases
  make the gap before the sample that closes the first lap, and say so in
  `placement_note`.
- A pause is made at one point in the interval, the middle. Other points
  were not tried.
- A gap length of 0 under `late_poll` is an on-time poll, and under
  `pause_resume` a pause and a resume at the same instant. Both are
  controls.

## What the recording shows

Run on Windows 11 x86_64, rustc 1.98.1, on 2026-10-12.

**Repeatability.** All 1056 cases gave the same record twice. This says the
run is repeatable in one process on one machine. It does not say the
outcomes are right.

**Across platforms.** The diagnostic was also run on macOS 15.7 arm64 with
the same rustc 1.98.1 (workflow run 38238724688, 2026-10-12) and compared
with the Windows recording:

- The outcome of every case is the same: `summary.md` is identical.
- 723 of the 1056 records differ in the last digits of numbers. The
  largest differences seen: latitude 8.9e-15 degrees and longitude 4.3e-14
  degrees; speed 1.7e-9 m/s; course 1.5e-4 degrees, in samples whose speed
  is close to zero. The numbers inside the recorded errors differ in the
  same way (a change of speed of 1.0308955218472524 m/s on Windows is
  1.0308955218446005 on macOS); the limits they are compared with do not.
- So the engine's output is not bit-for-bit the same on these two hosts.
  Why has not been investigated. The mathematics library of each platform
  is a candidate and nothing more than that.

A fixture that compares numbers exactly is therefore the fixture of one
platform. The C harness's byte-for-byte comparison is made between two
programs on the same machine and is not affected.

**Late polling.** All 396 applicable `late_poll` cases returned a sample at
the first poll after the gap and at the three after it, on both pipelines,
for every gap length. The skipped slots are counted as missed ticks. No
error was recorded.

**Pause and resume.** 17 of the 198 applicable cases on each pipeline ended a
run with a validation error:

| Scenario | Placement | Gap lengths with an error | Error |
|---|---|---|---|
| `antimeridian_loop` | `after_initial` | 1, 2, 5, 10, 30 s | first sample emitted; the next poll `AccelerationExceeded` |
| `antimeridian_loop` | `mid_run` | 1, 2, 5, 10, 30 s | first poll `DecelerationExceeded` |
| `antimeridian_loop` | `near_route_end` | 1, 2, 5, 10, 30 s | first poll `DecelerationExceeded` |
| `driving` | `mid_run` | 1, 2 s | first poll `DecelerationExceeded` |

After the error the bare provider is in `Error` and returns nothing. The
supervisor replaces the run: the samples that follow have a new run number
and start from the scenario's beginning. Gap lengths of 0 s and of 60 s and
more gave no error for `antimeridian_loop`; 0 s and 5 s and more gave none
for `driving` at `mid_run`; `driving` at `after_initial` and the other six
scenarios gave none at any length.

**The first sample after a resume, in every moving scenario.** This is in
the cases that gave no error as well as those that did. The sample's
position is one simulated interval on from the last sample, and its
timestamp is one interval plus the pause after it. The engine's reported
speed for that sample equals the recorded `step_distance_m / step_dt_s`.
Examples from `results.jsonl`, `bare`, `mid_run`:

| Scenario | Gap | Speed before | `step_dt_s` | `step_distance_m` | Speed of the first sample after | Speed of the sample after that |
|---|---|---|---|---|---|---|
| `driving` | 0 s | 24.37 | 0.5 | 12.81 | 25.63 | 26.88 |
| `driving` | 1 s | 24.37 | — | — | rejected, change 15.83 > limit 6.09 | — |
| `driving` | 5 s | 24.37 | 5.5 | 12.81 | 2.33 | 26.88 |
| `driving` | 3600 s | 24.37 | 3600.5 | 12.81 | 0.0036 | 26.88 |
| `antimeridian_loop` | 30 s | 2.013 | — | — | rejected, change 1.948 > limit 1.008 | — |
| `antimeridian_loop` | 60 s | 2.013 | 61.0 | 2.013 | 0.033 | 2.037 |

So with a 5 s pause, `driving` reports 24.4, then 2.3, then 26.9 m/s on
three consecutive samples, and the gate accepts all three.

**Why the longer pauses are accepted, read from the code.** The gate's
limit on a change of speed is `validation/mod.rs`:
`max_acceleration × 0.5 × (dt + previous dt) + allowance`. It grows with
the time between the two samples, and the change of speed cannot be larger
than the speed itself. Past some pause length the limit is the larger of
the two. The recorded limits agree: 0.053, 0.072, 0.137, 0.264 and
1.008 m/s for `antimeridian_loop` at 1, 2, 5, 10 and 30 s.

## What this does and does not establish

Established by the recording:

- The rejections need a pause. A late poll of the same length on the same
  scenario at the same place gives none.
- Which pause lengths are rejected follows from comparing a speed of
  `distance of one interval / (interval + pause)` with the gate's limit.
  That is the arithmetic `PROGRESS.md` suspected, now seen in every case
  and not in one.
- The cases that pass are not unaffected: they emit one sample whose speed
  is far below the speed on either side of it.

Not established:

- What the right behaviour across a pause is. Whether the first sample
  after a resume should carry a speed at all, be marked as a discontinuity,
  or be stamped differently is a decision about the engine's contract. It
  has not been made, and nothing here makes it.
- That any fix is correct. None has been written.
- Behaviour for pauses at other points of the interval, for pause lengths
  between those listed, or for a second pause in the same run.
- Anything on a platform other than the two above, and why those two
  differ in their last digits.
