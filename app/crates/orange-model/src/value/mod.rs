//! In-memory values (12 §7). Small scalars are inline; everything larger is an immutable
//! `Arc`, so cloning a value is at most a reference-count increment.
//!
//! The static type disambiguates: `option<T>` is `None` or the unboxed `T`, and boxes in
//! `Some` only when `T` can itself be a bare None. Missing exists only as an absent record
//! field. JSON null exists only inside `Json`. Together these keep None, missing, JSON null
//! and Error distinct by construction (FR-08). Result successes box only when their
//! payload can itself carry the Err tag. Secrets are never values.

mod base64;
pub mod literal_codec;
pub mod shape;

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::mem::discriminant;
use std::sync::Arc;

use crate::error::JsonTextError;
use crate::id::TypeExpr;
use crate::type_term::MAX_NESTING;

/// A runtime value. Equality and hashing are structural identity (floats compare by bit
/// pattern, lists by elements whatever their storage), not the language's `eq`/`hash`
/// constraints, which belong to the runtime.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
    /// Present option whose payload could otherwise be mistaken for outer None,
    /// such as a nested option or an unboxed result success that can be None (12 §7).
    Some(Arc<Value>),
    /// Only inside `result<T>`.
    Err(Arc<NodeError>),
    /// Successful result whose payload could otherwise be mistaken for outer Err.
    /// Ordinary result successes remain unboxed (12 §7).
    Ok(Arc<Value>),
    Json(Arc<JsonText>),
    Handle(HandleId),
}

const _: () = assert!(
    size_of::<Value>() <= 16,
    "12 §7: Value must stay within 16 bytes"
);

/// Homogeneous list storage; scalar element types use contiguous typed arrays.
#[derive(Debug, Clone)]
#[non_exhaustive]
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

    /// Concatenate raw immutable storage without asserting a language element type.
    /// Checked callers admit type and byte limits first; None reports capacity failure.
    pub fn concat(&self, other: &Self) -> Option<Self> {
        let len = self.len().checked_add(other.len())?;
        macro_rules! packed {
            ($($variant:ident),*) => {
                match (self,other) {
                    $((Self::$variant(a),Self::$variant(b))=>{
                        let mut values=Vec::new();
                        values.try_reserve_exact(len).ok()?;
                        values.extend_from_slice(a);
                        values.extend_from_slice(b);
                        Self::$variant(values.into_boxed_slice())
                    },)*
                    _=>{
                        let mut values=Vec::new();
                        values.try_reserve_exact(len).ok()?;
                        values.extend(self.iter());
                        values.extend(other.iter());
                        Self::Values(values.into_boxed_slice())
                    }
                }
            };
        }
        Some(packed!(Bool, I32, I64, U32, U64, F32, F64, Str))
    }
    pub fn iter(&self) -> impl Iterator<Item = Value> + '_ {
        (0..self.len()).map(|index| self.get(index).expect("index is in range"))
    }
}

/// Why [`ListBuilder::push`] refused an element. Nothing is appended on error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ListPushError {
    /// The element is not stored as the builder's packed element type.
    #[error("list element does not match the packed element type")]
    Mismatch,
    /// The allocation could not grow.
    #[error("list allocation could not grow")]
    Capacity,
}

/// Builds a [`ListRepr`] in the one packing every producer uses: scalar and string
/// elements are stored natively, everything else as generic values.
#[derive(Debug, Clone)]
pub struct ListBuilder(Packed);

#[derive(Debug, Clone)]
enum Packed {
    Bool(Vec<bool>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    Str(Vec<Arc<String>>),
    Values(Vec<Value>),
}

impl ListBuilder {
    /// `element` is the element shape when it is executable; None stores generic values.
    pub fn new(element: Option<&shape::ValueShape>) -> Self {
        use shape::ValueShape as S;
        Self(match element {
            Some(S::Bool) => Packed::Bool(Vec::new()),
            Some(S::I32) => Packed::I32(Vec::new()),
            Some(S::I64) => Packed::I64(Vec::new()),
            Some(S::U32) => Packed::U32(Vec::new()),
            Some(S::U64) => Packed::U64(Vec::new()),
            Some(S::F32) => Packed::F32(Vec::new()),
            Some(S::F64) => Packed::F64(Vec::new()),
            Some(S::String) => Packed::Str(Vec::new()),
            _ => Packed::Values(Vec::new()),
        })
    }

    /// Reserve room for exactly `additional` more elements, for a producer that knows its
    /// result length, so the list is built in one allocation instead of by doubling.
    pub fn reserve(&mut self, additional: usize) -> Result<(), ListPushError> {
        macro_rules! reserve {
            ($($variant:ident),*) => {
                match &mut self.0 {
                    $(Packed::$variant(items) => items.try_reserve_exact(additional),)*
                }
            };
        }
        reserve!(Bool, I32, I64, U32, U64, F32, F64, Str, Values)
            .map_err(|_| ListPushError::Capacity)
    }

    pub fn push(&mut self, value: Value) -> Result<(), ListPushError> {
        macro_rules! push {
            ($($variant:ident),*) => {
                match (&mut self.0, value) {
                    $((Packed::$variant(items), Value::$variant(item)) => {
                        items.try_reserve(1).map_err(|_| ListPushError::Capacity)?;
                        items.push(item);
                    })*
                    (Packed::Values(items), item) => {
                        items.try_reserve(1).map_err(|_| ListPushError::Capacity)?;
                        items.push(item);
                    }
                    _ => return Err(ListPushError::Mismatch),
                }
            };
        }
        push!(Bool, I32, I64, U32, U64, F32, F64, Str);
        Ok(())
    }

    pub fn finish(self) -> ListRepr {
        match self.0 {
            Packed::Bool(items) => ListRepr::Bool(items.into()),
            Packed::I32(items) => ListRepr::I32(items.into()),
            Packed::I64(items) => ListRepr::I64(items.into()),
            Packed::U32(items) => ListRepr::U32(items.into()),
            Packed::U64(items) => ListRepr::U64(items.into()),
            Packed::F32(items) => ListRepr::F32(items.into()),
            Packed::F64(items) => ListRepr::F64(items.into()),
            Packed::Str(items) => ListRepr::Str(items.into()),
            Packed::Values(items) => ListRepr::Values(items.into()),
        }
    }
}

/// Insertion-ordered entries with unique, non-float keys (11 §5.2). Uniqueness is checked
/// once, when the map is built, so shape checks and comparisons never search for
/// duplicates.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct MapRepr {
    entries: Box<[(Value, Value)]>,
}

/// [`MapRepr::new`] was given the same key twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("map entries repeat a key")]
pub struct DuplicateKey;

/// Maps with at most this many entries match keys pairwise, so building and comparing the
/// common small map allocates nothing. Larger maps use one presized hash table, keeping
/// the cost linear.
const SMALL_MAP: usize = 16;

impl MapRepr {
    pub fn new(entries: impl Into<Box<[(Value, Value)]>>) -> Result<Self, DuplicateKey> {
        let entries = entries.into();
        let unique = if entries.len() <= SMALL_MAP {
            entries
                .iter()
                .enumerate()
                .all(|(i, (k, _))| entries[..i].iter().all(|(other, _)| other != k))
        } else {
            let mut seen = HashSet::with_capacity(entries.len());
            entries.iter().all(|(k, _)| seen.insert(k))
        };
        unique.then_some(Self { entries }).ok_or(DuplicateKey)
    }

    pub fn entries(&self) -> &[(Value, Value)] {
        &self.entries
    }
}

/// Fields in schema order; `None` is an absent (missing) optional field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordRepr {
    pub ty: TypeExpr,
    pub fields: Box<[Option<Value>]>,
}

/// Valid JSON text kept verbatim, so exact number text and JSON null survive (03). The
/// structured tree for JSON nodes arrives with them (05d).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JsonText(String);

/// Size budget for one JSON text value (03 requires codec size budgets). Proposed value,
/// recorded as an open budget in 10.
pub const MAX_JSON_TEXT_BYTES: usize = 1 << 20;

impl JsonText {
    /// Accepts valid JSON text of at most [`MAX_JSON_TEXT_BYTES`] bytes whose arrays and
    /// objects nest at most [`MAX_NESTING`] deep. Size and depth are checked before parsing.
    pub fn new(text: String) -> Result<Self, JsonTextError> {
        Self::check(&text)?;
        Ok(Self(text))
    }

    /// Checks borrowed text before copying it, so rejected text is never copied.
    fn check(text: &str) -> Result<(), JsonTextError> {
        if text.len() > MAX_JSON_TEXT_BYTES {
            return Err(JsonTextError::TooLarge {
                len: text.len(),
                max: MAX_JSON_TEXT_BYTES,
            });
        }
        if nests_deeper_than(text, MAX_NESTING) {
            return Err(JsonTextError::TooDeep);
        }
        serde_json::from_str::<serde::de::IgnoredAny>(text)?;
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for JsonText {
    type Error = JsonTextError;

    /// Like [`JsonText::new`], but copies `text` only once it is accepted.
    fn try_from(text: &str) -> Result<Self, JsonTextError> {
        Self::check(text)?;
        Ok(Self(text.to_owned()))
    }
}

/// True if arrays and objects in `text` nest deeper than `max`, counting only brackets
/// outside strings. One pass, no allocation; malformed text is left to the parser.
fn nests_deeper_than(text: &str, max: usize) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for byte in text.bytes() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > max {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
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
/// WU-03 contracts; constant capture retains stable code and producer identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeError {
    /// Stable namespaced code, such as `core/NumericOverflow`.
    pub code: String,
    /// Original producer when known; absent for a detached host-created error.
    pub node: Option<crate::id::NodeId>,
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
            (Some(a), Some(b)) | (Ok(a), Ok(b)) => a == b,
            (Err(a), Err(b)) => a == b,
            (Json(a), Json(b)) => a == b,
            (Handle(a), Handle(b)) => a == b,
            _ => false,
        }
    }
}

/// Bit-pattern float equality is reflexive, so structural equality is a full equivalence.
impl Eq for Value {}

/// Consistent with `PartialEq`: the variant, then its content (floats by bit pattern).
impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        use Value::*;
        discriminant(self).hash(state);
        match self {
            Unit | None => {}
            Bool(v) => v.hash(state),
            I32(v) => v.hash(state),
            U32(v) => v.hash(state),
            I64(v) | Duration(v) | Timestamp(v) => v.hash(state),
            U64(v) => v.hash(state),
            F32(v) => v.to_bits().hash(state),
            F64(v) => v.to_bits().hash(state),
            Str(v) => v.hash(state),
            Bytes(v) => v.hash(state),
            List(v) => v.hash(state),
            Map(v) => v.hash(state),
            Record(v) => v.hash(state),
            Enum { tag, payload } => (tag, payload).hash(state),
            Some(v) | Ok(v) => v.hash(state),
            Err(v) => v.hash(state),
            Json(v) => v.hash(state),
            Handle(v) => v.hash(state),
        }
    }
}

/// Lists are equal when their elements are, whatever the storage.
/// Equal storage compares the slices directly, without building a [`Value`] per element;
/// floats still compare by bits. Mixed storage compares element values.
impl PartialEq for ListRepr {
    fn eq(&self, other: &Self) -> bool {
        use ListRepr as L;
        match (self, other) {
            (L::Bool(a), L::Bool(b)) => a == b,
            (L::I32(a), L::I32(b)) => a == b,
            (L::I64(a), L::I64(b)) => a == b,
            (L::U32(a), L::U32(b)) => a == b,
            (L::U64(a), L::U64(b)) => a == b,
            (L::F32(a), L::F32(b)) => a
                .iter()
                .map(|x| x.to_bits())
                .eq(b.iter().map(|x| x.to_bits())),
            (L::F64(a), L::F64(b)) => a
                .iter()
                .map(|x| x.to_bits())
                .eq(b.iter().map(|x| x.to_bits())),
            (L::Str(a), L::Str(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| Arc::ptr_eq(x, y) || x == y)
            }
            (L::Values(a), L::Values(b)) => a == b,
            _ => self.len() == other.len() && self.iter().zip(other.iter()).all(|(a, b)| a == b),
        }
    }
}

impl Eq for ListRepr {}

/// Hashes elements, not storage, so a typed array and an equal `Values` list agree.
impl Hash for ListRepr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        self.iter().for_each(|item| item.hash(state));
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
    fn same_storage_equality_matches_element_equality() {
        use std::hash::{BuildHasher, RandomState};
        let hasher = RandomState::new();
        let shared = Arc::new("x".to_owned());
        let floats = |items: &[f64]| ListRepr::F64(items.into());
        let strings = |items: Vec<Arc<String>>| ListRepr::Str(items.into());
        let cases = [
            (floats(&[f64::NAN, 1.0]), floats(&[f64::NAN, 1.0]), true),
            (floats(&[0.0]), floats(&[-0.0]), false),
            (
                ListRepr::F32([f32::NAN].into()),
                ListRepr::F32([f32::NAN].into()),
                true,
            ),
            (
                strings(vec![shared.clone()]),
                strings(vec![shared.clone()]),
                true,
            ),
            (
                strings(vec![shared.clone()]),
                strings(vec![Arc::new("x".into())]),
                true,
            ),
            (
                strings(vec![shared.clone()]),
                strings(vec![Arc::new("y".into())]),
                false,
            ),
            (
                ListRepr::I64([1, 2].into()),
                ListRepr::I64([1].into()),
                false,
            ),
        ];
        for (a, b, equal) in cases {
            let elementwise = a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y);
            assert_eq!(a == b, equal, "{a:?} vs {b:?}");
            assert_eq!(elementwise, equal, "{a:?} vs {b:?}");
            if equal {
                assert_eq!(hasher.hash_one(&a), hasher.hash_one(&b));
                let boxed = ListRepr::Values(a.iter().collect::<Vec<_>>().into());
                assert_eq!(boxed, b);
                assert_eq!(hasher.hash_one(&boxed), hasher.hash_one(&b));
            }
        }
    }

    #[test]
    fn json_text_must_parse() {
        assert!(JsonText::new("{\"n\": 1.50}".into()).is_ok());
        assert!(JsonText::new("null".into()).is_ok());
        assert!(JsonText::new("{".into()).is_err());
        // The borrowed constructor applies the same checks.
        for text in ["{\"n\": 1.50}", "null", "{"] {
            assert_eq!(
                JsonText::try_from(text).ok(),
                JsonText::new(text.into()).ok()
            );
        }
    }

    fn nested_arrays(levels: usize) -> String {
        format!("{}{}", "[".repeat(levels), "]".repeat(levels))
    }

    #[test]
    fn json_text_nesting_is_bounded() {
        assert!(JsonText::new(nested_arrays(MAX_NESTING)).is_ok());
        for levels in [MAX_NESTING + 1, 84] {
            assert!(
                matches!(
                    JsonText::new(nested_arrays(levels)),
                    Err(JsonTextError::TooDeep)
                ),
                "{levels}"
            );
        }
        // Brackets inside strings, including after escaped quotes, do not count.
        let quoted = format!(r#"["\"{}"]"#, "[".repeat(200));
        assert!(JsonText::new(quoted).is_ok());
    }

    #[test]
    fn json_text_size_is_bounded() {
        let at_limit = format!("\"{}\"", "a".repeat(MAX_JSON_TEXT_BYTES - 2));
        assert!(JsonText::new(at_limit).is_ok());
        let over = format!("\"{}\"", "a".repeat(MAX_JSON_TEXT_BYTES - 1));
        assert!(matches!(
            JsonText::new(over),
            Err(JsonTextError::TooLarge { .. })
        ));
    }

    fn hash_of(value: &Value) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn equal_values_hash_equally() {
        let typed = Value::List(Arc::new(ListRepr::I64(vec![1, 2].into())));
        let boxed = Value::List(Arc::new(ListRepr::Values(
            vec![Value::I64(1), Value::I64(2)].into(),
        )));
        assert_eq!(typed, boxed);
        assert_eq!(hash_of(&typed), hash_of(&boxed));
        assert_eq!(
            hash_of(&Value::F64(f64::NAN)),
            hash_of(&Value::F64(f64::NAN))
        );
        assert_ne!(hash_of(&Value::F64(0.0)), hash_of(&Value::F64(-0.0)));
        assert_ne!(hash_of(&Value::I64(1)), hash_of(&Value::Duration(1)));
    }

    #[test]
    fn nested_result_success_retains_its_own_tag_and_error_identity() {
        let error = Value::Err(Arc::new(NodeError {
            code: "core/DivideByZero".into(),
            node: None,
        }));
        let success = Value::Ok(Arc::new(error.clone()));
        let copy = success.clone();
        assert_eq!(success, copy);
        assert_eq!(hash_of(&success), hash_of(&copy));
        assert_ne!(success, error);
        assert_ne!(hash_of(&success), hash_of(&error));
        assert_ne!(success, Value::Some(Arc::new(error)));
        assert!(size_of::<Value>() <= 16);
    }
}
