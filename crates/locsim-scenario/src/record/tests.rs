use super::*;
use crate::json::{member_path, parse};
use crate::schema::tests::{all_nodes, int, node_mut, text as string_value, with, Step};
use locsim_core::domain::Coordinate;
use locsim_core::rng::Rng;

fn reference() -> LastKnown {
    LastKnown {
        scenario_fingerprint: 0x0123_4567_89ab_cdef,
        sample: SyntheticLocation {
            timestamp: Timestamp::from_nanos(1_700_000_123_000_000_000),
            coordinate: Coordinate::new(12.9352, 77.6245).unwrap(),
            altitude_m: 920.5,
            horizontal_accuracy_m: 5.25,
            vertical_accuracy_m: 8.125,
            speed_mps: Some(1.5),
            course_deg: Some(271.75),
            source: LocationSource::Simulation,
            simulation_state: SimulationState::Running,
        },
        state: SimulationState::Paused,
        sample_count: 124,
        failed_count: 3,
        missed_ticks: 7,
        trajectory_complete: true,
    }
}

/// The document of `reference()`, checked by eye against it.
const REFERENCE: &str = r#"{
  "record_version": 1,
  "scenario_fingerprint": "0123456789abcdef",
  "sample": {
    "timestamp_ns": 1700000123000000000,
    "coordinate": {
      "latitude": 12.9352,
      "longitude": 77.6245
    },
    "altitude_m": 920.5,
    "horizontal_accuracy_m": 5.25,
    "vertical_accuracy_m": 8.125,
    "speed_mps": 1.5,
    "course_deg": 271.75,
    "source": "simulation",
    "simulation_state": "running"
  },
  "provider": {
    "state": "paused",
    "sample_count": "124",
    "failed_count": "3",
    "missed_ticks": "7",
    "trajectory_complete": true
  }
}
"#;

fn document() -> Json {
    parse(REFERENCE).unwrap()
}

fn import_tree(document: &Json) -> Result<LastKnown, Errors> {
    import_last_known(&json::write(document).unwrap())
}

fn errors_of(document: &Json) -> Errors {
    import_tree(document).expect_err("record was accepted")
}

fn bits(record: &LastKnown) -> String {
    format!("{record:?}")
}

// --------------------------------------------------------------- fingerprint

#[test]
fn fnv_1a_64_matches_the_published_vectors() {
    assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
}

// ---------------------------------------------------------------- round trip

#[test]
fn the_reference_record_is_exactly_the_reference_text() {
    assert_eq!(export_last_known(&reference()).unwrap(), REFERENCE);
    assert_eq!(
        bits(&import_last_known(REFERENCE).unwrap()),
        bits(&reference())
    );
}

fn any_finite(rng: &mut Rng) -> f64 {
    loop {
        let v = f64::from_bits(rng.next_u64());
        if v.is_finite() {
            return v;
        }
    }
}

fn any_u64(rng: &mut Rng) -> u64 {
    match rng.next_u64() % 4 {
        0 => 0,
        1 => u64::MAX,
        2 => rng.next_u64() % 1_000,
        _ => rng.next_u64(),
    }
}

/// Any record that may be written: a sample fit to emit, at least one
/// sample counted, everything else arbitrary.
fn any_record(rng: &mut Rng) -> LastKnown {
    let speed_mps = match rng.next_u64() % 4 {
        0 => None,
        1 => Some(0.0),
        _ => Some(any_finite(rng).abs()),
    };
    let moving = matches!(speed_mps, Some(v) if v > 0.0);
    let course_deg = (moving && rng.next_u64() % 3 != 0).then(|| match rng.next_u64() % 4 {
        0 => 0.0,
        1 => 359.99999999999994,
        _ => rng.uniform(0.0, 360.0),
    });
    let timestamp = match rng.next_u64() % 5 {
        0 => i64::MIN,
        1 => i64::MAX,
        2 => 0,
        3 => -1,
        _ => rng.next_u64() as i64,
    };
    let latitude = [90.0, -90.0, -0.0, rng.uniform(-90.0, 90.0)][(rng.next_u64() % 4) as usize];
    let longitude =
        [180.0, -180.0, -0.0, rng.uniform(-180.0, 180.0)][(rng.next_u64() % 4) as usize];
    LastKnown {
        scenario_fingerprint: any_u64(rng),
        sample: SyntheticLocation {
            timestamp: Timestamp::from_nanos(timestamp),
            coordinate: Coordinate::new(latitude, longitude).unwrap(),
            altitude_m: any_finite(rng),
            horizontal_accuracy_m: any_finite(rng).abs(),
            vertical_accuracy_m: any_finite(rng).abs(),
            speed_mps,
            course_deg,
            source: SOURCES[(rng.next_u64() % 3) as usize],
            simulation_state: STATES[(rng.next_u64() % 7) as usize],
        },
        state: STATES[(rng.next_u64() % 7) as usize],
        sample_count: any_u64(rng).max(1),
        failed_count: any_u64(rng),
        missed_ticks: any_u64(rng),
        trajectory_complete: rng.next_u64() % 2 == 0,
    }
}

#[test]
fn arbitrary_records_round_trip_bit_exactly_and_canonically() {
    let mut rng = Rng::from_seed(0x09_01);
    let (mut unknown_speed, mut with_course) = (0, 0);
    for case in 0..20_000 {
        let record = any_record(&mut rng);
        let written = export_last_known(&record).unwrap_or_else(|e| panic!("case {case}: {e:?}"));
        let back =
            import_last_known(&written).unwrap_or_else(|e| panic!("case {case}: {e:?}\n{written}"));
        assert_eq!(bits(&back), bits(&record), "case {case}\n{written}");
        assert_eq!(export_last_known(&back).unwrap(), written, "case {case}");
        unknown_speed += usize::from(record.sample.speed_mps.is_none());
        with_course += usize::from(record.sample.course_deg.is_some());
    }
    assert!(unknown_speed > 3_000 && with_course > 3_000);
}

#[test]
fn unknown_speed_and_course_stay_unknown_and_zero_stays_zero() {
    let mut record = reference();
    record.sample.speed_mps = None;
    record.sample.course_deg = None;
    let written = export_last_known(&record).unwrap();
    assert!(written.contains("\"speed_mps\": null") && written.contains("\"course_deg\": null"));
    let back = import_last_known(&written).unwrap();
    assert_eq!(
        (back.sample.speed_mps, back.sample.course_deg),
        (None, None)
    );

    record.sample.speed_mps = Some(0.0);
    let back = import_last_known(&export_last_known(&record).unwrap()).unwrap();
    assert_eq!(back.sample.speed_mps, Some(0.0));
}

#[test]
fn every_state_and_source_has_a_distinct_name_that_reads_back() {
    let mut names = Vec::new();
    for state in STATES {
        let mut record = reference();
        record.state = state;
        record.sample.simulation_state = state;
        let back = import_last_known(&export_last_known(&record).unwrap()).unwrap();
        assert_eq!((back.state, back.sample.simulation_state), (state, state));
        names.push(state_name(state));
    }
    for source in SOURCES {
        let mut record = reference();
        record.sample.source = source;
        let back = import_last_known(&export_last_known(&record).unwrap()).unwrap();
        assert_eq!(back.sample.source, source);
        names.push(source_name(source));
    }
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count);
}

// ---------------------------------------------------------------- strictness

#[test]
fn each_missing_member_is_reported_by_its_path() {
    let doc = document();
    let mut removed = 0;
    for node in all_nodes(&doc) {
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
    // 4 at the root, 9 in the sample, 2 in its coordinate, 5 for the provider.
    assert_eq!(removed, 20);
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
    assert_eq!(objects.len(), 4);
    for (path, steps) in objects {
        let mut broken = doc.clone();
        match node_mut(&mut broken, &steps) {
            Json::Object(members) => members.push(("run_started_ns".into(), int(0))),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            errors_of(&broken),
            [ScenarioError::UnknownField {
                path: member_path(&path, "run_started_ns")
            }],
            "{path}"
        );
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
        string_value("1"),
        Json::Array(vec![]),
        Json::Object(vec![]),
    ];
    let (mut refused, mut taken) = (0, 0);
    for node in all_nodes(&doc) {
        let optional = matches!(node.path.as_str(), "sample.speed_mps" | "sample.course_deg");
        let accepted: &[&str] = match node.kind {
            "object" => &["object"],
            "boolean" => &["boolean"],
            "integer" => &["integer"],
            "string" => &["string"],
            "non-integer number" if optional => &["integer", "non-integer number", "null"],
            "non-integer number" => &["integer", "non-integer number"],
            other => panic!("{}: unexpected kind {other}", node.path),
        };
        for replacement in &replacements {
            if replacement.kind() == "object" && node.kind == "object" {
                continue;
            }
            let result = import_tree(&with(&doc, &node.path, replacement.clone()));
            if accepted.contains(&replacement.kind()) {
                taken += 1;
                if let Err(errors) = result {
                    assert_eq!(errors.len(), 1, "{}: {errors:?}", node.path);
                    match &errors[0] {
                        ScenarioError::InvalidValue { path, .. } => assert_eq!(path, &node.path),
                        ScenarioError::InvalidSample { path, .. } => assert_eq!(path, "sample"),
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
    // Seven replacements per member, six for an object:
    //   2 integers          take 1, refuse 6
    //   7 strings           take 1, refuse 6
    //   1 boolean           take 1, refuse 6
    //   5 required floats   take 2, refuse 5
    //   2 optional floats   take 3, refuse 4
    //   3 objects           refuse 6
    assert_eq!(refused, 12 + 42 + 6 + 25 + 8 + 18);
    assert_eq!(taken, 2 + 7 + 1 + 10 + 6);
}

#[test]
fn fingerprint_and_counters_have_one_spelling_each() {
    let doc = document();
    for written in [
        "",
        "123456789abcdef",
        "00123456789abcdef",
        "0123456789ABCDEF",
        "0x23456789abcdef",
        "+123456789abcdef",
        "0123456789abcdeg",
        " 123456789abcdef",
    ] {
        match errors_of(&with(&doc, "scenario_fingerprint", string_value(written))).as_slice() {
            [ScenarioError::InvalidValue { path, .. }] => assert_eq!(path, "scenario_fingerprint"),
            other => panic!("{written:?}: {other:?}"),
        }
    }
    for (written, value) in [("0000000000000000", 0), ("ffffffffffffffff", u64::MAX)] {
        let doc = with(&doc, "scenario_fingerprint", string_value(written));
        assert_eq!(import_tree(&doc).unwrap().scenario_fingerprint, value);
    }
    for counter in ["sample_count", "failed_count", "missed_ticks"] {
        let path = format!("provider.{counter}");
        for written in ["", "01", "+1", "-1", "1.0", " 1", "18446744073709551616"] {
            match errors_of(&with(&doc, &path, string_value(written))).as_slice() {
                [ScenarioError::InvalidValue { path: p, .. }] => assert_eq!(p, &path),
                other => panic!("{path} = {written:?}: {other:?}"),
            }
        }
        let max = with(&doc, &path, string_value("18446744073709551615"));
        assert!(import_tree(&max).is_ok(), "{path}");
    }
}

#[test]
fn stages_report_in_order() {
    let invalid = REFERENCE.replace("\"speed_mps\": 1.5", "\"speed_mps\": null");
    let unknown = invalid.replace("\"altitude_m\"", "\"altitude\"");
    let version = unknown.replace("\"record_version\": 1", "\"record_version\": 2");
    let duplicate = version.replace(
        "\"trajectory_complete\": true",
        "\"trajectory_complete\": true, \"trajectory_complete\": true",
    );
    let syntax = duplicate.replace("\"state\": \"paused\",", "\"state\": \"paused\"");
    for (a, b) in [
        (REFERENCE, invalid.as_str()),
        (&invalid, &unknown),
        (&unknown, &version),
        (&version, &duplicate),
        (&duplicate, &syntax),
    ] {
        assert_ne!(a, b);
    }

    assert!(matches!(
        import_last_known(&syntax).unwrap_err().as_slice(),
        [ScenarioError::Syntax { .. }]
    ));
    assert_eq!(
        import_last_known(&duplicate).unwrap_err(),
        [ScenarioError::DuplicateKey {
            path: "provider.trajectory_complete".into()
        }]
    );
    assert_eq!(
        import_last_known(&version).unwrap_err(),
        [ScenarioError::UnsupportedVersion {
            found: 2,
            supported: 1
        }]
    );
    assert_eq!(
        import_last_known(&unknown).unwrap_err(),
        [
            ScenarioError::MissingField {
                path: "sample.altitude_m".into()
            },
            ScenarioError::UnknownField {
                path: "sample.altitude".into()
            }
        ]
    );
    // A course without a speed: well formed, but not a sample.
    assert_eq!(
        import_last_known(&invalid).unwrap_err(),
        [ScenarioError::InvalidSample {
            path: "sample".into(),
            error: locsim_core::domain::LocationError::CourseWithoutMotion
        }]
    );
}

#[test]
fn versions_other_than_the_current_one_are_refused() {
    for (literal, found) in [("0", 0u64), ("2", 2), ("18446744073709551615", u64::MAX)] {
        let text = REFERENCE.replace(
            "\"record_version\": 1",
            &format!("\"record_version\": {literal}"),
        );
        assert_eq!(
            import_last_known(&text).unwrap_err(),
            [ScenarioError::UnsupportedVersion {
                found,
                supported: CURRENT_RECORD_VERSION
            }]
        );
    }
    // A scenario document is not a record, and the other way round.
    let as_scenario = REFERENCE.replace("record_version", "schema_version");
    assert_eq!(
        import_last_known(&as_scenario).unwrap_err(),
        [ScenarioError::MissingField {
            path: "record_version".into()
        }]
    );
    assert_eq!(
        crate::import_scenario(REFERENCE).unwrap_err(),
        [ScenarioError::MissingField {
            path: "schema_version".into()
        }]
    );
}

#[test]
fn a_record_must_hold_a_sample_that_was_fit_to_emit() {
    type Damage = fn(&mut LastKnown);
    let damage: [(Damage, &str); 7] = [
        (|r| r.sample.altitude_m = f64::NAN, "sample"),
        (|r| r.sample.horizontal_accuracy_m = -1.0, "sample"),
        (|r| r.sample.vertical_accuracy_m = f64::INFINITY, "sample"),
        (|r| r.sample.speed_mps = Some(-0.5), "sample"),
        (|r| r.sample.course_deg = Some(360.0), "sample"),
        (|r| r.sample.speed_mps = Some(0.0), "sample"),
        (|r| r.sample_count = 0, "provider.sample_count"),
    ];
    for (case, (apply, path)) in damage.iter().enumerate() {
        let mut record = reference();
        apply(&mut record);
        let errors = export_last_known(&record).expect_err("written");
        assert_eq!(errors.len(), 1, "case {case}");
        match &errors[0] {
            ScenarioError::InvalidSample { path: p, .. }
            | ScenarioError::InvalidValue { path: p, .. } => assert_eq!(p, path, "case {case}"),
            other => panic!("case {case}: {other:?}"),
        }
    }
    // The same rules hold for a document somebody else wrote.
    for (from, to, path) in [
        (
            "\"horizontal_accuracy_m\": 5.25",
            "\"horizontal_accuracy_m\": -5.25",
            "sample",
        ),
        ("\"course_deg\": 271.75", "\"course_deg\": 360", "sample"),
        ("\"speed_mps\": 1.5", "\"speed_mps\": -1.5", "sample"),
        (
            "\"sample_count\": \"124\"",
            "\"sample_count\": \"0\"",
            "provider.sample_count",
        ),
    ] {
        let text = REFERENCE.replace(from, to);
        assert_ne!(text, REFERENCE);
        let errors = import_last_known(&text).expect_err("read");
        match errors.as_slice() {
            [ScenarioError::InvalidSample { path: p, .. }]
            | [ScenarioError::InvalidValue { path: p, .. }] => assert_eq!(p, path),
            other => panic!("{to}: {other:?}"),
        }
    }
    let out_of_range = REFERENCE.replace("\"latitude\": 12.9352", "\"latitude\": 90.5");
    assert!(matches!(
        import_last_known(&out_of_range).unwrap_err().as_slice(),
        [ScenarioError::InvalidCoordinate { path, .. }] if path == "sample.coordinate"
    ));
}

#[test]
fn a_status_that_names_another_sample_is_refused() {
    let record = reference();
    let mut status = ProviderStatus {
        state: record.state,
        sample_count: record.sample_count,
        failed_count: record.failed_count,
        missed_ticks: record.missed_ticks,
        last_sample_time: Some(record.sample.timestamp),
        trajectory_complete: record.trajectory_complete,
    };
    assert_eq!(
        LastKnown::new(record.scenario_fingerprint, record.sample, &status),
        Ok(record)
    );
    for other in [
        None,
        Some(Timestamp::from_nanos(
            record.sample.timestamp.as_nanos() + 1,
        )),
    ] {
        status.last_sample_time = other;
        assert!(matches!(
            LastKnown::new(record.scenario_fingerprint, record.sample, &status),
            Err(ScenarioError::InvalidValue { path, .. }) if path == "sample.timestamp_ns"
        ));
    }
}

#[test]
fn every_truncation_is_a_syntax_error_and_damage_never_panics() {
    let text = REFERENCE.trim_end();
    for end in 0..text.len() {
        assert!(
            matches!(
                import_last_known(&text[..end]).unwrap_err().as_slice(),
                [ScenarioError::Syntax { .. }]
            ),
            "cut at {end}"
        );
    }
    let mut rng = Rng::from_seed(0x09_02);
    let (mut refused, mut different) = (0, 0);
    for _ in 0..20_000 {
        let mut bytes = REFERENCE.as_bytes().to_vec();
        let position = (rng.next_u64() % bytes.len() as u64) as usize;
        bytes[position] = b"0123456789-.eE\"{}[],:abcdefnul "[(rng.next_u64() % 31) as usize];
        let damaged = String::from_utf8(bytes).unwrap();
        match import_last_known(&damaged) {
            Err(errors) => {
                assert!(!errors.is_empty());
                refused += 1;
            }
            Ok(record) => {
                // Whatever is accepted is a writable record.
                assert!(export_last_known(&record).is_ok());
                different += usize::from(bits(&record) != bits(&reference()));
            }
        }
    }
    println!("record damage: {refused} refused, {different} accepted as a different valid record");
    // The codec alone cannot tell a changed digit from the original. That
    // is what the storage digest is for.
    assert!(refused > 10_000 && different > 500);
}
