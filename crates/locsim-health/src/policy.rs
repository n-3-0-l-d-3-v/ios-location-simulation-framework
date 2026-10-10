//! What the supervisor is allowed to do and when it calls something wrong:
//! the retry bound, the backoff, and the watchdog thresholds.
//!
//! Every value is an explicit choice. There is no default policy, for the
//! same reason the scenario has none.
//!
//! # Time
//!
//! Durations are given in seconds and converted **once**, when the policy
//! is accepted, to whole nanoseconds by `(seconds × 1e9).round()`: the rule
//! the core uses for the update interval. A value whose nanoseconds do not
//! fit an `i64` (more than about 292 years) is refused then, so nothing
//! done with these durations later can overflow in conversion. All later
//! arithmetic on times is on integers.

use std::fmt;

/// A rejected policy value.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyError {
    pub field: &'static str,
    pub reason: String,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

impl std::error::Error for PolicyError {}

/// The supervisor's configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HealthPolicy {
    /// How many automatic restarts may be attempted in one failure episode
    /// before the supervisor gives up. An episode begins at a failure and
    /// ends when a restarted run has proved stable, when the supervisor
    /// fails for good, or when it is stopped. `0` means a failure is final.
    ///
    /// This counts restart *attempts*, not runs: with `3`, at most four
    /// runs exist in an episode, the original and three restarts.
    pub max_recovery_attempts: u32,
    /// Delay before the first restart attempt of an episode. `0` means the
    /// attempt is made at the next call.
    pub backoff_initial_s: f64,
    /// Factor applied to the delay after each attempt. At least 1.
    pub backoff_multiplier: f64,
    /// Upper limit of the delay. At least `backoff_initial_s`.
    pub backoff_max_s: f64,
    /// How long a restarted run must have been emitting (from its first
    /// emitted sample to a later one, by their timestamps) before the
    /// episode is over and the attempt count returns to zero. `0` means at
    /// its first sample.
    pub stable_after_s: f64,
    /// A running provider that has emitted nothing for longer than this is
    /// reported as stalled. Measured between the times passed to the
    /// supervisor's calls. It must exceed the provider's update interval,
    /// or a healthy provider would look stalled between two samples;
    /// `Supervisor::for_simulation` checks that, [`HealthPolicy::validate`]
    /// cannot.
    pub stall_after_s: f64,
    /// Length, in tick slots, of the window over which missed ticks are
    /// judged. At least 1.
    pub missed_window_ticks: u64,
    /// More missed ticks than this within one window is "falling behind".
    pub max_missed_in_window: u64,
}

/// Whole nanoseconds of a non-negative duration in seconds, if it is
/// finite, not negative, and representable.
pub(crate) fn seconds_to_nanos(seconds: f64) -> Option<i64> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    let nanos = (seconds * 1e9).round();
    // 2^63 is the first value that does not fit.
    (nanos < i64::MAX as f64).then_some(nanos as i64)
}

fn duration(field: &'static str, seconds: f64, errors: &mut Vec<PolicyError>) -> Option<i64> {
    let reason = if seconds.is_nan() {
        "must be a number, got NaN".to_string()
    } else if seconds.is_infinite() {
        format!("must be finite, got {seconds}")
    } else if seconds < 0.0 {
        format!("must be >= 0, got {seconds}")
    } else if let Some(nanos) = seconds_to_nanos(seconds) {
        return Some(nanos);
    } else {
        format!("{seconds} s is not representable in nanoseconds")
    };
    errors.push(PolicyError { field, reason });
    None
}

impl HealthPolicy {
    /// Checks every field and reports every problem, not only the first.
    pub fn validate(&self) -> Result<(), Vec<PolicyError>> {
        self.limits().map(|_| ())
    }

    /// [`HealthPolicy::validate`], and additionally that the stall
    /// threshold is longer than the interval at which samples are due.
    pub fn validate_for_interval(&self, update_interval_s: f64) -> Result<(), Vec<PolicyError>> {
        let mut errors = self.limits().err().unwrap_or_default();
        // Written so that a NaN on either side is not accepted.
        let longer = self.stall_after_s > update_interval_s;
        if !longer && self.stall_after_s.is_finite() && self.stall_after_s > 0.0 {
            errors.push(PolicyError {
                field: "stall_after_s",
                reason: format!(
                    "{} s is not longer than the update interval, {update_interval_s} s; a \
                     provider that is on time would be reported as stalled",
                    self.stall_after_s
                ),
            });
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// The policy in the form the supervisor works with.
    pub(crate) fn limits(&self) -> Result<Limits, Vec<PolicyError>> {
        let mut errors = Vec::new();
        let initial = duration("backoff_initial_s", self.backoff_initial_s, &mut errors);
        let max = duration("backoff_max_s", self.backoff_max_s, &mut errors);
        if let (Some(_), Some(_)) = (initial, max) {
            if self.backoff_max_s < self.backoff_initial_s {
                errors.push(PolicyError {
                    field: "backoff_max_s",
                    reason: format!(
                        "{} s is below backoff_initial_s, {} s",
                        self.backoff_max_s, self.backoff_initial_s
                    ),
                });
            }
        }
        let m = self.backoff_multiplier;
        if !m.is_finite() || m < 1.0 {
            errors.push(PolicyError {
                field: "backoff_multiplier",
                reason: format!("must be finite and >= 1, got {m}"),
            });
        }
        let stable = duration("stable_after_s", self.stable_after_s, &mut errors);
        let stall = duration("stall_after_s", self.stall_after_s, &mut errors);
        if stall == Some(0) {
            errors.push(PolicyError {
                field: "stall_after_s",
                reason: format!(
                    "must be at least one nanosecond, got {} s",
                    self.stall_after_s
                ),
            });
        }
        if self.missed_window_ticks == 0 {
            errors.push(PolicyError {
                field: "missed_window_ticks",
                reason: "must be at least 1".into(),
            });
        }
        match (stable, stall, errors.is_empty()) {
            (Some(stable_after_ns), Some(stall_after_ns), true) => Ok(Limits {
                max_recovery_attempts: self.max_recovery_attempts,
                // Negative zero is zero.
                backoff_initial_s: self.backoff_initial_s + 0.0,
                backoff_multiplier: self.backoff_multiplier,
                backoff_max_s: self.backoff_max_s + 0.0,
                stable_after_ns,
                stall_after_ns,
                missed_window_ticks: self.missed_window_ticks,
                max_missed_in_window: self.max_missed_in_window,
            }),
            _ => Err(errors),
        }
    }
}

/// A validated policy: durations in nanoseconds, backoff values known to be
/// finite, ordered and representable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Limits {
    pub(crate) max_recovery_attempts: u32,
    backoff_initial_s: f64,
    backoff_multiplier: f64,
    backoff_max_s: f64,
    pub(crate) stable_after_ns: i64,
    pub(crate) stall_after_ns: i64,
    pub(crate) missed_window_ticks: u64,
    pub(crate) max_missed_in_window: u64,
}

impl Limits {
    /// The backoff at the start of a failure episode.
    pub(crate) fn backoff(&self) -> Backoff {
        Backoff {
            next_s: self.backoff_initial_s,
            multiplier: self.backoff_multiplier,
            max_s: self.backoff_max_s,
        }
    }
}

/// The delay before each restart attempt of one failure episode.
///
/// The delay for the first attempt is the initial delay. After each attempt
/// the delay is multiplied by the multiplier and limited to the maximum:
///
/// ```text
/// d(1) = initial            d(k+1) = min(max, d(k) × multiplier)
/// ```
///
/// One multiplication and one comparison per attempt, whatever the number
/// of attempts already made: no loop and no power function. The arithmetic
/// is IEEE-754 multiplication only, so the sequence is the same everywhere.
/// It never decreases (the multiplier is at least 1 and rounding is
/// monotonic) and never exceeds the maximum (the limit is applied at every
/// step, which also disposes of a product that overflows to infinity).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Backoff {
    next_s: f64,
    multiplier: f64,
    max_s: f64,
}

impl Backoff {
    /// The delay, in nanoseconds, before the next attempt; then advances.
    pub(crate) fn take(&mut self) -> i64 {
        let delay_s = self.next_s;
        self.next_s = (delay_s * self.multiplier).min(self.max_s);
        // `delay_s` is finite, not negative and at most `max_s`, which was
        // checked to be representable; rounding is monotonic.
        seconds_to_nanos(delay_s).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    pub(crate) fn policy() -> HealthPolicy {
        HealthPolicy {
            max_recovery_attempts: 3,
            backoff_initial_s: 1.0,
            backoff_multiplier: 2.0,
            backoff_max_s: 60.0,
            stable_after_s: 30.0,
            stall_after_s: 5.0,
            missed_window_ticks: 20,
            max_missed_in_window: 4,
        }
    }

    fn fields(errors: &[PolicyError]) -> Vec<&'static str> {
        errors.iter().map(|e| e.field).collect()
    }

    #[test]
    fn a_sound_policy_is_accepted_and_converted_once() {
        let limits = policy().limits().unwrap();
        assert_eq!(limits.stable_after_ns, 30_000_000_000);
        assert_eq!(limits.stall_after_ns, 5_000_000_000);
        assert_eq!(limits.max_recovery_attempts, 3);
        assert_eq!(
            (limits.missed_window_ticks, limits.max_missed_in_window),
            (20, 4)
        );
        assert_eq!(policy().validate(), Ok(()));
    }

    #[test]
    fn every_duration_refuses_nan_infinities_and_negative_values() {
        type Set = fn(&mut HealthPolicy, f64);
        let durations: [(&str, Set); 4] = [
            ("backoff_initial_s", |p, v| p.backoff_initial_s = v),
            ("backoff_max_s", |p, v| p.backoff_max_s = v),
            ("stable_after_s", |p, v| p.stable_after_s = v),
            ("stall_after_s", |p, v| p.stall_after_s = v),
        ];
        for (name, set) in durations {
            for bad in [
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                -1.0,
                -f64::MIN_POSITIVE,
                -1e-12,
            ] {
                let mut p = policy();
                set(&mut p, bad);
                let errors = p.validate().expect_err(name);
                assert!(
                    fields(&errors).contains(&name),
                    "{name} = {bad}: {errors:?}"
                );
            }
        }
    }

    #[test]
    fn the_multiplier_must_be_finite_and_at_least_one() {
        for bad in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -2.0,
            0.0,
            0.5,
            0.9999999999999999,
        ] {
            let mut p = policy();
            p.backoff_multiplier = bad;
            assert_eq!(
                fields(&p.validate().unwrap_err()),
                ["backoff_multiplier"],
                "{bad}"
            );
        }
        for good in [1.0, 1.0000000000000002, 2.0, 1e300, f64::MAX] {
            let mut p = policy();
            p.backoff_multiplier = good;
            assert_eq!(p.validate(), Ok(()), "{good}");
        }
    }

    #[test]
    fn zero_is_accepted_where_it_has_a_meaning_and_nowhere_else() {
        let mut p = policy();
        p.backoff_initial_s = 0.0;
        p.backoff_max_s = 0.0;
        p.stable_after_s = 0.0;
        p.max_recovery_attempts = 0;
        p.max_missed_in_window = 0;
        assert_eq!(p.validate(), Ok(()));
        // Negative zero is zero.
        p.backoff_initial_s = -0.0;
        p.backoff_max_s = -0.0;
        p.stable_after_s = -0.0;
        let limits = p.limits().unwrap();
        assert_eq!(limits.stable_after_ns, 0);
        assert_eq!(limits.backoff().take(), 0);

        let mut p = policy();
        p.stall_after_s = 0.0;
        assert_eq!(fields(&p.validate().unwrap_err()), ["stall_after_s"]);
        // Too small to be one nanosecond: also no stall threshold at all.
        p.stall_after_s = 4e-10;
        assert_eq!(fields(&p.validate().unwrap_err()), ["stall_after_s"]);
        p.stall_after_s = 6e-10; // rounds to 1 ns
        assert_eq!(p.limits().unwrap().stall_after_ns, 1);

        let mut p = policy();
        p.missed_window_ticks = 0;
        assert_eq!(fields(&p.validate().unwrap_err()), ["missed_window_ticks"]);
    }

    #[test]
    fn a_duration_must_fit_in_nanoseconds() {
        // The largest double below 2^63 nanoseconds, in seconds, and the
        // first value that reaches it.
        // 2^63 ns is 9 223 372 036.854 775 808 s. Doubles are about two
        // microseconds apart there, so the edge is probed a millisecond to
        // either side rather than at the last digit.
        let largest = 9_223_372_036.0;
        assert_eq!(seconds_to_nanos(largest), Some(9_223_372_036_000_000_000));
        assert!(seconds_to_nanos(9_223_372_036.854).is_some());
        assert_eq!(seconds_to_nanos(9_223_372_036.856), None);
        assert_eq!(seconds_to_nanos(9_223_372_037.0), None);
        // Walk the doubles across the edge one at a time. Every value whose
        // nanoseconds reach 2^63 is refused, including one that lands on
        // 2^63 exactly, if there is one (a cast would turn it into
        // i64::MAX, a different number); everything below is accepted with
        // the value it has.
        let edge = i64::MAX as f64; // 2^63
        let mut x = 9_223_372_036.854f64;
        let (mut below, mut exactly_on, mut above) = (0, 0, 0);
        while x < 9_223_372_036.856 {
            let nanos = (x * 1e9).round();
            match seconds_to_nanos(x) {
                Some(v) => {
                    assert!(nanos < edge, "{x}");
                    assert_eq!(v as f64, nanos, "{x}");
                    below += 1;
                }
                None => {
                    assert!(nanos >= edge, "{x}");
                    exactly_on += usize::from(nanos == edge);
                    above += 1;
                }
            }
            x = f64::from_bits(x.to_bits() + 1);
        }
        println!(
            "edge of the range: {below} doubles below, {exactly_on} exactly on 2^63, {above} above"
        );
        assert!(below > 100 && above > 100);
        assert_eq!(seconds_to_nanos(1e300), None);

        let mut p = policy();
        p.stable_after_s = largest;
        p.stall_after_s = largest;
        p.backoff_max_s = largest;
        assert_eq!(p.validate(), Ok(()));
        p.stable_after_s = 9_223_372_037.0;
        p.backoff_max_s = 1e300;
        assert_eq!(
            fields(&p.validate().unwrap_err()),
            ["backoff_max_s", "stable_after_s"]
        );
    }

    #[test]
    fn conversion_rounds_to_the_nearest_nanosecond() {
        assert_eq!(seconds_to_nanos(0.0), Some(0));
        assert_eq!(seconds_to_nanos(1e-9), Some(1));
        assert_eq!(seconds_to_nanos(1.4e-9), Some(1));
        assert_eq!(seconds_to_nanos(1.6e-9), Some(2));
        // Powers of two times 1e9 are exact products, so these are known
        // without any assumption about rounding: 2^-31 s is 0.4657 ns,
        // 2^-30 s is 0.9313 ns, 3 × 2^-31 s is 1.3970 ns, 2^-29 s is 1.8626 ns.
        let unit = 1.0 / 2_147_483_648.0;
        assert_eq!(seconds_to_nanos(unit), Some(0));
        assert_eq!(seconds_to_nanos(2.0 * unit), Some(1));
        assert_eq!(seconds_to_nanos(3.0 * unit), Some(1));
        assert_eq!(seconds_to_nanos(4.0 * unit), Some(2));
        assert_eq!(seconds_to_nanos(0.1), Some(100_000_000));
        assert_eq!(seconds_to_nanos(1.0 / 3.0), Some(333_333_333));
    }

    #[test]
    fn the_maximum_delay_may_not_be_below_the_initial_one() {
        let mut p = policy();
        p.backoff_initial_s = 10.0;
        p.backoff_max_s = 9.999;
        assert_eq!(fields(&p.validate().unwrap_err()), ["backoff_max_s"]);
        p.backoff_max_s = 10.0;
        assert_eq!(p.validate(), Ok(()));
    }

    #[test]
    fn every_problem_is_reported_together() {
        let p = HealthPolicy {
            max_recovery_attempts: 3,
            backoff_initial_s: -1.0,
            backoff_multiplier: 0.5,
            backoff_max_s: f64::NAN,
            stable_after_s: f64::INFINITY,
            stall_after_s: 0.0,
            missed_window_ticks: 0,
            max_missed_in_window: 0,
        };
        assert_eq!(
            fields(&p.validate().unwrap_err()),
            [
                "backoff_initial_s",
                "backoff_max_s",
                "backoff_multiplier",
                "stable_after_s",
                "stall_after_s",
                "missed_window_ticks"
            ]
        );
    }

    #[test]
    fn the_stall_threshold_must_exceed_the_update_interval() {
        let p = policy(); // stall after 5 s
        assert_eq!(p.validate_for_interval(1.0), Ok(()));
        assert_eq!(p.validate_for_interval(4.999), Ok(()));
        for interval in [5.0, 5.001, 10.0, f64::INFINITY, f64::NAN] {
            assert_eq!(
                fields(&p.validate_for_interval(interval).unwrap_err()),
                ["stall_after_s"],
                "{interval}"
            );
        }
        // An invalid threshold is reported once, as invalid.
        let mut bad = policy();
        bad.stall_after_s = f64::NAN;
        assert_eq!(
            fields(&bad.validate_for_interval(1.0).unwrap_err()),
            ["stall_after_s"]
        );
    }

    // --- Backoff ----------------------------------------------------------------------

    fn backoff(initial: f64, multiplier: f64, max: f64) -> Backoff {
        let mut p = policy();
        p.backoff_initial_s = initial;
        p.backoff_multiplier = multiplier;
        p.backoff_max_s = max;
        p.limits().unwrap().backoff()
    }

    fn take(backoff: &mut Backoff, count: usize) -> Vec<i64> {
        (0..count).map(|_| backoff.take()).collect()
    }

    const SEC: i64 = 1_000_000_000;

    #[test]
    fn doubling_from_one_second_to_a_cap_of_sixty_is_exact() {
        // Powers of two are exact in binary floating point, so every value
        // here can be stated by hand.
        let mut b = backoff(1.0, 2.0, 60.0);
        assert_eq!(
            take(&mut b, 9),
            [1, 2, 4, 8, 16, 32, 60, 60, 60].map(|s| s * SEC)
        );
    }

    #[test]
    fn a_multiplier_of_one_and_an_initial_delay_of_zero_stay_put() {
        assert_eq!(take(&mut backoff(2.5, 1.0, 60.0), 5), [2_500_000_000; 5]);
        assert_eq!(take(&mut backoff(0.0, 3.0, 60.0), 5), [0; 5]);
        assert_eq!(take(&mut backoff(7.0, 4.0, 7.0), 3), [7 * SEC; 3]);
    }

    #[test]
    fn a_product_that_overflows_is_capped_not_infinite() {
        let mut b = backoff(1.0, f64::MAX, 9_000_000_000.0);
        assert_eq!(
            take(&mut b, 4),
            [
                SEC,
                9_000_000_000 * SEC,
                9_000_000_000 * SEC,
                9_000_000_000 * SEC
            ]
        );
        // Values near the top of the range.
        let mut b = backoff(9_223_372_036.0, 1e300, 9_223_372_036.0);
        assert_eq!(take(&mut b, 3), [9_223_372_036 * SEC; 3]);
    }

    #[test]
    fn sub_nanosecond_delays_round_to_the_nearest_nanosecond() {
        // Doubling from 2^-31 s. Every value is a power of two, exact:
        // 0.4657, 0.9313, 1.8626, 3.7253, 7.4506, 14.9012 ns.
        let mut b = backoff(1.0 / 2_147_483_648.0, 2.0, 1.0);
        assert_eq!(take(&mut b, 6), [0, 1, 2, 4, 7, 15]);
    }

    /// A million attempts with the smallest multiplier above 1. Nothing is
    /// assumed about how each product rounds: the checks are the properties
    /// the supervisor relies on, and agreement with the same recurrence
    /// written out again here.
    #[test]
    fn a_million_attempts_with_a_multiplier_just_above_one_stay_cheap_and_sound() {
        let multiplier = 1.0 + f64::EPSILON;
        let (initial, max) = (1.0, 1.000_000_000_1);
        let max_ns = seconds_to_nanos(max).unwrap();
        let mut b = backoff(initial, multiplier, max);
        let mut reference_s = initial;
        let mut previous = 0;
        let begun = Instant::now();
        for attempt in 0..1_000_000u32 {
            let delay = b.take();
            assert!((0..=max_ns).contains(&delay), "attempt {attempt}: {delay}");
            assert!(
                delay >= previous,
                "attempt {attempt}: {delay} after {previous}"
            );
            // The same recurrence, independently.
            assert_eq!(
                delay,
                (reference_s * 1e9).round() as i64,
                "attempt {attempt}"
            );
            assert!(reference_s.is_finite());
            reference_s = (reference_s * multiplier).min(max);
            previous = delay;
        }
        // Still in step with the reference after the last attempt.
        assert_eq!(b.take(), (reference_s * 1e9).round() as i64);
        // Constant work per attempt. A second is two orders of magnitude
        // more than this takes; a loop over the attempts would be hours.
        assert!(begun.elapsed().as_secs() < 1, "{:?}", begun.elapsed());
    }

    #[test]
    fn a_small_multiplier_never_goes_down_and_reaches_a_distant_cap() {
        for multiplier in [1.0 + f64::EPSILON, 1.000_001, 1.01, 1.5] {
            let mut b = backoff(0.001, multiplier, 3_600.0);
            let mut previous = 0;
            for attempt in 0..200_000 {
                let delay = b.take();
                assert!(delay >= previous, "{multiplier} attempt {attempt}");
                assert!(delay <= 3_600 * SEC);
                previous = delay;
            }
            if multiplier >= 1.000_1 {
                assert_eq!(previous, 3_600 * SEC, "{multiplier}");
            }
        }
    }
}
