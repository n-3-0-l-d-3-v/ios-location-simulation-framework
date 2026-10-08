use super::error::ConfigError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MovementMode {
    Fixed,
    RandomWalk,
    Walking,
    Driving,
    Circular,
    RouteReplay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RotationDirection {
    Clockwise,
    CounterClockwise,
}

/// Kinematic limits and mode-specific settings.
///
/// There is deliberately no `Default`: every value is an explicit choice of
/// the scenario author, and [`MovementParameters::validate`] decides which
/// fields a given mode requires.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementParameters {
    pub min_speed_mps: f64,
    pub max_speed_mps: f64,
    pub max_acceleration_mps2: f64,
    pub max_deceleration_mps2: f64,
    /// Maximum rate of heading change, degrees per second.
    pub max_heading_rate_dps: f64,
    /// Boundary radius around the origin. Required for random walk and
    /// circular modes; optional geofence for walking and driving.
    pub radius_m: Option<f64>,
    /// Nominal distance per step (random walk).
    pub step_distance_m: Option<f64>,
    /// 0 = heading re-drawn every step, 1 = heading never changes.
    pub heading_persistence: f64,
    /// Per-second probability of starting a pause (walking/driving stops).
    pub pause_probability: f64,
    pub max_pause_s: f64,
    /// Angular velocity for circular mode, degrees per second.
    pub angular_velocity_dps: Option<f64>,
    pub direction: RotationDirection,
    /// Circular mode: bearing from the centre to the starting point, degrees.
    pub start_phase_deg: f64,
    /// Walking/driving: mean time a cruising speed is held before a new one
    /// is drawn from `[min_speed, max_speed]`. 0 keeps the first one forever.
    pub speed_change_interval_s: f64,
    /// Optional cap on the distance moved in one nominal update interval.
    /// It acts as an additional speed limit of `cap / update_interval`; see
    /// [`MovementParameters::effective_max_speed_mps`].
    pub max_displacement_per_sample_m: Option<f64>,
}

/// Relative headroom the movement models keep below every configured limit
/// (speed, acceleration, deceleration, heading rate, boundary radius).
///
/// The final validation gate is strict and recomputes each quantity from the
/// emitted samples with its own arithmetic, so a model that used a limit
/// exactly would be rejected by rounding alone. The largest rounding effects
/// measured in the test-suite are ~1e-9 relative; this leaves three orders
/// of magnitude. Configurations that sit closer than this to a limit of
/// their own (an orbit at exactly the maximum speed) are rejected up front.
pub const KINEMATIC_MARGIN: f64 = 1e-6;

fn finite_non_negative(field: &'static str, v: f64, errors: &mut Vec<ConfigError>) -> bool {
    if !v.is_finite() {
        errors.push(ConfigError::new(field, format!("must be finite, got {v}")));
        false
    } else if v < 0.0 {
        errors.push(ConfigError::new(field, format!("must be >= 0, got {v}")));
        false
    } else {
        true
    }
}

fn positive(field: &'static str, v: f64, errors: &mut Vec<ConfigError>) -> bool {
    if finite_non_negative(field, v, errors) {
        if v > 0.0 {
            return true;
        }
        errors.push(ConfigError::new(field, "must be > 0"));
    }
    false
}

fn unit_interval(field: &'static str, v: f64, errors: &mut Vec<ConfigError>) {
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        errors.push(ConfigError::new(
            field,
            format!("must be within [0, 1], got {v}"),
        ));
    }
}

fn required_positive(
    field: &'static str,
    v: Option<f64>,
    mode: MovementMode,
    errors: &mut Vec<ConfigError>,
) -> Option<f64> {
    match v {
        None => {
            errors.push(ConfigError::new(
                field,
                format!("required for mode {mode:?}"),
            ));
            None
        }
        Some(v) => positive(field, v, errors).then_some(v),
    }
}

impl MovementParameters {
    /// Pedestrian preset: ~3–6.5 km/h, gentle acceleration, quick turns,
    /// occasional short pauses. A starting point to edit, not a hidden default.
    pub fn walking_preset() -> Self {
        Self {
            min_speed_mps: 0.8,
            max_speed_mps: 1.8,
            max_acceleration_mps2: 0.8,
            max_deceleration_mps2: 1.2,
            max_heading_rate_dps: 60.0,
            radius_m: None,
            step_distance_m: None,
            heading_persistence: 0.9,
            pause_probability: 0.01,
            max_pause_s: 20.0,
            angular_velocity_dps: None,
            direction: RotationDirection::Clockwise,
            start_phase_deg: 0.0,
            speed_change_interval_s: 30.0,
            max_displacement_per_sample_m: None,
        }
    }

    /// Road-vehicle preset: 18–108 km/h, car-like acceleration and braking,
    /// slow heading changes, occasional longer stops.
    pub fn driving_preset() -> Self {
        Self {
            min_speed_mps: 5.0,
            max_speed_mps: 30.0,
            max_acceleration_mps2: 2.5,
            max_deceleration_mps2: 4.5,
            max_heading_rate_dps: 25.0,
            radius_m: None,
            step_distance_m: None,
            heading_persistence: 0.97,
            pause_probability: 0.005,
            max_pause_s: 45.0,
            angular_velocity_dps: None,
            direction: RotationDirection::Clockwise,
            start_phase_deg: 0.0,
            speed_change_interval_s: 60.0,
            max_displacement_per_sample_m: None,
        }
    }

    /// The speed limit that actually applies: `max_speed_mps`, further
    /// reduced by `max_displacement_per_sample_m / update_interval_s` when a
    /// displacement cap is set. Between two samples separated by `dt` the
    /// true position may therefore move at most `effective_max_speed × dt`
    /// (after missed ticks or a pause `dt` spans several intervals and the
    /// allowance scales with it).
    pub fn effective_max_speed_mps(&self, update_interval_s: f64) -> f64 {
        match self.max_displacement_per_sample_m {
            Some(cap) => self.max_speed_mps.min(cap / update_interval_s),
            None => self.max_speed_mps,
        }
    }

    /// Appends every problem found for `mode` to `errors`.
    pub fn validate(&self, mode: MovementMode, errors: &mut Vec<ConfigError>) {
        use MovementMode::*;

        let min_ok = finite_non_negative("movement.min_speed_mps", self.min_speed_mps, errors);
        let max_ok = finite_non_negative("movement.max_speed_mps", self.max_speed_mps, errors);
        if min_ok && max_ok && self.min_speed_mps > self.max_speed_mps {
            errors.push(ConfigError::new(
                "movement.min_speed_mps",
                format!(
                    "min speed {} exceeds max speed {}",
                    self.min_speed_mps, self.max_speed_mps
                ),
            ));
        }
        finite_non_negative(
            "movement.max_acceleration_mps2",
            self.max_acceleration_mps2,
            errors,
        );
        finite_non_negative(
            "movement.max_deceleration_mps2",
            self.max_deceleration_mps2,
            errors,
        );
        finite_non_negative(
            "movement.max_heading_rate_dps",
            self.max_heading_rate_dps,
            errors,
        );
        finite_non_negative("movement.max_pause_s", self.max_pause_s, errors);
        unit_interval(
            "movement.heading_persistence",
            self.heading_persistence,
            errors,
        );
        unit_interval("movement.pause_probability", self.pause_probability, errors);
        // Optional fields must be valid whenever present, required or not.
        if let Some(r) = self.radius_m {
            positive("movement.radius_m", r, errors);
        }
        if let Some(s) = self.step_distance_m {
            positive("movement.step_distance_m", s, errors);
        }
        if let Some(w) = self.angular_velocity_dps {
            positive("movement.angular_velocity_dps", w, errors);
        }
        if let Some(cap) = self.max_displacement_per_sample_m {
            positive("movement.max_displacement_per_sample_m", cap, errors);
        }
        finite_non_negative(
            "movement.speed_change_interval_s",
            self.speed_change_interval_s,
            errors,
        );
        if !self.start_phase_deg.is_finite() {
            errors.push(ConfigError::new(
                "movement.start_phase_deg",
                "must be finite",
            ));
        }

        let moving = matches!(mode, RandomWalk | Walking | Driving | Circular);
        if moving && max_ok && self.max_speed_mps == 0.0 {
            errors.push(ConfigError::new(
                "movement.max_speed_mps",
                format!("must be > 0 for mode {mode:?}"),
            ));
        }
        if mode == RandomWalk {
            if self.radius_m.is_none() {
                required_positive("movement.radius_m", None, mode, errors);
            }
            if self.step_distance_m.is_none() {
                required_positive("movement.step_distance_m", None, mode, errors);
            }
        }
        match mode {
            Fixed | RouteReplay => {}
            // Every self-propelled model starts from rest and steers, so it
            // needs non-zero dynamics.
            RandomWalk | Walking | Driving => {
                if self.max_acceleration_mps2 == 0.0 {
                    errors.push(ConfigError::new(
                        "movement.max_acceleration_mps2",
                        "must be > 0",
                    ));
                }
                if self.max_deceleration_mps2 == 0.0 {
                    errors.push(ConfigError::new(
                        "movement.max_deceleration_mps2",
                        "must be > 0",
                    ));
                }
                if self.max_heading_rate_dps == 0.0 {
                    errors.push(ConfigError::new(
                        "movement.max_heading_rate_dps",
                        "must be > 0",
                    ));
                }
            }
            Circular => {
                let radius = match self.radius_m {
                    None => required_positive("movement.radius_m", None, mode, errors),
                    Some(r) => (r.is_finite() && r > 0.0).then_some(r),
                };
                let omega = match self.angular_velocity_dps {
                    None => required_positive("movement.angular_velocity_dps", None, mode, errors),
                    Some(w) => (w.is_finite() && w > 0.0).then_some(w),
                };
                if let (Some(r), Some(w), true) = (radius, omega, max_ok) {
                    let linear = r * w.to_radians();
                    if linear > self.max_speed_mps * (1.0 - KINEMATIC_MARGIN) {
                        errors.push(ConfigError::new(
                            "movement.angular_velocity_dps",
                            format!(
                                "orbit speed {linear:.3} m/s exceeds max speed {} m/s",
                                self.max_speed_mps
                            ),
                        ));
                    }
                }
                // An orbit turns at its angular velocity.
                if let Some(w) = omega {
                    let limit = self.max_heading_rate_dps;
                    if limit.is_finite() && w > limit * (1.0 - KINEMATIC_MARGIN) {
                        errors.push(ConfigError::new(
                            "movement.max_heading_rate_dps",
                            format!(
                                "orbit turns at {w} deg/s, above the heading-rate limit {limit}"
                            ),
                        ));
                    }
                }
            }
        }
    }
}

/// Gaussian noise components are clipped at this many standard deviations,
/// which is what makes every noise channel strictly bounded.
pub const NOISE_CLIP_SIGMA: f64 = 3.0;

/// Measurement-noise configuration. Standard deviations are 1-σ; a value of
/// zero disables that component.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseParameters {
    /// High-frequency Gaussian position jitter (metres, per axis).
    pub position_noise_m: f64,
    /// Hard bound on total position offset (jitter + drift) from the true point.
    pub max_position_offset_m: f64,
    /// Observation noise on the reported speed: added to the speed derived
    /// from the emitted positions, clipped at ±3 σ. Not applied to a
    /// stationary sample.
    pub speed_noise_mps: f64,
    /// Observation noise on the reported course: added to the course derived
    /// from the emitted positions, clipped at ±3 σ.
    pub heading_noise_deg: f64,
    /// Noise on the reported accuracies around their configured values,
    /// clipped at ±3 σ. Accuracy is never derived from anything else.
    pub accuracy_noise_m: f64,
    /// Low-frequency drift speed (metres per second).
    pub drift_rate_mps: f64,
    /// Correlation time of the jitter and of the speed/heading/accuracy
    /// noise (first-order Gauss–Markov). 0 = uncorrelated from sample to sample.
    pub position_correlation_time_s: f64,
    /// Hard limit on how fast the position offset may change (m/s): the
    /// offset moves by at most this times `dt` between two samples, so the
    /// noisy output never moves more than
    /// `true displacement + max_offset_rate_mps × dt`. The one exception is
    /// an output that would leave the scenario boundary: it is pulled back
    /// onto the boundary however far the offset must change.
    pub max_offset_rate_mps: f64,
}

impl NoiseParameters {
    /// No noise at all: output equals the movement model exactly.
    pub const NONE: NoiseParameters = NoiseParameters {
        position_noise_m: 0.0,
        max_position_offset_m: 0.0,
        speed_noise_mps: 0.0,
        heading_noise_deg: 0.0,
        accuracy_noise_m: 0.0,
        drift_rate_mps: 0.0,
        position_correlation_time_s: 0.0,
        max_offset_rate_mps: 0.0,
    };

    /// Whether any component moves the reported position.
    pub fn has_position_noise(&self) -> bool {
        self.position_noise_m > 0.0 || self.drift_rate_mps > 0.0
    }

    pub fn validate(&self, errors: &mut Vec<ConfigError>) {
        let pos = finite_non_negative("noise.position_noise_m", self.position_noise_m, errors);
        let bound = finite_non_negative(
            "noise.max_position_offset_m",
            self.max_position_offset_m,
            errors,
        );
        finite_non_negative("noise.speed_noise_mps", self.speed_noise_mps, errors);
        finite_non_negative("noise.heading_noise_deg", self.heading_noise_deg, errors);
        finite_non_negative("noise.accuracy_noise_m", self.accuracy_noise_m, errors);
        let drift = finite_non_negative("noise.drift_rate_mps", self.drift_rate_mps, errors);
        finite_non_negative(
            "noise.position_correlation_time_s",
            self.position_correlation_time_s,
            errors,
        );
        let rate = finite_non_negative(
            "noise.max_offset_rate_mps",
            self.max_offset_rate_mps,
            errors,
        );
        if !(pos && bound && drift && rate) || !self.has_position_noise() {
            return;
        }
        if self.max_position_offset_m == 0.0 {
            errors.push(ConfigError::new(
                "noise.max_position_offset_m",
                "must be > 0 when position noise or drift is enabled",
            ));
        }
        if self.max_offset_rate_mps == 0.0 {
            errors.push(ConfigError::new(
                "noise.max_offset_rate_mps",
                "must be > 0 when position noise or drift is enabled",
            ));
        }
        if self.drift_rate_mps > 0.0 {
            if self.max_offset_rate_mps > 0.0 && self.drift_rate_mps > self.max_offset_rate_mps {
                errors.push(ConfigError::new(
                    "noise.drift_rate_mps",
                    format!(
                        "drift rate {} m/s exceeds max offset rate {} m/s",
                        self.drift_rate_mps, self.max_offset_rate_mps
                    ),
                ));
            }
            let jitter_reach = NOISE_CLIP_SIGMA * self.position_noise_m;
            if self.max_position_offset_m > 0.0 && self.max_position_offset_m <= jitter_reach {
                errors.push(ConfigError::new(
                    "noise.max_position_offset_m",
                    format!(
                        "leaves no room for drift: must exceed {NOISE_CLIP_SIGMA} x position noise                          ({jitter_reach} m)"
                    ),
                ));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn walking() -> MovementParameters {
        MovementParameters {
            min_speed_mps: 0.8,
            max_speed_mps: 1.8,
            max_acceleration_mps2: 0.8,
            max_deceleration_mps2: 1.2,
            max_heading_rate_dps: 45.0,
            radius_m: None,
            step_distance_m: None,
            heading_persistence: 0.9,
            pause_probability: 0.01,
            max_pause_s: 20.0,
            angular_velocity_dps: None,
            direction: RotationDirection::Clockwise,
            start_phase_deg: 0.0,
            speed_change_interval_s: 30.0,
            max_displacement_per_sample_m: None,
        }
    }

    #[test]
    fn presets_are_valid_and_driving_is_substantially_faster() {
        let (w, d) = (
            MovementParameters::walking_preset(),
            MovementParameters::driving_preset(),
        );
        assert!(errors_for(w, MovementMode::Walking).is_empty());
        assert!(errors_for(d, MovementMode::Driving).is_empty());
        assert!(d.max_speed_mps > 10.0 * w.max_speed_mps);
        assert!(d.max_acceleration_mps2 > 3.0 * w.max_acceleration_mps2);
        assert!(d.max_deceleration_mps2 > 3.0 * w.max_deceleration_mps2);
    }

    #[test]
    fn displacement_cap_lowers_the_effective_speed_limit() {
        let mut p = walking();
        assert_eq!(p.effective_max_speed_mps(0.5), 1.8);
        p.max_displacement_per_sample_m = Some(0.5);
        assert_eq!(p.effective_max_speed_mps(0.5), 1.0);
        assert_eq!(p.effective_max_speed_mps(0.1), 1.8);
    }

    #[test]
    fn new_fields_are_validated() {
        let p = MovementParameters {
            max_displacement_per_sample_m: Some(0.0),
            speed_change_interval_s: -1.0,
            start_phase_deg: f64::NAN,
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::Walking),
            [
                "movement.max_displacement_per_sample_m",
                "movement.speed_change_interval_s",
                "movement.start_phase_deg"
            ]
        );
    }

    #[test]
    fn random_walk_needs_dynamics_too() {
        let p = MovementParameters {
            radius_m: Some(25.0),
            step_distance_m: Some(1.0),
            max_acceleration_mps2: 0.0,
            max_heading_rate_dps: 0.0,
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::RandomWalk),
            [
                "movement.max_acceleration_mps2",
                "movement.max_heading_rate_dps"
            ]
        );
    }

    #[test]
    fn orbit_must_fit_the_heading_rate_limit() {
        let p = MovementParameters {
            radius_m: Some(1.0),
            angular_velocity_dps: Some(50.0),
            max_heading_rate_dps: 45.0,
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::Circular),
            ["movement.max_heading_rate_dps"]
        );
    }

    fn errors_for(p: MovementParameters, mode: MovementMode) -> Vec<&'static str> {
        let mut e = Vec::new();
        p.validate(mode, &mut e);
        e.into_iter().map(|e| e.field).collect()
    }

    #[test]
    fn valid_walking_has_no_errors() {
        assert!(errors_for(walking(), MovementMode::Walking).is_empty());
        assert!(errors_for(walking(), MovementMode::Driving).is_empty());
        assert!(errors_for(walking(), MovementMode::Fixed).is_empty());
    }

    #[test]
    fn rejects_bad_speeds() {
        let p = MovementParameters {
            min_speed_mps: -1.0,
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::Walking),
            ["movement.min_speed_mps"]
        );
        let p = MovementParameters {
            max_speed_mps: f64::NAN,
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::Walking),
            ["movement.max_speed_mps"]
        );
        let p = MovementParameters {
            min_speed_mps: 3.0,
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::Walking),
            ["movement.min_speed_mps"]
        );
        let p = MovementParameters {
            min_speed_mps: 0.0,
            max_speed_mps: 0.0,
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::Walking),
            ["movement.max_speed_mps"]
        );
        assert!(errors_for(p, MovementMode::Fixed).is_empty());
    }

    #[test]
    fn walking_needs_dynamics() {
        let p = MovementParameters {
            max_acceleration_mps2: 0.0,
            max_deceleration_mps2: 0.0,
            max_heading_rate_dps: 0.0,
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::Driving),
            [
                "movement.max_acceleration_mps2",
                "movement.max_deceleration_mps2",
                "movement.max_heading_rate_dps"
            ]
        );
    }

    #[test]
    fn random_walk_requirements() {
        assert_eq!(
            errors_for(walking(), MovementMode::RandomWalk),
            ["movement.radius_m", "movement.step_distance_m"]
        );
        let p = MovementParameters {
            radius_m: Some(25.0),
            step_distance_m: Some(1.0),
            ..walking()
        };
        assert!(errors_for(p, MovementMode::RandomWalk).is_empty());
        let p = MovementParameters {
            radius_m: Some(-5.0),
            step_distance_m: Some(0.0),
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::RandomWalk),
            ["movement.radius_m", "movement.step_distance_m"]
        );
    }

    #[test]
    fn circular_requirements_and_orbit_speed() {
        assert_eq!(
            errors_for(walking(), MovementMode::Circular),
            ["movement.radius_m", "movement.angular_velocity_dps"]
        );
        // 50 m at 1°/s is 0.87 m/s: fine. At 10°/s it is 8.7 m/s: too fast.
        let ok = MovementParameters {
            radius_m: Some(50.0),
            angular_velocity_dps: Some(1.0),
            ..walking()
        };
        assert!(errors_for(ok, MovementMode::Circular).is_empty());
        let fast = MovementParameters {
            angular_velocity_dps: Some(10.0),
            ..ok
        };
        assert_eq!(
            errors_for(fast, MovementMode::Circular),
            ["movement.angular_velocity_dps"]
        );
    }

    #[test]
    fn probabilities_must_be_unit_interval() {
        let p = MovementParameters {
            heading_persistence: 1.5,
            pause_probability: f64::NAN,
            ..walking()
        };
        assert_eq!(
            errors_for(p, MovementMode::Walking),
            ["movement.heading_persistence", "movement.pause_probability"]
        );
    }

    #[test]
    fn noise_validation() {
        let mut e = Vec::new();
        NoiseParameters::NONE.validate(&mut e);
        assert!(e.is_empty());

        let unbounded = NoiseParameters {
            position_noise_m: 2.0,
            ..NoiseParameters::NONE
        };
        unbounded.validate(&mut e);
        let fields: Vec<_> = e.iter().map(|e| e.field).collect();
        assert_eq!(
            fields,
            ["noise.max_position_offset_m", "noise.max_offset_rate_mps"]
        );

        e.clear();
        let jitter = NoiseParameters {
            max_position_offset_m: 10.0,
            max_offset_rate_mps: 5.0,
            position_correlation_time_s: 4.0,
            ..unbounded
        };
        assert!(jitter.has_position_noise());
        jitter.validate(&mut e);
        assert!(e.is_empty(), "{e:?}");

        // Drift faster than the offset may move, and no room left beside jitter.
        let drift = NoiseParameters {
            drift_rate_mps: 6.0,
            max_position_offset_m: 6.0,
            ..jitter
        };
        drift.validate(&mut e);
        let fields: Vec<_> = e.iter().map(|e| e.field).collect();
        assert_eq!(
            fields,
            ["noise.drift_rate_mps", "noise.max_position_offset_m"]
        );

        e.clear();
        let bad_time = NoiseParameters {
            position_correlation_time_s: -1.0,
            max_offset_rate_mps: f64::NAN,
            ..NoiseParameters::NONE
        };
        bad_time.validate(&mut e);
        let fields: Vec<_> = e.iter().map(|e| e.field).collect();
        assert_eq!(
            fields,
            [
                "noise.position_correlation_time_s",
                "noise.max_offset_rate_mps"
            ]
        );

        e.clear();
        let bad = NoiseParameters {
            speed_noise_mps: -1.0,
            heading_noise_deg: f64::INFINITY,
            ..NoiseParameters::NONE
        };
        bad.validate(&mut e);
        let fields: Vec<_> = e.iter().map(|e| e.field).collect();
        assert_eq!(fields, ["noise.speed_noise_mps", "noise.heading_noise_deg"]);
    }
}
