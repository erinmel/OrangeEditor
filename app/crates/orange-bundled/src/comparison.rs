//! Core `eq`/`ord` comparison kernels (11 §5.4) generic over one constrained `$T`.
//!
//! Binding proves that the host itself owns the exact equality or total order of the
//! resolved type: Core rules for Core types, and a package's declared structural `eq`
//! derive for its enums. A package-implemented constraint needs package code, so it never
//! selects these kernels. Evaluation walks the bound shape only; it performs no registry
//! lookup, parsing or I/O and never panics on a contract violation.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::{NodeDefinition, Ports, ProviderKey, RegistrySnapshot};
use orange_model::document::{
    ConfigValue, DefinitionRef, NodeKind, OutputSource, PortDescriptor, TypeParam,
};
use orange_model::id::TypeExpr;
use orange_model::type_term::{MAX_NESTING, TypeTerm};
use orange_model::types::{CapabilityEvidence, CapabilityRequest, Constraint, TypeRegistry};
use orange_model::value::shape::ValueShape;
use serde_json::json;

use crate::{BindingError, CatalogError, KernelError, MathPackage};

/// A comparison of two `$T` values producing `result: core/bool@1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl Comparison {
    pub const ALL: [Self; 6] = [
        Self::Equal,
        Self::NotEqual,
        Self::Less,
        Self::LessOrEqual,
        Self::Greater,
        Self::GreaterOrEqual,
    ];

    pub const fn definition_id(self) -> &'static str {
        match self {
            Self::Equal => "org.orange.math/equal",
            Self::NotEqual => "org.orange.math/not-equal",
            Self::Less => "org.orange.math/less",
            Self::LessOrEqual => "org.orange.math/less-or-equal",
            Self::Greater => "org.orange.math/greater",
            Self::GreaterOrEqual => "org.orange.math/greater-or-equal",
        }
    }

    /// The operation whose definition ID is exactly `id`.
    pub fn from_definition_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.definition_id() == id)
    }

    /// The language constraint `$T` declares: `eq` for (in)equality, otherwise `ord`.
    pub const fn constraint(self) -> Constraint {
        match self {
            Self::Equal | Self::NotEqual => Constraint::Eq,
            _ => Constraint::Ord,
        }
    }

    /// Declared data inputs in executable order.
    pub const fn input_ports(self) -> &'static [&'static str] {
        &["a", "b"]
    }

    pub const fn output_port(self) -> &'static str {
        "result"
    }

    fn decide(self, order: Ordering) -> bool {
        match self {
            Self::Equal => order.is_eq(),
            Self::NotEqual => order.is_ne(),
            Self::Less => order.is_lt(),
            Self::LessOrEqual => order.is_le(),
            Self::Greater => order.is_gt(),
            Self::GreaterOrEqual => order.is_ge(),
        }
    }
}

/// Pure, deterministic comparison definitions. They list no specializations: the
/// declared constraint decides at validation, and binding refuses what the host cannot
/// execute itself.
pub fn comparison_definitions() -> Result<Vec<NodeDefinition>, CatalogError> {
    let generic: TypeExpr = "$T".parse()?;
    let boolean: TypeExpr = "core/bool@1".parse()?;
    Comparison::ALL
        .into_iter()
        .map(|comparison| {
            let mut ports: Vec<_> = comparison
                .input_ports()
                .iter()
                .map(|&name| PortDescriptor::data_in(name, generic.clone()))
                .collect();
            ports.push(PortDescriptor::data_out(
                comparison.output_port(),
                boolean.clone(),
                OutputSource::Computed,
            ));
            Ok(NodeDefinition {
                definition: DefinitionRef {
                    id: comparison.definition_id().parse()?,
                    version: 1,
                },
                config_version: 1,
                kind: NodeKind::Pure,
                volatility: None,
                determinism: orange_model::catalog::Determinism::Deterministic,
                suspends: None,
                type_params: vec![TypeParam {
                    name: "T".into(),
                    constraints: vec![
                        match comparison.constraint() {
                            Constraint::Eq => "eq",
                            _ => "ord",
                        }
                        .into(),
                    ],
                    extensions: Default::default(),
                }],
                ports: Ports::Declared(ports),
                references: vec![],
                specializations: None,
                config_schema: Some(json!({
                    "$schema":"orange.config-schema/v1", "type":"object",
                    "properties":{}, "additionalProperties":false
                })),
            })
        })
        .collect()
}

/// An immutable exact comparison binding over one resolved shape. Retaining it pins its
/// registry metadata. This does not assert that a graph was validated.
#[derive(Debug, Clone)]
pub struct ResolvedComparison {
    snapshot: Arc<RegistrySnapshot>,
    provider: ProviderKey,
    comparison: Comparison,
    shape: ValueShape,
}

impl ResolvedComparison {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
    pub fn comparison(&self) -> Comparison {
        self.comparison
    }

    /// Compares `a` with `b` (declared order). A value that does not match the bound
    /// shape is a contract error, never coerced.
    pub fn evaluate(&self, a: &Value, b: &Value) -> Result<bool, KernelError> {
        let order = match self.comparison.constraint() {
            Constraint::Eq => {
                if equal(&self.shape, a, b, 0).ok_or(KernelError::InputContract)? {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            }
            _ => total_order(&self.shape, a, b).ok_or(KernelError::InputContract)?,
        };
        Ok(self.comparison.decide(order))
    }
}

impl MathPackage {
    /// Resolves an exact comparison binding once, before execution. `ty` is the fresh
    /// resolved `$T`. A type outside the host-owned rules is
    /// [`BindingError::UnsupportedType`], even when validation proved its constraint.
    pub fn resolve_comparison(
        &self,
        snapshot: Arc<RegistrySnapshot>,
        provider: &ProviderKey,
        definition: &DefinitionRef,
        config_version: u32,
        ty: &TypeExpr,
        config: &ConfigValue,
    ) -> Result<ResolvedComparison, BindingError> {
        let comparison = Comparison::from_definition_id(definition.id.as_str())
            .ok_or(BindingError::Unavailable)?;
        if provider != self.provider_key()
            || definition.version != 1
            || config_version != 1
            || !snapshot.provides(definition, config_version, provider)
        {
            return Err(BindingError::Unavailable);
        }
        if !config.is_empty() {
            return Err(BindingError::InvalidConfig);
        }
        let types = snapshot.types().as_ref();
        let shape = ValueShape::of_in(ty, types).ok_or(BindingError::UnsupportedType)?;
        let supported = match comparison.constraint() {
            Constraint::Eq => host_eq(types, &shape, 0),
            _ => host_ord(&shape),
        };
        if !supported {
            return Err(BindingError::UnsupportedType);
        }
        Ok(ResolvedComparison {
            snapshot,
            provider: provider.clone(),
            comparison,
            shape,
        })
    }
}

/// The Core total orders of 11 §5.4: integers numerically, bool with false before true,
/// strings by code point (equal to UTF-8 byte order), duration and timestamp by signed
/// nanoseconds. Floats have no Core order; enum orders are never derived.
fn host_ord(shape: &ValueShape) -> bool {
    matches!(
        shape,
        ValueShape::Bool
            | ValueShape::I32
            | ValueShape::I64
            | ValueShape::U32
            | ValueShape::U64
            | ValueShape::String
            | ValueShape::Duration
            | ValueShape::Timestamp
    )
}

/// Whether the host owns total equality for every level of `shape`: native Core
/// equality, or a package enum whose `eq` is a declared structural derive. Json, error,
/// result and floats have no executable Core equality yet.
fn host_eq(types: &TypeRegistry, shape: &ValueShape, depth: usize) -> bool {
    if depth > MAX_NESTING {
        return false;
    }
    match shape {
        ValueShape::Bool
        | ValueShape::I32
        | ValueShape::I64
        | ValueShape::U32
        | ValueShape::U64
        | ValueShape::String
        | ValueShape::Bytes
        | ValueShape::Duration
        | ValueShape::Timestamp => true,
        ValueShape::Option(inner) | ValueShape::List(inner) => host_eq(types, inner, depth + 1),
        ValueShape::Map(key, value) => key.native_hash() && host_eq(types, value, depth + 1),
        ValueShape::Enum(schema) => {
            let derived = TypeTerm::parse(schema.ty()).ok().is_some_and(|term| {
                matches!(
                    types.supports_term(&term, CapabilityRequest::Constraint(Constraint::Eq)),
                    Ok(Some(witness)) if witness.evidence == CapabilityEvidence::StructuralDerive
                )
            });
            derived
                && (0..schema.len()).all(|tag| {
                    u32::try_from(tag)
                        .ok()
                        .and_then(|tag| schema.payload(tag))
                        .is_some_and(|payload| {
                            payload.is_none_or(|inner| host_eq(types, inner, depth + 1))
                        })
                })
        }
        _ => false,
    }
}

fn total_order(shape: &ValueShape, a: &Value, b: &Value) -> Option<Ordering> {
    Some(match (shape, a, b) {
        (ValueShape::Bool, Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (ValueShape::I32, Value::I32(a), Value::I32(b)) => a.cmp(b),
        (ValueShape::I64, Value::I64(a), Value::I64(b)) => a.cmp(b),
        (ValueShape::U32, Value::U32(a), Value::U32(b)) => a.cmp(b),
        (ValueShape::U64, Value::U64(a), Value::U64(b)) => a.cmp(b),
        // Byte order of valid UTF-8 is code point order.
        (ValueShape::String, Value::Str(a), Value::Str(b)) => a.as_bytes().cmp(b.as_bytes()),
        (ValueShape::Duration, Value::Duration(a), Value::Duration(b))
        | (ValueShape::Timestamp, Value::Timestamp(a), Value::Timestamp(b)) => a.cmp(b),
        _ => return None,
    })
}

/// Maps up to this size match keys pairwise; larger ones build one presized index.
const SMALL_MAP: usize = 16;

/// Language equality of two values of `shape` (11 §5.2, §5.4); `None` when a visited
/// value does not match the shape. Map equality ignores insertion order. Enum values are
/// equal when their tags are equal and so are their payloads.
fn equal(shape: &ValueShape, a: &Value, b: &Value, depth: usize) -> Option<bool> {
    if depth > MAX_NESTING {
        return None;
    }
    let leaf = |shape: &ValueShape, value: &Value| {
        matches!(
            (shape, value),
            (ValueShape::Bool, Value::Bool(_))
                | (ValueShape::I32, Value::I32(_))
                | (ValueShape::I64, Value::I64(_))
                | (ValueShape::U32, Value::U32(_))
                | (ValueShape::U64, Value::U64(_))
                | (ValueShape::String, Value::Str(_))
                | (ValueShape::Bytes, Value::Bytes(_))
                | (ValueShape::Duration, Value::Duration(_))
                | (ValueShape::Timestamp, Value::Timestamp(_))
        )
    };
    match shape {
        ValueShape::Option(inner) => match (a, b) {
            (Value::None, Value::None) => Some(true),
            (Value::None, other) | (other, Value::None) => {
                // The present side must still be a valid present value.
                let valid = if inner.unboxed_none() {
                    matches!(other, Value::Some(_))
                } else {
                    !matches!(other, Value::Some(_))
                };
                valid.then_some(false)
            }
            (Value::Some(a), Value::Some(b)) if inner.unboxed_none() => {
                equal(inner, a, b, depth + 1)
            }
            _ if inner.unboxed_none() => None,
            _ => equal(inner, a, b, depth + 1),
        },
        ValueShape::List(inner) => {
            let (Value::List(a), Value::List(b)) = (a, b) else {
                return None;
            };
            if a.len() != b.len() {
                return Some(false);
            }
            for (a, b) in a.iter().zip(b.iter()) {
                if !equal(inner, &a, &b, depth + 1)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        ValueShape::Map(key, inner) => {
            let (Value::Map(a), Value::Map(b)) = (a, b) else {
                return None;
            };
            let (a, b) = (a.entries(), b.entries());
            if !a.iter().chain(b).all(|(k, _)| leaf(key, k)) {
                return None;
            }
            if a.len() != b.len() {
                return Some(false);
            }
            // Equal maps usually share insertion order: walk both while keys agree.
            let mut same = 0;
            for ((ka, va), (kb, vb)) in a.iter().zip(b) {
                if ka != kb {
                    break;
                }
                if !equal(inner, va, vb, depth + 1)? {
                    return Some(false);
                }
                same += 1;
            }
            let (a, b) = (&a[same..], &b[same..]);
            let index = (b.len() > SMALL_MAP)
                .then(|| b.iter().map(|(k, v)| (k, v)).collect::<HashMap<_, _>>());
            for (k, v) in a {
                let other = match &index {
                    Some(index) => index.get(k).copied(),
                    None => b.iter().find(|(other, _)| other == k).map(|(_, v)| v),
                };
                match other {
                    Some(other) if equal(inner, v, other, depth + 1)? => {}
                    _ => return Some(false),
                }
            }
            Some(true)
        }
        ValueShape::Enum(schema) => {
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
                return None;
            };
            let (sa, sb) = (schema.payload(*ta)?, schema.payload(*tb)?);
            let valid = |payload: Option<&ValueShape>, value: &Option<Arc<Value>>| {
                payload.is_some() == value.is_some()
            };
            if !valid(sa, pa) || !valid(sb, pb) {
                return None;
            }
            if ta != tb {
                return Some(false);
            }
            match (sa, pa, pb) {
                (Some(inner), Some(a), Some(b)) => equal(inner, a, b, depth + 1),
                _ => Some(true),
            }
        }
        _ if leaf(shape, a) && leaf(shape, b) => Some(a == b),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_comparison_identity_is_unique_and_round_trips() {
        for comparison in Comparison::ALL {
            assert_eq!(
                Comparison::from_definition_id(comparison.definition_id()),
                Some(comparison)
            );
        }
        assert_eq!(Comparison::from_definition_id("org.orange.math/add"), None);
    }

    #[test]
    fn decisions_follow_the_order() {
        use Ordering::*;
        let table = [
            (Comparison::Equal, [false, true, false]),
            (Comparison::NotEqual, [true, false, true]),
            (Comparison::Less, [true, false, false]),
            (Comparison::LessOrEqual, [true, true, false]),
            (Comparison::Greater, [false, false, true]),
            (Comparison::GreaterOrEqual, [false, true, true]),
        ];
        for (comparison, expected) in table {
            assert_eq!(
                [Less, Equal, Greater].map(|order| comparison.decide(order)),
                expected,
                "{comparison:?}"
            );
        }
    }
}
