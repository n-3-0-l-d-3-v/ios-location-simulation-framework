use super::error::ConfigError;
use super::params::{MovementMode, MovementParameters, NoiseParameters};
use super::time::Timestamp;
use crate::geographic::{self, Coordinate};

/// Version of the scenario schema this build reads and writes.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeedRange {
    pub min_mps: f64,
    pub max_mps: f64,
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
        self.noise.validate(&mut errors);

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
