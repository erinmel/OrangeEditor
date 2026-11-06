//! Resolved structural shape of an executable Core type, and the one bounded check of a
//! [`Value`] against it (12 §7). Shapes come from the parsed [`TypeTerm`], never from
//! type-expression text, and own the option/result boxing rule and nominal byte charges.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use thiserror::Error;

use super::literal_codec::Schemas;
use super::{ListRepr, SMALL_MAP, Value};
use crate::id::TypeExpr;
use crate::type_term::{MAX_NESTING, NamedTerm, TypeTerm};

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
    /// A nominal enum resolved from its registered schema ([`ValueShape::of_in`]).
    Enum(Arc<EnumShape>),
    /// A nominal record resolved from its registered schema ([`ValueShape::of_in`]).
    Record(Arc<RecordShape>),
}

/// A package record's identity and field shapes in schema order. Like [`EnumShape`],
/// only schema resolution builds one, so its fields are within the nesting limit and
/// never cyclic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordShape {
    ty: TypeExpr,
    fields: Box<[FieldShape]>,
    /// `eq` and `hash` are structural derives and every field is a map key itself.
    key: bool,
    /// `eq` is a structural derive over members the host compares itself.
    eq: bool,
    /// Structural `Value` identity is this type's equality: no member holds a map.
    structural: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldShape {
    id: String,
    shape: ValueShape,
    optional: bool,
}

impl RecordShape {
    /// The canonical nominal type, which every value's [`RecordRepr::ty`] must equal.
    ///
    /// [`RecordRepr::ty`]: super::RecordRepr::ty
    pub fn ty(&self) -> &TypeExpr {
        &self.ty
    }

    /// Number of fields in the resolved schema.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// The compiled index of a stable field ID, as [`RecordRepr::fields`] stores it.
    ///
    /// [`RecordRepr::fields`]: super::RecordRepr::fields
    pub fn index(&self, field_id: &str) -> Option<usize> {
        self.fields.iter().position(|field| field.id == field_id)
    }

    /// The stable field ID at a compiled index.
    pub fn field_id(&self, index: usize) -> Option<&str> {
        Some(self.fields.get(index)?.id.as_str())
    }

    /// The shape of a field and whether it may be absent (missing, not None).
    pub fn field(&self, index: usize) -> Option<(&ValueShape, bool)> {
        let field = self.fields.get(index)?;
        Some((&field.shape, field.optional))
    }

    /// Whether the record's `eq` and `hash` are declared structural derives over fields
    /// that are map keys themselves ([`ValueShape::map_key`]).
    pub fn derives_key(&self) -> bool {
        self.key
    }

    /// Whether the host executes this type's `eq` itself ([`ValueShape::host_eq`]).
    pub fn derives_eq(&self) -> bool {
        self.eq
    }
}

/// A package enum's identity, variant IDs and payload shapes in schema (tag) order.
/// Only schema resolution builds one, so its payloads are within the nesting limit and
/// never cyclic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumShape {
    ty: TypeExpr,
    variants: Box<[(String, Option<ValueShape>)]>,
    /// `eq` and `hash` are structural derives and every payload is a map key itself.
    key: bool,
    /// `eq` is a structural derive over members the host compares itself.
    eq: bool,
    /// Structural `Value` identity is this type's equality: no member holds a map.
    structural: bool,
}

impl EnumShape {
    /// The canonical nominal type.
    pub fn ty(&self) -> &TypeExpr {
        &self.ty
    }

    /// Number of variants in the resolved schema.
    pub fn len(&self) -> usize {
        self.variants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.variants.is_empty()
    }

    /// The tag of a stable variant ID, as [`Value::Enum`] carries it.
    pub fn tag(&self, variant_id: &str) -> Option<u32> {
        let index = self.variants.iter().position(|(id, _)| id == variant_id)?;
        u32::try_from(index).ok()
    }

    /// The stable variant ID of a tag.
    pub fn variant_id(&self, tag: u32) -> Option<&str> {
        Some(self.variant(tag)?.0.as_str())
    }

    /// The payload shape of a tag: `Some(None)` for a variant without payload.
    pub fn payload(&self, tag: u32) -> Option<Option<&ValueShape>> {
        Some(self.variant(tag)?.1.as_ref())
    }

    fn variant(&self, tag: u32) -> Option<&(String, Option<ValueShape>)> {
        self.variants.get(usize::try_from(tag).ok()?)
    }

    /// Whether the enum's `eq` and `hash` are declared structural derives over payloads
    /// that are map keys themselves ([`ValueShape::map_key`]).
    pub fn derives_key(&self) -> bool {
        self.key
    }

    /// Whether the host executes this type's `eq` itself ([`ValueShape::host_eq`]).
    pub fn derives_eq(&self) -> bool {
        self.eq
    }
}

/// Schema-aware resolution state for one top-level type. Each nominal record or enum
/// resolves once, keeping its height; any failure ends the whole resolution, so only successes
/// are memoized. A type reached again through its own payloads is refused: recursive
/// schemas have no executable shape.
struct Nominal<'a> {
    schemas: &'a dyn Schemas,
    resolved: HashMap<NamedTerm, (ValueShape, usize)>,
    active: HashSet<NamedTerm>,
}

/// A map's first complete check: an exact code of its key and value shapes and the
/// nesting depth it was checked at, plus the bytes its entries charged (the map's own
/// value charge excluded). Checks only fail at a greater depth, so the same shapes at
/// the same or a smaller depth reach the same verdict and charge; any other check walks
/// the entries again. Two words, no allocation (see [`MapCheck::code`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct MapCheck {
    code: u64,
    entries: usize,
}

/// Five-bit node tags of [`MapCheck::code`]; zero ends the code.
const SHAPE_TAG_BITS: u32 = 5;
/// Shape bits below the depth byte: eleven nodes.
const SHAPE_CODE_BITS: u32 = 55;

impl MapCheck {
    /// Preorder tags of the key then the value shape. Every tag is non-zero and fixes its
    /// arity, so the code is injective: equal codes are equal shapes, never a hash
    /// collision. The depth fills the top byte. Nominal shapes and shapes over eleven
    /// nodes have no code and are simply not remembered.
    fn code(key: &ValueShape, value: &ValueShape, depth: usize) -> Option<u64> {
        fn push(shape: &ValueShape, code: &mut u64, used: &mut u32) -> Option<()> {
            use ValueShape as S;
            let (tag, children): (u64, [Option<&ValueShape>; 2]) = match shape {
                S::Bool => (1, [None, None]),
                S::I32 => (2, [None, None]),
                S::I64 => (3, [None, None]),
                S::U32 => (4, [None, None]),
                S::U64 => (5, [None, None]),
                S::F32 => (6, [None, None]),
                S::F64 => (7, [None, None]),
                S::Json => (8, [None, None]),
                S::String => (9, [None, None]),
                S::Duration => (10, [None, None]),
                S::Timestamp => (11, [None, None]),
                S::Bytes => (12, [None, None]),
                S::Error => (13, [None, None]),
                S::List(inner) => (14, [Some(inner), None]),
                S::Option(inner) => (15, [Some(inner), None]),
                S::Result(inner) => (16, [Some(inner), None]),
                S::Map(k, v) => (17, [Some(k), Some(v)]),
                S::Enum(_) | S::Record(_) => return None,
            };
            if *used + SHAPE_TAG_BITS > SHAPE_CODE_BITS {
                return None;
            }
            *code |= tag << *used;
            *used += SHAPE_TAG_BITS;
            children
                .into_iter()
                .flatten()
                .try_for_each(|child| push(child, code, used))
        }
        let (mut code, mut used) = (0, 0);
        push(key, &mut code, &mut used)?;
        push(value, &mut code, &mut used)?;
        Some(code | (u64::try_from(depth).ok().filter(|d| *d <= 0xFF)? << 56))
    }
    pub(crate) fn new(
        key: &ValueShape,
        value: &ValueShape,
        depth: usize,
        entries: usize,
    ) -> Option<Self> {
        Some(Self {
            code: Self::code(key, value, depth)?,
            entries,
        })
    }
    pub(crate) fn charge(
        &self,
        key: &ValueShape,
        value: &ValueShape,
        depth: usize,
    ) -> Option<usize> {
        let stored = self.code >> 56;
        let code = Self::code(key, value, 0)?;
        (u64::try_from(depth).ok()? <= stored && code == self.code & ((1 << 56) - 1))
            .then_some(self.entries)
    }
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
    /// unknown types, wrong arities and keys that are not [`ValueShape::map_key`].
    pub fn of(expr: &TypeExpr) -> Option<Self> {
        Self::resolve(&TypeTerm::parse(expr).ok()?)
    }

    /// Shape of an already parsed term.
    pub fn resolve(term: &TypeTerm) -> Option<Self> {
        Self::resolve_at(term, 0, None).map(|(shape, _)| shape)
    }

    /// Shape of a canonical type expression whose nominal records and enums resolve
    /// through `schemas` (12 §5). Parameterized nominal types, handle kinds and recursive
    /// schemas stay unsupported.
    pub fn of_in(expr: &TypeExpr, schemas: &dyn Schemas) -> Option<Self> {
        Self::resolve_in(&TypeTerm::parse(expr).ok()?, schemas)
    }

    /// [`ValueShape::of_in`] for an already parsed term.
    pub fn resolve_in(term: &TypeTerm, schemas: &dyn Schemas) -> Option<Self> {
        let mut nominal = Nominal {
            schemas,
            resolved: HashMap::new(),
            active: HashSet::new(),
        };
        Self::resolve_at(term, 0, Some(&mut nominal)).map(|(shape, _)| shape)
    }

    /// The shape and its height (1 for a leaf), so a memoized nominal shape reused at
    /// another depth still respects [`MAX_NESTING`].
    fn resolve_at(
        term: &TypeTerm,
        depth: usize,
        mut nominal: Option<&mut Nominal<'_>>,
    ) -> Option<(Self, usize)> {
        if depth > MAX_NESTING {
            return None;
        }
        let TypeTerm::Named(named) = term else {
            return None;
        };
        if named.namespace != "core" || named.major != 1 {
            return Self::nominal(named, depth, nominal?);
        }
        let mut height = 1;
        let mut inner = |i: usize| {
            let (shape, h) = Self::resolve_at(&named.args[i], depth + 1, nominal.as_deref_mut())?;
            height = height.max(h + 1);
            Some(Box::new(shape))
        };
        let shape = match (named.name.as_str(), named.args.len()) {
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
                if !key.map_key() {
                    return None;
                }
                Self::Map(key, inner(1)?)
            }
            _ => return None,
        };
        Some((shape, height))
    }

    /// A registered record or enum, each member resolved one level deeper. Both kinds
    /// share the memo, the cycle guard and the height rule.
    fn nominal(
        named: &NamedTerm,
        depth: usize,
        nominal: &mut Nominal<'_>,
    ) -> Option<(Self, usize)> {
        if let Some((shape, height)) = nominal.resolved.get(named) {
            return (depth + height <= MAX_NESTING + 1).then(|| (shape.clone(), *height));
        }
        if !named.args.is_empty() || !nominal.active.insert(named.clone()) {
            return None;
        }
        let mut height = 1;
        let mut key = nominal.schemas.derives_key(named);
        let mut eq = nominal.schemas.derives_eq(named);
        let mut structural = true;
        let mut member = |term: &TypeTerm, nominal: &mut Nominal<'_>| {
            let (shape, h) = Self::resolve_at(term, depth + 1, Some(nominal))?;
            height = height.max(h + 1);
            key &= shape.map_key();
            eq &= shape.host_eq();
            structural &= shape.structural_eq();
            Some(shape)
        };
        let ty = TypeTerm::Named(named.clone()).to_type_expr();
        let shape = if let Some(schema) = nominal.schemas.record(named) {
            let mut fields = Vec::with_capacity(schema.fields.len());
            for field in &schema.fields {
                fields.push(FieldShape {
                    id: field.field_id.clone(),
                    shape: member(&field.ty, nominal)?,
                    optional: field.optional,
                });
            }
            Self::Record(Arc::new(RecordShape {
                ty,
                fields: fields.into(),
                key,
                eq,
                structural,
            }))
        } else {
            let schema = nominal.schemas.enumeration(named)?;
            let mut variants = Vec::with_capacity(schema.variants.len());
            for variant in &schema.variants {
                let payload = match &variant.payload {
                    None => None,
                    Some(term) => Some(member(term, nominal)?),
                };
                variants.push((variant.variant_id.clone(), payload));
            }
            Self::Enum(Arc::new(EnumShape {
                ty,
                variants: variants.into(),
                key,
                eq,
                structural,
            }))
        };
        nominal.active.remove(named);
        nominal
            .resolved
            .insert(named.clone(), (shape.clone(), height));
        Some((shape, height))
    }

    /// Whether the host implements this exact Core type's `hash`.
    pub fn native_hash(&self) -> bool {
        matches!(
            self,
            Self::Bool | Self::I32 | Self::I64 | Self::U32 | Self::U64 | Self::String | Self::Bytes
        )
    }

    /// Whether values of this type key an executable map (11 §5.2): a native Core `hash`
    /// type, or a package enum or record whose `eq` and `hash` are structural derives
    /// over such keys. Equal keys are then exactly equal tags and payloads, or equal
    /// field presence and fields, `Value`'s structural identity, so map storage and
    /// lookup run no package code.
    pub fn map_key(&self) -> bool {
        match self {
            Self::Enum(shape) => shape.key,
            Self::Record(shape) => shape.key,
            _ => self.native_hash(),
        }
    }

    /// Whether the host implements this exact Core type's total equality. Nominal record and
    /// enum equality belongs to the owning package, so it is never native.
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

    /// Whether the host executes this type's language `eq` itself (11 §5.4) at every
    /// level: native Core equality, maps keyed by [`ValueShape::map_key`]s, and package
    /// records and enums whose `eq` is a declared structural derive over such members.
    /// A package-implemented `eq` anywhere inside needs package code, so it is excluded.
    pub fn host_eq(&self) -> bool {
        self.host_eq_at(0)
    }

    fn host_eq_at(&self, depth: usize) -> bool {
        if depth > MAX_NESTING {
            return false;
        }
        match self {
            // Members were checked when the nominal shape was resolved.
            Self::Enum(shape) => shape.eq,
            Self::Record(shape) => shape.eq,
            Self::Option(inner) | Self::List(inner) => inner.host_eq_at(depth + 1),
            Self::Map(key, value) => key.map_key() && value.host_eq_at(depth + 1),
            _ => self.native_eq_at(depth),
        }
    }

    /// Whether structural `Value` identity is exactly this type's [`ValueShape::host_eq`]
    /// equality, so unequal identities are unequal values. Only maps differ: their
    /// equality ignores insertion order (§5.2), also inside records and enum payloads.
    pub fn structural_eq(&self) -> bool {
        match self {
            Self::Map(..) => false,
            Self::Option(inner) | Self::List(inner) => inner.structural_eq(),
            Self::Enum(shape) => shape.structural,
            Self::Record(shape) => shape.structural,
            _ => true,
        }
    }

    /// Whether a valid value of this type can carry the None discriminant directly.
    /// `option<T>` boxes its present value in `Some` exactly when this holds for `T` (12 §7).
    pub fn unboxed_none(&self) -> bool {
        match self {
            Self::Option(_) => true,
            Self::Result(inner) if !inner.unboxed_error() => inner.unboxed_none(),
            _ => false,
        }
    }

    /// Whether a valid value of this type can carry the Err discriminant directly.
    /// `result<T>` boxes its success in `Ok` exactly when this holds for `T` (12 §7).
    pub fn unboxed_error(&self) -> bool {
        match self {
            Self::Result(_) | Self::Error => true,
            Self::Option(inner) if !inner.unboxed_none() => inner.unboxed_error(),
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
            Self::Map(key, value) => key.map_key() && value.valid_shape(depth + 1),
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
                Value::Some(value) if inner.unboxed_none() => {
                    charge(left, VALUE_CHARGE)?;
                    inner.check(value, left, depth + 1)
                }
                _ if inner.unboxed_none() => Err(InvalidType),
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
            (Self::Map(key, ty), Value::Map(map)) if key.map_key() => {
                let memo = map.checked();
                if let Some(entries) = memo.get().and_then(|m| m.charge(key, ty, depth)) {
                    return charge(left, entries);
                }
                // Keys are unique by construction (`MapRepr::new`).
                let entries = map.entries();
                minimum(*left, entries.len(), MAP_ENTRY_CHARGE)?;
                let before = *left;
                for (k, v) in entries {
                    key.check(k, left, depth + 1)?;
                    ty.check(v, left, depth + 1)?;
                }
                // Only a complete success is remembered; a concurrent first check may win.
                if let Some(check) = MapCheck::new(key, ty, depth, before - *left) {
                    let _ = memo.set(check);
                }
                Ok(())
            }
            // Tags index the resolved schema; an unknown tag has no executable shape.
            (Self::Enum(shape), Value::Enum { tag, payload }) => {
                match (shape.payload(*tag).ok_or(InvalidType)?, payload) {
                    (None, None) => Ok(()),
                    (Some(ty), Some(value)) => ty.check(value, left, depth + 1),
                    _ => Err(InvalidType),
                }
            }
            // Fields sit at their schema index; a missing optional field still holds its
            // slot, and the value retains its canonical type text.
            (Self::Record(shape), Value::Record(record)) => {
                if record.ty != shape.ty || record.fields.len() != shape.fields.len() {
                    return Err(InvalidType);
                }
                charge(left, record.ty.as_str().len())?;
                minimum(*left, record.fields.len(), VALUE_CHARGE)?;
                for (slot, field) in record.fields.iter().zip(&shape.fields) {
                    match slot {
                        Some(value) => field.shape.check(value, left, depth + 1)?,
                        None if field.optional => charge(left, VALUE_CHARGE)?,
                        None => return Err(InvalidType),
                    }
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

    /// Checked language equality of a [`ValueShape::host_eq`] type, None for any other
    /// type or a value that does not check. Map equality ignores insertion order;
    /// iteration does not. Derived record and enum equality compares tags, then payloads,
    /// or field presence, then fields, under the same rules (11 §5.4).
    pub fn equal_values(&self, a: &Value, b: &Value) -> Option<bool> {
        if !self.host_eq() || !self.accepts(a) || !self.accepts(b) {
            return None;
        }
        Some(self.equal(a, b))
    }

    fn equal(&self, a: &Value, b: &Value) -> bool {
        match self {
            Self::Option(inner) => match (a, b) {
                (Value::None, Value::None) => true,
                (Value::None, _) | (_, Value::None) => false,
                (Value::Some(a), Value::Some(b)) if inner.unboxed_none() => inner.equal(a, b),
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
            Self::Enum(shape) if !shape.structural => {
                let (
                    Value::Enum {
                        tag: ta,
                        payload: pa,
                    },
                    Value::Enum {
                        tag: tb,
                        payload: pb,
                    },
                ) = (a, b)
                else {
                    return false;
                };
                ta == tb
                    && match (shape.payload(*ta).flatten(), pa, pb) {
                        (Some(inner), Some(a), Some(b)) => inner.equal(a, b),
                        (None, None, None) => true,
                        _ => false,
                    }
            }
            Self::Record(shape) if !shape.structural => {
                let (Value::Record(a), Value::Record(b)) = (a, b) else {
                    return false;
                };
                Arc::ptr_eq(a, b)
                    || a.fields
                        .iter()
                        .zip(b.fields.iter())
                        .zip(shape.fields.iter())
                        .all(|((a, b), field)| match (a, b) {
                            (Some(a), Some(b)) => field.shape.equal(a, b),
                            (None, None) => true,
                            _ => false,
                        })
            }
            // Structural identity is the language equality here (`structural_eq`).
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
    fn enums_resolve_through_schemas_and_check_tags_and_payloads() {
        use crate::value::literal_codec::{EnumSchema, SchemaSet, VariantSchema};
        let term = |text: &str| TypeTerm::parse(&text.parse().unwrap()).unwrap();
        let variant = |id: &str, payload: Option<&str>| VariantSchema {
            variant_id: id.into(),
            payload: payload.map(term),
        };
        let enumeration = |variants| EnumSchema { variants };
        let schemas = SchemaSet::default()
            .with_enum(
                "org.example/state@1",
                enumeration(vec![
                    variant("idle", None),
                    variant("count", Some("core/i64@1")),
                ]),
            )
            .with_enum(
                "org.example/other@1",
                enumeration(vec![
                    variant("idle", None),
                    variant("count", Some("core/i64@1")),
                ]),
            )
            .with_enum(
                "org.example/loop@1",
                enumeration(vec![variant(
                    "next",
                    Some("core/list@1<org.example/loop@1>"),
                )]),
            );
        let of = |text: &str| ValueShape::of_in(&text.parse().unwrap(), &schemas);
        // Without schemas a nominal type has no shape; with them its identity is kept.
        assert!(shape("org.example/state@1").is_none());
        let state = of("org.example/state@1").unwrap();
        assert_ne!(Some(&state), of("org.example/other@1").as_ref());
        let ValueShape::Enum(layout) = &state else {
            panic!("{state:?}");
        };
        assert_eq!((layout.len(), layout.tag("count")), (2, Some(1)));
        assert_eq!(layout.variant_id(0), Some("idle"));
        let value = |tag, payload: Option<Value>| Value::Enum {
            tag,
            payload: payload.map(Arc::new),
        };
        assert!(state.accepts(&value(0, None)));
        assert!(state.accepts(&value(1, Some(Value::I64(3)))));
        for invalid in [
            value(2, None),
            value(0, Some(Value::I64(3))),
            value(1, None),
            value(1, Some(Value::Bool(true))),
        ] {
            assert!(!state.accepts(&invalid), "{invalid:?}");
        }
        assert!(!state.native_eq() && !state.native_hash());
        // Option over an enum stays unboxed: None is not a variant.
        let optional = of("core/option@1<org.example/state@1>").unwrap();
        assert!(optional.accepts(&Value::None) && optional.accepts(&value(0, None)));
        for unsupported in [
            "org.example/loop@1",
            "org.example/missing@1",
            "core/map@1<org.example/state@1,core/i64@1>",
        ] {
            assert!(of(unsupported).is_none(), "{unsupported}");
        }
        // A memoized enum reused deeper still respects the nesting limit: `state` has
        // height 2, first resolved at depth 1, then reused below `n` options.
        let wrap = |n| {
            format!(
                "{}org.example/state@1{}",
                "core/option@1<".repeat(n),
                ">".repeat(n)
            )
        };
        for (n, fits) in [(MAX_NESTING - 2, true), (MAX_NESTING - 1, false)] {
            let pair = enumeration(vec![
                variant("near", Some("org.example/state@1")),
                variant("far", Some(wrap(n).as_str())),
            ]);
            let schemas = schemas.clone().with_enum("org.example/pair@1", pair);
            let pair = ValueShape::of_in(&"org.example/pair@1".parse().unwrap(), &schemas);
            assert_eq!(pair.is_some(), fits, "{n}");
        }
        assert!(of(&wrap(MAX_NESTING - 1)).is_some());
        assert!(of(&wrap(MAX_NESTING)).is_none());
    }

    #[test]
    fn records_resolve_through_schemas_and_check_fields_by_index() {
        use crate::value::RecordRepr;
        use crate::value::literal_codec::{
            EnumSchema, FieldSchema, RecordSchema, SchemaSet, VariantSchema,
        };
        let term = |text: &str| TypeTerm::parse(&text.parse().unwrap()).unwrap();
        let field = |id: &str, ty: &str, optional| FieldSchema {
            field_id: id.into(),
            ty: term(ty),
            optional,
        };
        let record = |fields| RecordSchema { fields };
        let point = "org.example/point@1";
        let schemas = SchemaSet::default()
            .with_record(
                point,
                record(vec![
                    field("x", "core/i64@1", false),
                    field("label", "core/string@1", true),
                ]),
            )
            .with_record(
                "org.example/other@1",
                record(vec![
                    field("x", "core/i64@1", false),
                    field("label", "core/string@1", true),
                ]),
            )
            .with_record(
                "org.example/link@1",
                record(vec![field("next", "org.example/link@1", true)]),
            )
            .with_record(
                "org.example/ring@1",
                record(vec![field("via", "org.example/turn@1", false)]),
            )
            .with_enum(
                "org.example/turn@1",
                EnumSchema {
                    variants: vec![VariantSchema {
                        variant_id: "back".into(),
                        payload: Some(term("org.example/ring@1")),
                    }],
                },
            )
            .with_enum(
                "org.example/shape@1",
                EnumSchema {
                    variants: vec![VariantSchema {
                        variant_id: "dot".into(),
                        payload: Some(term(point)),
                    }],
                },
            );
        let of = |text: &str| ValueShape::of_in(&text.parse().unwrap(), &schemas);
        assert!(shape(point).is_none());
        let shape = of(point).unwrap();
        assert_ne!(Some(&shape), of("org.example/other@1").as_ref());
        let ValueShape::Record(layout) = &shape else {
            panic!("{shape:?}");
        };
        assert_eq!((layout.len(), layout.index("label")), (2, Some(1)));
        assert_eq!(layout.field_id(0), Some("x"));
        assert_eq!(layout.field(1), Some((&ValueShape::String, true)));
        assert!(!shape.native_eq() && !shape.native_hash() && !shape.map_key());
        let value = |ty: &str, fields: Vec<Option<Value>>| {
            Value::Record(Arc::new(RecordRepr {
                ty: ty.parse().unwrap(),
                fields: fields.into(),
            }))
        };
        let label = || Some(Value::Str(Arc::new("ab".into())));
        let full = value(point, vec![Some(Value::I64(1)), label()]);
        let missing = value(point, vec![Some(Value::I64(1)), None]);
        // One value, the type text, then each slot: its value or an absent slot.
        let base = VALUE_CHARGE + point.len() + VALUE_CHARGE;
        assert_eq!(
            shape.checked_charge(&full, usize::MAX),
            Ok(base + VALUE_CHARGE + 2)
        );
        assert_eq!(
            shape.checked_charge(&missing, usize::MAX),
            Ok(base + VALUE_CHARGE)
        );
        assert_eq!(shape.checked_charge(&full, base), Err(ResourceLimit));
        for invalid in [
            value(point, vec![None, label()]),
            value(point, vec![Some(Value::I64(1))]),
            value(point, vec![Some(Value::Bool(true)), None]),
            value("org.example/other@1", vec![Some(Value::I64(1)), None]),
            Value::Enum {
                tag: 0,
                payload: None,
            },
        ] {
            assert_eq!(
                shape.checked_charge(&invalid, usize::MAX),
                Err(InvalidType),
                "{invalid:?}"
            );
        }
        // Records nest in enums and containers; an option over one stays unboxed.
        assert!(of("org.example/shape@1").unwrap().accepts(&Value::Enum {
            tag: 0,
            payload: Some(Arc::new(missing.clone())),
        }));
        let optional = of(&format!("core/option@1<{point}>")).unwrap();
        assert!(optional.accepts(&Value::None) && optional.accepts(&missing));
        // Direct and indirect recursion, unknown types and float keys have no shape.
        for unsupported in [
            "org.example/link@1",
            "org.example/ring@1",
            "org.example/missing@1",
            &format!("core/map@1<{point},core/i64@1>"),
        ] {
            assert!(of(unsupported).is_none(), "{unsupported}");
        }
        // A record reused deeper still respects the nesting limit (height 2).
        let wrap = |n| format!("{}{point}{}", "core/list@1<".repeat(n), ">".repeat(n));
        assert!(of(&wrap(MAX_NESTING - 1)).is_some());
        assert!(of(&wrap(MAX_NESTING)).is_none());
    }

    /// Derived record and enum `eq` is host equality only when every member's is too;
    /// it compares tags or field presence, then members, with maps ignoring order.
    #[test]
    fn derived_eq_records_and_enums_compare_members_by_language_equality() {
        use crate::value::literal_codec::{
            EnumSchema, FieldSchema, RecordSchema, SchemaSet, VariantSchema,
        };
        use crate::value::{MapRepr, RecordRepr};
        struct Derived(SchemaSet, &'static [&'static str]);
        impl Schemas for Derived {
            fn record(&self, ty: &NamedTerm) -> Option<&RecordSchema> {
                self.0.record(ty)
            }
            fn enumeration(&self, ty: &NamedTerm) -> Option<&EnumSchema> {
                self.0.enumeration(ty)
            }
            fn derives_eq(&self, ty: &NamedTerm) -> bool {
                self.1.contains(&ty.name.as_str())
            }
        }
        let term = |text: &str| TypeTerm::parse(&text.parse().unwrap()).unwrap();
        let variant = |id: &str, payload: Option<&str>| VariantSchema {
            variant_id: id.into(),
            payload: payload.map(term),
        };
        let ints = "core/map@1<core/string@1,core/i64@1>";
        let set = SchemaSet::default()
            .with_enum(
                "org.example/bag@1",
                EnumSchema {
                    variants: vec![variant("empty", None), variant("items", Some(ints))],
                },
            )
            .with_enum(
                "org.example/flat@1",
                EnumSchema {
                    variants: vec![variant("n", Some("core/list@1<core/i64@1>"))],
                },
            )
            .with_enum(
                "org.example/floaty@1",
                EnumSchema {
                    variants: vec![variant("x", Some("core/f64@1"))],
                },
            )
            .with_record(
                "org.example/holder@1",
                RecordSchema {
                    fields: vec![
                        FieldSchema {
                            field_id: "bag".into(),
                            ty: term("org.example/bag@1"),
                            optional: false,
                        },
                        FieldSchema {
                            field_id: "note".into(),
                            ty: term("core/string@1"),
                            optional: true,
                        },
                    ],
                },
            );
        let all = Derived(set.clone(), &["bag", "flat", "floaty", "holder"]);
        let of = |schemas: &Derived, text: &str| {
            ValueShape::of_in(&text.parse().unwrap(), schemas).unwrap()
        };
        let bag = of(&all, "org.example/bag@1");
        let holder = of(&all, "org.example/holder@1");
        let flat = of(&all, "org.example/flat@1");
        assert!(bag.host_eq() && holder.host_eq() && flat.host_eq());
        assert!(!bag.native_eq() && !holder.native_eq());
        // Only a map anywhere inside makes identity differ from equality.
        assert!(!bag.structural_eq() && !holder.structural_eq() && flat.structural_eq());
        // A float payload has no host equality, whatever the derive claims.
        assert!(!of(&all, "org.example/floaty@1").host_eq());
        // A derive on the outer type does not cover a member without one.
        let outer_only = Derived(set.clone(), &["holder"]);
        assert!(!of(&outer_only, "org.example/holder@1").host_eq());
        assert!(!of(&Derived(set, &[]), "org.example/bag@1").host_eq());

        let map = |entries: &[(&str, i64)]| {
            Value::Map(Arc::new(
                MapRepr::new(
                    entries
                        .iter()
                        .map(|(k, v)| (Value::Str(Arc::new((*k).into())), Value::I64(*v)))
                        .collect::<Vec<_>>(),
                )
                .unwrap(),
            ))
        };
        let items = |m| Value::Enum {
            tag: 1,
            payload: Some(Arc::new(m)),
        };
        let empty = Value::Enum {
            tag: 0,
            payload: None,
        };
        let (ab, ba) = (map(&[("a", 1), ("b", 2)]), map(&[("b", 2), ("a", 1)]));
        assert_ne!(items(ab.clone()), items(ba.clone()));
        assert_eq!(
            bag.equal_values(&items(ab.clone()), &items(ba.clone())),
            Some(true)
        );
        assert_eq!(
            bag.equal_values(&items(ab.clone()), &items(map(&[("a", 1)]))),
            Some(false)
        );
        assert_eq!(bag.equal_values(&items(map(&[])), &empty), Some(false));
        assert_eq!(bag.equal_values(&empty, &empty), Some(true));
        // A value outside the shape is no comparison at all.
        let unknown = Value::Enum {
            tag: 2,
            payload: None,
        };
        assert_eq!(bag.equal_values(&empty, &unknown), None);
        let held = |b, note: Option<&str>| {
            Value::Record(Arc::new(RecordRepr {
                ty: "org.example/holder@1".parse().unwrap(),
                fields: Box::new([Some(b), note.map(|n| Value::Str(Arc::new(n.into())))]),
            }))
        };
        assert_eq!(
            holder.equal_values(&held(items(ab.clone()), None), &held(items(ba), None)),
            Some(true)
        );
        assert_eq!(
            holder.equal_values(&held(empty.clone(), None), &held(empty.clone(), Some(""))),
            Some(false)
        );
        assert_eq!(
            holder.equal_values(&held(empty.clone(), Some("x")), &held(empty, Some("x"))),
            Some(true)
        );
    }

    /// A record keys maps when its `eq` and `hash` derive over key fields; optional
    /// fields keep presence in the key's identity.
    #[test]
    fn derived_key_records_key_maps() {
        use crate::value::literal_codec::{EnumSchema, FieldSchema, RecordSchema, SchemaSet};
        use crate::value::{MapRepr, RecordRepr};
        struct Derived(SchemaSet);
        impl Schemas for Derived {
            fn record(&self, ty: &NamedTerm) -> Option<&RecordSchema> {
                self.0.record(ty)
            }
            fn enumeration(&self, ty: &NamedTerm) -> Option<&EnumSchema> {
                self.0.enumeration(ty)
            }
            fn derives_key(&self, _: &NamedTerm) -> bool {
                true
            }
        }
        let field = |id: &str, ty: &str, optional| FieldSchema {
            field_id: id.into(),
            ty: TypeTerm::parse(&ty.parse().unwrap()).unwrap(),
            optional,
        };
        let schemas = Derived(
            SchemaSet::default()
                .with_record(
                    "org.example/key@1",
                    RecordSchema {
                        fields: vec![
                            field("id", "core/u32@1", false),
                            field("tag", "core/string@1", true),
                        ],
                    },
                )
                .with_record(
                    "org.example/listed@1",
                    RecordSchema {
                        fields: vec![field("ids", "core/list@1<core/u32@1>", false)],
                    },
                ),
        );
        let of = |key: &str| {
            ValueShape::of_in(
                &format!("core/map@1<{key},core/bool@1>").parse().unwrap(),
                &schemas,
            )
        };
        assert!(of("org.example/listed@1").is_none());
        let map = of("org.example/key@1").unwrap();
        let key = |tag: Option<&str>| {
            Value::Record(Arc::new(RecordRepr {
                ty: "org.example/key@1".parse().unwrap(),
                fields: Box::new([
                    Some(Value::U32(1)),
                    tag.map(|t| Value::Str(Arc::new(t.into()))),
                ]),
            }))
        };
        let entries = MapRepr::new(vec![
            (key(None), Value::Bool(true)),
            (key(Some("")), Value::Bool(false)),
        ])
        .unwrap();
        assert!(map.accepts(&Value::Map(Arc::new(entries))));
    }

    /// An enum keys maps only when its schemas prove derived `eq` + `hash` for it and
    /// for every enum among its payloads; keys then check tags and payloads.
    #[test]
    fn derived_key_enums_key_maps_and_others_do_not() {
        use crate::value::MapRepr;
        use crate::value::literal_codec::{EnumSchema, RecordSchema, SchemaSet, VariantSchema};
        struct Derived(SchemaSet, &'static [&'static str]);
        impl Schemas for Derived {
            fn record(&self, ty: &NamedTerm) -> Option<&RecordSchema> {
                self.0.record(ty)
            }
            fn enumeration(&self, ty: &NamedTerm) -> Option<&EnumSchema> {
                self.0.enumeration(ty)
            }
            fn derives_key(&self, ty: &NamedTerm) -> bool {
                self.1.contains(&ty.name.as_str())
            }
        }
        let term = |text: &str| TypeTerm::parse(&text.parse().unwrap()).unwrap();
        let variant = |id: &str, payload: Option<&str>| VariantSchema {
            variant_id: id.into(),
            payload: payload.map(term),
        };
        let set = SchemaSet::default()
            .with_enum(
                "org.example/key@1",
                EnumSchema {
                    variants: vec![variant("a", None), variant("b", Some("core/string@1"))],
                },
            )
            .with_enum(
                "org.example/outer@1",
                EnumSchema {
                    variants: vec![variant("wrap", Some("org.example/key@1"))],
                },
            )
            .with_enum(
                "org.example/listed@1",
                EnumSchema {
                    variants: vec![variant("many", Some("core/list@1<core/i64@1>"))],
                },
            );
        let map = |key: &str| format!("core/map@1<{key},core/u32@1>");
        let of = |schemas: &Derived, text: &str| ValueShape::of_in(&text.parse().unwrap(), schemas);
        let all = Derived(set.clone(), &["key", "outer", "listed"]);
        let keyed = of(&all, &map("org.example/key@1")).unwrap();
        assert!(of(&all, &map("org.example/outer@1")).is_some());
        // A list payload is no map key, whatever the schemas claim.
        assert!(of(&all, &map("org.example/listed@1")).is_none());
        // An outer derive does not cover a payload enum without one.
        let outer_only = Derived(set.clone(), &["outer"]);
        assert!(of(&outer_only, &map("org.example/outer@1")).is_none());
        assert!(of(&outer_only, &map("org.example/key@1")).is_none());
        let ValueShape::Map(key, _) = &keyed else {
            panic!("{keyed:?}");
        };
        assert!(key.map_key() && !key.native_hash() && !key.native_eq());
        assert!(!keyed.native_eq());
        let b = |text: &str| Value::Enum {
            tag: 1,
            payload: Some(Arc::new(Value::Str(Arc::new(text.into())))),
        };
        let a = Value::Enum {
            tag: 0,
            payload: None,
        };
        let entries = |keys: Vec<Value>| {
            Value::Map(Arc::new(
                MapRepr::new(
                    keys.into_iter()
                        .map(|k| (k, Value::U32(1)))
                        .collect::<Vec<_>>(),
                )
                .unwrap(),
            ))
        };
        assert!(keyed.accepts(&entries(vec![a.clone(), b("x"), b("y")])));
        assert_eq!(
            MapRepr::new(vec![(b("x"), Value::U32(1)), (b("x"), Value::U32(2))]),
            Err(crate::value::DuplicateKey)
        );
        let unknown = Value::Enum {
            tag: 2,
            payload: None,
        };
        assert!(!keyed.accepts(&entries(vec![a, unknown])));
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
    fn option_boxes_when_an_unboxed_result_success_can_be_none() {
        let some = |v: Value| Value::Some(Arc::new(v));
        // option<result<option<i64>>>: the result's success hides no tag of its own, so
        // its None reaches the option layer and Some(Ok(None)) must stay distinguishable.
        let ty = ValueShape::Option(Box::new(ValueShape::Result(Box::new(ValueShape::Option(
            Box::new(ValueShape::I64),
        )))));
        assert!(ty.accepts(&Value::None)); // None
        assert!(ty.accepts(&some(Value::None))); // Some(Ok(None))
        assert!(ty.accepts(&some(Value::I64(5)))); // Some(Ok(Some(5)))
        assert!(ty.accepts(&some(err("core/Test")))); // Some(Err)
        assert!(!ty.accepts(&Value::I64(5)));
        assert!(!ty.accepts(&err("core/Test")));
        assert!(!ty.unboxed_error());
        let wrapped = ValueShape::Result(Box::new(ty.clone()));
        assert!(!wrapped.accepts(&Value::Ok(Arc::new(Value::None))));
        assert!(wrapped.accepts(&some(err("core/Test")))); // Ok(Some(Err))
        // A boxed result success hides the payload's None: no extra option box.
        let success = ValueShape::Result(Box::new(ValueShape::Result(Box::new(
            ValueShape::Option(Box::new(ValueShape::I64)),
        ))));
        assert!(!success.unboxed_none());
        let boxed = ValueShape::Option(Box::new(success));
        assert!(boxed.accepts(&Value::Ok(Arc::new(Value::None))));
        assert!(!boxed.accepts(&some(Value::Ok(Arc::new(Value::None)))));
    }

    /// Every option/result layering up to depth 6 over i64: each semantic value has one
    /// accepted compact encoding, and distinct semantic values never share one.
    #[test]
    fn option_result_layerings_encode_injectively() {
        // Independent of `unboxed_*`: a layer wraps when its payload can already show the
        // layer's own bare tag. Tags: 1 = None, 2 = Err, 4 = anything else.
        fn bare(layers: &[bool]) -> u8 {
            let Some((&option, rest)) = layers.split_first() else {
                return 4;
            };
            let (own, inner) = (if option { 1 } else { 2 }, bare(rest));
            if inner & own != 0 {
                own | 4
            } else {
                own | inner
            }
        }
        // All semantic values with one leaf and one error, encoded compactly.
        fn values(layers: &[bool]) -> Vec<(String, Value)> {
            let Some((&option, rest)) = layers.split_first() else {
                return vec![("7".into(), Value::I64(7))];
            };
            let wrap = bare(rest) & if option { 1 } else { 2 } != 0;
            let (empty, label) = if option {
                (Value::None, "Some")
            } else {
                (err("core/Test"), "Ok")
            };
            let mut out = vec![(if option { "None" } else { "Err" }.into(), empty)];
            for (text, value) in values(rest) {
                let value = match (wrap, option) {
                    (false, _) => value,
                    (true, true) => Value::Some(Arc::new(value)),
                    (true, false) => Value::Ok(Arc::new(value)),
                };
                out.push((format!("{label}({text})"), value));
            }
            out
        }
        for depth in 1..=6 {
            for bits in 0..1u32 << depth {
                let layers: Vec<bool> = (0..depth).map(|i| bits >> i & 1 == 1).collect();
                let mut ty = ValueShape::I64;
                for &option in layers.iter().rev() {
                    ty = if option {
                        ValueShape::Option(Box::new(ty))
                    } else {
                        ValueShape::Result(Box::new(ty))
                    };
                }
                let all = values(&layers);
                for (i, (text, value)) in all.iter().enumerate() {
                    assert!(ty.accepts(value), "{ty:?} rejects {text}");
                    for (other, encoded) in &all[..i] {
                        assert_ne!(value, encoded, "{ty:?}: {text} collides with {other}");
                    }
                }
            }
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

    /// `MapRepr` is untyped storage and accepts a float key; the typed shape check refuses it.
    #[test]
    fn float_map_keys_are_refused_at_the_shape_boundary() {
        let floats = MapRepr::new(vec![(Value::F64(1.0), Value::U64(1))]).unwrap();
        let value = Value::Map(Arc::new(floats));
        for key in [ValueShape::U64, ValueShape::F64] {
            let ty = ValueShape::Map(Box::new(key), Box::new(ValueShape::U64));
            assert!(ty.checked_charge(&value, usize::MAX).is_err(), "{ty:?}");
        }
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
    fn a_map_remembers_only_complete_checks_for_its_exact_shapes() {
        use std::hash::BuildHasher;
        let ty = ValueShape::Map(Box::new(ValueShape::String), Box::new(ValueShape::U64));
        let entries = || {
            (0..3u64)
                .map(|i| (Value::Str(Arc::new(format!("k{i}"))), Value::U64(i)))
                .collect::<Vec<_>>()
        };
        let repr = Arc::new(MapRepr::new(entries()).unwrap());
        let map = Value::Map(repr.clone());
        let full = 3 * (VALUE_CHARGE + 2 + VALUE_CHARGE) + VALUE_CHARGE;
        // A refused check leaves nothing behind; the next complete one is remembered.
        assert_eq!(ty.checked_charge(&map, full - 1), Err(ResourceLimit));
        assert!(repr.checked().get().is_none());
        assert_eq!(ty.checked_charge(&map, usize::MAX), Ok(full));
        assert!(repr.checked().get().is_some());
        // The remembered charge still answers to every later limit.
        assert_eq!(ty.checked_charge(&map, full), Ok(full));
        assert_eq!(ty.checked_charge(&map, full - 1), Err(ResourceLimit));
        // Other shapes walk the entries again and reach their own verdict.
        let other = ValueShape::Map(Box::new(ValueShape::String), Box::new(ValueShape::I64));
        assert_eq!(other.checked_charge(&map, usize::MAX), Err(InvalidType));
        let wrapped = ValueShape::Option(Box::new(ty.clone()));
        assert_eq!(wrapped.checked_charge(&map, usize::MAX), Ok(full));
        // Verdicts only tighten with depth, so a deeper check is never answered from memo.
        let memo = repr.checked().get().unwrap();
        let (key, value) = (&ValueShape::String, &ValueShape::U64);
        assert_eq!(memo.charge(key, value, 0), Some(full - VALUE_CHARGE));
        assert_eq!(memo.charge(key, value, 1), None);
        // Codes are exact: rearranged or re-nested shapes never share one.
        let list = |s| ValueShape::List(Box::new(s));
        let option = |s| ValueShape::Option(Box::new(s));
        let shapes = [
            (list(ValueShape::U64), ValueShape::U64),
            (ValueShape::U64, list(ValueShape::U64)),
            (option(list(ValueShape::U64)), ValueShape::Bool),
            (list(option(ValueShape::U64)), ValueShape::Bool),
            (list(ValueShape::U64), option(ValueShape::Bool)),
        ];
        let codes: HashSet<_> = shapes
            .iter()
            .map(|(k, v)| MapCheck::code(k, v, 0).unwrap())
            .collect();
        assert_eq!(codes.len(), shapes.len());
        // Shapes too large to code are walked every time, never misremembered.
        let mut deep = ValueShape::U64;
        for _ in 0..11 {
            deep = list(deep);
        }
        assert_eq!(MapCheck::code(&ValueShape::String, &deep, 0), None);
        let nested = Value::Map(Arc::new(
            MapRepr::new(vec![(
                Value::Str(Arc::new("k".into())),
                Value::List(Arc::new(ListRepr::Values(vec![].into()))),
            )])
            .unwrap(),
        ));
        let wide = ValueShape::Map(Box::new(ValueShape::String), Box::new(deep));
        assert!(wide.accepts(&nested) && wide.accepts(&nested));
        // The memo is derived: equality and hashing see only the entries.
        let fresh = MapRepr::new(entries()).unwrap();
        let hasher = std::hash::RandomState::new();
        assert_eq!(*repr, fresh);
        assert_eq!(hasher.hash_one(&*repr), hasher.hash_one(&fresh));
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
