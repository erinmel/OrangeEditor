//! Bounded exact Core value validation and nominal retained charges.
use crate::PlanType;
use orange_model::{Value, value::ListRepr};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
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
impl PlanType {
    pub(crate) fn native_hash(&self) -> bool {
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
        if depth > orange_model::type_term::MAX_NESTING {
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
        if depth > orange_model::type_term::MAX_NESTING {
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
        if depth > orange_model::type_term::MAX_NESTING {
            return Err(InvalidType);
        }
        if let Self::Result(inner) = self {
            return match value {
                Value::Err(error) if crate::compile::valid_identifier(&error.code) => {
                    charge(left, error.code.len().checked_add(32).ok_or(ResourceLimit)?)
                }
                Value::Ok(value) if inner.unboxed_error() => {
                    charge(left, 16)?;
                    inner.check(value, left, depth + 1)
                }
                _ if inner.unboxed_error() => Err(InvalidType),
                _ => inner.check(value, left, depth + 1),
            };
        }
        if let Self::Option(inner) = self {
            return match value {
                Value::None => charge(left, 16),
                Value::Some(value) if matches!(inner.as_ref(), Self::Option(_)) => {
                    charge(left, 16)?;
                    inner.check(value, left, depth + 1)
                }
                _ if matches!(inner.as_ref(), Self::Option(_)) => Err(InvalidType),
                _ => inner.check(value, left, depth + 1),
            };
        }
        charge(left, 16)?;
        match (self, value) {
            (Self::List(inner), Value::List(items)) => {
                minimum(*left, items.len(), 16)?;
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
                    return charge(left, items.len().checked_mul(16).ok_or(ResourceLimit)?);
                }
                for value in items.iter() {
                    inner.check(&value, left, depth + 1)?;
                }
                Ok(())
            }
            (Self::Map(key, ty), Value::Map(map)) if key.native_hash() => {
                minimum(*left, map.entries.len(), 32)?;
                let mut keys = HashSet::new();
                for (k, v) in &map.entries {
                    key.check(k, left, depth + 1)?;
                    if !keys.insert(k) {
                        return Err(InvalidType);
                    }
                    ty.check(v, left, depth + 1)?;
                }
                Ok(())
            }
            (Self::String, Value::Str(text)) => charge(left, text.len()),
            (Self::Bytes, Value::Bytes(bytes)) => charge(left, bytes.len()),
            (Self::Json, Value::Json(text)) => charge(left, text.as_str().len()),
            (Self::Error, Value::Err(error)) if crate::compile::valid_identifier(&error.code) => {
                charge(left, error.code.len().checked_add(16).ok_or(ResourceLimit)?)
            }
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
                if a.entries.len() != b.entries.len() {
                    return false;
                }
                let values: HashMap<_, _> = b.entries.iter().map(|(k, v)| (k, v)).collect();
                a.entries
                    .iter()
                    .all(|(k, v)| values.get(k).is_some_and(|other| inner.equal(v, other)))
            }
            _ => a == b,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn exact_container_shapes_and_byte_limits_are_checked() {
        let mut deep = PlanType::U64;
        for _ in 0..=orange_model::type_term::MAX_NESTING {
            deep = PlanType::Option(Box::new(deep));
        }
        assert_eq!(deep.checked_charge(&Value::None, 16), Err(InvalidType));
        assert!(!deep.native_eq());
        let list = PlanType::List(Box::new(PlanType::U64));
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
        let map = PlanType::Map(Box::new(PlanType::U64), Box::new(PlanType::String));
        let duplicate = Value::Map(Arc::new(orange_model::value::MapRepr {
            entries: vec![
                (Value::U64(1), Value::Str(Arc::new("a".into()))),
                (Value::U64(1), Value::Str(Arc::new("b".into()))),
            ]
            .into(),
        }));
        assert_eq!(map.checked_charge(&duplicate, 1024), Err(InvalidType));
        assert!(!PlanType::List(Box::new(PlanType::F64)).native_eq());
    }
    #[test]
    fn map_equality_ignores_order_but_checks_nested_values() {
        let ty = PlanType::Map(
            Box::new(PlanType::String),
            Box::new(PlanType::List(Box::new(PlanType::I32))),
        );
        let entry = |key: &str, n| {
            (
                Value::Str(Arc::new(key.into())),
                Value::List(Arc::new(ListRepr::I32(vec![n].into()))),
            )
        };
        let a = Value::Map(Arc::new(orange_model::value::MapRepr {
            entries: vec![entry("z", 1), entry("a", 2)].into(),
        }));
        let b = Value::Map(Arc::new(orange_model::value::MapRepr {
            entries: vec![entry("a", 2), entry("z", 1)].into(),
        }));
        let c = Value::Map(Arc::new(orange_model::value::MapRepr {
            entries: vec![entry("a", 2), entry("z", 3)].into(),
        }));
        assert_ne!(a, b);
        assert_eq!(ty.equal_values(&a, &b), Some(true));
        assert_eq!(ty.equal_values(&a, &c), Some(false));
    }
}
