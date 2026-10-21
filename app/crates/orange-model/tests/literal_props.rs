//! Property: for generated `(type, value)` pairs, encoding then decoding returns the value
//! and re-encoding gives the identical literal.

use std::sync::Arc;

use orange_model::Value;
use orange_model::value::literal_codec::{SchemaSet, decode, encode};
use orange_model::value::{ListRepr, MapRepr};
use proptest::prelude::*;

/// A generated type expression with a matching value.
#[derive(Debug, Clone)]
struct Typed {
    ty: String,
    value: Value,
}

fn typed(ty: &str, value: Value) -> Typed {
    Typed {
        ty: ty.into(),
        value,
    }
}

fn scalar() -> impl Strategy<Value = Typed> {
    prop_oneof![
        any::<bool>().prop_map(|v| typed("core/bool@1", Value::Bool(v))),
        any::<i32>().prop_map(|v| typed("core/i32@1", Value::I32(v))),
        any::<u32>().prop_map(|v| typed("core/u32@1", Value::U32(v))),
        any::<i64>().prop_map(|v| typed("core/i64@1", Value::I64(v))),
        any::<u64>().prop_map(|v| typed("core/u64@1", Value::U64(v))),
        any::<i64>().prop_map(|v| typed("core/duration@1", Value::Duration(v))),
        any::<i64>().prop_map(|v| typed("core/timestamp@1", Value::Timestamp(v))),
        any::<u32>().prop_map(|bits| typed("core/f32@1", Value::F32(f32::from_bits(bits)))),
        any::<u64>().prop_map(|bits| typed("core/f64@1", Value::F64(f64::from_bits(bits)))),
        ".{0,12}".prop_map(|s| typed("core/string@1", Value::Str(Arc::new(s)))),
        proptest::collection::vec(any::<u8>(), 0..12)
            .prop_map(|b| typed("core/bytes@1", Value::Bytes(Arc::new(b)))),
    ]
}

/// Lists, string-keyed maps and options over generated element types.
fn typed_value() -> impl Strategy<Value = Typed> {
    scalar().prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            // A list of copies of one generated element keeps the list homogeneous.
            (inner.clone(), 0..4usize).prop_map(|(element, n)| {
                let items: Vec<Value> = std::iter::repeat_n(element.value, n).collect();
                typed(
                    &format!("core/list@1<{}>", element.ty),
                    Value::List(Arc::new(ListRepr::Values(items.into()))),
                )
            }),
            (
                proptest::collection::btree_set("[a-z]{1,4}", 0..3),
                inner.clone()
            )
                .prop_map(|(keys, value)| {
                    let entries: Vec<(Value, Value)> = keys
                        .into_iter()
                        .map(|k| (Value::Str(Arc::new(k)), value.value.clone()))
                        .collect();
                    typed(
                        &format!("core/map@1<core/string@1,{}>", value.ty),
                        Value::Map(Arc::new(MapRepr {
                            entries: entries.into(),
                        })),
                    )
                }),
            (inner, any::<bool>()).prop_map(|(inner, some)| {
                let ty = format!("core/option@1<{}>", inner.ty);
                let nested = inner.ty.starts_with("core/option@1<");
                let value = match (some, nested) {
                    (false, _) => Value::None,
                    (true, true) => Value::Some(Arc::new(inner.value)),
                    (true, false) => inner.value,
                };
                typed(&ty, value)
            }),
        ]
    })
}

proptest! {
    #[test]
    fn encode_decode_round_trip(typed in typed_value()) {
        let schemas = SchemaSet::default();
        let ty = typed.ty.parse().unwrap();
        let literal = encode(&ty, &typed.value, &schemas).unwrap();
        prop_assert_eq!(&decode(&literal, &schemas).unwrap(), &typed.value);
        prop_assert_eq!(encode(&ty, &decode(&literal, &schemas).unwrap(), &schemas).unwrap(), literal);
    }
}
