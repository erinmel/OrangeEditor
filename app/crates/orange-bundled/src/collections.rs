//! `org.orange.collections` list, map and option vocabulary. Bind once against the
//! concrete container type; evaluation checks inputs against it and reads no registry.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::{NodeDefinition, Ports, ProviderKey, RegistrySnapshot};
use orange_model::document::{
    ConfigValue, DefinitionRef, NodeKind, OutputSource, PortDescriptor, TypeParam,
};
use orange_model::id::TypeExpr;
use orange_model::value::shape::{ValueCheckError, ValueShape};
use orange_model::value::{ListBuilder, MapRepr};
use serde_json::json;

use crate::{BindingError, CatalogError, KernelError, MathPackage};

/// Largest nominal value charge (11 §5.2 byte accounting) a collection node produces.
/// Growing results that would exceed it fail with `core/LimitExceeded`.
pub const MAX_COLLECTION_BYTES: usize = 16 * 1024 * 1024;

/// Pure generic collection operations. Lists and options are generic over `T`; maps
/// over `K: eq + hash` and `V`. Results are new values; inputs are never mutated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionOperation {
    ListLength,
    ListGet,
    ListSlice,
    ListConcat,
    ListAppend,
    ListReverse,
    ListContains,
    ListIndexOf,
    MapLength,
    MapGet,
    MapContainsKey,
    MapInsert,
    MapRemove,
    MapKeys,
    MapValues,
    OptionIsSome,
}

const LIST: &str = "core/list@1<$T>";
const MAP: &str = "core/map@1<$K,$V>";
const U64: &str = "core/u64@1";
const BOOL: &str = "core/bool@1";

impl CollectionOperation {
    pub const ALL: [Self; 16] = [
        Self::ListLength,
        Self::ListGet,
        Self::ListSlice,
        Self::ListConcat,
        Self::ListAppend,
        Self::ListReverse,
        Self::ListContains,
        Self::ListIndexOf,
        Self::MapLength,
        Self::MapGet,
        Self::MapContainsKey,
        Self::MapInsert,
        Self::MapRemove,
        Self::MapKeys,
        Self::MapValues,
        Self::OptionIsSome,
    ];

    pub const fn definition_id(self) -> &'static str {
        match self {
            Self::ListLength => "org.orange.collections/list-length",
            Self::ListGet => "org.orange.collections/list-get",
            Self::ListSlice => "org.orange.collections/list-slice",
            Self::ListConcat => "org.orange.collections/list-concat",
            Self::ListAppend => "org.orange.collections/list-append",
            Self::ListReverse => "org.orange.collections/list-reverse",
            Self::ListContains => "org.orange.collections/list-contains",
            Self::ListIndexOf => "org.orange.collections/list-index-of",
            Self::MapLength => "org.orange.collections/map-length",
            Self::MapGet => "org.orange.collections/map-get",
            Self::MapContainsKey => "org.orange.collections/map-contains-key",
            Self::MapInsert => "org.orange.collections/map-insert",
            Self::MapRemove => "org.orange.collections/map-remove",
            Self::MapKeys => "org.orange.collections/map-keys",
            Self::MapValues => "org.orange.collections/map-values",
            Self::OptionIsSome => "org.orange.collections/option-is-some",
        }
    }

    pub fn from_definition_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.definition_id() == id)
    }

    /// Strict executable inputs with their declared types. The first input is the
    /// container whose concrete type binds the node.
    pub const fn inputs(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::ListLength | Self::ListReverse => &[("items", LIST)],
            Self::ListGet => &[("items", LIST), ("index", U64)],
            Self::ListSlice => &[("items", LIST), ("start", U64), ("length", U64)],
            Self::ListConcat => &[("a", LIST), ("b", LIST)],
            Self::ListAppend => &[("items", LIST), ("item", "$T")],
            Self::ListContains | Self::ListIndexOf => &[("items", LIST), ("value", "$T")],
            Self::MapLength | Self::MapKeys | Self::MapValues => &[("map", MAP)],
            Self::MapGet | Self::MapContainsKey | Self::MapRemove => &[("map", MAP), ("key", "$K")],
            Self::MapInsert => &[("map", MAP), ("key", "$K"), ("value", "$V")],
            Self::OptionIsSome => &[("value", "core/option@1<$T>")],
        }
    }

    pub const fn output_port(self) -> &'static str {
        "result"
    }

    pub const fn output_type(self) -> &'static str {
        match self {
            Self::ListLength | Self::MapLength => U64,
            Self::ListContains | Self::MapContainsKey | Self::OptionIsSome => BOOL,
            Self::ListGet => "core/option@1<$T>",
            Self::ListIndexOf => "core/option@1<core/u64@1>",
            Self::ListSlice | Self::ListConcat | Self::ListAppend | Self::ListReverse => LIST,
            Self::MapGet => "core/option@1<$V>",
            Self::MapInsert | Self::MapRemove => MAP,
            Self::MapKeys => "core/list@1<$K>",
            Self::MapValues => "core/list@1<$V>",
        }
    }

    fn is_map(self) -> bool {
        matches!(
            self,
            Self::MapLength
                | Self::MapGet
                | Self::MapContainsKey
                | Self::MapInsert
                | Self::MapRemove
                | Self::MapKeys
                | Self::MapValues
        )
    }

    fn type_params(self) -> Vec<TypeParam> {
        let param = |name: &str, constraints: &[&str]| TypeParam {
            name: name.into(),
            constraints: constraints.iter().map(|c| (*c).into()).collect(),
            extensions: Default::default(),
        };
        if self.is_map() {
            vec![param("K", &["eq", "hash"]), param("V", &[])]
        } else if matches!(self, Self::ListContains | Self::ListIndexOf) {
            vec![param("T", &["eq"])]
        } else {
            vec![param("T", &[])]
        }
    }
}

/// Ordinary generic definitions in the `org.orange.collections` namespace. The
/// provider lists no specializations: binding checks the concrete shapes instead.
pub fn collection_definitions() -> Result<Vec<NodeDefinition>, CatalogError> {
    CollectionOperation::ALL
        .into_iter()
        .map(|operation| {
            let mut ports = operation
                .inputs()
                .iter()
                .map(|(name, ty)| Ok(PortDescriptor::data_in(name, ty.parse::<TypeExpr>()?)))
                .collect::<Result<Vec<_>, CatalogError>>()?;
            ports.push(PortDescriptor::data_out(
                operation.output_port(),
                operation.output_type().parse::<TypeExpr>()?,
                OutputSource::Computed,
            ));
            Ok(NodeDefinition {
                definition: DefinitionRef {
                    id: operation.definition_id().parse()?,
                    version: 1,
                },
                config_version: 1,
                kind: NodeKind::Pure,
                volatility: None,
                determinism: orange_model::catalog::Determinism::Deterministic,
                suspends: None,
                type_params: operation.type_params(),
                ports: Ports::Declared(ports),
                references: vec![],
                specializations: None,
                config_schema: Some(json!({"$schema":"orange.config-schema/v1","type":"object",
                "properties":{},"additionalProperties":false})),
            })
        })
        .collect()
}

/// A pinned exact provider binding for one concrete container shape.
#[derive(Debug, Clone)]
pub struct ResolvedCollection {
    snapshot: Arc<RegistrySnapshot>,
    provider: ProviderKey,
    operation: CollectionOperation,
    /// The bound container: `list<T>`, `map<K,V>` or `option<T>`.
    container: ValueShape,
}

pub(crate) fn check(shape: &ValueShape, value: &Value) -> Result<usize, KernelError> {
    shape
        .checked_charge(value, MAX_COLLECTION_BYTES)
        .map_err(|error| match error {
            ValueCheckError::ResourceLimit => KernelError::LimitExceeded,
            _ => KernelError::InputContract,
        })
}
/// `option<T>` stores a present value bare unless T itself can be a bare None.
pub(crate) fn some(inner: &ValueShape, value: Value) -> Value {
    if inner.unboxed_none() {
        Value::Some(Arc::new(value))
    } else {
        value
    }
}
fn list_from(
    element: &ValueShape,
    len: usize,
    items: impl Iterator<Item = Value>,
) -> Result<Value, KernelError> {
    let mut builder = ListBuilder::new(Some(element));
    builder
        .reserve(len)
        .map_err(|_| KernelError::LimitExceeded)?;
    for item in items {
        builder.push(item).map_err(|_| KernelError::InputContract)?;
    }
    Ok(Value::List(Arc::new(builder.finish())))
}
fn map_from(entries: Vec<(Value, Value)>) -> Result<Value, KernelError> {
    Ok(Value::Map(Arc::new(
        MapRepr::new(entries).map_err(|_| KernelError::InputContract)?,
    )))
}

impl ResolvedCollection {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
    pub fn operation(&self) -> CollectionOperation {
        self.operation
    }

    /// Values in declared input order. Every input is checked against the bound shape
    /// first (`InputContract`); out-of-range slices are `core/OutOfRange`, and a result
    /// whose nominal charge would exceed [`MAX_COLLECTION_BYTES`] is `core/LimitExceeded`.
    pub fn evaluate(&self, inputs: &[Value]) -> Result<Value, KernelError> {
        use CollectionOperation::*;
        let operation = self.operation;
        if inputs.len() != operation.inputs().len() {
            return Err(KernelError::InputContract);
        }
        let container = &self.container;
        let charged = check(container, &inputs[0])?;
        match (container, &inputs[0]) {
            (ValueShape::Option(_), value) => Ok(Value::Bool(!matches!(value, Value::None))),
            (ValueShape::List(element), Value::List(items)) => {
                let element = element.as_ref();
                let u64_at = |index: usize| match inputs[index] {
                    Value::U64(value) => Ok(value),
                    _ => Err(KernelError::InputContract),
                };
                Ok(match operation {
                    ListLength => Value::U64(items.len() as u64),
                    ListGet => {
                        let index = u64_at(1)?;
                        match usize::try_from(index).ok().and_then(|i| items.get(i)) {
                            Some(item) => some(element, item),
                            None => Value::None,
                        }
                    }
                    ListSlice => {
                        let (start, length) = (u64_at(1)?, u64_at(2)?);
                        let end = start.checked_add(length).ok_or(KernelError::OutOfRange)?;
                        if end > items.len() as u64 {
                            return Err(KernelError::OutOfRange);
                        }
                        let (start, end) = (start as usize, end as usize);
                        list_from(
                            element,
                            end - start,
                            (start..end).filter_map(|i| items.get(i)),
                        )?
                    }
                    ListReverse => {
                        let len = items.len();
                        list_from(element, len, (0..len).rev().filter_map(|i| items.get(i)))?
                    }
                    ListConcat => {
                        let other = check(container, &inputs[1])?;
                        if charged.saturating_add(other) > MAX_COLLECTION_BYTES {
                            return Err(KernelError::LimitExceeded);
                        }
                        let Value::List(other) = &inputs[1] else {
                            return Err(KernelError::InputContract);
                        };
                        Value::List(Arc::new(
                            items.concat(other).ok_or(KernelError::LimitExceeded)?,
                        ))
                    }
                    ListAppend => {
                        let item = check(element, &inputs[1])?;
                        if charged.saturating_add(item) > MAX_COLLECTION_BYTES {
                            return Err(KernelError::LimitExceeded);
                        }
                        let single = list_from(element, 1, std::iter::once(inputs[1].clone()))?;
                        let Value::List(single) = &single else {
                            return Err(KernelError::InputContract);
                        };
                        Value::List(Arc::new(
                            items.concat(single).ok_or(KernelError::LimitExceeded)?,
                        ))
                    }
                    ListContains | ListIndexOf => {
                        check(element, &inputs[1])?;
                        let mut found = None;
                        for index in 0..items.len() {
                            let item = items.get(index).ok_or(KernelError::InputContract)?;
                            if element
                                .equal_values(&item, &inputs[1])
                                .ok_or(KernelError::InputContract)?
                            {
                                found = Some(index);
                                break;
                            }
                        }
                        match (operation, found) {
                            (ListContains, found) => Value::Bool(found.is_some()),
                            (_, Some(index)) => Value::U64(index as u64),
                            (_, None) => Value::None,
                        }
                    }
                    _ => return Err(KernelError::InputContract),
                })
            }
            (ValueShape::Map(key_shape, value_shape), Value::Map(map)) => {
                let entries = map.entries();
                // Map keys are Core Hash/Eq types, whose language equality is value
                // equality, so lookup compares stored values directly.
                let position = |key: &Value| entries.iter().position(|(k, _)| k == key);
                Ok(match operation {
                    MapLength => Value::U64(entries.len() as u64),
                    MapKeys => list_from(
                        key_shape,
                        entries.len(),
                        entries.iter().map(|(k, _)| k.clone()),
                    )?,
                    MapValues => list_from(
                        value_shape,
                        entries.len(),
                        entries.iter().map(|(_, v)| v.clone()),
                    )?,
                    MapGet | MapContainsKey | MapRemove => {
                        check(key_shape, &inputs[1])?;
                        let at = position(&inputs[1]);
                        match operation {
                            MapGet => match at {
                                Some(i) => some(value_shape, entries[i].1.clone()),
                                None => Value::None,
                            },
                            MapContainsKey => Value::Bool(at.is_some()),
                            _ => match at {
                                None => inputs[0].clone(),
                                Some(i) => map_from(
                                    entries
                                        .iter()
                                        .enumerate()
                                        .filter(|(j, _)| *j != i)
                                        .map(|(_, entry)| entry.clone())
                                        .collect(),
                                )?,
                            },
                        }
                    }
                    MapInsert => {
                        let added = check(key_shape, &inputs[1])?
                            .saturating_add(check(value_shape, &inputs[2])?);
                        if charged.saturating_add(added) > MAX_COLLECTION_BYTES {
                            return Err(KernelError::LimitExceeded);
                        }
                        let mut entries = entries.to_vec();
                        match position(&inputs[1]) {
                            // An existing key keeps its position; only the value changes.
                            Some(i) => entries[i].1 = inputs[2].clone(),
                            None => entries.push((inputs[1].clone(), inputs[2].clone())),
                        }
                        map_from(entries)?
                    }
                    _ => return Err(KernelError::InputContract),
                })
            }
            _ => Err(KernelError::InputContract),
        }
    }
}

impl MathPackage {
    /// Resolves only this build's exact `org.orange.collections` provider for the
    /// concrete type of the node's first input (`list<T>`, `map<K,V>` or `option<T>`).
    pub fn resolve_collection(
        &self,
        snapshot: Arc<RegistrySnapshot>,
        provider: &ProviderKey,
        definition: &DefinitionRef,
        config_version: u32,
        container: &TypeExpr,
        config: &ConfigValue,
    ) -> Result<ResolvedCollection, BindingError> {
        let operation = CollectionOperation::from_definition_id(definition.id.as_str())
            .ok_or(BindingError::Unavailable)?;
        if provider != self.collections_provider_key()
            || definition.version != 1
            || config_version != 1
            || !snapshot.provides(definition, config_version, provider)
        {
            return Err(BindingError::Unavailable);
        }
        if !config.is_empty() {
            return Err(BindingError::InvalidConfig);
        }
        let shape = ValueShape::of_in(container, snapshot.types().as_ref())
            .ok_or(BindingError::UnsupportedType)?;
        let supported = match (&shape, operation) {
            (ValueShape::Option(_), CollectionOperation::OptionIsSome) => true,
            (ValueShape::Map(..), operation) => operation.is_map(),
            (ValueShape::List(element), CollectionOperation::ListContains)
            | (ValueShape::List(element), CollectionOperation::ListIndexOf) => element.host_eq(),
            (ValueShape::List(_), operation) => {
                !operation.is_map() && operation != CollectionOperation::OptionIsSome
            }
            _ => false,
        };
        if !supported {
            return Err(BindingError::UnsupportedType);
        }
        Ok(ResolvedCollection {
            snapshot,
            provider: provider.clone(),
            operation,
            container: shape,
        })
    }
}
