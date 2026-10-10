use super::*;
use crate::json::{parse, write};
use locsim_core::domain::CURRENT_SCHEMA_VERSION;
use locsim_core::rng::Rng;

// ------------------------------------------------------------------ fixtures

fn at(latitude: f64, longitude: f64) -> Coordinate {
    Coordinate::new(latitude, longitude).unwrap()
}

/// A scenario with every optional value present, so that every member of
/// the document is a non-null leaf. Not meant to be a valid simulation.
pub(crate) fn sample() -> Scenario {
    Scenario {
        schema_version: CURRENT_SCHEMA_VERSION,
        name: "sample".into(),
        origin: at(12.9352, 77.6245),
        altitude_m: 920.5,
        mode: MovementMode::RouteReplay,
        movement: MovementParameters {
            min_speed_mps: 0.25,
            max_speed_mps: 30.5,
            max_acceleration_mps2: 2.5,
            max_deceleration_mps2: 4.5,
            max_heading_rate_dps: 25.5,
            radius_m: Some(5000.5),
            step_distance_m: Some(1.5),
            heading_persistence: 0.5,
            pause_probability: 0.125,
            max_pause_s: 45.5,
            angular_velocity_dps: Some(3.5),
            direction: RotationDirection::CounterClockwise,
            start_phase_deg: 90.5,
            speed_change_interval_s: 60.5,
            max_displacement_per_sample_m: Some(40.5),
        },
        noise: NoiseParameters {
            position_noise_m: 1.5,
            max_position_offset_m: 8.5,
            speed_noise_mps: 0.125,
            heading_noise_deg: 4.5,
            accuracy_noise_m: 0.75,
            drift_rate_mps: 0.0625,
            position_correlation_time_s: 6.5,
            max_offset_rate_mps: 1.25,
        },
        horizontal_accuracy_m: 6.5,
        vertical_accuracy_m: 9.5,
        update_interval_s: 0.5,
        seed: 12345,
        route: Some(
            Route::new(vec![
                RoutePoint::new(0, at(12.9352, 77.6245)).with_altitude(920.5),
                RoutePoint::new(2_500_000_000, at(12.93525, 77.62455)).with_altitude(921.5),
                RoutePoint::new(5_000_000_001, at(12.9353, 77.6246)).with_altitude(922.5),
            ])
            .unwrap()
            .with_name("loop"),
        ),
        playback: PlaybackParameters {
            speed: 1.5,
            looping: true,
            reverse: false,
        },
    }
}

fn document() -> Json {
    encode(&sample()).unwrap()
}

/// `Debug` prints every float in its shortest round-trip form and keeps the
/// sign of zero, so equal text means equal bits in every field.
pub(crate) fn fingerprint(s: &Scenario) -> String {
    format!("{s:?}")
}

// ------------------------------------------------------- document surgery

#[derive(Debug, Clone)]
enum Step {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone)]
struct Node {
    path: String,
    steps: Vec<Step>,
    kind: &'static str,
}

/// Every node of the document below the root, depth first.
fn nodes(value: &Json, path: &str, steps: &[Step], out: &mut Vec<Node>) {
    let mut visit = |child: &Json, path: String, step: Step| {
        let mut steps = steps.to_vec();
        steps.push(step);
        out.push(Node {
            path: path.clone(),
            steps: steps.clone(),
            kind: child.kind(),
        });
        nodes(child, &path, &steps, out);
    };
    match value {
        Json::Object(members) => {
            for (key, child) in members {
                visit(child, member_path(path, key), Step::Key(key.clone()));
            }
        }
        Json::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                visit(child, element_path(path, index), Step::Index(index));
            }
        }
        _ => {}
    }
}

fn all_nodes(doc: &Json) -> Vec<Node> {
    let mut out = Vec::new();
    nodes(doc, ROOT, &[], &mut out);
    out
}

fn node_mut<'a>(doc: &'a mut Json, steps: &[Step]) -> &'a mut Json {
    let mut current = doc;
    for step in steps {
        current = match (current, step) {
            (Json::Object(members), Step::Key(key)) => {
                &mut members.iter_mut().find(|(k, _)| k == key).unwrap().1
            }
            (Json::Array(items), Step::Index(index)) => &mut items[*index],
            (other, step) => panic!("{step:?} does not apply to {other:?}"),
        };
    }
    current
}

fn with(doc: &Json, path: &str, value: Json) -> Json {
    let node = all_nodes(doc)
        .into_iter()
        .find(|n| n.path == path)
        .unwrap_or_else(|| panic!("no node {path}"));
    let mut doc = doc.clone();
    *node_mut(&mut doc, &node.steps) = value;
    doc
}

fn text(s: &str) -> Json {
    Json::String(s.into())
}

fn int(v: u64) -> Json {
    Json::Number(Number::PosInt(v))
}

fn errors_of(doc: &Json) -> Vec<ScenarioError> {
    decode(doc).expect_err("document was accepted")
}

fn last_key(path: &str) -> &str {
    path.rsplit('.').next().unwrap()
}

// -------------------------------------------------------------- round trip

#[test]
fn the_document_has_exactly_the_documented_members_in_order() {
    let doc = document();
    let keys =
        |v: &Json| -> Vec<String> { v.members().unwrap().iter().map(|m| m.0.clone()).collect() };
    assert_eq!(
        keys(&doc),
        [
            "schema_version",
            "name",
            "origin",
            "altitude_m",
            "mode",
            "movement",
            "noise",
            "horizontal_accuracy_m",
            "vertical_accuracy_m",
            "update_interval_s",
            "seed",
            "route",
            "playback"
        ]
    );
    assert_eq!(keys(doc.get("origin").unwrap()), ["latitude", "longitude"]);
    assert_eq!(keys(doc.get("movement").unwrap()).len(), 15);
    assert_eq!(keys(doc.get("noise").unwrap()).len(), 8);
    assert_eq!(
        keys(doc.get("playback").unwrap()),
        ["speed", "looping", "reverse"]
    );
    assert_eq!(keys(doc.get("route").unwrap()), ["name", "points"]);
    assert_eq!(doc.get("seed"), Some(&text("12345")));
    assert_eq!(doc.get("schema_version"), Some(&int(1)));
    assert_eq!(doc.get("mode"), Some(&text("route_replay")));
}

/// The document of `sample()`, written out by hand-checked text. A member
/// stored under the wrong name on both the writing and the reading side
/// would survive every round trip; it does not survive this.
const SAMPLE_V1: &str = include_str!("sample_v1.json");

#[test]
fn the_sample_document_is_exactly_the_reference_text() {
    assert_eq!(write(&document()).unwrap(), SAMPLE_V1);
    let read = decode(&parse(SAMPLE_V1).unwrap()).unwrap();
    assert_eq!(fingerprint(&read), fingerprint(&sample()));
}

#[test]
fn the_sample_survives_text_and_back() {
    let s = sample();
    let back = decode(&parse(&write(&encode(&s).unwrap()).unwrap()).unwrap()).unwrap();
    assert_eq!(fingerprint(&back), fingerprint(&s));
    assert_eq!(back, s);
}

#[test]
fn every_variant_has_a_distinct_name_that_reads_back() {
    let mut seen = Vec::new();
    for mode in MODES {
        let mut s = sample();
        s.mode = mode;
        assert_eq!(decode(&encode(&s).unwrap()).unwrap().mode, mode);
        seen.push(mode_name(mode));
    }
    for direction in DIRECTIONS {
        let mut s = sample();
        s.movement.direction = direction;
        assert_eq!(
            decode(&encode(&s).unwrap()).unwrap().movement.direction,
            direction
        );
        seen.push(direction_name(direction));
    }
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), seen.len());
}

fn any_finite(rng: &mut Rng) -> f64 {
    loop {
        let v = f64::from_bits(rng.next_u64());
        if v.is_finite() {
            return v;
        }
    }
}

fn any_coordinate(rng: &mut Rng) -> Coordinate {
    const LATITUDES: [f64; 8] = [
        90.0,
        -90.0,
        0.0,
        -0.0,
        89.99999999999999,
        -89.99999999999999,
        5e-324,
        66.5,
    ];
    const LONGITUDES: [f64; 8] = [
        180.0,
        -180.0,
        0.0,
        -0.0,
        179.99999999999997,
        -179.99999999999997,
        -5e-324,
        77.6245,
    ];
    let pick = |rng: &mut Rng, edges: &[f64; 8], range: f64| {
        if rng.next_u64() % 3 == 0 {
            edges[(rng.next_u64() % 8) as usize]
        } else {
            rng.uniform(-range, range)
        }
    };
    let latitude = pick(rng, &LATITUDES, 90.0);
    let longitude = pick(rng, &LONGITUDES, 180.0);
    at(latitude, longitude)
}

fn any_string(rng: &mut Rng) -> String {
    let len = (rng.next_u64() % 10) as usize;
    (0..len)
        .filter_map(|_| char::from_u32((rng.next_u64() % 0x11_0000) as u32))
        .collect()
}

/// A scenario whose every number is an arbitrary finite bit pattern. Almost
/// never a valid simulation; the codec must carry it unchanged regardless.
fn any_scenario(rng: &mut Rng) -> Scenario {
    let optional = |rng: &mut Rng| (rng.next_u64() % 2 == 0).then(|| any_finite(rng));
    let route = (rng.next_u64() % 2 == 0).then(|| {
        let count = 2 + (rng.next_u64() % 5) as usize;
        let with_altitude = rng.next_u64() % 2 == 0;
        // Strictly increasing times that end anywhere up to i64::MAX.
        let last = match rng.next_u64() % 3 {
            0 => i64::MAX,
            1 => count as i64 - 1,
            _ => (rng.next_u64() >> 1).max(count as u64) as i64,
        };
        let points = (0..count)
            .map(|j| {
                let elapsed_ns = if j + 1 == count {
                    last
                } else {
                    (last / (count as i64 - 1)) * j as i64
                };
                let p = RoutePoint::new(elapsed_ns, any_coordinate(rng));
                if with_altitude {
                    p.with_altitude(any_finite(rng))
                } else {
                    p
                }
            })
            .collect();
        let route = Route::new(points).unwrap();
        if rng.next_u64() % 2 == 0 {
            route.with_name(any_string(rng))
        } else {
            route
        }
    });
    Scenario {
        schema_version: rng.next_u64() as u32,
        name: any_string(rng),
        origin: any_coordinate(rng),
        altitude_m: any_finite(rng),
        mode: MODES[(rng.next_u64() % 6) as usize],
        movement: MovementParameters {
            min_speed_mps: any_finite(rng),
            max_speed_mps: any_finite(rng),
            max_acceleration_mps2: any_finite(rng),
            max_deceleration_mps2: any_finite(rng),
            max_heading_rate_dps: any_finite(rng),
            radius_m: optional(rng),
            step_distance_m: optional(rng),
            heading_persistence: any_finite(rng),
            pause_probability: any_finite(rng),
            max_pause_s: any_finite(rng),
            angular_velocity_dps: optional(rng),
            direction: DIRECTIONS[(rng.next_u64() % 2) as usize],
            start_phase_deg: any_finite(rng),
            speed_change_interval_s: any_finite(rng),
            max_displacement_per_sample_m: optional(rng),
        },
        noise: NoiseParameters {
            position_noise_m: any_finite(rng),
            max_position_offset_m: any_finite(rng),
            speed_noise_mps: any_finite(rng),
            heading_noise_deg: any_finite(rng),
            accuracy_noise_m: any_finite(rng),
            drift_rate_mps: any_finite(rng),
            position_correlation_time_s: any_finite(rng),
            max_offset_rate_mps: any_finite(rng),
        },
        horizontal_accuracy_m: any_finite(rng),
        vertical_accuracy_m: any_finite(rng),
        update_interval_s: any_finite(rng),
        seed: match rng.next_u64() % 4 {
            0 => 0,
            1 => u64::MAX,
            _ => rng.next_u64(),
        },
        route,
        playback: PlaybackParameters {
            speed: any_finite(rng),
            looping: rng.next_u64() % 2 == 0,
            reverse: rng.next_u64() % 2 == 0,
        },
    }
}

#[test]
fn arbitrary_finite_scenarios_round_trip_bit_exactly_through_text() {
    let mut rng = Rng::from_seed(0x7_08);
    for case in 0..4_000 {
        let s = any_scenario(&mut rng);
        let doc = encode(&s).unwrap();
        let written = write(&doc).unwrap();
        let back = decode(&parse(&written).unwrap())
            .unwrap_or_else(|e| panic!("case {case}: {e:?}\n{written}"));
        assert_eq!(
            fingerprint(&back),
            fingerprint(&s),
            "case {case}\n{written}"
        );
        // Writing what was read gives the same text: the form is canonical.
        assert_eq!(
            write(&encode(&back).unwrap()).unwrap(),
            written,
            "case {case}"
        );
    }
}

// ------------------------------------------------------------- strictness

#[test]
fn each_missing_member_is_reported_by_its_path() {
    let doc = document();
    let all = all_nodes(&doc);
    let mut removed = 0;
    for node in &all {
        let Some((Step::Key(key), parent)) = node.steps.split_last() else {
            continue;
        };
        let mut broken = doc.clone();
        match node_mut(&mut broken, parent) {
            Json::Object(members) => members.retain(|(k, _)| k != key),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            errors_of(&broken),
            [ScenarioError::MissingField {
                path: node.path.clone()
            }],
            "{}",
            node.path
        );
        removed += 1;
    }
    // 13 + origin 2 + movement 15 + noise 8 + route 2 + 3 points × (3 + 2)
    // + playback 3.
    assert_eq!(removed, 58);
}

#[test]
fn an_unknown_member_is_rejected_in_every_object() {
    let doc = document();
    let mut objects = vec![(ROOT.to_string(), Vec::new())];
    objects.extend(
        all_nodes(&doc)
            .into_iter()
            .filter(|n| n.kind == "object")
            .map(|n| (n.path, n.steps)),
    );
    assert_eq!(objects.len(), 12);
    for (path, steps) in objects {
        for extra in [
            Json::Null,
            int(1),
            text("x"),
            Json::Object(vec![]),
            Json::Array(vec![]),
        ] {
            let mut broken = doc.clone();
            match node_mut(&mut broken, &steps) {
                Json::Object(members) => members.push(("comment".into(), extra)),
                other => panic!("{other:?}"),
            }
            assert_eq!(
                errors_of(&broken),
                [ScenarioError::UnknownField {
                    path: member_path(&path, "comment")
                }],
                "{path}"
            );
        }
    }
}

#[test]
fn member_names_are_case_sensitive_and_exact() {
    let mut doc = document();
    let Json::Object(members) = &mut doc else {
        panic!()
    };
    members.iter_mut().find(|m| m.0 == "seed").unwrap().0 = "Seed".into();
    assert_eq!(
        errors_of(&doc),
        [
            ScenarioError::MissingField {
                path: "seed".into()
            },
            ScenarioError::UnknownField {
                path: "Seed".into()
            }
        ]
    );
}

/// The JSON kinds a member accepts, by where it is.
fn accepted_kinds(node: &Node) -> &'static [&'static str] {
    const OPTIONAL_FLOATS: [&str; 5] = [
        "radius_m",
        "step_distance_m",
        "angular_velocity_dps",
        "max_displacement_per_sample_m",
        "altitude_m",
    ];
    let key = last_key(&node.path);
    let in_route_point = node.path.starts_with("route.points[");
    match node.kind {
        "object" if node.path == "route" => &["object", "null"],
        "object" => &["object"],
        "array" => &["array"],
        "boolean" => &["boolean"],
        "integer" => &["integer"],
        "string" if node.path == "route.name" => &["string", "null"],
        "string" => &["string"],
        "non-integer number" if OPTIONAL_FLOATS.contains(&key) && key != "altitude_m" => {
            &["integer", "non-integer number", "null"]
        }
        "non-integer number" if key == "altitude_m" && in_route_point => {
            &["integer", "non-integer number", "null"]
        }
        "non-integer number" => &["integer", "non-integer number"],
        other => panic!("{}: unexpected kind {other}", node.path),
    }
}

#[test]
fn every_member_refuses_every_json_type_it_does_not_take() {
    let doc = document();
    let replacements = [
        Json::Null,
        Json::Bool(true),
        int(1),
        Json::float(1.5),
        text("1"),
        Json::Array(vec![]),
        Json::Object(vec![]),
    ];
    let (mut refused, mut taken) = (0, 0);
    for node in all_nodes(&doc) {
        let accepted = accepted_kinds(&node);
        for replacement in &replacements {
            if replacement.kind() == node.kind && matches!(node.kind, "object" | "array") {
                continue; // an empty container is a different failure
            }
            let broken = with(&doc, &node.path, replacement.clone());
            let result = decode(&broken);
            if accepted.contains(&replacement.kind()) {
                // The type is right; the value may still be refused, but
                // then for what it is and where it is.
                taken += 1;
                if let Err(errors) = result {
                    assert_eq!(errors.len(), 1, "{}: {errors:?}", node.path);
                    match &errors[0] {
                        ScenarioError::InvalidValue { path, .. } => assert_eq!(path, &node.path),
                        ScenarioError::InvalidRoute { path, .. } => {
                            assert_eq!(path, "route.points")
                        }
                        other => panic!("{}: {other:?}", node.path),
                    }
                }
            } else {
                refused += 1;
                match result.expect_err(&node.path).as_slice() {
                    [ScenarioError::WrongType { path, found, .. }] => {
                        assert_eq!(path, &node.path);
                        assert_eq!(*found, replacement.kind());
                    }
                    other => panic!("{} <- {replacement:?}: {other:?}", node.path),
                }
            }
        }
    }
    // Fixed by the schema; a change here means a member changed its type.
    // Seven replacements per member (six for a container, which is not
    // replaced by its own kind):
    //   4 integers            take 1, refuse 6
    //   4 strings             take 1, refuse 6   (route.name: take 2, refuse 5)
    //   2 booleans            take 1, refuse 6
    //   31 required floats    take 2, refuse 5
    //   7 optional floats     take 3, refuse 4
    //   10 objects, 1 array   refuse 6           (route: take null, refuse 5)
    assert_eq!(refused, 24 + 24 + 5 + 12 + 155 + 28 + 60 + 5 + 6);
    assert_eq!(taken, 4 + 4 + 2 + 2 + 62 + 21 + 1);
}

#[test]
fn optional_members_take_an_explicit_null() {
    let mut doc = document();
    for path in [
        "movement.radius_m",
        "movement.step_distance_m",
        "movement.angular_velocity_dps",
        "movement.max_displacement_per_sample_m",
        "route.name",
        "route.points[0].altitude_m",
        "route.points[1].altitude_m",
        "route.points[2].altitude_m",
    ] {
        doc = with(&doc, path, Json::Null);
    }
    let s = decode(&doc).unwrap();
    assert_eq!(s.movement.radius_m, None);
    assert_eq!(s.movement.step_distance_m, None);
    assert_eq!(s.movement.angular_velocity_dps, None);
    assert_eq!(s.movement.max_displacement_per_sample_m, None);
    let route = s.route.unwrap();
    assert_eq!(route.name(), None);
    assert!(!route.has_altitude());

    let s = decode(&with(&document(), "route", Json::Null)).unwrap();
    assert_eq!(s.route, None);
}

#[test]
fn seed_is_a_canonical_decimal_string_over_the_whole_u64_range() {
    let doc = document();
    for (written, value) in [
        ("0", 0),
        ("1", 1),
        ("9007199254740993", (1u64 << 53) + 1),
        ("18446744073709551615", u64::MAX),
    ] {
        assert_eq!(
            decode(&with(&doc, "seed", text(written))).unwrap().seed,
            value
        );
        let mut s = sample();
        s.seed = value;
        assert_eq!(encode(&s).unwrap().get("seed"), Some(&text(written)));
    }
    for written in [
        "",
        " ",
        "-0",
        "-1",
        "+1",
        "00",
        "01",
        "007",
        " 1",
        "1 ",
        "1\n",
        "1.0",
        "1e3",
        "0x1f",
        "1_000",
        "1,000",
        "١٢٣",
        "１２３",
        "abc",
        "18446744073709551616",
        "99999999999999999999",
        "118446744073709551615",
    ] {
        match errors_of(&with(&doc, "seed", text(written))).as_slice() {
            [ScenarioError::InvalidValue { path, .. }] => assert_eq!(path, "seed"),
            other => panic!("{written:?}: {other:?}"),
        }
    }
    // A number is the wrong type, however small.
    for number in [int(0), int(12345), Json::float(1.0)] {
        assert!(matches!(
            errors_of(&with(&doc, "seed", number)).as_slice(),
            [ScenarioError::WrongType { path, .. }] if path == "seed"
        ));
    }
}

#[test]
fn integer_members_take_only_integer_literals_in_range() {
    // Through text, because the literal's spelling is what matters.
    let template = write(&document()).unwrap();
    let with_elapsed = |literal: &str| {
        let text = template.replace(
            "\"elapsed_ns\": 5000000001",
            &format!("\"elapsed_ns\": {literal}"),
        );
        assert_ne!(text, template);
        decode(&parse(&text).unwrap())
    };
    assert_eq!(
        with_elapsed("9223372036854775807")
            .unwrap()
            .route
            .unwrap()
            .duration_ns(),
        i64::MAX
    );
    for literal in ["5000000001.0", "5e9", "5000000001.5", "\"5000000001\""] {
        assert!(
            matches!(
                with_elapsed(literal).unwrap_err().as_slice(),
                [ScenarioError::WrongType { path, expected: "integer", .. }]
                    if path == "route.points[2].elapsed_ns"
            ),
            "{literal}"
        );
    }
    // One past i64::MAX is still an integer literal, but out of range; one
    // past u64::MAX is no longer exactly representable at all.
    assert!(matches!(
        with_elapsed("9223372036854775808").unwrap_err().as_slice(),
        [ScenarioError::InvalidValue { path, .. }] if path == "route.points[2].elapsed_ns"
    ));
    assert!(matches!(
        with_elapsed("18446744073709551616").unwrap_err().as_slice(),
        [ScenarioError::WrongType { path, .. }] if path == "route.points[2].elapsed_ns"
    ));

    let with_version = |literal: &str| {
        let text = template.replace(
            "\"schema_version\": 1",
            &format!("\"schema_version\": {literal}"),
        );
        assert_ne!(text, template);
        decode(&parse(&text).unwrap())
    };
    assert_eq!(with_version("4294967295").unwrap().schema_version, u32::MAX);
    for literal in ["4294967296", "-1", "18446744073709551615"] {
        assert!(
            matches!(
                with_version(literal).unwrap_err().as_slice(),
                [ScenarioError::InvalidValue { path, .. }] if path == "schema_version"
            ),
            "{literal}"
        );
    }
    for literal in ["1.0", "1e0", "\"1\"", "null", "-0"] {
        assert!(
            matches!(
                with_version(literal).unwrap_err().as_slice(),
                [ScenarioError::WrongType { path, .. }] if path == "schema_version"
            ),
            "{literal}"
        );
    }
}

#[test]
fn coordinates_are_validated_never_wrapped_or_clamped() {
    let doc = document();
    for (path, value) in [
        ("origin.latitude", 90.0),
        ("origin.latitude", -90.0),
        ("origin.longitude", 180.0),
        ("origin.longitude", -180.0),
        ("route.points[1].coordinate.longitude", 180.0),
        ("route.points[1].coordinate.latitude", -90.0),
    ] {
        let s = decode(&with(&doc, path, Json::float(value))).unwrap();
        let c = if path.starts_with("origin") {
            s.origin
        } else {
            s.route.unwrap().points()[1].coordinate
        };
        let read = if path.ends_with("latitude") {
            c.latitude()
        } else {
            c.longitude()
        };
        assert_eq!(read.to_bits(), value.to_bits(), "{path}");
    }
    for (path, value, parent) in [
        ("origin.latitude", 90.00000000000001, "origin"),
        ("origin.latitude", -90.00000000000001, "origin"),
        ("origin.longitude", 180.00000000000003, "origin"),
        ("origin.longitude", -180.00000000000003, "origin"),
        ("origin.longitude", 540.0, "origin"),
        ("origin.latitude", 1e300, "origin"),
        (
            "route.points[2].coordinate.longitude",
            181.0,
            "route.points[2].coordinate",
        ),
    ] {
        match errors_of(&with(&doc, path, Json::float(value))).as_slice() {
            [ScenarioError::InvalidCoordinate { path, .. }] => assert_eq!(path, parent),
            other => panic!("{path} = {value}: {other:?}"),
        }
    }
}

#[test]
fn names_of_modes_and_directions_are_exact() {
    let doc = document();
    for (path, written) in [
        ("mode", "Walking"),
        ("mode", "WALKING"),
        ("mode", "walk"),
        ("mode", "walking "),
        ("mode", "RouteReplay"),
        ("mode", "route-replay"),
        ("mode", ""),
        ("movement.direction", "Clockwise"),
        ("movement.direction", "cw"),
        ("movement.direction", "counterclockwise"),
        ("movement.direction", "anticlockwise"),
    ] {
        match errors_of(&with(&doc, path, text(written))).as_slice() {
            [ScenarioError::InvalidValue { path: p, reason }] => {
                assert_eq!(p, path);
                assert!(reason.contains("expected one of"), "{reason}");
            }
            other => panic!("{path} = {written:?}: {other:?}"),
        }
    }
}

#[test]
fn a_structurally_bad_route_is_reported_not_repaired() {
    let doc = document();
    // Out of order, first point not at zero, altitude on some points only,
    // a single point: all errors from `Route::new`, none sorted or dropped.
    let cases = [
        with(&doc, "route.points[1].elapsed_ns", int(6_000_000_000)),
        with(&doc, "route.points[0].elapsed_ns", int(1)),
        with(&doc, "route.points[1].altitude_m", Json::Null),
        with(
            &doc,
            "route.points",
            Json::Array(vec![doc
                .get("route")
                .and_then(|r| r.get("points"))
                .map(|p| match p {
                    Json::Array(items) => items[0].clone(),
                    other => panic!("{other:?}"),
                })
                .unwrap()]),
        ),
        with(&doc, "route.points", Json::Array(vec![])),
    ];
    for case in cases {
        match errors_of(&case).as_slice() {
            [ScenarioError::InvalidRoute { path, .. }] => assert_eq!(path, "route.points"),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn every_problem_in_a_document_is_reported_together() {
    let mut doc = document();
    doc = with(&doc, "name", int(1));
    doc = with(&doc, "origin.latitude", Json::float(91.0));
    doc = with(&doc, "mode", text("flying"));
    doc = with(&doc, "seed", text("-1"));
    doc = with(&doc, "noise.drift_rate_mps", Json::Bool(false));
    doc = with(&doc, "route.points[1].coordinate.longitude", text("east"));
    doc = with(&doc, "route.points[2].elapsed_ns", Json::float(1.5));
    doc = with(&doc, "playback", Json::Array(vec![]));
    match &mut doc {
        Json::Object(members) => {
            members.retain(|m| m.0 != "altitude_m");
            members.push(("notes".into(), Json::Null));
        }
        other => panic!("{other:?}"),
    }
    let found: Vec<String> = errors_of(&doc).iter().map(|e| e.to_string()).collect();
    assert_eq!(
        found,
        [
            "name: expected string, found integer",
            "origin: latitude 91 outside [-90, 90]",
            "altitude_m: missing",
            "mode: unknown value \"flying\"; expected one of fixed, random_walk, walking, \
             driving, circular, route_replay",
            "noise.drift_rate_mps: expected number, found boolean",
            "seed: \"-1\" is not a canonical decimal integer (digits only, no leading zeros)",
            "route.points[1].coordinate.longitude: expected number, found string",
            "route.points[2].elapsed_ns: expected integer, found non-integer number",
            "playback: expected object, found array",
            "notes: unknown field",
        ]
    );
}

#[test]
fn the_root_must_be_an_object() {
    for root in [
        Json::Null,
        Json::Array(vec![document()]),
        text("{}"),
        int(1),
    ] {
        assert!(matches!(
            errors_of(&root).as_slice(),
            [ScenarioError::WrongType { path, expected: "object", .. }] if path == ROOT
        ));
    }
}

// ------------------------------------------------------------- non-finite

type Setter = fn(&mut Scenario, f64);

macro_rules! plain {
    ($($field:ident).+) => {
        (stringify!($($field).+), (|s, v| s.$($field).+ = v) as Setter)
    };
}

macro_rules! optional {
    ($($field:ident).+) => {
        (stringify!($($field).+), (|s, v| s.$($field).+ = Some(v)) as Setter)
    };
}

fn float_fields() -> Vec<(&'static str, Setter)> {
    vec![
        plain!(altitude_m),
        plain!(movement.min_speed_mps),
        plain!(movement.max_speed_mps),
        plain!(movement.max_acceleration_mps2),
        plain!(movement.max_deceleration_mps2),
        plain!(movement.max_heading_rate_dps),
        optional!(movement.radius_m),
        optional!(movement.step_distance_m),
        plain!(movement.heading_persistence),
        plain!(movement.pause_probability),
        plain!(movement.max_pause_s),
        optional!(movement.angular_velocity_dps),
        plain!(movement.start_phase_deg),
        plain!(movement.speed_change_interval_s),
        optional!(movement.max_displacement_per_sample_m),
        plain!(noise.position_noise_m),
        plain!(noise.max_position_offset_m),
        plain!(noise.speed_noise_mps),
        plain!(noise.heading_noise_deg),
        plain!(noise.accuracy_noise_m),
        plain!(noise.drift_rate_mps),
        plain!(noise.position_correlation_time_s),
        plain!(noise.max_offset_rate_mps),
        plain!(horizontal_accuracy_m),
        plain!(vertical_accuracy_m),
        plain!(update_interval_s),
        plain!(playback.speed),
    ]
}

#[test]
fn a_non_finite_number_cannot_be_encoded_and_is_named() {
    let fields = float_fields();
    // The table covers every float of the document that a `Scenario` can
    // hold non-finite (coordinates and route altitudes cannot be).
    let mut in_document: Vec<String> = all_nodes(&document())
        .into_iter()
        .filter(|n| n.kind == "non-integer number")
        .map(|n| n.path)
        .filter(|p| !p.starts_with("origin.") && !p.starts_with("route."))
        .collect();
    let mut in_table: Vec<String> = fields.iter().map(|f| f.0.replace(' ', "")).collect();
    in_document.sort();
    in_table.sort();
    assert_eq!(in_table, in_document);

    for (name, set) in &fields {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut s = sample();
            set(&mut s, v);
            assert_eq!(
                encode(&s).unwrap_err(),
                [ScenarioError::NonFiniteNumber {
                    path: name.replace(' ', "")
                }],
                "{name}"
            );
        }
    }
    let mut s = sample();
    for (_, set) in &fields {
        set(&mut s, f64::NAN);
    }
    assert_eq!(encode(&s).unwrap_err().len(), fields.len());
}

#[test]
fn a_tree_with_a_non_finite_number_is_not_decoded() {
    // The parser never produces one; a migration step could.
    let doc = with(&document(), "altitude_m", Json::float(f64::NAN));
    assert!(matches!(
        errors_of(&doc).as_slice(),
        [ScenarioError::InvalidValue { path, .. }] if path == "altitude_m"
    ));
}
