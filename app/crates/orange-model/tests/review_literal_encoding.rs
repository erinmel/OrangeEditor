use std::sync::Arc;

use orange_model::literal::LiteralPayload;
use orange_model::literal_codec::{SchemaSet, decode, encode};
use orange_model::value::MapRepr;
use orange_model::{LiteralError, Value};

fn string(text: &str) -> Value {
    Value::Str(Arc::new(text.to_owned()))
}

fn map(entries: Vec<(Value, Value)>) -> Value {
    Value::Map(Arc::new(MapRepr {
        entries: entries.into(),
    }))
}

#[test]
fn floats_require_the_exact_runtime_width_and_preserve_bits() {
    let schemas = SchemaSet::default();
    let f32_ty = "core/f32@1".parse().unwrap();
    let f64_ty = "core/f64@1".parse().unwrap();

    assert_eq!(
        encode(&f32_ty, &Value::F64(1.0), &schemas),
        Err(LiteralError::ValueMismatch {
            ty: "core/f32@1".into(),
        })
    );
    assert_eq!(
        encode(&f64_ty, &Value::F32(1.0), &schemas),
        Err(LiteralError::ValueMismatch {
            ty: "core/f64@1".into(),
        })
    );

    let f32 = Value::F32(f32::from_bits(0x8000_0000));
    let f64 = Value::F64(f64::from_bits(0x7ff8_0000_0000_0042));
    let encoded_f32 = encode(&f32_ty, &f32, &schemas).unwrap();
    let encoded_f64 = encode(&f64_ty, &f64, &schemas).unwrap();
    assert_eq!(encoded_f32.payload, LiteralPayload::Bits("80000000".into()));
    assert_eq!(
        encoded_f64.payload,
        LiteralPayload::Bits("7ff8000000000042".into())
    );
    assert_eq!(decode(&encoded_f32, &schemas).unwrap(), f32);
    assert_eq!(decode(&encoded_f64, &schemas).unwrap(), f64);
}

#[test]
fn map_encoding_rejects_duplicate_runtime_keys() {
    let schemas = SchemaSet::default();
    let ty = "core/map@1<core/string@1,core/i32@1>".parse().unwrap();
    let value = map(vec![
        (string("repeat"), Value::I32(1)),
        (string("repeat"), Value::I32(2)),
    ]);

    assert_eq!(
        encode(&ty, &value, &schemas),
        Err(LiteralError::DuplicateMapKey)
    );
}

#[test]
fn map_encoding_rejects_float_keys_even_when_empty() {
    let schemas = SchemaSet::default();
    let ty = "core/map@1<core/f64@1,core/i32@1>".parse().unwrap();
    let value = map(Vec::new());

    assert_eq!(
        encode(&ty, &value, &schemas),
        Err(LiteralError::FloatMapKey {
            ty: "core/f64@1".into(),
        })
    );
}

#[test]
fn valid_ordered_maps_encode_and_decode_with_the_same_order() {
    let schemas = SchemaSet::default();
    let ty = "core/map@1<core/string@1,core/i32@1>".parse().unwrap();
    let value = map(vec![
        (string("b"), Value::I32(2)),
        (string("a"), Value::I32(1)),
    ]);

    let literal = encode(&ty, &value, &schemas).unwrap();
    assert_eq!(decode(&literal, &schemas).unwrap(), value);
    let LiteralPayload::Value(json) = literal.payload else {
        panic!("map literals use JSON value payloads");
    };
    assert_eq!(json, serde_json::json!([["b", 2], ["a", 1]]));
}
