//! A recording of what the engine does after a gap in time. Diagnostic only.
//!
//! Every example scenario is run on the bare `SimulationProvider` and on a
//! `Supervisor` around one. A number of samples is taken on time, then a
//! gap of a given length is made in one of two ways that are kept apart:
//!
//! - `late_poll`: nothing is called for the length of the gap, and the next
//!   poll comes that much after its deadline;
//! - `pause_resume`: the run is paused half an interval after its last
//!   sample and resumed after the length of the gap.
//!
//! What the first poll after the gap returns is recorded, and the three
//! polls after it. Nothing is asserted and nothing in the engine is
//! changed: an outcome written here is what happened, not what should
//! happen. Each case is run twice and the two records are compared.
//!
//!     cargo run -p locsim-health --example gap_diagnostic -- \
//!         Examples/Scenarios Docs/Diagnostics/long_gap
//!
//! writes `results.jsonl` (one case per line) and `summary.md` there.
//! `tests/gap_recording.rs` compares both files with a fresh run.

use locsim_core::domain::{Scenario, SyntheticLocation, Timestamp};
use locsim_core::geographic::distance;
use locsim_core::provider::{LocationProvider, ProviderError, SimulationProvider};
use locsim_health::{HealthPolicy, NullSink, Supervisor};
use std::fmt::Write as _;

pub const EXAMPLES: [&str; 8] = [
    "antimeridian_loop",
    "circular",
    "driving",
    "fixed",
    "high_latitude",
    "random_walk",
    "route_replay",
    "walking",
];
pub const GAPS_S: [i64; 11] = [0, 1, 2, 5, 10, 30, 60, 300, 1800, 3600, 86400];
pub const PIPELINES: [&str; 2] = ["bare", "supervised"];
pub const KINDS: [&str; 2] = ["late_poll", "pause_resume"];
pub const PLACEMENTS: [&str; 3] = ["after_initial", "mid_run", "near_route_end"];

const SEC: i64 = 1_000_000_000;
/// Where the caller's time scale starts in these runs. Arbitrary.
const T0: i64 = 1_700_000_000_000_000_000;
/// `mid_run` makes the gap after the sample with this index.
const MID_RUN_INDEX: u64 = 20;
const FOLLOW_UP_POLLS: usize = 3;

/// The policy of the C harness's `good_policy` and of `reference.rs`.
fn policy() -> HealthPolicy {
    HealthPolicy {
        max_recovery_attempts: 3,
        backoff_initial_s: 1.0,
        backoff_multiplier: 2.0,
        backoff_max_s: 60.0,
        stable_after_s: 10.0,
        stall_after_s: 5.0,
        missed_window_ticks: 10,
        max_missed_in_window: 2,
    }
}

enum Pipeline {
    Bare(Box<SimulationProvider>),
    Supervised(Box<Supervisor<SimulationProvider>>),
}

impl Pipeline {
    fn new(name: &str, scenario: &Scenario) -> Self {
        let provider = SimulationProvider::new(scenario.clone());
        match name {
            "bare" => Pipeline::Bare(Box::new(provider)),
            _ => Pipeline::Supervised(Box::new(
                Supervisor::for_simulation(provider, policy(), Box::new(NullSink))
                    .expect("the policy suits every example"),
            )),
        }
    }

    fn provider(&mut self) -> &mut dyn LocationProvider {
        match self {
            Pipeline::Bare(provider) => provider.as_mut(),
            Pipeline::Supervised(supervisor) => supervisor.as_mut(),
        }
    }

    /// When to call next: the tick of the run, or the supervisor's pending
    /// restart, whichever is earlier. `None` when neither exists.
    fn next_deadline(&self) -> Option<Timestamp> {
        match self {
            Pipeline::Bare(provider) => provider.next_deadline(),
            Pipeline::Supervised(supervisor) => {
                match (
                    supervisor.provider().next_deadline(),
                    supervisor.next_deadline(),
                ) {
                    (Some(tick), Some(retry)) => Some(tick.min(retry)),
                    (tick, retry) => tick.or(retry),
                }
            }
        }
    }

    fn run_number(&self) -> Option<u64> {
        match self {
            Pipeline::Bare(_) => None,
            Pipeline::Supervised(supervisor) => Some(supervisor.health().run),
        }
    }

    /// The counters of the run, and for a supervisor its health report.
    fn state_json(&mut self) -> String {
        let status = self.provider().status();
        let mut out = format!(
            "{{\"state\":\"{:?}\",\"sample_count\":{},\"failed_count\":{},\"missed_ticks\":{},\"trajectory_complete\":{}",
            status.state,
            status.sample_count,
            status.failed_count,
            status.missed_ticks,
            status.trajectory_complete
        );
        if let Pipeline::Supervised(supervisor) = self {
            let report = supervisor.health();
            let totals = report.totals;
            write!(
                out,
                ",\"health\":{{\"health\":\"{:?}\",\"lifecycle\":\"{:?}\",\"causes\":{},\"emission\":{},\"run\":{},\"attempts_used\":{},\"retry_at_ns\":{},\"last_error\":{},\"totals\":{{\"runs_started\":{},\"runs_ended\":{},\"samples\":{},\"withheld\":{},\"missed_ticks\":{},\"failures\":{},\"recovery_attempts\":{},\"recoveries_completed\":{},\"terminal_failures\":{},\"rejected_operations\":{}}}}}",
                report.health,
                report.lifecycle,
                text(&format!("{:?}", report.causes)),
                text(&format!("{:?}", report.emission)),
                report.run,
                report.attempts_used,
                optional(report.retry_at.map(|at| at.as_nanos())),
                report.last_error.as_ref().map_or("null".to_string(), error_json),
                totals.runs_started,
                totals.runs_ended,
                totals.samples,
                totals.withheld,
                totals.missed_ticks,
                totals.failures,
                totals.recovery_attempts,
                totals.recoveries_completed,
                totals.terminal_failures,
                totals.rejected_operations,
            )
            .unwrap();
        }
        out.push('}');
        out
    }
}

/// A JSON string.
fn text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON number. A value that is not finite has no JSON number and is
/// written as a string, so that it is seen rather than lost.
fn number(value: f64) -> String {
    if value.is_finite() {
        format!("{value:?}")
    } else {
        text(&format!("{value:?}"))
    }
}

fn optional<T: ToString>(value: Option<T>) -> String {
    value.map_or("null".to_string(), |v| v.to_string())
}

fn optional_number(value: Option<f64>) -> String {
    value.map_or("null".to_string(), number)
}

/// The outer variant of an error, the variant inside it, and all of it.
fn error_json(error: &ProviderError) -> String {
    let detail = format!("{error:?}");
    let mut names = detail
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty());
    let class = names.next().unwrap_or("");
    let variant = names.next().unwrap_or("");
    format!(
        "{{\"class\":{},\"variant\":{},\"detail\":{}}}",
        text(class),
        text(variant),
        text(&detail)
    )
}

/// A sample, with what separates it from the sample emitted before it.
/// `step_distance_m / step_dt_s` is computed here from the two emitted
/// positions and timestamps; `speed_mps` and `course_deg` are the engine's.
fn sample_json(
    sample: &SyntheticLocation,
    previous: Option<&SyntheticLocation>,
    run: Option<u64>,
) -> String {
    let mut out = format!(
        "{{\"timestamp_ns\":{},\"latitude\":{},\"longitude\":{},\"speed_mps\":{},\"course_deg\":{},\"run\":{}",
        sample.timestamp.as_nanos(),
        number(sample.coordinate.latitude()),
        number(sample.coordinate.longitude()),
        optional_number(sample.speed_mps),
        optional_number(sample.course_deg),
        optional(run),
    );
    if let Some(previous) = previous {
        let dt = sample.timestamp.seconds_since(previous.timestamp);
        let metres = distance(previous.coordinate, sample.coordinate).ok();
        write!(
            out,
            ",\"step_dt_s\":{},\"step_distance_m\":{},\"step_mean_speed_mps\":{}",
            number(dt),
            optional_number(metres),
            optional_number(metres.filter(|_| dt > 0.0).map(|m| m / dt)),
        )
        .unwrap();
    }
    out.push('}');
    out
}

/// One poll: when, what came back, and the state afterwards.
struct Polled {
    json: String,
    /// `S` a sample, `R` a sample of a later run than the one before the
    /// gap, `N` nothing, `E(variant)` an error.
    code: String,
}

struct Case<'a> {
    pipeline: Pipeline,
    last: Option<SyntheticLocation>,
    run_before_gap: Option<u64>,
    interval_ns: i64,
    scenario: &'a Scenario,
}

impl Case<'_> {
    fn poll(&mut self, at: i64) -> Polled {
        let result = self.pipeline.provider().poll(Timestamp::from_nanos(at));
        let run = self.pipeline.run_number();
        let (outcome, code) = match &result {
            Ok(Some(sample)) => {
                let json = sample_json(sample, self.last.as_ref(), run);
                self.last = Some(*sample);
                let code = if run == self.run_before_gap { "S" } else { "R" };
                (
                    format!("\"outcome\":\"sample\",\"sample\":{json}"),
                    code.to_string(),
                )
            }
            Ok(None) => ("\"outcome\":\"none\"".to_string(), "N".to_string()),
            Err(error) => {
                let json = error_json(error);
                let variant = format!("{error:?}");
                let variant = variant
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .filter(|part| !part.is_empty())
                    .nth(1)
                    .unwrap_or("?")
                    .to_string();
                (
                    format!("\"outcome\":\"error\",\"error\":{json}"),
                    format!("E({variant})"),
                )
            }
        };
        let json = format!(
            "{{\"poll_at_ns\":{at},{outcome},\"after\":{}}}",
            self.pipeline.state_json()
        );
        Polled { json, code }
    }
}

/// The index of the sample after which `placement` makes its gap, or why
/// the placement does not apply to the scenario.
fn placement_index(placement: &str, scenario: &Scenario) -> Result<u64, String> {
    match placement {
        "after_initial" => Ok(0),
        "mid_run" => Ok(MID_RUN_INDEX),
        _ => {
            let Some(route) = scenario.route.as_ref() else {
                return Err(
                    "the scenario has no route, so its trajectory has no end to be near"
                        .to_string(),
                );
            };
            let interval_ns = (scenario.update_interval_s * 1e9).round() as i64;
            let slots = (route.duration_ns() as f64 * scenario.playback.speed.recip()
                / interval_ns as f64)
                .floor() as u64;
            // The last sample strictly before the end of the route (of the
            // lap, when the route loops).
            Ok(slots.saturating_sub(1))
        }
    }
}

/// One case, as a JSON line and as the short code of the summary.
fn run_case(
    name: &str,
    scenario: &Scenario,
    pipeline: &str,
    kind: &str,
    placement: &str,
    gap_s: i64,
) -> (String, String) {
    let interval_ns = (scenario.update_interval_s * 1e9).round() as i64;
    let mut head = format!(
        "\"scenario\":{},\"mode\":\"{:?}\",\"update_interval_s\":{},\"looping\":{},\"pipeline\":\"{pipeline}\",\"kind\":\"{kind}\",\"placement\":\"{placement}\",\"gap_s\":{gap_s},\"start_ns\":{T0}",
        text(name),
        scenario.mode,
        number(scenario.update_interval_s),
        scenario.playback.looping,
    );
    let index = match placement_index(placement, scenario) {
        Ok(index) => index,
        Err(reason) => {
            return (
                format!("{{{head},\"applicable\":false,\"reason\":{}", text(&reason)),
                "n/a".to_string(),
            )
        }
    };
    write!(
        head,
        ",\"applicable\":true,\"last_index_before_gap\":{index}"
    )
    .unwrap();
    if placement == "near_route_end" {
        let note = if scenario.playback.looping {
            "the route loops: this is the last sample before a lap closes, not an end"
        } else {
            "the last sample before the route's final point"
        };
        write!(head, ",\"placement_note\":{}", text(note)).unwrap();
    }

    let mut case = Case {
        pipeline: Pipeline::new(pipeline, scenario),
        last: None,
        run_before_gap: None,
        interval_ns,
        scenario,
    };
    if let Err(error) = case.pipeline.provider().start(Timestamp::from_nanos(T0)) {
        return (
            format!("{{{head},\"setup_error\":{}", error_json(&error)),
            "setup".to_string(),
        );
    }
    case.run_before_gap = case.pipeline.run_number();
    // On-time polling up to the gap.
    for _ in 0..=index {
        let Some(deadline) = case.pipeline.next_deadline() else {
            break;
        };
        let polled = case.poll(deadline.as_nanos());
        if polled.code != "S" {
            return (
                format!(
                    "{{{head},\"setup_error\":\"a poll before the gap did not return a sample\",\"poll\":{}",
                    polled.json
                ),
                "setup".to_string(),
            );
        }
    }
    let last = case.last.expect("at least one sample was taken");
    write!(
        head,
        ",\"last_before_gap\":{},\"before_gap\":{}",
        sample_json(&last, None, case.run_before_gap),
        case.pipeline.state_json()
    )
    .unwrap();

    let gap_ns = gap_s * SEC;
    let first_poll_at = if kind == "pause_resume" {
        let pause_at = last.timestamp.as_nanos() + case.interval_ns / 2;
        let resume_at = pause_at + gap_ns;
        let paused = case
            .pipeline
            .provider()
            .pause(Timestamp::from_nanos(pause_at));
        let resumed = case
            .pipeline
            .provider()
            .resume(Timestamp::from_nanos(resume_at));
        let deadline = case.pipeline.next_deadline().map(|d| d.as_nanos());
        let outcome = |result: &Result<(), ProviderError>| match result {
            Ok(()) => "\"ok\"".to_string(),
            Err(error) => error_json(error),
        };
        write!(
            head,
            ",\"pause_at_ns\":{pause_at},\"pause_result\":{},\"resume_at_ns\":{resume_at},\"resume_result\":{},\"deadline_after_resume_ns\":{}",
            outcome(&paused),
            outcome(&resumed),
            optional(deadline),
        )
        .unwrap();
        // Polled when the resumed run says it is due. Without a deadline
        // the poll is made one interval after the resume, and says so.
        deadline.unwrap_or(resume_at + case.interval_ns)
    } else {
        let deadline = case.pipeline.next_deadline().map(|d| d.as_nanos());
        write!(head, ",\"deadline_before_gap_ns\":{}", optional(deadline)).unwrap();
        deadline.unwrap_or(last.timestamp.as_nanos() + case.interval_ns) + gap_ns
    };

    let first = case.poll(first_poll_at);
    let mut code = format!("{}|", first.code);
    let mut follow_ups = Vec::new();
    let mut previous_poll_at = first_poll_at;
    for _ in 0..FOLLOW_UP_POLLS {
        // The next deadline when there is one; otherwise one interval on,
        // to show what a caller that keeps polling gets.
        let at = case
            .pipeline
            .next_deadline()
            .map(|d| d.as_nanos())
            .filter(|at| *at > previous_poll_at)
            .unwrap_or(previous_poll_at + case.interval_ns);
        let polled = case.poll(at);
        code.push_str(&polled.code);
        follow_ups.push(polled.json);
        previous_poll_at = at;
    }
    let _ = case.scenario;
    (
        format!(
            "{{{head},\"first_poll_after_gap\":{},\"next_polls\":[{}]",
            first.json,
            follow_ups.join(",")
        ),
        code,
    )
}

pub struct Recording {
    /// One JSON object per line, one line per case.
    pub results: String,
    pub summary: String,
    pub cases: usize,
    pub repeat_mismatches: usize,
}

/// Runs every case twice and writes the two documents.
pub fn record(scenario_directory: &str) -> Recording {
    let mut results = String::new();
    let mut summary = String::from(
        "# Long-gap recording: summary\n\n\
         Written by `cargo run -p locsim-health --example gap_diagnostic`. Do not edit:\n\
         `tests/gap_recording.rs` compares this file with a fresh run. It records what\n\
         happened, not what should happen. The raw record of every case is `results.jsonl`.\n\n\
         A cell is `first poll after the gap|the next three polls`. `S` a sample of the\n\
         run that was running before the gap; `R` a sample of a later run (the supervisor\n\
         replaced the run); `N` nothing; `E(x)` the error `x`; `n/a` the placement does\n\
         not apply to the scenario.\n",
    );
    let mut cases = 0;
    let mut repeat_mismatches = 0;
    for kind in KINDS {
        for pipeline in PIPELINES {
            write!(
                summary,
                "\n## `{kind}`, `{pipeline}`\n\n| scenario | placement |"
            )
            .unwrap();
            for gap in GAPS_S {
                write!(summary, " {gap} s |").unwrap();
            }
            summary.push_str("\n|---|---|");
            summary.push_str(&"---|".repeat(GAPS_S.len()));
            summary.push('\n');
            for name in EXAMPLES {
                let document = std::fs::read_to_string(format!("{scenario_directory}/{name}.json"))
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                let scenario = locsim_scenario::import_scenario(&document)
                    .unwrap_or_else(|e| panic!("{name}: {e:?}"));
                for placement in PLACEMENTS {
                    write!(summary, "| {name} | {placement} |").unwrap();
                    for gap in GAPS_S {
                        let (first, code) =
                            run_case(name, &scenario, pipeline, kind, placement, gap);
                        let (second, _) = run_case(name, &scenario, pipeline, kind, placement, gap);
                        let identical = first == second;
                        cases += 1;
                        repeat_mismatches += usize::from(!identical);
                        writeln!(results, "{first},\"repeat_identical\":{identical}}}").unwrap();
                        write!(
                            summary,
                            " `{code}`{} |",
                            if identical {
                                ""
                            } else {
                                " (differs on repeat)"
                            }
                        )
                        .unwrap();
                    }
                    summary.push('\n');
                }
            }
        }
    }
    write!(
        summary,
        "\n{cases} cases, each run twice; {repeat_mismatches} gave a different record the second time.\n"
    )
    .unwrap();
    Recording {
        results,
        summary,
        cases,
        repeat_mismatches,
    }
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let (Some(scenarios), Some(output)) = (arguments.next(), arguments.next()) else {
        eprintln!("usage: gap_diagnostic <scenario directory> <output directory>");
        std::process::exit(2);
    };
    let recording = record(&scenarios);
    std::fs::create_dir_all(&output).expect("the output directory");
    std::fs::write(format!("{output}/results.jsonl"), &recording.results).expect("results.jsonl");
    std::fs::write(format!("{output}/summary.md"), &recording.summary).expect("summary.md");
    println!(
        "{} cases written to {output}; {} differed on repeat",
        recording.cases, recording.repeat_mismatches
    );
}
