//! In-memory values (12 §7). Small scalars are inline; everything larger is an immutable
//! `Arc`, so cloning a value is at most a reference-count increment.
//!
//! The static type disambiguates: `option<T>` is `None` or the unboxed `T`, and only a
//! nested option boxes its inner layer in `Some`. Missing exists only as an absent record
//! field. JSON null exists only inside `Json`. Together these keep None, missing, JSON null
//! and Error distinct by construction (FR-08). Secrets are never values.

mod base64;
pub mod literal_codec;

use std::sync::Arc;

use crate::id::TypeExpr;

/// A runtime value. Equality is structural identity (floats compare by bit pattern), not
/// the language's `eq` constraint, which belongs to the runtime.
#[derive(Debug, Clone)]
pub enum Value {
    Unit,
    Bool(bool),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    /// Signed nanoseconds.
    Duration(i64),
    /// UTC nanoseconds since the Unix epoch.
    Timestamp(i64),
    /// Thin pointer: a fat `Arc<str>` would make `Value` 24 bytes.
    Str(Arc<String>),
    Bytes(Arc<Vec<u8>>),
    List(Arc<ListRepr>),
    Map(Arc<MapRepr>),
    Record(Arc<RecordRepr>),
    /// `tag` is the variant's index in schema order; persisted literals use `variant_id`.
    Enum {
        tag: u32,
        payload: Option<Arc<Value>>,
    },
    None,
    /// Only for the inner layer of a nested option.
    Some(Arc<Value>),
    /// Only inside `result<T>`.
    Err(Arc<NodeError>),
    Json(Arc<JsonText>),
    Handle(HandleId),
}

const _: () = assert!(
    size_of::<Value>() <= 16,
    "12 §7: Value must stay within 16 bytes"
);

/// Homogeneous list storage; scalar element types use contiguous typed arrays.
#[derive(Debug, Clone)]
pub enum ListRepr {
    Bool(Box<[bool]>),
    I32(Box<[i32]>),
    I64(Box<[i64]>),
    U32(Box<[u32]>),
    U64(Box<[u64]>),
    F32(Box<[f32]>),
    F64(Box<[f64]>),
    Str(Box<[Arc<String>]>),
    Values(Box<[Value]>),
}

impl ListRepr {
    pub fn len(&self) -> usize {
        match self {
            ListRepr::Bool(items) => items.len(),
            ListRepr::I32(items) => items.len(),
            ListRepr::I64(items) => items.len(),
            ListRepr::U32(items) => items.len(),
            ListRepr::U64(items) => items.len(),
            ListRepr::F32(items) => items.len(),
            ListRepr::F64(items) => items.len(),
            ListRepr::Str(items) => items.len(),
            ListRepr::Values(items) => items.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Element `index` as a `Value`.
    pub fn get(&self, index: usize) -> Option<Value> {
        Some(match self {
            ListRepr::Bool(items) => Value::Bool(*items.get(index)?),
            ListRepr::I32(items) => Value::I32(*items.get(index)?),
            ListRepr::I64(items) => Value::I64(*items.get(index)?),
            ListRepr::U32(items) => Value::U32(*items.get(index)?),
            ListRepr::U64(items) => Value::U64(*items.get(index)?),
            ListRepr::F32(items) => Value::F32(*items.get(index)?),
            ListRepr::F64(items) => Value::F64(*items.get(index)?),
            ListRepr::Str(items) => Value::Str(items.get(index)?.clone()),
            ListRepr::Values(items) => items.get(index)?.clone(),
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = Value> + '_ {
        (0..self.len()).map(|index| self.get(index).expect("index is in range"))
    }
}

/// Insertion-ordered entries with unique, non-float keys (11 §5.2).
#[derive(Debug, Clone, PartialEq)]
pub struct MapRepr {
    pub entries: Box<[(Value, Value)]>,
}

/// Fields in schema order; `None` is an absent (missing) optional field.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordRepr {
    pub ty: TypeExpr,
    pub fields: Box<[Option<Value>]>,
}

/// Valid JSON text kept verbatim, so exact number text and JSON null survive (03). The
/// structured tree for JSON nodes arrives with them (05d).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonText(String);

impl JsonText {
    /// Accepts any valid JSON text.
    pub fn new(text: String) -> Result<Self, serde_json::Error> {
        serde_json::from_str::<serde::de::IgnoredAny>(&text)?;
        Ok(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Generation-checked resource handle (12 §2); never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleId {
    pub index: u32,
    pub generation: u32,
    /// Plan-local interned handle kind (`TypeKey`).
    pub kind: u32,
}

/// Structured node error (`core/error@1`). The message fields of 12 §6.2 arrive with the
/// WU-03 contracts; a value only needs the stable code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeError {
    /// Stable namespaced code, such as `core/NumericOverflow`.
    pub code: String,
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (Unit, Unit) | (None, None) => true,
            (Bool(a), Bool(b)) => a == b,
            (I32(a), I32(b)) => a == b,
            (U32(a), U32(b)) => a == b,
            (I64(a), I64(b)) | (Duration(a), Duration(b)) | (Timestamp(a), Timestamp(b)) => a == b,
            (U64(a), U64(b)) => a == b,
            (F32(a), F32(b)) => a.to_bits() == b.to_bits(),
            (F64(a), F64(b)) => a.to_bits() == b.to_bits(),
            (Str(a), Str(b)) => a == b,
            (Bytes(a), Bytes(b)) => a == b,
            (List(a), List(b)) => a == b,
            (Map(a), Map(b)) => a == b,
            (Record(a), Record(b)) => a == b,
            (
                Enum {
                    tag: a,
                    payload: pa,
                },
                Enum {
                    tag: b,
                    payload: pb,
                },
            ) => a == b && pa == pb,
            (Some(a), Some(b)) => a == b,
            (Err(a), Err(b)) => a == b,
            (Json(a), Json(b)) => a == b,
            (Handle(a), Handle(b)) => a == b,
            _ => false,
        }
    }
}

/// Lists are equal when their elements are, whatever the storage.
impl PartialEq for ListRepr {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_fits_sixteen_bytes() {
        assert!(size_of::<Value>() <= 16, "{} bytes", size_of::<Value>());
    }

    #[test]
    fn floats_compare_by_bits() {
        assert_eq!(Value::F64(f64::NAN), Value::F64(f64::NAN));
        assert_ne!(Value::F64(0.0), Value::F64(-0.0));
        assert_ne!(
            Value::I64(1),
            Value::Duration(1),
            "different variants differ"
        );
    }

    #[test]
    fn list_equality_ignores_storage() {
        let typed = ListRepr::I64(vec![1, 2].into());
        let boxed = ListRepr::Values(vec![Value::I64(1), Value::I64(2)].into());
        assert_eq!(typed, boxed);
    }

    #[test]
    fn json_text_must_parse() {
        assert!(JsonText::new("{\"n\": 1.50}".into()).is_ok());
        assert!(JsonText::new("null".into()).is_ok());
        assert!(JsonText::new("{".into()).is_err());
    }
}
