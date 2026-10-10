//! The JSON document layer: text ⇄ [`Json`] tree, nothing scenario-specific.
//!
//! `serde_json` does the reading and writing. The tree is our own type rather
//! than `serde_json::Value` for three reasons:
//!
//! - **Duplicate keys.** `Value` silently keeps the last of two members with
//!   the same name. Here an object is an ordered list of members, duplicates
//!   included, and [`parse`] reports every duplicate with its path.
//! - **Numbers keep their kind.** An integer literal stays an integer (exact
//!   over the whole `u64` / `i64` range); anything else is an `f64` parsed
//!   with correct rounding (`float_roundtrip`). The decoder can then refuse
//!   `1.0` where an integer is required.
//! - **Member order is preserved**, so exported text is deterministic.

use crate::error::ScenarioError;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use std::fmt;

/// A JSON number as written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Number {
    /// An integer literal `>= 0` that fits `u64`.
    PosInt(u64),
    /// An integer literal `< 0` that fits `i64`.
    NegInt(i64),
    /// Any other number: it has a fraction or exponent, or is an integer
    /// outside the ranges above (then only approximately representable).
    Float(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Json>),
    /// Members in document order. May hold duplicate names until
    /// [`parse`] has rejected them.
    Object(Vec<(String, Json)>),
}

impl Json {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Json::Null => "null",
            Json::Bool(_) => "boolean",
            Json::Number(Number::Float(_)) => "non-integer number",
            Json::Number(_) => "integer",
            Json::String(_) => "string",
            Json::Array(_) => "array",
            Json::Object(_) => "object",
        }
    }

    pub(crate) fn float(v: f64) -> Json {
        Json::Number(Number::Float(v))
    }

    pub(crate) fn members(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(members) => Some(members),
            _ => None,
        }
    }

    /// The member called `key`, if the value is an object that has one.
    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        self.members()?
            .iter()
            .find_map(|(k, v)| (k == key).then_some(v))
    }
}

/// Path of the member `key` of the object at `parent`.
pub(crate) fn member_path(parent: &str, key: &str) -> String {
    if parent == ROOT {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

/// Path of element `index` of the array at `parent`.
pub(crate) fn element_path(parent: &str, index: usize) -> String {
    format!("{parent}[{index}]")
}

/// Path of the document root.
pub(crate) const ROOT: &str = "$";

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_bool<E>(self, v: bool) -> Result<Json, E> {
        Ok(Json::Bool(v))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Json, E> {
        Ok(Json::Number(Number::PosInt(v)))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Json, E> {
        Ok(Json::Number(match u64::try_from(v) {
            Ok(v) => Number::PosInt(v),
            Err(_) => Number::NegInt(v),
        }))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Json, E> {
        Ok(Json::float(v))
    }

    fn visit_str<E>(self, v: &str) -> Result<Json, E> {
        Ok(Json::String(v.to_string()))
    }

    fn visit_string<E>(self, v: String) -> Result<Json, E> {
        Ok(Json::String(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut members = Vec::new();
        while let Some(member) = map.next_entry::<String, Json>()? {
            members.push(member);
        }
        Ok(Json::Object(members))
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Json, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Json::Null => serializer.serialize_unit(),
            Json::Bool(v) => serializer.serialize_bool(*v),
            Json::Number(Number::PosInt(v)) => serializer.serialize_u64(*v),
            Json::Number(Number::NegInt(v)) => serializer.serialize_i64(*v),
            Json::Number(Number::Float(v)) => {
                if !v.is_finite() {
                    // serde_json would write `null`, silently changing the
                    // document. The encoder never builds such a tree.
                    return Err(serde::ser::Error::custom("non-finite number"));
                }
                serializer.serialize_f64(*v)
            }
            Json::String(v) => serializer.serialize_str(v),
            Json::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Json::Object(members) => {
                let mut map = serializer.serialize_map(Some(members.len()))?;
                for (key, value) in members {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

fn find_duplicates(value: &Json, path: &str, errors: &mut Vec<ScenarioError>) {
    match value {
        Json::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                find_duplicates(item, &element_path(path, index), errors);
            }
        }
        Json::Object(members) => {
            for (index, (key, member)) in members.iter().enumerate() {
                let child = member_path(path, key);
                let earlier = &members[..index];
                let later = &members[index + 1..];
                // Reported once per name, at its first occurrence.
                if !earlier.iter().any(|(k, _)| k == key) && later.iter().any(|(k, _)| k == key) {
                    errors.push(ScenarioError::DuplicateKey {
                        path: child.clone(),
                    });
                }
                find_duplicates(member, &child, errors);
            }
        }
        _ => {}
    }
}

/// Reads exactly one JSON document. Trailing content, a byte-order mark,
/// comments, non-finite numbers and duplicate member names are all errors.
pub(crate) fn parse(text: &str) -> Result<Json, Vec<ScenarioError>> {
    let document: Json = serde_json::from_str(text).map_err(|e| {
        vec![ScenarioError::Syntax {
            line: e.line(),
            column: e.column(),
            message: e.to_string(),
        }]
    })?;
    let mut errors = Vec::new();
    find_duplicates(&document, ROOT, &mut errors);
    if errors.is_empty() {
        Ok(document)
    } else {
        Err(errors)
    }
}

/// Writes a document as indented JSON ending in a newline. Member order is
/// the tree's order; floats are written in their shortest form that reads
/// back to the same bits.
pub(crate) fn write(document: &Json) -> Result<String, ScenarioError> {
    let mut text =
        serde_json::to_string_pretty(document).map_err(|e| ScenarioError::Serialization {
            message: e.to_string(),
        })?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use locsim_core::rng::Rng;

    fn syntax_error(text: &str) -> (usize, usize) {
        match parse(text) {
            Err(errors) => match errors.as_slice() {
                [ScenarioError::Syntax { line, column, .. }] => (*line, *column),
                other => panic!("{text:?}: expected one syntax error, got {other:?}"),
            },
            Ok(v) => panic!("{text:?}: accepted as {v:?}"),
        }
    }

    #[test]
    fn numbers_keep_their_kind_and_full_integer_range() {
        let doc = parse(
            "[0, 18446744073709551615, -1, -9223372036854775808, 1.0, 1e3, \
             18446744073709551616, -9223372036854775809, -0, -0.0]",
        )
        .unwrap();
        let Json::Array(items) = doc else {
            panic!("not an array")
        };
        assert_eq!(items[0], Json::Number(Number::PosInt(0)));
        assert_eq!(items[1], Json::Number(Number::PosInt(u64::MAX)));
        assert_eq!(items[2], Json::Number(Number::NegInt(-1)));
        assert_eq!(items[3], Json::Number(Number::NegInt(i64::MIN)));
        assert_eq!(items[4], Json::float(1.0));
        assert_eq!(items[5], Json::float(1000.0));
        // Integers beyond the exact ranges are floats, never wrapped.
        assert_eq!(items[6].kind(), "non-integer number");
        assert_eq!(items[7].kind(), "non-integer number");
        // A negative zero is not an integer; its sign survives.
        for item in &items[8..] {
            match item {
                Json::Number(Number::Float(v)) => assert_eq!(v.to_bits(), (-0.0f64).to_bits()),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn rejects_everything_that_is_not_one_json_document() {
        for text in [
            "",
            " ",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "{'a':1}",
            "{a:1}",
            "{\"a\":1} x",
            "{\"a\":1}{\"a\":1}",
            "// c\n{}",
            "/* c */{}",
            "\u{feff}{}",
            "NaN",
            "Infinity",
            "-Infinity",
            "[1e999]",
            "[-1e999]",
            "[01]",
            "[+1]",
            "[.5]",
            "[1.]",
            "[0x10]",
            "[tru]",
            "[\"\\x41\"]",
            "[\"\\ud800\"]",
            "[\"\\udc00\\ud800\"]",
            "[\"a\nb\"]",
            "[\"\t\"]",
            "\"unterminated",
        ] {
            syntax_error(text);
        }
    }

    #[test]
    fn syntax_errors_carry_line_and_column() {
        assert_eq!(syntax_error("{\n  \"a\": 1,\n  \"b\": ?\n}"), (3, 8));
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_stack_overflow() {
        let text = "[".repeat(100_000) + &"]".repeat(100_000);
        syntax_error(&text);
    }

    #[test]
    fn every_duplicate_key_is_reported_with_its_path() {
        let errors = parse(
            r#"{"a": 1, "b": {"x": 1, "x": 2, "x": 3}, "a": 2,
                "c": [{"k": 0}, {"k": 0, "k": 1}], "d": {"a": 1}}"#,
        )
        .unwrap_err();
        let paths: Vec<_> = errors
            .iter()
            .map(|e| match e {
                ScenarioError::DuplicateKey { path } => path.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(paths, ["a", "b.x", "c[1].k"]);
    }

    #[test]
    fn duplicate_keys_differing_only_by_escape_are_duplicates() {
        let errors = parse(r#"{"seed": "1", "s\u0065ed": "2"}"#).unwrap_err();
        assert_eq!(
            errors,
            [ScenarioError::DuplicateKey {
                path: "seed".into()
            }]
        );
    }

    #[test]
    fn writer_refuses_non_finite_numbers_instead_of_writing_null() {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let doc = Json::Array(vec![Json::float(v)]);
            assert!(matches!(
                write(&doc),
                Err(ScenarioError::Serialization { .. })
            ));
        }
    }

    #[test]
    fn output_is_ordered_indented_and_newline_terminated() {
        let doc = Json::Object(vec![
            ("z".into(), Json::Number(Number::PosInt(1))),
            ("a".into(), Json::Array(vec![Json::Null, Json::Bool(true)])),
        ]);
        assert_eq!(
            write(&doc).unwrap(),
            "{\n  \"z\": 1,\n  \"a\": [\n    null,\n    true\n  ]\n}\n"
        );
    }

    fn float_round_trip(v: f64) {
        let text = write(&Json::Array(vec![Json::float(v)])).unwrap();
        let back = parse(&text).unwrap();
        let Json::Array(items) = back else {
            panic!("not an array")
        };
        match &items[0] {
            Json::Number(Number::Float(r)) => {
                assert_eq!(r.to_bits(), v.to_bits(), "{v:e} written as {text}")
            }
            other => panic!("{v:e} written as {text} read back as {other:?}"),
        }
    }

    #[test]
    fn every_finite_float_round_trips_bit_exactly() {
        for v in [
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.1,
            1.0 / 3.0,
            f64::MIN_POSITIVE,
            f64::MIN_POSITIVE / 2.0,
            f64::from_bits(1),
            f64::MAX,
            f64::MIN,
            f64::EPSILON,
            1e21,
            1e-7,
            9007199254740993.0,
            1.7976931348623157e308,
            5e-324,
            12.9352,
            77.6245,
            179.99999999999997,
            -179.99999999999997,
            89.99999999999999,
        ] {
            float_round_trip(v);
        }
        // Arbitrary bit patterns: every exponent, subnormals included.
        let mut rng = Rng::from_seed(0x08_F1_0A_75);
        let mut tested = 0;
        while tested < 200_000 {
            let v = f64::from_bits(rng.next_u64());
            if v.is_finite() {
                float_round_trip(v);
                tested += 1;
            }
        }
    }

    #[test]
    fn integers_round_trip_exactly_at_the_ends_of_their_ranges() {
        for n in [
            Number::PosInt(0),
            Number::PosInt(u64::MAX),
            Number::PosInt(i64::MAX as u64),
            Number::PosInt(i64::MAX as u64 + 1),
            Number::PosInt((1 << 53) + 1),
            Number::NegInt(-1),
            Number::NegInt(i64::MIN),
        ] {
            let text = write(&Json::Array(vec![Json::Number(n)])).unwrap();
            assert_eq!(
                parse(&text).unwrap(),
                Json::Array(vec![Json::Number(n)]),
                "{text}"
            );
        }
    }

    #[test]
    fn strings_round_trip_including_controls_quotes_and_astral_characters() {
        let mut rng = Rng::from_seed(11);
        let mut cases = vec![
            String::new(),
            "\"\\/\u{8}\u{c}\n\r\t".to_string(),
            "\u{0}\u{1f}\u{7f}\u{80}\u{2028}\u{2029}\u{feff}\u{fffd}".to_string(),
            "नमस्ते 🌍 \u{10ffff}".to_string(),
        ];
        for _ in 0..2_000 {
            let len = (rng.next_u64() % 12) as usize;
            cases.push(
                (0..len)
                    .filter_map(|_| char::from_u32((rng.next_u64() % 0x11_0000) as u32))
                    .collect(),
            );
        }
        for s in cases {
            let doc = Json::Object(vec![(s.clone(), Json::String(s.clone()))]);
            assert_eq!(parse(&write(&doc).unwrap()).unwrap(), doc, "{s:?}");
        }
    }
}
