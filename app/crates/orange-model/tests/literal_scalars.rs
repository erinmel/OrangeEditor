//! Core scalar literals (12 §6.1): round trips, boundaries, canonical forms and ranges.

use std::sync::Arc;

use orange_model::document::literal::{Literal, LiteralPayload};
use orange_model::value::literal_codec::{SchemaSet, decode, encode};
use orange_model::{LiteralError, Value};
use serde_json::{Value as Json, json};

fn literal(ty: &str, value: Json) -> Literal {
    Literal::value(ty.parse().unwrap(), value)
}

fn bits(ty: &str, bits: &str) -> Literal {
    Literal {
        ty: ty.parse().unwrap(),
        payload: LiteralPayload::Bits(bits.into()),
        display: None,
    }
}

/// Decodes to `expected`, and encoding `expected` gives back the same literal.
fn round_trip(literal: Literal, expected: Value) {
    let schemas = SchemaSet::default();
    assert_eq!(decode(&literal, &schemas).unwrap(), expected, "{literal:?}");
    assert_eq!(encode(&literal.ty, &expected, &schemas).unwrap(), literal);
}

fn decode_err(literal: Literal) -> LiteralError {
    decode(&literal, &SchemaSet::default()).unwrap_err()
}

#[test]
fn scalars_round_trip_at_their_boundaries() {
    round_trip(literal("core/bool@1", json!(true)), Value::Bool(true));
    round_trip(literal("core/i32@1", json!(i32::MIN)), Value::I32(i32::MIN));
    round_trip(literal("core/u32@1", json!(u32::MAX)), Value::U32(u32::MAX));
    round_trip(
        literal("core/i64@1", json!(i64::MIN.to_string())),
        Value::I64(i64::MIN),
    );
    round_trip(
        literal("core/u64@1", json!(u64::MAX.to_string())),
        Value::U64(u64::MAX),
    );
    round_trip(
        literal("core/duration@1", json!("-1500000000")),
        Value::Duration(-1_500_000_000),
    );
    round_trip(literal("core/timestamp@1", json!("0")), Value::Timestamp(0));
    round_trip(
        literal("core/string@1", json!("")),
        Value::Str(Arc::new(String::new())),
    );
    round_trip(
        literal("core/string@1", json!("naïve 🍊")),
        Value::Str(Arc::new("naïve 🍊".into())),
    );
    round_trip(
        literal("core/bytes@1", json!("")),
        Value::Bytes(Arc::new(vec![])),
    );
    round_trip(
        literal("core/bytes@1", json!("AP8=")),
        Value::Bytes(Arc::new(vec![0, 255])),
    );
    round_trip(
        literal("core/secret-ref@1", json!("credential://obs/main")),
        Value::Str(Arc::new("credential://obs/main".into())),
    );
}

#[test]
fn floats_round_trip_by_bit_pattern() {
    round_trip(bits("core/f64@1", "3ff0000000000000"), Value::F64(1.0));
    round_trip(bits("core/f64@1", "8000000000000000"), Value::F64(-0.0));
    round_trip(
        bits("core/f64@1", "7ff8000000000001"),
        Value::F64(f64::from_bits(0x7ff8_0000_0000_0001)),
    );
    round_trip(bits("core/f32@1", "3f800000"), Value::F32(1.0));
    round_trip(
        bits("core/f32@1", "ff800000"),
        Value::F32(f32::NEG_INFINITY),
    );
}

#[test]
fn non_canonical_scalars_are_rejected() {
    for (ty, value) in [
        ("core/i64@1", json!("+1")),
        ("core/i64@1", json!("01")),
        ("core/i64@1", json!("-0")),
        ("core/i64@1", json!(" 1")),
        ("core/u64@1", json!("1e3")),
        ("core/bytes@1", json!("AP8")),
    ] {
        assert!(
            matches!(
                decode_err(literal(ty, value.clone())),
                LiteralError::NonCanonical { .. }
            ),
            "{ty} {value}"
        );
    }
    assert!(matches!(
        decode_err(bits("core/f64@1", "3FF0000000000000")),
        LiteralError::NonCanonical { .. }
    ));
    assert!(matches!(
        decode_err(bits("core/f32@1", "3ff0000000000000")),
        LiteralError::NonCanonical { .. }
    ));
}

#[test]
fn out_of_range_and_wrong_shapes_are_rejected() {
    let out_of_range = |ty: &str, value: Json| {
        matches!(
            decode_err(literal(ty, value)),
            LiteralError::OutOfRange { .. }
        )
    };
    assert!(out_of_range("core/i32@1", json!(i64::from(i32::MAX) + 1)));
    assert!(out_of_range("core/u32@1", json!(-1)));
    assert!(out_of_range("core/u32@1", json!(u64::MAX)));
    assert!(out_of_range("core/i64@1", json!("9223372036854775808")));
    assert!(out_of_range("core/u64@1", json!("-1")));

    let mismatch = |ty: &str, value: Json| {
        matches!(
            decode_err(literal(ty, value)),
            LiteralError::TypeMismatch { .. }
        )
    };
    assert!(mismatch("core/i64@1", json!(1)), "i64 uses decimal strings");
    assert!(mismatch("core/i32@1", json!("1")), "i32 uses JSON integers");
    assert!(mismatch("core/i32@1", json!(1.0)));
    assert!(mismatch("core/bool@1", json!("true")));
    assert!(
        mismatch("core/f64@1", json!("3ff0000000000000")),
        "top-level floats use `bits`"
    );
    assert!(matches!(
        decode_err(bits("core/i64@1", "00000001")),
        LiteralError::TypeMismatch { .. }
    ));
    assert_eq!(
        decode_err(literal("core/secret-ref@1", json!("hunter2"))),
        LiteralError::InvalidSecretRef
    );
}

#[test]
fn types_without_literals_and_unknown_types_are_rejected() {
    for ty in [
        "core/result@1<core/i64@1>",
        "core/error@1",
        "core/stream@1<core/bytes@1>",
        "core/handle@1<org.x/session@1>",
    ] {
        assert!(
            matches!(
                decode_err(literal(ty, json!("x"))),
                LiteralError::NoLiteralForm { .. }
            ),
            "{ty}"
        );
    }
    assert!(matches!(
        decode_err(literal("core/i8@1", json!(1))),
        LiteralError::UnknownType { .. }
    ));
    assert!(matches!(
        decode_err(literal("org.x/point@1", json!({}))),
        LiteralError::UnknownType { .. }
    ));
    assert!(matches!(
        decode_err(literal("$T", json!(1))),
        LiteralError::UnresolvedParameter { .. }
    ));
    assert!(matches!(
        decode_err(literal("core/i64", json!("1"))),
        LiteralError::InvalidType(_)
    ));
}

#[test]
fn encoding_checks_the_value_against_the_type() {
    let schemas = SchemaSet::default();
    let ty = "core/i64@1".parse().unwrap();
    assert!(matches!(
        encode(&ty, &Value::I32(1), &schemas),
        Err(LiteralError::ValueMismatch { .. })
    ));
    let f64_ty = "core/f64@1".parse().unwrap();
    assert!(matches!(
        encode(&f64_ty, &Value::F32(1.0), &schemas),
        Err(LiteralError::ValueMismatch { .. })
    ));
}

#[test]
fn float_encoding_requires_the_exact_width_and_keeps_bits() {
    let schemas = SchemaSet::default();
    let f32_ty = "core/f32@1".parse().unwrap();
    let f64_ty = "core/f64@1".parse().unwrap();
    assert_eq!(
        encode(&f32_ty, &Value::F64(1.0), &schemas),
        Err(LiteralError::ValueMismatch {
            ty: "core/f32@1".into()
        })
    );
    assert_eq!(
        encode(&f64_ty, &Value::F32(1.0), &schemas),
        Err(LiteralError::ValueMismatch {
            ty: "core/f64@1".into()
        })
    );
    round_trip(
        bits("core/f32@1", "80000000"),
        Value::F32(f32::from_bits(0x8000_0000)),
    );
    round_trip(
        bits("core/f64@1", "7ff8000000000042"),
        Value::F64(f64::from_bits(0x7ff8_0000_0000_0042)),
    );
}
