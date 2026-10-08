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
}

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

        let moving = matches!(mode, RandomWalk | Walking | Driving | Circular);
        if moving && max_ok && self.max_speed_mps == 0.0 {
            errors.push(ConfigError::new(
                "movement.max_speed_mps",
                format!("must be > 0 for mode {mode:?}"),
            ));
        }
        match mode {
            Fixed | RouteReplay => {}
            RandomWalk => {
                if self.radius_m.is_none() {
                    required_positive("movement.radius_m", None, mode, errors);
                }
                if self.step_distance_m.is_none() {
                    required_positive("movement.step_distance_m", None, mode, errors);
                }
            }
            Walking | Driving => {
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
                    if linear > self.max_speed_mps {
                        errors.push(ConfigError::new(
                            "movement.angular_velocity_dps",
                            format!(
                                "orbit speed {linear:.3} m/s exceeds max speed {} m/s",
                                self.max_speed_mps
                            ),
                        ));
                    }
                }
            }
        }
    }
}

/// Measurement-noise configuration. Standard deviations are 1-σ; a value of
/// zero disables that component.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseParameters {
    /// High-frequency Gaussian position jitter (metres, per axis).
    pub position_noise_m: f64,
    /// Hard bound on total position offset (jitter + drift) from the true point.
    pub max_position_offset_m: f64,
    pub speed_noise_mps: f64,
    pub heading_noise_deg: f64,
    pub accuracy_noise_m: f64,
    /// Low-frequency drift speed (metres per second).
    pub drift_rate_mps: f64,
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
    };

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
        let has_position_noise = self.position_noise_m > 0.0 || self.drift_rate_mps > 0.0;
        if pos && bound && drift && has_position_noise && self.max_position_offset_m == 0.0 {
            errors.push(ConfigError::new(
                "noise.max_position_offset_m",
                "must be > 0 when position noise or drift is enabled",
            ));
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
        }
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
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].field, "noise.max_position_offset_m");

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
