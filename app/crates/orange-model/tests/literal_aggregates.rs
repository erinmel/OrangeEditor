//! Aggregate literals (12 §6.1) and FR-08: list, map, option, record, enum and json round
//! trips with stable IDs; None, missing, JSON null and Error stay distinct.

use std::sync::Arc;

use orange_model::document::literal::Literal;
use orange_model::type_term::TypeTerm;
use orange_model::value::literal_codec::{
    EnumSchema, FieldSchema, RecordSchema, SchemaSet, VariantSchema, decode, encode,
};
use orange_model::value::{DuplicateKey, JsonText, ListRepr, MapRepr, RecordRepr};
use orange_model::{LiteralError, Value};
use serde_json::{Value as Json, json};

fn term(text: &str) -> TypeTerm {
    TypeTerm::parse(&text.parse().unwrap()).unwrap()
}

fn str_value(text: &str) -> Value {
    Value::Str(Arc::new(text.into()))
}

/// `org.example/point@1` record and the 12 §5 playback-state enum.
fn schemas() -> SchemaSet {
    SchemaSet::default()
        .with_record(
            "org.example/point@1",
            RecordSchema {
                fields: vec![
                    FieldSchema {
                        field_id: "f-x".into(),
                        ty: term("core/i64@1"),
                        optional: false,
                    },
                    FieldSchema {
                        field_id: "f-label".into(),
                        ty: term("core/option@1<core/string@1>"),
                        optional: true,
                    },
                ],
            },
        )
        .with_enum(
            "org.example.media/playback-state@1",
            EnumSchema {
                variants: vec![
                    VariantSchema {
                        variant_id: "a1c0".into(),
                        payload: None,
                    },
                    VariantSchema {
                        variant_id: "b7e2".into(),
                        payload: Some(term("core/duration@1")),
                    },
                ],
            },
        )
}

fn literal(ty: &str, value: Json) -> Literal {
    Literal::value(ty.parse().unwrap(), value)
}

fn round_trip(ty: &str, value: Json, expected: Value) {
    let literal = literal(ty, value);
    assert_eq!(decode(&literal, &schemas()).unwrap(), expected, "{ty}");
    assert_eq!(
        encode(&literal.ty, &expected, &schemas()).unwrap(),
        literal,
        "{ty}"
    );
}

fn decode_err(ty: &str, value: Json) -> LiteralError {
    decode(&literal(ty, value), &schemas()).unwrap_err()
}

#[test]
fn lists_use_typed_storage_and_nest() {
    let decoded = decode(
        &literal("core/list@1<core/i64@1>", json!(["1", "-2"])),
        &schemas(),
    )
    .unwrap();
    let Value::List(list) = &decoded else {
        panic!("not a list")
    };
    assert!(
        matches!(**list, ListRepr::I64(_)),
        "scalar elements use a typed array"
    );
    round_trip(
        "core/list@1<core/i64@1>",
        json!(["1", "-2"]),
        Value::List(Arc::new(ListRepr::I64(vec![1, -2].into()))),
    );
    round_trip(
        "core/list@1<core/f32@1>",
        json!(["3f800000"]),
        Value::List(Arc::new(ListRepr::F32(vec![1.0].into()))),
    );
    round_trip(
        "core/list@1<core/string@1>",
        json!([]),
        Value::List(Arc::new(ListRepr::Str(vec![].into()))),
    );
    let inner = |items: Vec<bool>| Value::List(Arc::new(ListRepr::Bool(items.into())));
    round_trip(
        "core/list@1<core/list@1<core/bool@1>>",
        json!([[true], []]),
        Value::List(Arc::new(ListRepr::Values(
            vec![inner(vec![true]), inner(vec![])].into(),
        ))),
    );
    assert!(matches!(
        decode_err("core/list@1<core/i64@1>", json!(["1", 2])),
        LiteralError::TypeMismatch { .. }
    ));
}

#[test]
fn maps_keep_order_and_reject_duplicates_and_float_keys() {
    round_trip(
        "core/map@1<core/string@1,core/i32@1>",
        json!([["b", 1], ["a", 2]]),
        Value::Map(Arc::new(
            MapRepr::new(vec![
                (str_value("b"), Value::I32(1)),
                (str_value("a"), Value::I32(2)),
            ])
            .unwrap(),
        )),
    );
    assert_eq!(
        decode_err(
            "core/map@1<core/string@1,core/i32@1>",
            json!([["a", 1], ["a", 2]])
        ),
        LiteralError::DuplicateMapKey
    );
    assert!(matches!(
        decode_err("core/map@1<core/f64@1,core/i32@1>", json!([])),
        LiteralError::FloatMapKey { .. }
    ));
    assert!(matches!(
        decode_err("core/map@1<core/string@1,core/i32@1>", json!([["a"]])),
        LiteralError::TypeMismatch { .. }
    ));
}

#[test]
fn options_are_explicit_and_nested_options_box() {
    round_trip(
        "core/option@1<core/i64@1>",
        json!({"none": true}),
        Value::None,
    );
    round_trip(
        "core/option@1<core/i64@1>",
        json!({"some": "5"}),
        Value::I64(5),
    );
    round_trip(
        "core/option@1<core/option@1<core/i64@1>>",
        json!({"some": {"none": true}}),
        Value::Some(Arc::new(Value::None)),
    );
    round_trip(
        "core/option@1<core/option@1<core/i64@1>>",
        json!({"some": {"some": "5"}}),
        Value::Some(Arc::new(Value::I64(5))),
    );
    for bad in [
        json!(null),
        json!({"none": false}),
        json!({"none": true, "some": "1"}),
        json!({}),
    ] {
        assert!(
            matches!(
                decode_err("core/option@1<core/i64@1>", bad.clone()),
                LiteralError::TypeMismatch { .. }
            ),
            "{bad}"
        );
    }
}

fn point(x: i64, label: Option<Value>) -> Value {
    Value::Record(Arc::new(RecordRepr {
        ty: "org.example/point@1".parse().unwrap(),
        fields: vec![Some(Value::I64(x)), label].into(),
    }))
}

#[test]
fn records_distinguish_missing_from_present_none() {
    round_trip("org.example/point@1", json!({"f-x": "3"}), point(3, None));
    round_trip(
        "org.example/point@1",
        json!({"f-x": "3", "f-label": {"none": true}}),
        point(3, Some(Value::None)),
    );
    round_trip(
        "org.example/point@1",
        json!({"f-x": "3", "f-label": {"some": "p"}}),
        point(3, Some(str_value("p"))),
    );
    assert_ne!(
        point(3, None),
        point(3, Some(Value::None)),
        "FR-08: missing ≠ None"
    );
    assert_eq!(
        decode_err("org.example/point@1", json!({})),
        LiteralError::MissingField {
            field: "f-x".into()
        }
    );
    assert_eq!(
        decode_err("org.example/point@1", json!({"f-x": "1", "f-new": 1})),
        LiteralError::UnknownField {
            field: "f-new".into()
        }
    );
}

#[test]
fn enums_use_stable_variant_ids() {
    let ty = "org.example.media/playback-state@1";
    round_trip(
        ty,
        json!({"variant": "a1c0"}),
        Value::Enum {
            tag: 0,
            payload: None,
        },
    );
    round_trip(
        ty,
        json!({"variant": "b7e2", "payload": "1000"}),
        Value::Enum {
            tag: 1,
            payload: Some(Arc::new(Value::Duration(1000))),
        },
    );
    assert_eq!(
        decode_err(ty, json!({"variant": "zzzz"})),
        LiteralError::UnknownVariant {
            variant: "zzzz".into()
        }
    );
    assert!(
        matches!(
            decode_err(ty, json!({"variant": "b7e2"})),
            LiteralError::TypeMismatch { .. }
        ),
        "payload required"
    );
    assert!(matches!(
        decode_err(ty, json!({"variant": "a1c0", "payload": "1"})),
        LiteralError::TypeMismatch { .. }
    ));
}

#[test]
fn json_keeps_exact_text_and_its_own_null() {
    let text = r#"{"n": 1.50, "big": 12345678901234567890, "z": null}"#;
    round_trip(
        "core/json@1",
        json!(text),
        Value::Json(Arc::new(JsonText::new(text.into()).unwrap())),
    );
    let null = decode(&literal("core/json@1", json!("null")), &schemas()).unwrap();
    assert_ne!(null, Value::None, "FR-08: JSON null ≠ None");
    assert!(matches!(
        decode_err("core/json@1", json!("{")),
        LiteralError::TypeMismatch { .. }
    ));
}

/// `levels` nested options around an i64 literal.
fn nested_options(levels: usize) -> Literal {
    let mut value = json!("1");
    let mut ty = "core/i64@1".to_string();
    for _ in 0..levels {
        value = json!({"some": value});
        ty = format!("core/option@1<{ty}>");
    }
    literal(&ty, value)
}

#[test]
fn nesting_is_bounded() {
    assert!(decode(&nested_options(40), &schemas()).is_ok());
    // Past the budget the type itself no longer parses, so no deep recursion happens.
    assert!(matches!(
        decode(&nested_options(80), &schemas()),
        Err(LiteralError::InvalidType(_))
    ));
}

#[test]
fn every_literal_in_the_saved_graph_example_decodes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/threshold-v1.json"
    );
    let document = orange_model::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut count = 0;
    for node in document.root().nodes.values() {
        for literal in node.inputs.values() {
            decode(literal, &schemas()).unwrap();
            count += 1;
        }
    }
    for variable in document.variables().values() {
        if let Some(init) = &variable.init {
            decode(init, &schemas()).unwrap();
            count += 1;
        }
    }
    assert_eq!(count, 2, "n5.b and the `total` initializer");
}

#[test]
fn map_encoding_rejects_what_decoding_rejects() {
    // A map with a repeated key cannot be built, so there is nothing to encode.
    assert_eq!(
        MapRepr::new(vec![
            (str_value("repeat"), Value::I32(1)),
            (str_value("repeat"), Value::I32(2))
        ]),
        Err(DuplicateKey)
    );
    assert_eq!(
        encode(
            &"core/map@1<core/f64@1,core/i32@1>".parse().unwrap(),
            &Value::Map(Arc::default()),
            &schemas()
        ),
        Err(LiteralError::FloatMapKey {
            ty: "core/f64@1".into()
        }),
        "float key types are rejected even for an empty map"
    );
}

#[test]
fn json_literals_respect_the_codec_budgets() {
    let deep = format!("{}{}", "[".repeat(84), "]".repeat(84));
    assert_eq!(
        decode_err("core/json@1", json!(deep)),
        LiteralError::TooDeep
    );
    let large = format!(
        "\"{}\"",
        "a".repeat(orange_model::value::MAX_JSON_TEXT_BYTES)
    );
    assert!(matches!(
        decode_err("core/json@1", json!(large)),
        LiteralError::TooLarge { .. }
    ));
}

#[test]
fn large_maps_decode_and_encode_with_hashed_key_checks() {
    // 20 000 keys: linear duplicate checks would need about 200 million comparisons.
    let pairs: Vec<Json> = (0..20_000).map(|i| json!([format!("k{i}"), i])).collect();
    let ty = "core/map@1<core/string@1,core/i32@1>";
    let literal = literal(ty, Json::Array(pairs));
    let value = decode(&literal, &schemas()).unwrap();
    assert_eq!(encode(&literal.ty, &value, &schemas()).unwrap(), literal);

    let mut duplicated: Vec<Json> = (0..20_000).map(|i| json!([format!("k{i}"), i])).collect();
    duplicated.push(json!(["k19999", 0]));
    assert_eq!(
        decode_err(ty, Json::Array(duplicated)),
        LiteralError::DuplicateMapKey
    );
}
