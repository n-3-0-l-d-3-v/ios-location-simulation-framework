//! Independent re-check of an emitted stream, shared by the pipeline tests.

use locsim_core::domain::{Scenario, SyntheticLocation, NOISE_CLIP_SIGMA};
use locsim_core::geographic::{bearing_difference, inverse};

/// What a re-check of a stream found.
#[derive(Debug, Default, Clone, Copy)]
pub struct Recheck {
    /// Consecutive pairs examined.
    pub pairs: usize,
    /// Pairs whose later sample carries a course.
    pub with_course: usize,
    /// Pairs whose later sample is stationary (speed exactly zero).
    pub stationary: usize,
    /// Largest |reported speed − distance/elapsed|, m/s.
    pub max_speed_error_mps: f64,
    /// Largest |reported course − geodesic arrival bearing|, degrees.
    pub max_course_error_deg: f64,
}

impl Recheck {
    pub fn absorb(&mut self, other: Recheck) {
        self.pairs += other.pairs;
        self.with_course += other.with_course;
        self.stationary += other.stationary;
        self.max_speed_error_mps = self.max_speed_error_mps.max(other.max_speed_error_mps);
        self.max_course_error_deg = self.max_course_error_deg.max(other.max_course_error_deg);
    }
}

/// Re-checks an emitted stream against its scenario, sample pair by sample
/// pair, without using the validation gate or the consistency engine.
///
/// Written out independently here on purpose: the constants and formulas are
/// restated, so a mistake in the library's own copy would show up as a
/// disagreement. Panics on the first violation.
///
/// Consistency: speed equals geodesic distance over elapsed time (exactly 0
/// below 4 nm), course equals the geodesic arrival bearing (absent below
/// 0.1 mm), both to within configured observation noise; the first sample
/// has neither; accuracy stays in its band. Physics: boundary, displacement,
/// change of speed judged between interval midpoints, change of course over
/// both intervals, with the allowances documented on the gate.
pub fn recheck_stream(sc: &Scenario, stream: &[SyntheticLocation], label: &str) -> Recheck {
    const DISTANCE_RESOLUTION_M: f64 = 4e-9;
    const MIN_COURSE_DISPLACEMENT_M: f64 = 1e-4;
    let m = &sc.movement;
    let speed_limit = sc.effective_max_speed_mps();
    let noise_rate = if sc.noise.has_position_noise() {
        sc.noise.max_offset_rate_mps
    } else {
        0.0
    };
    let speed_reach = NOISE_CLIP_SIGMA * sc.noise.speed_noise_mps;
    let heading_reach = NOISE_CLIP_SIGMA * sc.noise.heading_noise_deg;
    let accuracy_reach = NOISE_CLIP_SIGMA * sc.noise.accuracy_noise_m;
    let boundary = sc.boundary();
    let mut out = Recheck::default();

    let Some(first) = stream.first() else {
        return out;
    };
    assert_eq!(
        (first.speed_mps, first.course_deg),
        (None, None),
        "{label}: a first sample cannot know its speed or course"
    );

    // A fix that position noise had to hold at the fence has an unbounded
    // noise step; the speed-change and heading checks skip it and its
    // successor. The engine parks such a fix 1e-6·R + 1e-7 m inside.
    let held_at_fence = |s: &SyntheticLocation| match boundary {
        Some(fence) if noise_rate > 0.0 => {
            let from_centre = inverse(fence.center, s.coordinate).unwrap().distance_m;
            fence.radius_m - from_centre <= 2.0 * (1e-6 * fence.radius_m + 1e-7)
        }
        _ => false,
    };

    // (elapsed, distance) of the previous interval.
    let mut before: Option<(f64, f64)> = None;
    for (index, pair) in stream.windows(2).enumerate() {
        let (a, b) = (&pair[0], &pair[1]);
        let ctx = || format!("{label} sample {}: {b:?} after {a:?}", index + 1);
        assert_eq!(b.validate(), Ok(()), "{}", ctx());
        assert!(b.timestamp > a.timestamp, "{}", ctx());
        let dt = b.timestamp.seconds_since(a.timestamp);
        let line = inverse(a.coordinate, b.coordinate).unwrap();
        let d = line.distance_m;
        let (before_dt, before_d) = before.unwrap_or((dt, d));

        // Position: fence and teleportation.
        if let Some(fence) = boundary {
            let from_centre = inverse(fence.center, b.coordinate).unwrap().distance_m;
            assert!(
                from_centre <= fence.radius_m,
                "outside the fence: {}",
                ctx()
            );
        }
        assert!(
            d <= (speed_limit + noise_rate) * dt,
            "teleported {d} m: {}",
            ctx()
        );

        // Accuracy: configured value plus clipped noise, nothing else.
        for (reported, nominal) in [
            (b.horizontal_accuracy_m, sc.horizontal_accuracy_m),
            (b.vertical_accuracy_m, sc.vertical_accuracy_m),
        ] {
            assert!(
                reported > 0.0 && (reported - nominal).abs() <= accuracy_reach + 1e-12,
                "{}",
                ctx()
            );
        }

        // Speed describes these two positions and these two timestamps.
        let speed = b.speed_mps.unwrap_or_else(|| panic!("no speed: {}", ctx()));
        let expected_speed = if d < DISTANCE_RESOLUTION_M {
            0.0
        } else {
            d / dt
        };
        let speed_error = (speed - expected_speed).abs();
        if d < DISTANCE_RESOLUTION_M {
            assert_eq!(speed, 0.0, "stationary but moving: {}", ctx());
            out.stationary += 1;
        } else {
            assert!(
                speed_error <= speed_reach + 1e-12 * (expected_speed + speed_reach),
                "speed {speed} but {d} m in {dt} s is {expected_speed}: {}",
                ctx()
            );
        }
        out.max_speed_error_mps = out.max_speed_error_mps.max(speed_error);

        // Course describes the direction between these two positions.
        let expects_course = d >= MIN_COURSE_DISPLACEMENT_M && speed > 0.0;
        assert_eq!(
            b.course_deg.is_some(),
            expects_course,
            "course presence: {}",
            ctx()
        );
        if let Some(course) = b.course_deg {
            let error = bearing_difference(line.final_bearing_deg, course).abs();
            assert!(
                error <= heading_reach + 1e-9,
                "course {course} but the positions say {}: {}",
                line.final_bearing_deg,
                ctx()
            );
            out.max_course_error_deg = out.max_course_error_deg.max(error);
            out.with_course += 1;
        }

        // Physics of the reported values.
        assert!(
            speed <= (speed_limit + noise_rate) * (1.0 + 1e-12) + speed_reach,
            "{}",
            ctx()
        );
        let bounded_noise = !held_at_fence(a) && !held_at_fence(b);
        if let (Some(earlier), true) = (a.speed_mps, bounded_noise) {
            let longest = dt.max(before_dt);
            let half_turn = 0.5 * m.max_heading_rate_dps * longest;
            let chord_shortfall = if half_turn >= 90.0 {
                speed_limit
            } else {
                speed_limit * (1.0 - half_turn.to_radians().cos())
            };
            let allowance = chord_shortfall
                + 2.0 * noise_rate
                + 2.0 * speed_reach
                + DISTANCE_RESOLUTION_M / dt
                + DISTANCE_RESOLUTION_M / before_dt;
            let between_midpoints = 0.5 * (dt + before_dt);
            let change = speed - earlier;
            assert!(
                change <= m.max_acceleration_mps2 * between_midpoints + allowance,
                "accelerated by {change}: {}",
                ctx()
            );
            assert!(
                -change <= m.max_deceleration_mps2 * between_midpoints + allowance,
                "braked by {}: {}",
                -change,
                ctx()
            );
        }
        if let (Some(earlier), Some(later), true) = (a.course_deg, b.course_deg, bounded_noise) {
            let deflection = |distance: f64, elapsed: f64| {
                let step = noise_rate * elapsed;
                if step <= 0.0 {
                    0.0
                } else if distance <= 2.0 * step {
                    180.0
                } else {
                    (step / (distance - step)).asin().to_degrees()
                }
            };
            let limit = m.max_heading_rate_dps * (dt + before_dt)
                + deflection(d, dt)
                + deflection(before_d, before_dt)
                + 2.0 * heading_reach
                + (DISTANCE_RESOLUTION_M / d).to_degrees()
                + (DISTANCE_RESOLUTION_M / before_d).to_degrees();
            if limit < 180.0 {
                let turn = bearing_difference(earlier + line.convergence_deg, later).abs();
                assert!(turn <= limit, "turned {turn} > {limit}: {}", ctx());
            }
        }
        before = Some((dt, d));
        out.pairs += 1;
    }
    out
}
