//! Schema version 1: the mapping between a [`Scenario`] and its document.
//!
//! The document mirrors the domain model. Member names are the Rust field
//! names, units included, so a path in an error here is the same path
//! `Scenario::validate` reports.
//!
//! Rules, all strict:
//!
//! - Every member is required. An absent optional value is written `null`;
//!   leaving the member out is an error, and so is any member not listed.
//! - A float field takes any JSON number. An integer field
//!   (`schema_version`, `elapsed_ns`) takes only an integer literal: `1.0`
//!   and `1e9` are rejected, not rounded.
//! - `seed` is a `u64` written as a canonical decimal **string** (`"0"` to
//!   `"18446744073709551615"`, no sign, no leading zeros, no spaces). A JSON
//!   number is refused: seeds above 2^53 do not survive tools that read
//!   numbers as doubles.
//! - `mode` and `movement.direction` are lower-case snake-case names.
//! - Coordinates go through `Coordinate::new` and routes through
//!   `Route::new`; nothing is wrapped, clamped, sorted or defaulted.
//!
//! Decoding reports every problem it can find, not only the first.

use crate::error::ScenarioError;
use crate::json::{element_path, member_path, Json, Number, ROOT};
use locsim_core::domain::{
    Coordinate, MovementMode, MovementParameters, NoiseParameters, PlaybackParameters,
    RotationDirection, Route, RoutePoint, Scenario,
};

const MODES: [MovementMode; 6] = [
    MovementMode::Fixed,
    MovementMode::RandomWalk,
    MovementMode::Walking,
    MovementMode::Driving,
    MovementMode::Circular,
    MovementMode::RouteReplay,
];

fn mode_name(mode: MovementMode) -> &'static str {
    match mode {
        MovementMode::Fixed => "fixed",
        MovementMode::RandomWalk => "random_walk",
        MovementMode::Walking => "walking",
        MovementMode::Driving => "driving",
        MovementMode::Circular => "circular",
        MovementMode::RouteReplay => "route_replay",
    }
}

const DIRECTIONS: [RotationDirection; 2] = [
    RotationDirection::Clockwise,
    RotationDirection::CounterClockwise,
];

fn direction_name(direction: RotationDirection) -> &'static str {
    match direction {
        RotationDirection::Clockwise => "clockwise",
        RotationDirection::CounterClockwise => "counter_clockwise",
    }
}

pub(crate) type Errors = Vec<ScenarioError>;

// ---------------------------------------------------------------- encoding

struct Encoder {
    errors: Errors,
}

impl Encoder {
    pub(crate) fn float(&mut self, path: &str, v: f64) -> Json {
        if !v.is_finite() {
            self.errors
                .push(ScenarioError::NonFiniteNumber { path: path.into() });
        }
        Json::float(v)
    }

    fn optional_float(&mut self, path: &str, v: Option<f64>) -> Json {
        match v {
            Some(v) => self.float(path, v),
            None => Json::Null,
        }
    }

    fn coordinate(&mut self, path: &str, c: Coordinate) -> Json {
        object(vec![
            (
                "latitude",
                self.float(&member_path(path, "latitude"), c.latitude()),
            ),
            (
                "longitude",
                self.float(&member_path(path, "longitude"), c.longitude()),
            ),
        ])
    }

    fn movement(&mut self, m: &MovementParameters) -> Json {
        let mut f = |name: &'static str, v: f64| (name, self.float(&format!("movement.{name}"), v));
        let min_speed = f("min_speed_mps", m.min_speed_mps);
        let max_speed = f("max_speed_mps", m.max_speed_mps);
        let max_acceleration = f("max_acceleration_mps2", m.max_acceleration_mps2);
        let max_deceleration = f("max_deceleration_mps2", m.max_deceleration_mps2);
        let max_heading_rate = f("max_heading_rate_dps", m.max_heading_rate_dps);
        let heading_persistence = f("heading_persistence", m.heading_persistence);
        let pause_probability = f("pause_probability", m.pause_probability);
        let max_pause = f("max_pause_s", m.max_pause_s);
        let start_phase = f("start_phase_deg", m.start_phase_deg);
        let speed_change_interval = f("speed_change_interval_s", m.speed_change_interval_s);
        let mut o = |name: &'static str, v: Option<f64>| {
            (name, self.optional_float(&format!("movement.{name}"), v))
        };
        let radius = o("radius_m", m.radius_m);
        let step_distance = o("step_distance_m", m.step_distance_m);
        let angular_velocity = o("angular_velocity_dps", m.angular_velocity_dps);
        let max_displacement = o(
            "max_displacement_per_sample_m",
            m.max_displacement_per_sample_m,
        );
        object(vec![
            min_speed,
            max_speed,
            max_acceleration,
            max_deceleration,
            max_heading_rate,
            radius,
            step_distance,
            heading_persistence,
            pause_probability,
            max_pause,
            angular_velocity,
            (
                "direction",
                Json::String(direction_name(m.direction).into()),
            ),
            start_phase,
            speed_change_interval,
            max_displacement,
        ])
    }

    fn noise(&mut self, n: &NoiseParameters) -> Json {
        let fields = [
            ("position_noise_m", n.position_noise_m),
            ("max_position_offset_m", n.max_position_offset_m),
            ("speed_noise_mps", n.speed_noise_mps),
            ("heading_noise_deg", n.heading_noise_deg),
            ("accuracy_noise_m", n.accuracy_noise_m),
            ("drift_rate_mps", n.drift_rate_mps),
            ("position_correlation_time_s", n.position_correlation_time_s),
            ("max_offset_rate_mps", n.max_offset_rate_mps),
        ];
        object(
            fields
                .into_iter()
                .map(|(name, v)| (name, self.float(&format!("noise.{name}"), v)))
                .collect(),
        )
    }

    fn route(&mut self, route: &Route) -> Json {
        let points = route
            .points()
            .iter()
            .enumerate()
            .map(|(index, p)| {
                let path = element_path("route.points", index);
                let elapsed = match u64::try_from(p.elapsed_ns) {
                    Ok(v) => Number::PosInt(v),
                    Err(_) => Number::NegInt(p.elapsed_ns),
                };
                object(vec![
                    ("elapsed_ns", Json::Number(elapsed)),
                    (
                        "coordinate",
                        self.coordinate(&member_path(&path, "coordinate"), p.coordinate),
                    ),
                    (
                        "altitude_m",
                        self.optional_float(&member_path(&path, "altitude_m"), p.altitude_m),
                    ),
                ])
            })
            .collect();
        object(vec![
            (
                "name",
                match route.name() {
                    Some(name) => Json::String(name.to_string()),
                    None => Json::Null,
                },
            ),
            ("points", Json::Array(points)),
        ])
    }

    fn scenario(&mut self, s: &Scenario) -> Json {
        object(vec![
            (
                "schema_version",
                Json::Number(Number::PosInt(s.schema_version.into())),
            ),
            ("name", Json::String(s.name.clone())),
            ("origin", self.coordinate("origin", s.origin)),
            ("altitude_m", self.float("altitude_m", s.altitude_m)),
            ("mode", Json::String(mode_name(s.mode).into())),
            ("movement", self.movement(&s.movement)),
            ("noise", self.noise(&s.noise)),
            (
                "horizontal_accuracy_m",
                self.float("horizontal_accuracy_m", s.horizontal_accuracy_m),
            ),
            (
                "vertical_accuracy_m",
                self.float("vertical_accuracy_m", s.vertical_accuracy_m),
            ),
            (
                "update_interval_s",
                self.float("update_interval_s", s.update_interval_s),
            ),
            ("seed", Json::String(s.seed.to_string())),
            (
                "route",
                match &s.route {
                    Some(route) => self.route(route),
                    None => Json::Null,
                },
            ),
            (
                "playback",
                object(vec![
                    ("speed", self.float("playback.speed", s.playback.speed)),
                    ("looping", Json::Bool(s.playback.looping)),
                    ("reverse", Json::Bool(s.playback.reverse)),
                ]),
            ),
        ])
    }
}

pub(crate) fn object(members: Vec<(&'static str, Json)>) -> Json {
    Json::Object(
        members
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

/// Builds the version-1 document of `scenario`. The scenario is written as
/// it is, valid or not; the only thing that cannot be written is a
/// non-finite number.
pub(crate) fn encode(scenario: &Scenario) -> Result<Json, Errors> {
    let mut encoder = Encoder { errors: Vec::new() };
    let document = encoder.scenario(scenario);
    if encoder.errors.is_empty() {
        Ok(document)
    } else {
        Err(encoder.errors)
    }
}

// ---------------------------------------------------------------- decoding

/// One JSON object being read: hands out its members by name and, when
/// closed, reports the ones nobody asked for.
pub(crate) struct Fields<'a> {
    path: String,
    members: &'a [(String, Json)],
    used: Vec<bool>,
}

impl<'a> Fields<'a> {
    pub(crate) fn open(value: &'a Json, path: &str, errors: &mut Errors) -> Option<Self> {
        let Some(members) = value.members() else {
            wrong_type::<()>(path, "object", value, errors);
            return None;
        };
        Some(Self {
            path: path.to_string(),
            members,
            used: vec![false; members.len()],
        })
    }

    /// The member `key` and its path, or a `MissingField` error.
    pub(crate) fn take(&mut self, key: &str, errors: &mut Errors) -> Option<(&'a Json, String)> {
        let path = member_path(&self.path, key);
        match self.members.iter().position(|(k, _)| k == key) {
            Some(index) => {
                self.used[index] = true;
                Some((&self.members[index].1, path))
            }
            None => {
                errors.push(ScenarioError::MissingField { path });
                None
            }
        }
    }

    pub(crate) fn float(&mut self, key: &str, errors: &mut Errors) -> Option<f64> {
        let (value, path) = self.take(key, errors)?;
        float(value, &path, errors)
    }

    /// `Some(None)` for an explicit `null`; `None` when there was an error.
    pub(crate) fn optional_float(&mut self, key: &str, errors: &mut Errors) -> Option<Option<f64>> {
        let (value, path) = self.take(key, errors)?;
        match value {
            Json::Null => Some(None),
            _ => float(value, &path, errors).map(Some),
        }
    }

    pub(crate) fn boolean(&mut self, key: &str, errors: &mut Errors) -> Option<bool> {
        let (value, path) = self.take(key, errors)?;
        match value {
            Json::Bool(v) => Some(*v),
            _ => wrong_type(&path, "boolean", value, errors),
        }
    }

    pub(crate) fn string(&mut self, key: &str, errors: &mut Errors) -> Option<&'a str> {
        let (value, path) = self.take(key, errors)?;
        string(value, &path, errors)
    }

    pub(crate) fn named<T: Copy>(
        &mut self,
        key: &str,
        variants: &[T],
        name: fn(T) -> &'static str,
        errors: &mut Errors,
    ) -> Option<T> {
        let (value, path) = self.take(key, errors)?;
        let text = string(value, &path, errors)?;
        let found = variants.iter().copied().find(|v| name(*v) == text);
        if found.is_none() {
            let names: Vec<_> = variants.iter().map(|v| name(*v)).collect();
            errors.push(ScenarioError::InvalidValue {
                path,
                reason: format!(
                    "unknown value {text:?}; expected one of {}",
                    names.join(", ")
                ),
            });
        }
        found
    }

    pub(crate) fn close(self, errors: &mut Errors) {
        for ((key, _), used) in self.members.iter().zip(&self.used) {
            if !used {
                errors.push(ScenarioError::UnknownField {
                    path: member_path(&self.path, key),
                });
            }
        }
    }
}

pub(crate) fn wrong_type<T>(
    path: &str,
    expected: &'static str,
    value: &Json,
    errors: &mut Errors,
) -> Option<T> {
    errors.push(ScenarioError::WrongType {
        path: path.to_string(),
        expected,
        found: value.kind(),
    });
    None
}

pub(crate) fn invalid<T>(path: &str, reason: impl Into<String>, errors: &mut Errors) -> Option<T> {
    errors.push(ScenarioError::InvalidValue {
        path: path.to_string(),
        reason: reason.into(),
    });
    None
}

pub(crate) fn float(value: &Json, path: &str, errors: &mut Errors) -> Option<f64> {
    let v = match value {
        // The nearest double to the literal, as for any decimal literal.
        Json::Number(Number::PosInt(v)) => *v as f64,
        Json::Number(Number::NegInt(v)) => *v as f64,
        Json::Number(Number::Float(v)) => *v,
        _ => return wrong_type(path, "number", value, errors),
    };
    if v.is_finite() {
        Some(v)
    } else {
        invalid(path, "not a finite number", errors)
    }
}

pub(crate) fn string<'a>(value: &'a Json, path: &str, errors: &mut Errors) -> Option<&'a str> {
    match value {
        Json::String(v) => Some(v),
        _ => wrong_type(path, "string", value, errors),
    }
}

fn schema_version(value: &Json, path: &str, errors: &mut Errors) -> Option<u32> {
    match value {
        Json::Number(Number::PosInt(v)) => match u32::try_from(*v) {
            Ok(v) => Some(v),
            Err(_) => invalid(path, format!("{v} is not a schema version"), errors),
        },
        Json::Number(Number::NegInt(v)) => {
            invalid(path, format!("{v} is not a schema version"), errors)
        }
        _ => wrong_type(path, "integer", value, errors),
    }
}

pub(crate) fn integer_i64(value: &Json, path: &str, errors: &mut Errors) -> Option<i64> {
    match value {
        Json::Number(Number::PosInt(v)) => match i64::try_from(*v) {
            Ok(v) => Some(v),
            Err(_) => invalid(
                path,
                format!("{v} is outside the signed 64-bit range"),
                errors,
            ),
        },
        Json::Number(Number::NegInt(v)) => Some(*v),
        _ => wrong_type(path, "integer", value, errors),
    }
}

/// A `u64` in canonical decimal: digits only, no leading zero except `"0"`.
pub(crate) fn decimal_u64(value: &Json, path: &str, errors: &mut Errors) -> Option<u64> {
    let Json::String(text) = value else {
        return wrong_type(path, "string of decimal digits", value, errors);
    };
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let canonical = digits && (text == "0" || !text.starts_with('0'));
    if !canonical {
        return invalid(
            path,
            format!("{text:?} is not a canonical decimal integer (digits only, no leading zeros)"),
            errors,
        );
    }
    match text.parse::<u64>() {
        Ok(v) => Some(v),
        Err(_) => invalid(
            path,
            format!("{text} is above the largest value, {}", u64::MAX),
            errors,
        ),
    }
}

pub(crate) fn coordinate(value: &Json, path: &str, errors: &mut Errors) -> Option<Coordinate> {
    let mut fields = Fields::open(value, path, errors)?;
    let latitude = fields.float("latitude", errors);
    let longitude = fields.float("longitude", errors);
    fields.close(errors);
    match Coordinate::new(latitude?, longitude?) {
        Ok(c) => Some(c),
        Err(error) => {
            errors.push(ScenarioError::InvalidCoordinate {
                path: path.to_string(),
                error,
            });
            None
        }
    }
}

fn movement(value: &Json, path: &str, errors: &mut Errors) -> Option<MovementParameters> {
    let mut f = Fields::open(value, path, errors)?;
    let min_speed_mps = f.float("min_speed_mps", errors);
    let max_speed_mps = f.float("max_speed_mps", errors);
    let max_acceleration_mps2 = f.float("max_acceleration_mps2", errors);
    let max_deceleration_mps2 = f.float("max_deceleration_mps2", errors);
    let max_heading_rate_dps = f.float("max_heading_rate_dps", errors);
    let radius_m = f.optional_float("radius_m", errors);
    let step_distance_m = f.optional_float("step_distance_m", errors);
    let heading_persistence = f.float("heading_persistence", errors);
    let pause_probability = f.float("pause_probability", errors);
    let max_pause_s = f.float("max_pause_s", errors);
    let angular_velocity_dps = f.optional_float("angular_velocity_dps", errors);
    let direction = f.named("direction", &DIRECTIONS, direction_name, errors);
    let start_phase_deg = f.float("start_phase_deg", errors);
    let speed_change_interval_s = f.float("speed_change_interval_s", errors);
    let max_displacement_per_sample_m = f.optional_float("max_displacement_per_sample_m", errors);
    f.close(errors);
    Some(MovementParameters {
        min_speed_mps: min_speed_mps?,
        max_speed_mps: max_speed_mps?,
        max_acceleration_mps2: max_acceleration_mps2?,
        max_deceleration_mps2: max_deceleration_mps2?,
        max_heading_rate_dps: max_heading_rate_dps?,
        radius_m: radius_m?,
        step_distance_m: step_distance_m?,
        heading_persistence: heading_persistence?,
        pause_probability: pause_probability?,
        max_pause_s: max_pause_s?,
        angular_velocity_dps: angular_velocity_dps?,
        direction: direction?,
        start_phase_deg: start_phase_deg?,
        speed_change_interval_s: speed_change_interval_s?,
        max_displacement_per_sample_m: max_displacement_per_sample_m?,
    })
}

fn noise(value: &Json, path: &str, errors: &mut Errors) -> Option<NoiseParameters> {
    let mut f = Fields::open(value, path, errors)?;
    let position_noise_m = f.float("position_noise_m", errors);
    let max_position_offset_m = f.float("max_position_offset_m", errors);
    let speed_noise_mps = f.float("speed_noise_mps", errors);
    let heading_noise_deg = f.float("heading_noise_deg", errors);
    let accuracy_noise_m = f.float("accuracy_noise_m", errors);
    let drift_rate_mps = f.float("drift_rate_mps", errors);
    let position_correlation_time_s = f.float("position_correlation_time_s", errors);
    let max_offset_rate_mps = f.float("max_offset_rate_mps", errors);
    f.close(errors);
    Some(NoiseParameters {
        position_noise_m: position_noise_m?,
        max_position_offset_m: max_position_offset_m?,
        speed_noise_mps: speed_noise_mps?,
        heading_noise_deg: heading_noise_deg?,
        accuracy_noise_m: accuracy_noise_m?,
        drift_rate_mps: drift_rate_mps?,
        position_correlation_time_s: position_correlation_time_s?,
        max_offset_rate_mps: max_offset_rate_mps?,
    })
}

fn playback(value: &Json, path: &str, errors: &mut Errors) -> Option<PlaybackParameters> {
    let mut f = Fields::open(value, path, errors)?;
    let speed = f.float("speed", errors);
    let looping = f.boolean("looping", errors);
    let reverse = f.boolean("reverse", errors);
    f.close(errors);
    Some(PlaybackParameters {
        speed: speed?,
        looping: looping?,
        reverse: reverse?,
    })
}

fn route_point(value: &Json, path: &str, errors: &mut Errors) -> Option<RoutePoint> {
    let mut f = Fields::open(value, path, errors)?;
    let elapsed = f
        .take("elapsed_ns", errors)
        .and_then(|(v, p)| integer_i64(v, &p, errors));
    let position = f
        .take("coordinate", errors)
        .and_then(|(v, p)| coordinate(v, &p, errors));
    let altitude_m = f.optional_float("altitude_m", errors);
    f.close(errors);
    Some(RoutePoint {
        elapsed_ns: elapsed?,
        coordinate: position?,
        altitude_m: altitude_m?,
    })
}

fn route(value: &Json, path: &str, errors: &mut Errors) -> Option<Route> {
    let mut f = Fields::open(value, path, errors)?;
    let name = f.take("name", errors).and_then(|(v, p)| match v {
        Json::Null => Some(None),
        _ => string(v, &p, errors).map(Some),
    });
    let points = f.take("points", errors).and_then(|(v, p)| {
        let Json::Array(items) = v else {
            return wrong_type(&p, "array", v, errors);
        };
        // Decode every point before giving up, so each bad one is reported.
        let decoded: Vec<_> = items
            .iter()
            .enumerate()
            .map(|(index, item)| route_point(item, &element_path(&p, index), errors))
            .collect();
        let points = decoded.into_iter().collect::<Option<Vec<_>>>()?;
        match Route::new(points) {
            Ok(route) => Some(route),
            Err(error) => {
                errors.push(ScenarioError::InvalidRoute { path: p, error });
                None
            }
        }
    });
    f.close(errors);
    let route = points?;
    Some(match name? {
        Some(name) => route.with_name(name),
        None => route,
    })
}

fn scenario(value: &Json, errors: &mut Errors) -> Option<Scenario> {
    let mut f = Fields::open(value, ROOT, errors)?;
    let version = f
        .take("schema_version", errors)
        .and_then(|(v, p)| schema_version(v, &p, errors));
    let name = f.string("name", errors);
    let origin = f
        .take("origin", errors)
        .and_then(|(v, p)| coordinate(v, &p, errors));
    let altitude_m = f.float("altitude_m", errors);
    let mode = f.named("mode", &MODES, mode_name, errors);
    let movement = f
        .take("movement", errors)
        .and_then(|(v, p)| movement(v, &p, errors));
    let noise = f
        .take("noise", errors)
        .and_then(|(v, p)| noise(v, &p, errors));
    let horizontal_accuracy_m = f.float("horizontal_accuracy_m", errors);
    let vertical_accuracy_m = f.float("vertical_accuracy_m", errors);
    let update_interval_s = f.float("update_interval_s", errors);
    let seed = f
        .take("seed", errors)
        .and_then(|(v, p)| decimal_u64(v, &p, errors));
    let route = f.take("route", errors).and_then(|(v, p)| match v {
        Json::Null => Some(None),
        _ => route(v, &p, errors).map(Some),
    });
    let playback = f
        .take("playback", errors)
        .and_then(|(v, p)| playback(v, &p, errors));
    f.close(errors);
    Some(Scenario {
        schema_version: version?,
        name: name?.to_string(),
        origin: origin?,
        altitude_m: altitude_m?,
        mode: mode?,
        movement: movement?,
        noise: noise?,
        horizontal_accuracy_m: horizontal_accuracy_m?,
        vertical_accuracy_m: vertical_accuracy_m?,
        update_interval_s: update_interval_s?,
        seed: seed?,
        route: route?,
        playback: playback?,
    })
}

/// Reads a version-1 document into a `Scenario`, reporting every structural
/// problem. The result is well typed but not yet validated as a scenario.
pub(crate) fn decode(document: &Json) -> Result<Scenario, Errors> {
    let mut errors = Vec::new();
    match scenario(document, &mut errors) {
        Some(scenario) if errors.is_empty() => Ok(scenario),
        _ => Err(errors),
    }
}

#[cfg(test)]
pub(crate) mod tests;
