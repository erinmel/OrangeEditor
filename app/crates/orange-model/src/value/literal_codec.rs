//! Typed literal codec (12 §6.1): [`Literal`] ↔ [`Value`] for every type with a literal
//! form.
//!
//! Only the top-level literal carries `type`; nested values are untyped JSON whose types
//! come from the outer type. Nested floats use the same lowercase-hex bit-pattern strings
//! as top-level `bits`. Decimal strings and bit patterns must already be canonical, so
//! load→save stays byte-stable.
//!
//! Unknown record fields and enum variants survive storage because the document keeps the
//! raw literal; only decoding them to a `Value` fails (03).

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use serde_json::{Map, Value as Json, json};

use super::base64;
use super::{JsonText, ListRepr, MapRepr, RecordRepr, Value};
use crate::document::literal::{Literal, LiteralPayload};
use crate::error::{JsonTextError, LiteralError};
use crate::id::TypeExpr;
use crate::type_term::{MAX_NESTING, NamedTerm, TypeTerm};

/// Record and enum layouts by nominal type, supplied by the registry (12 §5).
pub trait Schemas {
    fn record(&self, ty: &NamedTerm) -> Option<&RecordSchema>;
    fn enumeration(&self, ty: &NamedTerm) -> Option<&EnumSchema>;

    /// Validate a map key before codec duplicate detection. The default preserves the
    /// codec-only `SchemaSet` compatibility behavior (float exclusion); registries should
    /// override this to enforce language `hash` and `eq` constraints.
    fn validate_map_key(&self, term: &TypeTerm) -> Result<(), LiteralError> {
        if float_width(term).is_some() {
            Err(LiteralError::FloatMapKey {
                ty: term.to_string(),
            })
        } else {
            Ok(())
        }
    }
}

/// Field IDs in schema order (12 §5).
#[derive(Debug, Clone, PartialEq)]
pub struct RecordSchema {
    pub fields: Vec<FieldSchema>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldSchema {
    pub field_id: String,
    pub ty: TypeTerm,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumSchema {
    pub variants: Vec<VariantSchema>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantSchema {
    pub variant_id: String,
    pub payload: Option<TypeTerm>,
}

/// In-memory schemas keyed by canonical named type (`org.example/point@1`).
#[derive(Debug, Clone, Default)]
pub struct SchemaSet {
    records: HashMap<String, RecordSchema>,
    enums: HashMap<String, EnumSchema>,
}

impl SchemaSet {
    pub fn with_record(mut self, ty: &str, schema: RecordSchema) -> Self {
        self.records.insert(ty.to_owned(), schema);
        self
    }

    pub fn with_enum(mut self, ty: &str, schema: EnumSchema) -> Self {
        self.enums.insert(ty.to_owned(), schema);
        self
    }
}

impl Schemas for SchemaSet {
    fn record(&self, ty: &NamedTerm) -> Option<&RecordSchema> {
        self.records.get(&TypeTerm::Named(ty.clone()).to_string())
    }

    fn enumeration(&self, ty: &NamedTerm) -> Option<&EnumSchema> {
        self.enums.get(&TypeTerm::Named(ty.clone()).to_string())
    }
}

/// Decodes a literal into the value it denotes.
pub fn decode(literal: &Literal, schemas: &impl Schemas) -> Result<Value, LiteralError> {
    let term = TypeTerm::parse(&literal.ty)?;
    match &literal.payload {
        LiteralPayload::Bits(bits) => match float_width(&term) {
            Some(width) => float_from_bits(bits, width, &term),
            None => Err(mismatch(&term)),
        },
        LiteralPayload::Value(_) if float_width(&term).is_some() => Err(mismatch(&term)),
        LiteralPayload::Value(json) => Codec { schemas }.decode(json, &term, 0),
    }
}

/// Encodes `value` as a literal of type `ty`; fails if the value does not have that type.
pub fn encode(
    ty: &TypeExpr,
    value: &Value,
    schemas: &impl Schemas,
) -> Result<Literal, LiteralError> {
    let term = TypeTerm::parse(ty)?;
    let payload = match (float_width(&term), value) {
        (Some(32), Value::F32(x)) => LiteralPayload::Bits(format!("{:08x}", x.to_bits())),
        (Some(64), Value::F64(x)) => LiteralPayload::Bits(format!("{:016x}", x.to_bits())),
        (Some(_), _) => return Err(value_mismatch(&term)),
        (None, _) => LiteralPayload::Value(Codec { schemas }.encode(value, &term, 0)?),
    };
    Ok(Literal {
        ty: ty.clone(),
        payload,
        display: None,
    })
}

fn float_width(term: &TypeTerm) -> Option<u32> {
    if term.core("f32").is_some_and(<[_]>::is_empty) {
        Some(32)
    } else if term.core("f64").is_some_and(<[_]>::is_empty) {
        Some(64)
    } else {
        None
    }
}

fn float_from_bits(bits: &str, width: u32, term: &TypeTerm) -> Result<Value, LiteralError> {
    let digits = width as usize / 4;
    let canonical = bits.len() == digits
        && bits
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !canonical {
        return Err(LiteralError::NonCanonical {
            ty: term.to_string(),
        });
    }
    let raw = u64::from_str_radix(bits, 16).map_err(|_| LiteralError::NonCanonical {
        ty: term.to_string(),
    })?;
    Ok(if width == 32 {
        Value::F32(f32::from_bits(raw as u32))
    } else {
        Value::F64(f64::from_bits(raw))
    })
}

fn mismatch(term: &TypeTerm) -> LiteralError {
    LiteralError::TypeMismatch {
        ty: term.to_string(),
    }
}

fn value_mismatch(term: &TypeTerm) -> LiteralError {
    LiteralError::ValueMismatch {
        ty: term.to_string(),
    }
}

/// Exactly `N` type arguments, or the type is malformed for its constructor.
fn args<const N: usize>(term: &TypeTerm, args: &[TypeTerm]) -> Result<[TypeTerm; N], LiteralError> {
    <[TypeTerm; N]>::try_from(args.to_vec()).map_err(|_| LiteralError::UnknownType {
        ty: term.to_string(),
    })
}

/// Canonical decimal string of an integer type.
fn decimal<T: FromStr + ToString>(json: &Json, term: &TypeTerm) -> Result<T, LiteralError> {
    let text = json.as_str().ok_or_else(|| mismatch(term))?;
    let numeric = !text.is_empty()
        && text
            .trim_start_matches('-')
            .bytes()
            .all(|b| b.is_ascii_digit());
    match text.parse::<T>() {
        Ok(value) if value.to_string() == text => Ok(value),
        Ok(_) => Err(LiteralError::NonCanonical {
            ty: term.to_string(),
        }),
        Err(_) if numeric => Err(LiteralError::OutOfRange {
            ty: term.to_string(),
        }),
        Err(_) => Err(LiteralError::NonCanonical {
            ty: term.to_string(),
        }),
    }
}

/// JSON integer of a 32-bit type.
fn integer<T: TryFrom<i64>>(json: &Json, term: &TypeTerm) -> Result<T, LiteralError> {
    let number = json
        .as_number()
        .filter(|n| n.is_i64() || n.is_u64())
        .ok_or_else(|| mismatch(term))?;
    let out_of_range = || LiteralError::OutOfRange {
        ty: term.to_string(),
    };
    let wide = number.as_i64().ok_or_else(out_of_range)?;
    T::try_from(wide).map_err(|_| out_of_range())
}

struct Codec<'a, S> {
    schemas: &'a S,
}

impl<S: Schemas> Codec<'_, S> {
    fn decode(&self, json: &Json, term: &TypeTerm, depth: usize) -> Result<Value, LiteralError> {
        if depth > MAX_NESTING {
            return Err(LiteralError::TooDeep);
        }
        let named = match term {
            TypeTerm::Param(_) => {
                return Err(LiteralError::UnresolvedParameter {
                    ty: term.to_string(),
                });
            }
            TypeTerm::Named(named) => named,
        };
        if named.namespace != "core" || named.major != 1 {
            return self.decode_schema(json, term, named, depth);
        }
        let string = || json.as_str().ok_or_else(|| mismatch(term));
        Ok(match (named.name.as_str(), named.args.as_slice()) {
            ("bool", []) => Value::Bool(json.as_bool().ok_or_else(|| mismatch(term))?),
            ("i32", []) => Value::I32(integer(json, term)?),
            ("u32", []) => Value::U32(integer(json, term)?),
            ("i64", []) => Value::I64(decimal(json, term)?),
            ("u64", []) => Value::U64(decimal(json, term)?),
            ("duration", []) => Value::Duration(decimal(json, term)?),
            ("timestamp", []) => Value::Timestamp(decimal(json, term)?),
            ("f32", []) => float_from_bits(string()?, 32, term)?,
            ("f64", []) => float_from_bits(string()?, 64, term)?,
            ("string", []) => Value::Str(Arc::new(string()?.to_owned())),
            ("bytes", []) => {
                let bytes = base64::decode(string()?).ok_or(LiteralError::NonCanonical {
                    ty: term.to_string(),
                })?;
                Value::Bytes(Arc::new(bytes))
            }
            ("json", []) => {
                let text = JsonText::new(string()?.to_owned()).map_err(|error| match error {
                    JsonTextError::TooDeep => LiteralError::TooDeep,
                    JsonTextError::TooLarge { .. } => LiteralError::TooLarge {
                        ty: term.to_string(),
                    },
                    JsonTextError::Invalid(_) => mismatch(term),
                })?;
                Value::Json(Arc::new(text))
            }
            ("secret-ref", []) => {
                let reference = string()?;
                if !reference.starts_with("credential://") {
                    return Err(LiteralError::InvalidSecretRef);
                }
                Value::Str(Arc::new(reference.to_owned()))
            }
            ("list", items) => {
                let [element] = args(term, items)?;
                let array = json.as_array().ok_or_else(|| mismatch(term))?;
                let values = array
                    .iter()
                    .map(|item| self.decode(item, &element, depth + 1));
                Value::List(Arc::new(pack(&element, values.collect::<Result<_, _>>()?)))
            }
            ("map", items) => {
                let [key, value] = args(term, items)?;
                self.schemas.validate_map_key(&key)?;
                let pairs = json.as_array().ok_or_else(|| mismatch(term))?;
                let mut entries: Vec<(Value, Value)> = Vec::with_capacity(pairs.len());
                let mut keys = HashSet::with_capacity(pairs.len());
                for pair in pairs {
                    let [k, v] = pair
                        .as_array()
                        .map(Vec::as_slice)
                        .ok_or_else(|| mismatch(term))?
                    else {
                        return Err(mismatch(term));
                    };
                    let k = self.decode(k, &key, depth + 1)?;
                    if !keys.insert(k.clone()) {
                        return Err(LiteralError::DuplicateMapKey);
                    }
                    entries.push((k, self.decode(v, &value, depth + 1)?));
                }
                Value::Map(Arc::new(MapRepr {
                    entries: entries.into(),
                }))
            }
            ("option", items) => {
                let [inner] = args(term, items)?;
                let object = json
                    .as_object()
                    .filter(|o| o.len() == 1)
                    .ok_or_else(|| mismatch(term))?;
                match (object.get("none"), object.get("some")) {
                    (Some(Json::Bool(true)), None) => Value::None,
                    (None, Some(some)) => {
                        let value = self.decode(some, &inner, depth + 1)?;
                        // Only a nested option boxes its inner layer (12 §7).
                        if inner.core("option").is_some() {
                            Value::Some(Arc::new(value))
                        } else {
                            value
                        }
                    }
                    _ => return Err(mismatch(term)),
                }
            }
            ("result" | "error" | "stream" | "handle", _) => {
                return Err(LiteralError::NoLiteralForm {
                    ty: term.to_string(),
                });
            }
            _ => {
                return Err(LiteralError::UnknownType {
                    ty: term.to_string(),
                });
            }
        })
    }

    fn decode_schema(
        &self,
        json: &Json,
        term: &TypeTerm,
        named: &NamedTerm,
        depth: usize,
    ) -> Result<Value, LiteralError> {
        let object = json.as_object().ok_or_else(|| mismatch(term))?;
        if let Some(schema) = self.schemas.record(named) {
            if let Some(field) = object
                .keys()
                .find(|key| !schema.fields.iter().any(|f| &f.field_id == *key))
            {
                return Err(LiteralError::UnknownField {
                    field: field.clone(),
                });
            }
            let fields = schema
                .fields
                .iter()
                .map(|field| match object.get(&field.field_id) {
                    Some(value) => self.decode(value, &field.ty, depth + 1).map(Some),
                    None if field.optional => Ok(None),
                    None => Err(LiteralError::MissingField {
                        field: field.field_id.clone(),
                    }),
                })
                .collect::<Result<_, _>>()?;
            let ty = term
                .to_string()
                .parse()
                .expect("a parsed type re-displays as a non-blank expression");
            return Ok(Value::Record(Arc::new(RecordRepr { ty, fields })));
        }
        if let Some(schema) = self.schemas.enumeration(named) {
            let variant = object
                .get("variant")
                .and_then(Json::as_str)
                .ok_or_else(|| mismatch(term))?;
            let (tag, declared) = schema
                .variants
                .iter()
                .enumerate()
                .find(|(_, v)| v.variant_id == variant)
                .ok_or_else(|| LiteralError::UnknownVariant {
                    variant: variant.to_owned(),
                })?;
            let expected_keys = 1 + usize::from(declared.payload.is_some());
            if object.len() != expected_keys {
                return Err(mismatch(term));
            }
            let payload = match &declared.payload {
                Some(payload_ty) => {
                    let json = object.get("payload").ok_or_else(|| mismatch(term))?;
                    Some(Arc::new(self.decode(json, payload_ty, depth + 1)?))
                }
                None => None,
            };
            return Ok(Value::Enum {
                tag: tag as u32,
                payload,
            });
        }
        Err(LiteralError::UnknownType {
            ty: term.to_string(),
        })
    }

    fn encode(&self, value: &Value, term: &TypeTerm, depth: usize) -> Result<Json, LiteralError> {
        if depth > MAX_NESTING {
            return Err(LiteralError::TooDeep);
        }
        let named = match term {
            TypeTerm::Param(_) => {
                return Err(LiteralError::UnresolvedParameter {
                    ty: term.to_string(),
                });
            }
            TypeTerm::Named(named) => named,
        };
        if named.namespace != "core" || named.major != 1 {
            return self.encode_schema(value, term, named, depth);
        }
        let wrong = || value_mismatch(term);
        Ok(match (named.name.as_str(), named.args.as_slice(), value) {
            ("bool", [], Value::Bool(b)) => json!(b),
            ("i32", [], Value::I32(n)) => json!(n),
            ("u32", [], Value::U32(n)) => json!(n),
            ("i64", [], Value::I64(n))
            | ("duration", [], Value::Duration(n))
            | ("timestamp", [], Value::Timestamp(n)) => {
                json!(n.to_string())
            }
            ("u64", [], Value::U64(n)) => json!(n.to_string()),
            ("f32", [], Value::F32(x)) => json!(format!("{:08x}", x.to_bits())),
            ("f64", [], Value::F64(x)) => json!(format!("{:016x}", x.to_bits())),
            ("string", [], Value::Str(s)) => json!(s.as_str()),
            ("bytes", [], Value::Bytes(b)) => json!(base64::encode(b)),
            ("json", [], Value::Json(text)) => json!(text.as_str()),
            ("secret-ref", [], Value::Str(s)) if s.starts_with("credential://") => {
                json!(s.as_str())
            }
            ("list", items, Value::List(list)) => {
                let [element] = args(term, items)?;
                let items = list
                    .iter()
                    .map(|item| self.encode(&item, &element, depth + 1));
                Json::Array(items.collect::<Result<_, _>>()?)
            }
            ("map", items, Value::Map(map)) => {
                let [key, value] = args(term, items)?;
                self.schemas.validate_map_key(&key)?;
                let mut seen = HashSet::with_capacity(map.entries.len());
                let pairs = map.entries.iter().map(|(k, v)| {
                    if !seen.insert(k) {
                        return Err(LiteralError::DuplicateMapKey);
                    }
                    Ok(json!([
                        self.encode(k, &key, depth + 1)?,
                        self.encode(v, &value, depth + 1)?
                    ]))
                });
                Json::Array(pairs.collect::<Result<_, LiteralError>>()?)
            }
            ("option", items, value) => {
                let [inner] = args(term, items)?;
                match value {
                    Value::None => json!({"none": true}),
                    Value::Some(boxed) if inner.core("option").is_some() => {
                        json!({"some": self.encode(boxed, &inner, depth + 1)?})
                    }
                    _ if inner.core("option").is_some() => return Err(wrong()),
                    value => json!({"some": self.encode(value, &inner, depth + 1)?}),
                }
            }
            ("result" | "error" | "stream" | "handle", _, _) => {
                return Err(LiteralError::NoLiteralForm {
                    ty: term.to_string(),
                });
            }
            _ => return Err(wrong()),
        })
    }

    fn encode_schema(
        &self,
        value: &Value,
        term: &TypeTerm,
        named: &NamedTerm,
        depth: usize,
    ) -> Result<Json, LiteralError> {
        let wrong = || value_mismatch(term);
        if let Some(schema) = self.schemas.record(named) {
            let Value::Record(record) = value else {
                return Err(wrong());
            };
            if record.ty.as_str() != term.to_string() || record.fields.len() != schema.fields.len()
            {
                return Err(wrong());
            }
            let mut object = Map::new();
            for (field, slot) in schema.fields.iter().zip(record.fields.iter()) {
                match slot {
                    Some(value) => {
                        object.insert(
                            field.field_id.clone(),
                            self.encode(value, &field.ty, depth + 1)?,
                        );
                    }
                    None if field.optional => {}
                    None => {
                        return Err(LiteralError::MissingField {
                            field: field.field_id.clone(),
                        });
                    }
                }
            }
            return Ok(Json::Object(object));
        }
        if let Some(schema) = self.schemas.enumeration(named) {
            let Value::Enum { tag, payload } = value else {
                return Err(wrong());
            };
            let variant = schema.variants.get(*tag as usize).ok_or_else(wrong)?;
            return match (&variant.payload, payload) {
                (None, None) => Ok(json!({"variant": variant.variant_id})),
                (Some(ty), Some(payload)) => Ok(
                    json!({"variant": variant.variant_id, "payload": self.encode(payload, ty, depth + 1)?}),
                ),
                _ => Err(wrong()),
            };
        }
        Err(LiteralError::UnknownType {
            ty: term.to_string(),
        })
    }
}

/// Stores decoded elements in the typed array for scalar element types (12 §7).
fn pack(element: &TypeTerm, values: Vec<Value>) -> ListRepr {
    macro_rules! typed {
        ($variant:ident) => {
            ListRepr::$variant(
                values
                    .into_iter()
                    .map(|value| match value {
                        Value::$variant(item) => item,
                        _ => unreachable!("elements were decoded as the element type"),
                    })
                    .collect(),
            )
        };
    }
    let scalar = |name| element.core(name).is_some_and(<[_]>::is_empty);
    if scalar("bool") {
        typed!(Bool)
    } else if scalar("i32") {
        typed!(I32)
    } else if scalar("i64") {
        typed!(I64)
    } else if scalar("u32") {
        typed!(U32)
    } else if scalar("u64") {
        typed!(U64)
    } else if scalar("f32") {
        typed!(F32)
    } else if scalar("f64") {
        typed!(F64)
    } else if scalar("string") {
        typed!(Str)
    } else {
        ListRepr::Values(values.into())
    }
}
