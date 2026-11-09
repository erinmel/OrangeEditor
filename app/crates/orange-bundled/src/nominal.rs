//! `org.orange.collections` record/enum construction and projection. Ports follow the
//! registered schema named by the node config (11 §5.9); kernels bind to its shape.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::nominal::{NominalTarget, nominal_target};
use orange_model::catalog::{NodeDefinition, NominalAccess, Ports, ProviderKey, RegistrySnapshot};
use orange_model::document::{ConfigValue, DefinitionRef, NodeKind};
use orange_model::value::RecordRepr;
use orange_model::value::shape::ValueShape;
use serde_json::json;

use crate::collections::{check, some};
use crate::{BindingError, CatalogError, KernelError, MathPackage};

/// Record and enum nodes. Each names its nominal type (and field or variant) in config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NominalOperation {
    MakeRecord,
    GetField,
    SetField,
    MakeEnum,
    EnumPayload,
}

impl NominalOperation {
    pub const ALL: [Self; 5] = [
        Self::MakeRecord,
        Self::GetField,
        Self::SetField,
        Self::MakeEnum,
        Self::EnumPayload,
    ];

    pub const fn definition_id(self) -> &'static str {
        match self {
            Self::MakeRecord => "org.orange.collections/make-record",
            Self::GetField => "org.orange.collections/get-field",
            Self::SetField => "org.orange.collections/set-field",
            Self::MakeEnum => "org.orange.collections/make-enum",
            Self::EnumPayload => "org.orange.collections/enum-payload",
        }
    }

    pub fn from_definition_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.definition_id() == id)
    }

    pub const fn access(self) -> NominalAccess {
        match self {
            Self::MakeRecord => NominalAccess::MakeRecord,
            Self::GetField => NominalAccess::GetField,
            Self::SetField => NominalAccess::SetField,
            Self::MakeEnum => NominalAccess::MakeEnum,
            Self::EnumPayload => NominalAccess::EnumPayload,
        }
    }

    fn member(self) -> Option<&'static str> {
        match self {
            Self::MakeRecord => None,
            Self::GetField | Self::SetField => Some("field"),
            Self::MakeEnum | Self::EnumPayload => Some("variant"),
        }
    }
}

/// Ordinary definitions whose ports the registry derives from config (`Ports::Nominal`).
pub fn nominal_definitions() -> Result<Vec<NodeDefinition>, CatalogError> {
    NominalOperation::ALL
        .into_iter()
        .map(|operation| {
            let mut properties = json!({"type":{"type":"string"}});
            let mut required = vec!["type"];
            if let Some(member) = operation.member() {
                properties[member] = json!({"type":"string"});
                required.push(member);
            }
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
                type_params: vec![],
                ports: Ports::Nominal(operation.access()),
                references: vec![],
                specializations: None,
                config_schema: Some(json!({"$schema":"orange.config-schema/v1","type":"object",
                "properties":properties,"required":required,"additionalProperties":false})),
            })
        })
        .collect()
}

/// A pinned binding to one record/enum member's shape.
#[derive(Debug, Clone)]
pub struct ResolvedNominal {
    snapshot: Arc<RegistrySnapshot>,
    provider: ProviderKey,
    operation: NominalOperation,
    /// The record or enum shape named by `config.type`.
    shape: ValueShape,
    /// Field index or variant tag.
    member: u32,
    inputs: Box<[String]>,
}

impl ResolvedNominal {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
    pub fn operation(&self) -> NominalOperation {
        self.operation
    }
    /// Executable input port IDs in order: record field IDs for `make-record`.
    pub fn input_ports(&self) -> &[String] {
        &self.inputs
    }

    /// Values in [`ResolvedNominal::input_ports`] order. Optional record fields travel
    /// as `option<F>` (None is missing). Results are checked against the bound shape.
    pub fn evaluate(&self, inputs: &[Value]) -> Result<Value, KernelError> {
        if inputs.len() != self.inputs.len() {
            return Err(KernelError::InputContract);
        }
        let member = self.member as usize;
        match (self.operation, &self.shape) {
            (NominalOperation::MakeRecord, ValueShape::Record(record)) => {
                let mut fields = Vec::with_capacity(record.len());
                for (index, input) in inputs.iter().enumerate() {
                    let (field, optional) =
                        record.field(index).ok_or(KernelError::InputContract)?;
                    fields.push(match (optional, input) {
                        (false, value) => Some(value.clone()),
                        (true, Value::None) => None,
                        (true, Value::Some(inner)) if field.unboxed_none() => {
                            Some(inner.as_ref().clone())
                        }
                        (true, value) => Some(value.clone()),
                    });
                }
                let value = Value::Record(Arc::new(RecordRepr {
                    ty: record.ty().clone(),
                    fields: fields.into(),
                }));
                check(&self.shape, &value)?;
                Ok(value)
            }
            (NominalOperation::GetField, ValueShape::Record(record)) => {
                check(&self.shape, &inputs[0])?;
                let (Value::Record(value), Some((field, optional))) =
                    (&inputs[0], record.field(member))
                else {
                    return Err(KernelError::InputContract);
                };
                let stored = value.fields.get(member).ok_or(KernelError::InputContract)?;
                match (optional, stored) {
                    (false, Some(value)) => Ok(value.clone()),
                    (true, Some(value)) => Ok(some(field, value.clone())),
                    (true, None) => Ok(Value::None),
                    (false, None) => Err(KernelError::InputContract),
                }
            }
            (NominalOperation::SetField, ValueShape::Record(record)) => {
                check(&self.shape, &inputs[0])?;
                let (Value::Record(value), Some((field, optional))) =
                    (&inputs[0], record.field(member))
                else {
                    return Err(KernelError::InputContract);
                };
                let mut fields = value.fields.to_vec();
                let slot = fields.get_mut(member).ok_or(KernelError::InputContract)?;
                *slot = match (optional, &inputs[1]) {
                    (false, new) => Some(new.clone()),
                    (true, Value::None) => None,
                    (true, Value::Some(inner)) if field.unboxed_none() => {
                        Some(inner.as_ref().clone())
                    }
                    (true, new) => Some(new.clone()),
                };
                let updated = Value::Record(Arc::new(RecordRepr {
                    ty: value.ty.clone(),
                    fields: fields.into(),
                }));
                check(&self.shape, &updated)?;
                Ok(updated)
            }
            (NominalOperation::MakeEnum, ValueShape::Enum(_)) => {
                let value = Value::Enum {
                    tag: self.member,
                    payload: inputs.first().cloned().map(Arc::new),
                };
                check(&self.shape, &value)?;
                Ok(value)
            }
            (NominalOperation::EnumPayload, ValueShape::Enum(enumeration)) => {
                check(&self.shape, &inputs[0])?;
                let Value::Enum { tag, payload } = &inputs[0] else {
                    return Err(KernelError::InputContract);
                };
                if *tag != self.member {
                    return Ok(Value::None);
                }
                let (Some(Some(shape)), Some(payload)) = (enumeration.payload(*tag), payload)
                else {
                    return Err(KernelError::InputContract);
                };
                Ok(some(shape, payload.as_ref().clone()))
            }
            _ => Err(KernelError::InputContract),
        }
    }
}

impl MathPackage {
    /// Resolves this build's exact `org.orange.collections` record/enum node against the
    /// schema named by its config, as the registry's derived contract does.
    pub fn resolve_nominal(
        &self,
        snapshot: Arc<RegistrySnapshot>,
        provider: &ProviderKey,
        definition: &DefinitionRef,
        config_version: u32,
        config: &ConfigValue,
    ) -> Result<ResolvedNominal, BindingError> {
        let operation = NominalOperation::from_definition_id(definition.id.as_str())
            .ok_or(BindingError::Unavailable)?;
        if provider != self.collections_provider_key()
            || definition.version != 1
            || config_version != 1
            || !snapshot.provides(definition, config_version, provider)
        {
            return Err(BindingError::Unavailable);
        }
        let types = snapshot.types().clone();
        let target = nominal_target(operation.access(), config, &types)
            .map_err(|_| BindingError::InvalidConfig)?;
        let (ty, inputs, member_id) = match &target {
            NominalTarget::Record { ty, fields } => (
                ty,
                fields
                    .iter()
                    .map(|f| f.field_id.clone())
                    .collect::<Vec<_>>(),
                None,
            ),
            NominalTarget::Field { ty, field } => (
                ty,
                if operation == NominalOperation::SetField {
                    vec!["record".to_owned(), "value".to_owned()]
                } else {
                    vec!["record".to_owned()]
                },
                Some(field.field_id.as_str()),
            ),
            NominalTarget::Variant { ty, variant } => (
                ty,
                match operation {
                    NominalOperation::EnumPayload => vec!["value".to_owned()],
                    _ if variant.payload.is_some() => vec!["payload".to_owned()],
                    _ => vec![],
                },
                Some(variant.variant_id.as_str()),
            ),
        };
        let shape = ValueShape::of_in(ty, types.as_ref()).ok_or(BindingError::UnsupportedType)?;
        let member = match (&shape, member_id) {
            (ValueShape::Record(_), None) => 0,
            (ValueShape::Record(record), Some(field)) => {
                record.index(field).ok_or(BindingError::InvalidConfig)? as u32
            }
            (ValueShape::Enum(enumeration), Some(variant)) => enumeration
                .tag(variant)
                .ok_or(BindingError::InvalidConfig)?,
            _ => return Err(BindingError::UnsupportedType),
        };
        let inputs = inputs.into_boxed_slice();
        drop(target);
        Ok(ResolvedNominal {
            snapshot,
            provider: provider.clone(),
            operation,
            shape,
            member,
            inputs,
        })
    }
}
