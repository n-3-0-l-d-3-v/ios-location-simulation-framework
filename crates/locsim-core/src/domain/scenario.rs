use super::error::ConfigError;
use super::params::{
    MovementMode, MovementParameters, NoiseParameters, KINEMATIC_MARGIN, NOISE_CLIP_SIGMA,
};
use super::time::Timestamp;
use crate::geographic::{self, Coordinate};

/// Version of the scenario schema this build reads and writes.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeedRange {
    pub min_mps: f64,
    pub max_mps: f64,
}

/// A circular geofence: every emitted position must lie within `radius_m`
/// (geodesic) of `center`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Boundary {
    pub center: Coordinate,
    pub radius_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutePoint {
    pub timestamp: Timestamp,
    pub coordinate: Coordinate,
}

/// A recorded track: at least two points with strictly increasing timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    points: Vec<RoutePoint>,
}

impl Route {
    pub fn new(points: Vec<RoutePoint>) -> Result<Self, ConfigError> {
        if points.len() < 2 {
            return Err(ConfigError::new(
                "route.points",
                format!("need at least 2 points, got {}", points.len()),
            ));
        }
        if let Some(i) = points
            .windows(2)
            .position(|w| w[1].timestamp <= w[0].timestamp)
        {
            return Err(ConfigError::new(
                "route.points",
                format!(
                    "timestamps must strictly increase (violated at index {})",
                    i + 1
                ),
            ));
        }
        Ok(Self { points })
    }

    pub fn points(&self) -> &[RoutePoint] {
        &self.points
    }

    pub fn duration_s(&self) -> f64 {
        let (first, last) = (self.points[0], self.points[self.points.len() - 1]);
        last.timestamp.seconds_since(first.timestamp)
    }

    /// Highest point-to-point speed implied by the recording, in m/s.
    /// Fails if a segment's geodesic cannot be computed (antipodal jump).
    pub fn max_segment_speed_mps(&self) -> Result<f64, ConfigError> {
        let mut max = 0.0f64;
        for (i, w) in self.points.windows(2).enumerate() {
            let dt = w[1].timestamp.seconds_since(w[0].timestamp);
            let d = geographic::distance(w[0].coordinate, w[1].coordinate).map_err(|e| {
                ConfigError::new(
                    "route.points",
                    format!("segment {i} is not computable: {e}"),
                )
            })?;
            max = max.max(d / dt);
        }
        Ok(max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackParameters {
    /// Multiplier on recorded time; 1 = real time.
    pub speed: f64,
    pub looping: bool,
    pub reverse: bool,
}

impl PlaybackParameters {
    /// Real-time, forward, single pass.
    pub const REAL_TIME: PlaybackParameters = PlaybackParameters {
        speed: 1.0,
        looping: false,
        reverse: false,
    };
}

/// A complete, self-contained description of one simulation run.
/// Together with a start timestamp it fully determines the sample stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    pub schema_version: u32,
    pub name: String,
    pub origin: Coordinate,
    pub altitude_m: f64,
    pub mode: MovementMode,
    pub movement: MovementParameters,
    pub noise: NoiseParameters,
    /// Reported accuracy before accuracy noise is applied.
    pub horizontal_accuracy_m: f64,
    pub vertical_accuracy_m: f64,
    /// Seconds between samples.
    pub update_interval_s: f64,
    pub seed: u64,
    pub route: Option<Route>,
    pub playback: PlaybackParameters,
}

impl Scenario {
    pub fn speed_range(&self) -> SpeedRange {
        SpeedRange {
            min_mps: self.movement.min_speed_mps,
            max_mps: self.movement.max_speed_mps,
        }
    }

    /// The scenario's geofence, if `movement.radius_m` is set. In circular
    /// mode the radius is the orbit itself, not a fence, so there is none.
    pub fn boundary(&self) -> Option<Boundary> {
        if self.mode == MovementMode::Circular {
            return None;
        }
        self.movement.radius_m.map(|radius_m| Boundary {
            center: self.origin,
            radius_m,
        })
    }

    /// Speed limit after applying the per-sample displacement cap, if any.
    pub fn effective_max_speed_mps(&self) -> f64 {
        self.movement
            .effective_max_speed_mps(self.update_interval_s)
    }

    /// Checks that only make sense once movement limits and the update
    /// interval are known together.
    fn validate_speed_budget(&self, errors: &mut Vec<ConfigError>) {
        let m = &self.movement;
        let interval = self.update_interval_s;
        let usable = |v: f64| v.is_finite() && v >= 0.0;
        if !(interval.is_finite() && interval > 0.0 && usable(m.max_speed_mps)) {
            return; // reported elsewhere
        }
        let effective = self.effective_max_speed_mps();
        let capped = matches!(m.max_displacement_per_sample_m, Some(c) if c.is_finite() && c > 0.0);
        let moving = !matches!(self.mode, MovementMode::Fixed | MovementMode::RouteReplay);
        if capped && moving && usable(m.min_speed_mps) && effective < m.min_speed_mps {
            errors.push(ConfigError::new(
                "movement.max_displacement_per_sample_m",
                format!(
                    "allows only {effective} m/s at this update interval, below min speed {}",
                    m.min_speed_mps
                ),
            ));
        }
        match self.mode {
            MovementMode::Circular => {
                if let (Some(r), Some(w)) = (m.radius_m, m.angular_velocity_dps) {
                    let linear = r * w.to_radians();
                    let fits_uncapped = linear <= m.max_speed_mps * (1.0 - KINEMATIC_MARGIN);
                    if capped && fits_uncapped && linear > effective * (1.0 - KINEMATIC_MARGIN) {
                        errors.push(ConfigError::new(
                            "movement.max_displacement_per_sample_m",
                            format!(
                                "orbit speed {linear:.3} m/s exceeds the capped speed {effective} m/s"
                            ),
                        ));
                    }
                }
            }
            MovementMode::RandomWalk => {
                if let Some(step) = m.step_distance_m.filter(|s| s.is_finite() && *s > 0.0) {
                    let nominal = step / interval;
                    if nominal > effective || (usable(m.min_speed_mps) && nominal < m.min_speed_mps)
                    {
                        errors.push(ConfigError::new(
                            "movement.step_distance_m",
                            format!(
                                "step distance per update interval is {nominal} m/s, outside \
                                 [{}, {effective}] m/s",
                                m.min_speed_mps
                            ),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    /// Validates the whole scenario, reporting every problem rather than
    /// only the first. A scenario must pass before a simulation may start.
    pub fn validate(&self) -> Result<(), Vec<ConfigError>> {
        let mut errors = Vec::new();

        if self.schema_version != CURRENT_SCHEMA_VERSION {
            errors.push(ConfigError::new(
                "schema_version",
                format!(
                    "unsupported version {} (this build supports {CURRENT_SCHEMA_VERSION})",
                    self.schema_version
                ),
            ));
        }
        if self.name.trim().is_empty() {
            errors.push(ConfigError::new("name", "must not be empty"));
        }
        if !self.altitude_m.is_finite() {
            errors.push(ConfigError::new("altitude_m", "must be finite"));
        }
        for (field, v) in [
            ("horizontal_accuracy_m", self.horizontal_accuracy_m),
            ("vertical_accuracy_m", self.vertical_accuracy_m),
            ("update_interval_s", self.update_interval_s),
        ] {
            if !v.is_finite() || v <= 0.0 {
                errors.push(ConfigError::new(
                    field,
                    format!("must be finite and > 0, got {v}"),
                ));
            }
        }

        self.movement.validate(self.mode, &mut errors);
        self.validate_speed_budget(&mut errors);
        self.noise.validate(&mut errors);
        // Clipped accuracy noise must not be able to drive a reported
        // accuracy to zero or below.
        let reach = NOISE_CLIP_SIGMA * self.noise.accuracy_noise_m;
        let smallest = self.horizontal_accuracy_m.min(self.vertical_accuracy_m);
        if reach.is_finite() && reach > 0.0 && smallest > 0.0 && reach >= smallest {
            errors.push(ConfigError::new(
                "noise.accuracy_noise_m",
                format!(
                    "{NOISE_CLIP_SIGMA} x accuracy noise ({reach} m) must be below the smallest                      base accuracy ({smallest} m)"
                ),
            ));
        }

        if !self.playback.speed.is_finite() || self.playback.speed <= 0.0 {
            errors.push(ConfigError::new(
                "playback.speed",
                format!("must be finite and > 0, got {}", self.playback.speed),
            ));
        }
        match (self.mode, &self.route) {
            (MovementMode::RouteReplay, None) => {
                errors.push(ConfigError::new("route", "required for mode RouteReplay"));
            }
            (MovementMode::RouteReplay, Some(route)) => {
                self.validate_route_speed(route, &mut errors);
            }
            (mode, Some(_)) => {
                errors.push(ConfigError::new(
                    "route",
                    format!("a route is only meaningful for RouteReplay, not {mode:?}"),
                ));
            }
            (_, None) => {}
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn validate_route_speed(&self, route: &Route, errors: &mut Vec<ConfigError>) {
        let max = self.movement.max_speed_mps;
        let speed = self.playback.speed;
        if !(max.is_finite() && speed.is_finite() && speed > 0.0) {
            return; // already reported above
        }
        match route.max_segment_speed_mps() {
            Err(e) => errors.push(e),
            Ok(recorded) if recorded * speed > max => errors.push(ConfigError::new(
                "route.points",
                format!(
                    "impossible route: playback reaches {:.3} m/s, above max speed {max} m/s",
                    recorded * speed
                ),
            )),
            Ok(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::params::tests::walking;

    fn scenario() -> Scenario {
        Scenario {
            schema_version: CURRENT_SCHEMA_VERSION,
            name: "Walking Test".into(),
            origin: Coordinate::new(12.9352, 77.6245).unwrap(),
            altitude_m: 920.0,
            mode: MovementMode::Walking,
            movement: walking(),
            noise: NoiseParameters::NONE,
            horizontal_accuracy_m: 5.0,
            vertical_accuracy_m: 8.0,
            update_interval_s: 1.0,
            seed: 12345,
            route: None,
            playback: PlaybackParameters::REAL_TIME,
        }
    }

    fn point(t_s: i64, lat: f64, lon: f64) -> RoutePoint {
        RoutePoint {
            timestamp: Timestamp::from_nanos(t_s * 1_000_000_000),
            coordinate: Coordinate::new(lat, lon).unwrap(),
        }
    }

    fn fields(s: &Scenario) -> Vec<&'static str> {
        s.validate()
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.field)
            .collect()
    }

    #[test]
    fn valid_scenario_passes() {
        assert_eq!(scenario().validate(), Ok(()));
        assert_eq!(
            scenario().speed_range(),
            SpeedRange {
                min_mps: 0.8,
                max_mps: 1.8
            }
        );
    }

    #[test]
    fn reports_all_top_level_errors_together() {
        let s = Scenario {
            schema_version: 99,
            name: "  ".into(),
            altitude_m: f64::NAN,
            update_interval_s: 0.0,
            horizontal_accuracy_m: -1.0,
            ..scenario()
        };
        assert_eq!(
            fields(&s),
            [
                "schema_version",
                "name",
                "altitude_m",
                "horizontal_accuracy_m",
                "update_interval_s"
            ]
        );
    }

    #[test]
    fn nested_parameter_errors_surface() {
        let s = Scenario {
            movement: MovementParameters {
                max_speed_mps: -2.0,
                ..walking()
            },
            noise: NoiseParameters {
                drift_rate_mps: f64::NAN,
                ..NoiseParameters::NONE
            },
            ..scenario()
        };
        assert_eq!(
            fields(&s),
            ["movement.max_speed_mps", "noise.drift_rate_mps"]
        );
    }

    #[test]
    fn boundary_follows_movement_radius() {
        assert_eq!(scenario().boundary(), None);
        let mut s = scenario();
        s.movement.radius_m = Some(40.0);
        assert_eq!(
            s.boundary(),
            Some(Boundary {
                center: s.origin,
                radius_m: 40.0
            })
        );
    }

    #[test]
    fn orbit_radius_is_not_a_boundary() {
        let mut s = scenario();
        s.mode = MovementMode::Circular;
        s.movement.radius_m = Some(50.0);
        s.movement.angular_velocity_dps = Some(1.0);
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(s.boundary(), None);
    }

    #[test]
    fn displacement_cap_is_checked_against_the_update_interval() {
        // Walking 0.8–1.8 m/s at 1 Hz. A 1 m cap means at most 1 m/s: fine.
        let mut s = scenario();
        s.movement.max_displacement_per_sample_m = Some(1.0);
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(s.effective_max_speed_mps(), 1.0);
        // A 0.5 m cap allows only 0.5 m/s, below the minimum cruising speed.
        s.movement.max_displacement_per_sample_m = Some(0.5);
        assert_eq!(fields(&s), ["movement.max_displacement_per_sample_m"]);
        // The same cap at 4 Hz allows 2 m/s again.
        s.update_interval_s = 0.25;
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(s.effective_max_speed_mps(), 1.8);

        // An orbit that fits max speed but not the cap.
        let mut orbit = scenario();
        orbit.mode = MovementMode::Circular;
        orbit.movement.min_speed_mps = 0.0;
        orbit.movement.radius_m = Some(50.0);
        orbit.movement.angular_velocity_dps = Some(1.0); // 0.87 m/s
        orbit.movement.max_displacement_per_sample_m = Some(0.5);
        assert_eq!(fields(&orbit), ["movement.max_displacement_per_sample_m"]);
    }

    #[test]
    fn random_walk_step_must_be_a_reachable_speed() {
        let mut s = scenario();
        s.mode = MovementMode::RandomWalk;
        s.movement.radius_m = Some(30.0);
        s.movement.step_distance_m = Some(1.2); // 1.2 m/s at 1 Hz
        assert_eq!(s.validate(), Ok(()));
        s.movement.step_distance_m = Some(2.5);
        assert_eq!(fields(&s), ["movement.step_distance_m"]);
        s.movement.step_distance_m = Some(0.5);
        assert_eq!(fields(&s), ["movement.step_distance_m"]);
        s.update_interval_s = 0.5; // 1.0 m/s
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn accuracy_noise_must_leave_accuracy_positive() {
        let mut s = scenario();
        // Base accuracies are 5 m and 8 m; 3 x 1.6 = 4.8 m is fine.
        s.noise.accuracy_noise_m = 1.6;
        assert_eq!(s.validate(), Ok(()));
        s.noise.accuracy_noise_m = 5.0 / 3.0;
        assert_eq!(fields(&s), ["noise.accuracy_noise_m"]);
    }

    #[test]
    fn route_construction_rules() {
        assert!(Route::new(vec![]).is_err());
        assert!(Route::new(vec![point(0, 0.0, 0.0)]).is_err());
        assert!(Route::new(vec![point(5, 0.0, 0.0), point(5, 0.0, 0.1)]).is_err());
        assert!(Route::new(vec![point(5, 0.0, 0.0), point(4, 0.0, 0.1)]).is_err());
        let r = Route::new(vec![point(0, 0.0, 0.0), point(10, 0.0, 0.0001)]).unwrap();
        assert_eq!(r.points().len(), 2);
        assert_eq!(r.duration_s(), 10.0);
        // 0.0001° of longitude on the equator ≈ 11.13 m in 10 s.
        assert!((r.max_segment_speed_mps().unwrap() - 1.1132).abs() < 1e-3);
    }

    #[test]
    fn route_replay_requirements() {
        let replay = Scenario {
            mode: MovementMode::RouteReplay,
            ..scenario()
        };
        assert_eq!(fields(&replay), ["route"]);

        let slow = Route::new(vec![point(0, 0.0, 0.0), point(10, 0.0, 0.0001)]).unwrap();
        let ok = Scenario {
            route: Some(slow.clone()),
            ..replay.clone()
        };
        assert_eq!(ok.validate(), Ok(()));

        // Doubling playback speed pushes 1.11 m/s past the 1.8 m/s limit.
        let too_fast = Scenario {
            playback: PlaybackParameters {
                speed: 2.0,
                ..PlaybackParameters::REAL_TIME
            },
            ..ok.clone()
        };
        assert_eq!(fields(&too_fast), ["route.points"]);

        let bad_playback = Scenario {
            playback: PlaybackParameters {
                speed: 0.0,
                ..PlaybackParameters::REAL_TIME
            },
            ..ok.clone()
        };
        assert_eq!(fields(&bad_playback), ["playback.speed"]);

        let antipodal = Route::new(vec![point(0, 0.0, 0.0), point(10, 0.0, 180.0)]).unwrap();
        assert_eq!(
            fields(&Scenario {
                route: Some(antipodal),
                ..replay
            }),
            ["route.points"]
        );

        // A route on a non-replay scenario is a configuration mistake.
        assert_eq!(
            fields(&Scenario {
                route: Some(slow),
                ..scenario()
            }),
            ["route"]
        );
    }
}
