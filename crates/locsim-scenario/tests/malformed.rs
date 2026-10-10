//! T08: what `import_scenario` and `export_scenario` refuse, and how.
//!
//! Everything here goes through the public entry points with text, the way
//! a caller would. Nothing invalid may come back as a scenario, nothing may
//! be repaired on the way in, and every refusal must say what and where.

mod support;

use locsim_core::domain::{ConfigError, Coordinate, Route, RoutePoint, Scenario};
use locsim_core::geographic::destination;
use locsim_core::rng::Rng;
use locsim_scenario::{export_scenario, import_scenario, ScenarioError};
use support::{example, fingerprint, EXAMPLES};

fn walking() -> &'static str {
    example("walking")
}

/// `text` with `from` replaced by `to`; `from` must occur exactly once.
fn edit(text: &str, from: &str, to: &str) -> String {
    assert_eq!(text.matches(from).count(), 1, "{from:?}");
    text.replace(from, to)
}

fn errors(text: &str) -> Vec<ScenarioError> {
    import_scenario(text).expect_err("accepted")
}

fn as_scenario_errors(errors: Vec<ConfigError>) -> Vec<ScenarioError> {
    errors
        .into_iter()
        .map(ScenarioError::InvalidScenario)
        .collect()
}

// --- Not a document ---------------------------------------------------------------

#[test]
fn every_truncation_of_a_document_is_a_syntax_error() {
    let mut cut = 0;
    for name in ["walking", "route_replay"] {
        let text = example(name).trim_end();
        for end in 0..text.len() {
            match errors(&text[..end]).as_slice() {
                [ScenarioError::Syntax { .. }] => cut += 1,
                other => panic!("{name} cut at {end}: {other:?}"),
            }
        }
        assert!(import_scenario(text).is_ok());
    }
    assert!(cut > 4_000, "{cut}");
}

#[test]
fn text_that_is_not_one_json_object_is_refused() {
    for text in ["", "   ", "\n", "nonsense", "name: walking", "<scenario/>"] {
        assert!(
            matches!(errors(text).as_slice(), [ScenarioError::Syntax { .. }]),
            "{text:?}"
        );
    }
    for text in ["null", "[]", "42", "\"walking\"", "true"] {
        assert!(
            matches!(
                errors(text).as_slice(),
                [ScenarioError::WrongType { path, expected: "object", .. }] if path == "$"
            ),
            "{text:?}"
        );
    }
    let twice = format!("{}{}", walking(), walking());
    assert!(matches!(
        errors(&twice).as_slice(),
        [ScenarioError::Syntax { .. }]
    ));
    let commented = edit(
        walking(),
        "{\n  \"schema_version\"",
        "{ // v1\n  \"schema_version\"",
    );
    assert!(matches!(
        errors(&commented).as_slice(),
        [ScenarioError::Syntax { line: 1, .. }]
    ));
    let with_bom = format!("\u{feff}{}", walking());
    assert!(matches!(
        errors(&with_bom).as_slice(),
        [ScenarioError::Syntax { .. }]
    ));
    for literal in ["NaN", "Infinity", "-Infinity", "1e999", "0x398", "920."] {
        let text = edit(
            walking(),
            "\"altitude_m\": 920.0",
            &format!("\"altitude_m\": {literal}"),
        );
        assert!(
            matches!(
                errors(&text).as_slice(),
                [ScenarioError::Syntax { line: 8, .. }]
            ),
            "{literal}"
        );
    }
}

/// The short example of the original specification. It has no schema
/// version and a different, smaller shape. It is refused; nothing is filled
/// in to make it load.
#[test]
fn the_specifications_short_example_is_refused_not_completed() {
    const SHORT: &str = r#"{
        "name": "Walking Test",
        "origin": { "latitude": 12.9352, "longitude": 77.6245 },
        "mode": "walking",
        "speed": { "min": 0.8, "max": 1.8 },
        "jitter": { "radius": 3 },
        "updateInterval": 1,
        "seed": 12345
    }"#;
    assert_eq!(
        errors(SHORT),
        [ScenarioError::MissingField {
            path: "schema_version".into()
        }]
    );
    let versioned = SHORT.replacen('{', "{ \"schema_version\": 1,", 1);
    let found: Vec<String> = errors(&versioned).iter().map(|e| e.to_string()).collect();
    assert_eq!(
        found,
        [
            "altitude_m: missing",
            "movement: missing",
            "noise: missing",
            "horizontal_accuracy_m: missing",
            "vertical_accuracy_m: missing",
            "update_interval_s: missing",
            "seed: expected string of decimal digits, found integer",
            "route: missing",
            "playback: missing",
            "speed: unknown field",
            "jitter: unknown field",
            "updateInterval: unknown field",
        ]
    );
}

// --- Stages -----------------------------------------------------------------------

#[test]
fn each_stage_reports_alone_and_before_the_next() {
    // One document with something wrong for every stage; then the faults
    // are removed one stage at a time.
    let invalid = edit(
        walking(),
        "\"update_interval_s\": 1.0",
        "\"update_interval_s\": 0.0",
    );
    let unknown = edit(&invalid, "\"altitude_m\": 920.0", "\"altitude\": 920.0");
    let version = edit(&unknown, "\"schema_version\": 1", "\"schema_version\": 2");
    let duplicate = edit(
        &version,
        "\"seed\": \"12345\"",
        "\"seed\": \"12345\",\n  \"seed\": \"12345\"",
    );
    let syntax = edit(
        &duplicate,
        "\"mode\": \"walking\",",
        "\"mode\": \"walking\"",
    );

    assert!(matches!(
        errors(&syntax).as_slice(),
        [ScenarioError::Syntax { line: 10, .. }]
    ));
    assert_eq!(
        errors(&duplicate),
        [ScenarioError::DuplicateKey {
            path: "seed".into()
        }]
    );
    assert_eq!(
        errors(&version),
        [ScenarioError::UnsupportedVersion {
            found: 2,
            supported: 1
        }]
    );
    assert_eq!(
        errors(&unknown),
        [
            ScenarioError::MissingField {
                path: "altitude_m".into()
            },
            ScenarioError::UnknownField {
                path: "altitude".into()
            }
        ]
    );
    match errors(&invalid).as_slice() {
        [ScenarioError::InvalidScenario(e)] => assert_eq!(e.field, "update_interval_s"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn unsupported_versions_are_refused_whatever_else_the_document_holds() {
    for (literal, found) in [("0", 0), ("2", 2), ("18446744073709551615", u64::MAX)] {
        let text = edit(
            walking(),
            "\"schema_version\": 1",
            &format!("\"schema_version\": {literal}"),
        );
        assert_eq!(
            errors(&text),
            [ScenarioError::UnsupportedVersion {
                found,
                supported: 1
            }]
        );
    }
    for literal in ["\"1\"", "1.0", "null", "[1]"] {
        let text = edit(
            walking(),
            "\"schema_version\": 1",
            &format!("\"schema_version\": {literal}"),
        );
        assert!(
            matches!(
                errors(&text).as_slice(),
                [ScenarioError::WrongType { path, expected: "integer", .. }]
                    if path == "schema_version"
            ),
            "{literal}"
        );
    }
    let missing = edit(walking(), "  \"schema_version\": 1,\n", "");
    assert_eq!(
        errors(&missing),
        [ScenarioError::MissingField {
            path: "schema_version".into()
        }]
    );
}

#[test]
fn duplicate_keys_are_refused_wherever_they_are_even_with_equal_values() {
    let route = example("route_replay");
    let cases = [
        (
            edit(
                walking(),
                "\"name\": \"Walking Test\",",
                "\"name\": \"Walking Test\",\n  \"name\": \"Walking Test\",",
            ),
            "name",
        ),
        (
            edit(
                walking(),
                "\"schema_version\": 1,",
                "\"schema_version\": 1,\n  \"schema_version\": 1,",
            ),
            "schema_version",
        ),
        (
            edit(
                walking(),
                "\"max_pause_s\": 20.0,",
                "\"max_pause_s\": 20.0,\n  \"max_pause_s\": 0.0,",
            ),
            "movement.max_pause_s",
        ),
        (
            edit(
                walking(),
                "\"latitude\": 12.9352,",
                "\"latitude\": 12.9352,\n  \"latitude\": 13.0,",
            ),
            "origin.latitude",
        ),
        (
            edit(
                route,
                "\"elapsed_ns\": 8000000000,",
                "\"elapsed_ns\": 8000000000, \"elapsed_ns\": 8000000001,",
            ),
            "route.points[2].elapsed_ns",
        ),
        (
            edit(
                walking(),
                "\"seed\": \"12345\",",
                "\"seed\": \"12345\",\n  \"s\\u0065ed\": \"54321\",",
            ),
            "seed",
        ),
    ];
    for (text, path) in cases {
        assert_eq!(
            errors(&text),
            [ScenarioError::DuplicateKey { path: path.into() }]
        );
    }
}

// --- Invalid scenarios ------------------------------------------------------------

#[test]
fn an_invalid_scenario_is_reported_exactly_as_scenario_validate_reports_it() {
    let mut text = walking().to_string();
    let mut scenario = import_scenario(walking()).unwrap();
    for (from, to) in [
        ("\"name\": \"Walking Test\"", "\"name\": \"  \""),
        ("\"update_interval_s\": 1.0", "\"update_interval_s\": -1.0"),
        ("\"max_speed_mps\": 1.8", "\"max_speed_mps\": 0.5"),
        (
            "\"heading_persistence\": 0.9",
            "\"heading_persistence\": 1.5",
        ),
        ("\"accuracy_noise_m\": 0.8", "\"accuracy_noise_m\": 4.0"),
        (
            "\"vertical_accuracy_m\": 8.0",
            "\"vertical_accuracy_m\": 0.0",
        ),
        ("\"speed\": 1.0", "\"speed\": 0.0"),
    ] {
        text = edit(&text, from, to);
    }
    scenario.name = "  ".into();
    scenario.update_interval_s = -1.0;
    scenario.movement.max_speed_mps = 0.5;
    scenario.movement.heading_persistence = 1.5;
    scenario.noise.accuracy_noise_m = 4.0;
    scenario.vertical_accuracy_m = 0.0;
    scenario.playback.speed = 0.0;
    let expected = scenario.validate().unwrap_err();
    assert_eq!(expected.len(), 6, "{expected:?}");
    assert_eq!(errors(&text), as_scenario_errors(expected));
}

#[test]
fn a_route_where_none_belongs_and_a_missing_route_are_scenario_errors() {
    let replay = example("route_replay");
    let without = {
        let start = replay.find("\"route\": {").unwrap();
        let end = replay.find("  \"playback\"").unwrap();
        format!("{}\"route\": null,\n{}", &replay[..start], &replay[end..])
    };
    match errors(&without).as_slice() {
        [ScenarioError::InvalidScenario(e)] => assert_eq!(e.field, "route"),
        other => panic!("{other:?}"),
    }
    let walking_with_route = edit(
        replay,
        "\"mode\": \"route_replay\"",
        "\"mode\": \"walking\"",
    );
    assert!(errors(&walking_with_route)
        .iter()
        .any(|e| matches!(e, ScenarioError::InvalidScenario(c) if c.field == "route")));
    // Looping an open route is refused, not closed for the author.
    let looping = edit(replay, "\"looping\": false", "\"looping\": true");
    match errors(&looping).as_slice() {
        [ScenarioError::InvalidScenario(e)] => assert_eq!(e.field, "playback.looping"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_circuit_that_does_not_quite_close_is_not_closed_for_the_author() {
    let circuit = example("antimeridian_loop");
    let last = circuit.rfind("\"latitude\": -16.7992771").unwrap();
    let nearly = format!(
        "{}\"latitude\": -16.7992772{}",
        &circuit[..last],
        &circuit[last + "\"latitude\": -16.7992771".len()..]
    );
    match errors(&nearly).as_slice() {
        [ScenarioError::InvalidScenario(e)] => assert_eq!(e.field, "playback.looping"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn coordinates_out_of_range_are_refused_not_wrapped() {
    for (from, to, path) in [
        (
            "\"latitude\": 12.9352",
            "\"latitude\": 90.00000000000001",
            "origin",
        ),
        ("\"latitude\": 12.9352", "\"latitude\": -91", "origin"),
        (
            "\"longitude\": 77.6245",
            "\"longitude\": 180.00000000000003",
            "origin",
        ),
        (
            "\"longitude\": 77.6245",
            "\"longitude\": 437.6245",
            "origin",
        ),
        ("\"longitude\": 77.6245", "\"longitude\": -180.1", "origin"),
    ] {
        match errors(&edit(walking(), from, to)).as_slice() {
            [ScenarioError::InvalidCoordinate { path: p, .. }] => assert_eq!(p, path),
            other => panic!("{to}: {other:?}"),
        }
    }
    // The limits themselves are valid and arrive untouched.
    let circuit = example("antimeridian_loop");
    let east = circuit.replacen("\"longitude\": 180.0", "\"longitude\": -180.0", 1);
    assert_ne!(east, circuit);
    assert_eq!(import_scenario(&east).unwrap().origin.longitude(), -180.0);
}

// --- Export -----------------------------------------------------------------------

#[test]
fn export_refuses_an_invalid_scenario_and_says_why() {
    let valid = import_scenario(walking()).unwrap();
    let broken: [fn(&mut Scenario); 7] = [
        |s| s.schema_version = 2,
        |s| s.schema_version = 0,
        |s| s.name.clear(),
        |s| s.altitude_m = f64::NAN,
        |s| s.movement.max_acceleration_mps2 = f64::INFINITY,
        |s| s.update_interval_s = 0.0,
        |s| {
            s.name.clear();
            s.noise.position_noise_m = -1.0;
            s.horizontal_accuracy_m = f64::NEG_INFINITY;
        },
    ];
    for (case, damage) in broken.iter().enumerate() {
        let mut scenario = valid.clone();
        damage(&mut scenario);
        let expected = scenario.validate().expect_err("damage was harmless");
        assert_eq!(
            export_scenario(&scenario),
            Err(as_scenario_errors(expected)),
            "case {case}"
        );
    }
}

// --- Accepted spellings -----------------------------------------------------------

#[test]
fn a_number_may_be_spelled_any_valid_way_and_formatting_is_free() {
    let reference = fingerprint(&import_scenario(walking()).unwrap());
    for spelling in [
        "920",
        "920.000",
        "9.2e2",
        "0.92E+3",
        "9200e-1",
        "920.00000000000001",
    ] {
        let text = edit(
            walking(),
            "\"altitude_m\": 920.0",
            &format!("\"altitude_m\": {spelling}"),
        );
        assert_eq!(
            fingerprint(&import_scenario(&text).unwrap()),
            reference,
            "{spelling}"
        );
    }
    let crlf = walking().replace('\n', "\r\n").replace("  ", "\t");
    assert_eq!(fingerprint(&import_scenario(&crlf).unwrap()), reference);
    // Member order carries no meaning on input.
    let moved = edit(walking(), "  \"schema_version\": 1,\n", "");
    let moved = edit(
        &moved,
        "\"reverse\": false\n  }\n}",
        "\"reverse\": false\n  },\n  \"schema_version\": 1\n}",
    );
    assert_eq!(fingerprint(&import_scenario(&moved).unwrap()), reference);
    // The exported form, however, is always the same text.
    let exported = export_scenario(&import_scenario(&moved).unwrap()).unwrap();
    assert_eq!(exported, walking());
}

#[test]
fn a_name_keeps_every_character() {
    let mut scenario = import_scenario(walking()).unwrap();
    for name in [
        "\"quoted\" \\ back/slash",
        "tab\there, newline\nthere, nul\u{0}",
        "ಬೆಂಗಳೂರು — 🚶 \u{10ffff}",
        "\u{2028}line separators\u{2029}",
        " leading and trailing ",
    ] {
        scenario.name = name.into();
        let text = export_scenario(&scenario).unwrap();
        assert_eq!(import_scenario(&text).unwrap().name, name);
    }
}

// --- Damage -----------------------------------------------------------------------

#[test]
fn random_damage_never_panics_and_never_lets_an_invalid_scenario_through() {
    let mut rng = Rng::from_seed(0xD0C);
    let (mut refused, mut accepted, mut changed) = (0u32, 0u32, 0u32);
    for (name, text) in EXAMPLES {
        assert!(text.is_ascii(), "{name}");
        let original = fingerprint(&import_scenario(text).unwrap());
        for _ in 0..4_000 {
            let mut bytes = text.as_bytes().to_vec();
            let position = (rng.next_u64() % bytes.len() as u64) as usize;
            let random = b' ' + (rng.next_u64() % 95) as u8;
            match rng.next_u64() % 4 {
                0 => {
                    bytes.remove(position);
                }
                1 => bytes.insert(position, random),
                2 => bytes[position] = random,
                // Bias towards the characters that matter in JSON.
                _ => bytes[position] = b"0123456789-.eE\"{}[],:"[(rng.next_u64() % 21) as usize],
            }
            let damaged = String::from_utf8(bytes).unwrap();
            match import_scenario(&damaged) {
                Err(errors) => {
                    assert!(!errors.is_empty(), "{name}: refused without a reason");
                    refused += 1;
                }
                Ok(scenario) => {
                    // What comes back is valid and survives a round trip.
                    assert_eq!(scenario.validate(), Ok(()), "{name}:\n{damaged}");
                    let again = import_scenario(&export_scenario(&scenario).unwrap()).unwrap();
                    assert_eq!(fingerprint(&again), fingerprint(&scenario));
                    accepted += 1;
                    if fingerprint(&scenario) != original {
                        changed += 1;
                    }
                }
            }
        }
    }
    println!(
        "damage: {refused} refused, {accepted} accepted ({changed} of them a different, valid scenario)"
    );
    assert!(refused > 20_000 && accepted > 500 && changed > 100);
}

// --- Size -------------------------------------------------------------------------

#[test]
fn a_route_of_twenty_thousand_points_round_trips_exactly() {
    let replay = import_scenario(example("route_replay")).unwrap();
    let start = Coordinate::new(-45.0, 179.999).unwrap();
    // A straight, slow line across the antimeridian: one point per second,
    // a few centimetres apart.
    let points: Vec<RoutePoint> = (0..20_000)
        .map(|j| {
            let position = destination(start, 80.0, 0.05 * j as f64).unwrap();
            RoutePoint::new(j * 1_000_000_000, position).with_altitude(j as f64 / 7.0)
        })
        .collect();
    let scenario = Scenario {
        origin: start,
        route: Some(Route::new(points).unwrap()),
        ..replay
    };
    let text = export_scenario(&scenario).unwrap();
    let back = import_scenario(&text).unwrap();
    assert_eq!(fingerprint(&back), fingerprint(&scenario));
    assert_eq!(export_scenario(&back).unwrap(), text);
    let longitudes: Vec<f64> = back
        .route
        .unwrap()
        .points()
        .iter()
        .map(|p| p.coordinate.longitude())
        .collect();
    assert!(longitudes[0] > 179.0 && longitudes[19_999] < -179.0);
}
