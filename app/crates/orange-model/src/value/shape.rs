//! Resolved structural shape of an executable Core type, and the one bounded check of a
//! [`Value`] against it (12 §7). Shapes come from the parsed [`TypeTerm`], never from
//! type-expression text, and own the option/result boxing rule and nominal byte charges.

use std::collections::HashMap;
use std::sync::Arc;

use thiserror::Error;

use super::{ListRepr, SMALL_MAP, Value};
use crate::id::TypeExpr;
use crate::type_term::{MAX_NESTING, TypeTerm};

/// Nominal retained bytes of one value or list element, before any payload.
pub const VALUE_CHARGE: usize = 16;
/// Nominal retained bytes of one map entry, before its key and value.
pub const MAP_ENTRY_CHARGE: usize = 32;
/// Nominal retained bytes of an error envelope, before its code text.
pub const ERROR_CHARGE: usize = 2 * VALUE_CHARGE;

/// Supported Core leaves and recursively typed containers. Values follow the model's
/// boxed/unboxed representation, not JSON null.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValueShape {
    Bool,
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
    Json,
    String,
    Duration,
    Timestamp,
    Bytes,
    List(Box<ValueShape>),
    Map(Box<ValueShape>, Box<ValueShape>),
    Error,
    Option(Box<ValueShape>),
    Result(Box<ValueShape>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ValueCheckError {
    #[error("The value does not match its checked plan type.")]
    InvalidType,
    #[error("The value exceeds its retained byte limit.")]
    ResourceLimit,
}

impl ValueCheckError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::InvalidType => "core/value-invalid-type",
            Self::ResourceLimit => "core/value-resource-limit",
        }
    }
}

use ValueCheckError::{InvalidType, ResourceLimit};

/// A stable namespaced error or message code: `namespace/name`, ASCII, at most 256 bytes.
pub fn is_error_code(text: &str) -> bool {
    let Some((namespace, name)) = text.split_once('/') else {
        return false;
    };
    !namespace.is_empty()
        && !name.is_empty()
        && !name.contains('/')
        && text.len() <= 256
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
}

fn charge(left: &mut usize, bytes: usize) -> Result<(), ValueCheckError> {
    *left = left.checked_sub(bytes).ok_or(ResourceLimit)?;
    Ok(())
}

fn minimum(left: usize, len: usize, per: usize) -> Result<(), ValueCheckError> {
    if len.checked_mul(per).is_none_or(|n| n > left) {
        Err(ResourceLimit)
    } else {
        Ok(())
    }
}

impl ValueShape {
    /// Shape of a canonical type expression; None for parameters, unsupported or
    /// unknown types, wrong arities and non-hashable map keys.
    pub fn of(expr: &TypeExpr) -> Option<Self> {
        Self::resolve(&TypeTerm::parse(expr).ok()?)
    }

    /// Shape of an already parsed term.
    pub fn resolve(term: &TypeTerm) -> Option<Self> {
        Self::resolve_at(term, 0)
    }

    fn resolve_at(term: &TypeTerm, depth: usize) -> Option<Self> {
        if depth > MAX_NESTING {
            return None;
        }
        let TypeTerm::Named(named) = term else {
            return None;
        };
        if named.namespace != "core" || named.major != 1 {
            return None;
        }
        let inner = |i: usize| Self::resolve_at(&named.args[i], depth + 1).map(Box::new);
        Some(match (named.name.as_str(), named.args.len()) {
            ("bool", 0) => Self::Bool,
            ("i32", 0) => Self::I32,
            ("i64", 0) => Self::I64,
            ("u32", 0) => Self::U32,
            ("u64", 0) => Self::U64,
            ("f32", 0) => Self::F32,
            ("f64", 0) => Self::F64,
            ("json", 0) => Self::Json,
            ("string", 0) => Self::String,
            ("duration", 0) => Self::Duration,
            ("timestamp", 0) => Self::Timestamp,
            ("bytes", 0) => Self::Bytes,
            ("error", 0) => Self::Error,
            ("option", 1) => Self::Option(inner(0)?),
            ("result", 1) => Self::Result(inner(0)?),
            ("list", 1) => Self::List(inner(0)?),
            ("map", 2) => {
                let key = inner(0)?;
                if !key.native_hash() {
                    return None;
                }
                Self::Map(key, inner(1)?)
            }
            _ => return None,
        })
    }

    pub fn native_hash(&self) -> bool {
        matches!(
            self,
            Self::Bool | Self::I32 | Self::I64 | Self::U32 | Self::U64 | Self::String | Self::Bytes
        )
    }

    /// Whether the host implements this exact Core type's total equality.
    pub fn native_eq(&self) -> bool {
        self.native_eq_at(0)
    }

    fn native_eq_at(&self, depth: usize) -> bool {
        if depth > MAX_NESTING {
            return false;
        }
        match self {
            Self::Bool
            | Self::I32
            | Self::I64
            | Self::U32
            | Self::U64
            | Self::String
            | Self::Bytes
            | Self::Duration
            | Self::Timestamp => true,
            Self::Option(inner) | Self::List(inner) => inner.native_eq_at(depth + 1),
            Self::Map(key, value) => key.native_hash() && value.native_eq_at(depth + 1),
            _ => false,
        }
    }

    /// Whether a valid value of this type can carry the Err discriminant directly.
    pub fn unboxed_error(&self) -> bool {
        match self {
            Self::Result(_) | Self::Error => true,
            Self::Option(inner) if !matches!(inner.as_ref(), Self::Option(_)) => {
                inner.unboxed_error()
            }
            _ => false,
        }
    }

    pub fn accepts(&self, value: &Value) -> bool {
        self.checked_charge(value, usize::MAX).is_ok()
    }

    /// Validate type/shape and charge immutable data, stopping before oversized traversals.
    pub fn checked_charge(&self, value: &Value, limit: usize) -> Result<usize, ValueCheckError> {
        // Public enum construction must not bypass depth checks through None/empty containers.
        if !self.valid_shape(0) {
            return Err(InvalidType);
        }
        let mut left = limit;
        self.check(value, &mut left, 0)?;
        Ok(limit - left)
    }

    fn valid_shape(&self, depth: usize) -> bool {
        if depth > MAX_NESTING {
            return false;
        }
        match self {
            Self::Option(inner) | Self::Result(inner) | Self::List(inner) => {
                inner.valid_shape(depth + 1)
            }
            Self::Map(key, value) => key.native_hash() && value.valid_shape(depth + 1),
            _ => true,
        }
    }

    fn check(&self, value: &Value, left: &mut usize, depth: usize) -> Result<(), ValueCheckError> {
        if depth > MAX_NESTING {
            return Err(InvalidType);
        }
        if let Self::Result(inner) = self {
            return match value {
                Value::Err(error) if is_error_code(&error.code) => charge(
                    left,
                    error
                        .code
                        .len()
                        .checked_add(ERROR_CHARGE)
                        .ok_or(ResourceLimit)?,
                ),
                Value::Ok(value) if inner.unboxed_error() => {
                    charge(left, VALUE_CHARGE)?;
                    inner.check(value, left, depth + 1)
                }
                _ if inner.unboxed_error() => Err(InvalidType),
                _ => inner.check(value, left, depth + 1),
            };
        }
        if let Self::Option(inner) = self {
            return match value {
                Value::None => charge(left, VALUE_CHARGE),
                Value::Some(value) if matches!(inner.as_ref(), Self::Option(_)) => {
                    charge(left, VALUE_CHARGE)?;
                    inner.check(value, left, depth + 1)
                }
                _ if matches!(inner.as_ref(), Self::Option(_)) => Err(InvalidType),
                _ => inner.check(value, left, depth + 1),
            };
        }
        charge(left, VALUE_CHARGE)?;
        match (self, value) {
            (Self::List(inner), Value::List(items)) => {
                minimum(*left, items.len(), VALUE_CHARGE)?;
                let packed = matches!(
                    (inner.as_ref(), items.as_ref()),
                    (Self::Bool, ListRepr::Bool(_))
                        | (Self::I32, ListRepr::I32(_))
                        | (Self::I64, ListRepr::I64(_))
                        | (Self::U32, ListRepr::U32(_))
                        | (Self::U64, ListRepr::U64(_))
                        | (Self::F32, ListRepr::F32(_))
                        | (Self::F64, ListRepr::F64(_))
                );
                if packed {
                    return charge(
                        left,
                        items.len().checked_mul(VALUE_CHARGE).ok_or(ResourceLimit)?,
                    );
                }
                for value in items.iter() {
                    inner.check(&value, left, depth + 1)?;
                }
                Ok(())
            }
            (Self::Map(key, ty), Value::Map(map)) if key.native_hash() => {
                // Keys are unique by construction (`MapRepr::new`).
                let entries = map.entries();
                minimum(*left, entries.len(), MAP_ENTRY_CHARGE)?;
                for (k, v) in entries {
                    key.check(k, left, depth + 1)?;
                    ty.check(v, left, depth + 1)?;
                }
                Ok(())
            }
            (Self::String, Value::Str(text)) => charge(left, text.len()),
            (Self::Bytes, Value::Bytes(bytes)) => charge(left, bytes.len()),
            (Self::Json, Value::Json(text)) => charge(left, text.as_str().len()),
            // The value charge above plus this rest equals one error envelope.
            (Self::Error, Value::Err(error)) if is_error_code(&error.code) => charge(
                left,
                error
                    .code
                    .len()
                    .checked_add(ERROR_CHARGE - VALUE_CHARGE)
                    .ok_or(ResourceLimit)?,
            ),
            (Self::Bool, Value::Bool(_))
            | (Self::I32, Value::I32(_))
            | (Self::I64, Value::I64(_))
            | (Self::U32, Value::U32(_))
            | (Self::U64, Value::U64(_))
            | (Self::F32, Value::F32(_))
            | (Self::F64, Value::F64(_))
            | (Self::Duration, Value::Duration(_))
            | (Self::Timestamp, Value::Timestamp(_)) => Ok(()),
            _ => Err(InvalidType),
        }
    }

    /// Checked Core equality. Map equality ignores insertion order; iteration does not.
    pub fn equal_values(&self, a: &Value, b: &Value) -> Option<bool> {
        if !self.native_eq() || !self.accepts(a) || !self.accepts(b) {
            return None;
        }
        Some(self.equal(a, b))
    }

    fn equal(&self, a: &Value, b: &Value) -> bool {
        match self {
            Self::Option(inner) => match (a, b) {
                (Value::None, Value::None) => true,
                (Value::None, _) | (_, Value::None) => false,
                (Value::Some(a), Value::Some(b)) if matches!(inner.as_ref(), Self::Option(_)) => {
                    inner.equal(a, b)
                }
                _ => inner.equal(a, b),
            },
            Self::List(inner) => {
                let (Value::List(a), Value::List(b)) = (a, b) else {
                    return false;
                };
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| inner.equal(&a, &b))
            }
            Self::Map(_, inner) => {
                let (Value::Map(a), Value::Map(b)) = (a, b) else {
                    return false;
                };
                if Arc::ptr_eq(a, b) {
                    return true;
                }
                let (a, b) = (a.entries(), b.entries());
                if a.len() != b.len() {
                    return false;
                }
                // Equal maps usually share insertion order: walk both while the keys agree.
                // Keys are unique, so the rest of `a` can only match the rest of `b`.
                let mut same = 0;
                for ((ka, va), (kb, vb)) in a.iter().zip(b) {
                    if ka != kb {
                        break;
                    }
                    if !inner.equal(va, vb) {
                        return false;
                    }
                    same += 1;
                }
                let (a, b) = (&a[same..], &b[same..]);
                let index = (b.len() > SMALL_MAP)
                    .then(|| b.iter().map(|(k, v)| (k, v)).collect::<HashMap<_, _>>());
                let find = |k: &Value| match &index {
                    Some(values) => values.get(k).copied(),
                    None => b.iter().find(|(other, _)| other == k).map(|(_, v)| v),
                };
                a.iter()
                    .all(|(k, v)| find(k).is_some_and(|other| inner.equal(v, other)))
            }
            _ => a == b,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{DuplicateKey, ListBuilder, ListPushError, MapRepr, NodeError};
    use std::sync::Arc;

    fn shape(text: &str) -> Option<ValueShape> {
        ValueShape::of(&text.parse().ok()?)
    }
    fn err(code: &str) -> Value {
        Value::Err(Arc::new(NodeError {
            code: code.into(),
            node: None,
        }))
    }

    #[test]
    fn shapes_resolve_from_parsed_terms_and_reject_what_cannot_execute() {
        assert_eq!(
            shape("core/map@1<core/string@1,core/list@1<core/i32@1>>"),
            Some(ValueShape::Map(
                Box::new(ValueShape::String),
                Box::new(ValueShape::List(Box::new(ValueShape::I32)))
            ))
        );
        let wrap = |n| format!("{}core/json@1{}", "core/option@1<".repeat(n), ">".repeat(n));
        assert!(shape(&wrap(MAX_NESTING)).is_some());
        assert!(shape(&wrap(MAX_NESTING + 1)).is_none());
        for invalid in [
            "core/option@1<org.example/custom@1>",
            "core/option@1<core/i64@1,core/i64@1>",
            "core/option@1<$T>",
            "core/option@2<core/i64@1>",
            "core/map@1<core/f64@1,core/i64@1>",
            "core/list@1",
        ] {
            assert!(shape(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn option_types_preserve_unboxed_and_nested_value_contracts() {
        let option = ValueShape::Option(Box::new(ValueShape::I64));
        assert!(option.accepts(&Value::None));
        assert!(option.accepts(&Value::I64(7)));
        assert!(!option.accepts(&Value::Bool(false)));
        assert!(!option.accepts(&Value::Some(Arc::new(Value::I64(7)))));
        let nested = ValueShape::Option(Box::new(option));
        assert!(nested.accepts(&Value::None));
        assert!(nested.accepts(&Value::Some(Arc::new(Value::None))));
        assert!(nested.accepts(&Value::Some(Arc::new(Value::I64(7)))));
        assert!(!nested.accepts(&Value::I64(7)));
        assert!(!nested.accepts(&Value::Some(Arc::new(Value::Bool(false)))));
        assert!(!nested.accepts(&err("core/Test")));
    }

    #[test]
    fn result_types_validate_error_codes_and_success_boxing() {
        let error = err("core/DivideByZero");
        let scalar = ValueShape::Result(Box::new(ValueShape::I64));
        assert!(scalar.accepts(&error));
        assert!(scalar.accepts(&Value::I64(1)));
        assert!(!scalar.accepts(&Value::Ok(Arc::new(Value::I64(1)))));
        let nested = ValueShape::Result(Box::new(scalar));
        assert!(nested.accepts(&Value::Ok(Arc::new(error.clone()))));
        assert!(!nested.accepts(&Value::I64(1)));
        assert!(nested.accepts(&error)); // An actual outer Err, not Ok(Err).
        let optional = ValueShape::Result(Box::new(ValueShape::Option(Box::new(
            ValueShape::Result(Box::new(ValueShape::I64)),
        ))));
        assert!(optional.accepts(&Value::Ok(Arc::new(Value::None))));
        assert!(!optional.accepts(&Value::None));
        let long = format!("core/{}", "x".repeat(257));
        for code in [
            "no_namespace",
            "core/secret\ntext",
            "core/a/b",
            long.as_str(),
        ] {
            assert!(!nested.accepts(&err(code)));
        }
    }

    #[test]
    fn exact_container_shapes_and_byte_limits_are_checked() {
        let mut deep = ValueShape::U64;
        for _ in 0..=MAX_NESTING {
            deep = ValueShape::Option(Box::new(deep));
        }
        assert_eq!(deep.checked_charge(&Value::None, 16), Err(InvalidType));
        assert!(!deep.native_eq());
        let list = ValueShape::List(Box::new(ValueShape::U64));
        let packed = Value::List(Arc::new(ListRepr::U64(vec![1, 2, 3].into())));
        let generic = Value::List(Arc::new(ListRepr::Values(
            vec![Value::U64(1), Value::U64(2), Value::U64(3)].into(),
        )));
        assert_eq!(list.checked_charge(&packed, 64), Ok(64));
        assert_eq!(list.checked_charge(&generic, 64), Ok(64));
        assert_eq!(list.equal_values(&packed, &generic), Some(true));
        assert_eq!(list.checked_charge(&packed, 63), Err(ResourceLimit));
        assert_eq!(
            list.checked_charge(&Value::List(Arc::new(ListRepr::I64(vec![1].into()))), 64),
            Err(InvalidType)
        );
        assert!(!ValueShape::List(Box::new(ValueShape::F64)).native_eq());
        // Bare and result errors charge the same envelope.
        let code = "core/Test";
        assert_eq!(
            ValueShape::Error.checked_charge(&err(code), usize::MAX),
            Ok(ERROR_CHARGE + code.len())
        );
        assert_eq!(
            ValueShape::Result(Box::new(ValueShape::I64)).checked_charge(&err(code), usize::MAX),
            Ok(ERROR_CHARGE + code.len())
        );
    }

    #[test]
    fn map_equality_ignores_order_but_checks_nested_values() {
        let ty = ValueShape::Map(
            Box::new(ValueShape::String),
            Box::new(ValueShape::List(Box::new(ValueShape::I32))),
        );
        let entry = |key: &str, n| {
            (
                Value::Str(Arc::new(key.into())),
                Value::List(Arc::new(ListRepr::I32(vec![n].into()))),
            )
        };
        let map = |entries: Vec<_>| Value::Map(Arc::new(MapRepr::new(entries).unwrap()));
        let a = map(vec![entry("z", 1), entry("a", 2)]);
        let b = map(vec![entry("a", 2), entry("z", 1)]);
        let c = map(vec![entry("a", 2), entry("z", 3)]);
        assert_ne!(a, b);
        assert_eq!(ty.equal_values(&a, &b), Some(true));
        assert_eq!(ty.equal_values(&a, &c), Some(false));
    }

    #[test]
    fn small_and_large_maps_reject_duplicates_and_compare_by_key() {
        let map =
            |entries: Vec<(Value, Value)>| Value::Map(Arc::new(MapRepr::new(entries).unwrap()));
        let pairs = |keys: &[u64]| -> Vec<(Value, Value)> {
            keys.iter()
                .map(|&k| (Value::U64(k), Value::I64(k as i64)))
                .collect()
        };
        let ints = |keys: &[u64]| map(pairs(keys));
        let ty = ValueShape::Map(Box::new(ValueShape::U64), Box::new(ValueShape::I64));
        let nested = ValueShape::Map(Box::new(ValueShape::String), Box::new(ty.clone()));
        let text = |s: &str| Value::Str(Arc::new(s.into()));
        // Both sides of the pairwise/hash cutoff.
        for n in [2, SMALL_MAP, SMALL_MAP + 1, 100] {
            let keys: Vec<u64> = (0..n as u64).collect();
            let reversed: Vec<u64> = keys.iter().rev().copied().collect();
            let (a, b) = (ints(&keys), ints(&reversed));
            assert_eq!(ty.equal_values(&a, &b), Some(true), "{n}");
            // A shared prefix, then the same keys in another order.
            let mut tail_swapped = keys.clone();
            tail_swapped.swap(n - 2, n - 1);
            assert_eq!(ty.equal_values(&a, &ints(&tail_swapped)), Some(true), "{n}");
            let mut shifted = keys.clone();
            shifted[n - 1] = n as u64; // Same length, one different key.
            assert_eq!(ty.equal_values(&a, &ints(&shifted)), Some(false), "{n}");
            let mut changed = pairs(&keys);
            changed[n - 1].1 = Value::I64(-1); // Same keys in order, one different value.
            assert_eq!(ty.equal_values(&a, &map(changed)), Some(false), "{n}");
            let mut repeated = keys.clone();
            repeated.push(0); // The repeat comes last, after the first key.
            assert_eq!(MapRepr::new(pairs(&repeated)), Err(DuplicateKey), "{n}");
            // Nested maps compare by key.
            let outer = |inner| map(vec![(text("x"), inner)]);
            assert_eq!(nested.equal_values(&outer(a), &outer(b)), Some(true), "{n}");
        }
        // Equal values under different keys are not equal maps.
        let swapped = map(vec![
            (Value::U64(0), Value::I64(1)),
            (Value::U64(1), Value::I64(0)),
        ]);
        assert_eq!(ty.equal_values(&ints(&[0, 1]), &swapped), Some(false));
    }

    #[test]
    fn decoded_literals_satisfy_their_shape_and_lists_share_one_packing() {
        use crate::document::literal::Literal;
        use crate::value::literal_codec::{SchemaSet, decode};
        use serde_json::json;
        for (ty, json) in [
            ("core/list@1<core/u64@1>", json!(["1", "2"])),
            ("core/list@1<core/string@1>", json!(["a", "b"])),
            ("core/list@1<core/f64@1>", json!(["3ff0000000000000"])),
            ("core/list@1<core/list@1<core/i32@1>>", json!([[1], [2, 3]])),
            (
                "core/option@1<core/option@1<core/i64@1>>",
                json!({"some": {"none": true}}),
            ),
            (
                "core/map@1<core/string@1,core/list@1<core/bool@1>>",
                json!([["k", [true]]]),
            ),
        ] {
            let ty: TypeExpr = ty.parse().unwrap();
            let value = decode(&Literal::value(ty.clone(), json), &SchemaSet::default()).unwrap();
            let shape = ValueShape::of(&ty).unwrap();
            assert!(shape.accepts(&value), "{ty}");
            if let (ValueShape::List(element), Value::List(items)) = (&shape, &value) {
                let mut rebuilt = ListBuilder::new(Some(element));
                for item in items.iter() {
                    rebuilt.push(item).unwrap();
                }
                let rebuilt = rebuilt.finish();
                assert_eq!(
                    std::mem::discriminant(&rebuilt),
                    std::mem::discriminant(items.as_ref()),
                    "{ty}"
                );
            }
        }
        let mut strings = ListBuilder::new(Some(&ValueShape::String));
        assert_eq!(strings.push(Value::U64(1)), Err(ListPushError::Mismatch));
        assert!(strings.finish().is_empty());
    }
}
