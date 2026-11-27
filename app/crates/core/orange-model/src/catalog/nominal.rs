//! Ports of record/enum construction and projection nodes, derived from the registered
//! nominal schema named by the node's config (11 §5.9).

use crate::catalog::{ContractError, NodeContract, NodeDefinition, NominalAccess};
use crate::document::{ConfigValue, NodeInstance, OutputSource, PortDescriptor};
use crate::id::TypeExpr;
use crate::type_term::TypeTerm;
use crate::types::TypeRegistry;
use crate::value::literal_codec::Schemas;

/// The schema member a nominal node addresses, checked against its config.
#[derive(Debug)]
pub enum NominalTarget<'a> {
    Record {
        ty: TypeExpr,
        fields: &'a [crate::value::literal_codec::FieldSchema],
    },
    Field {
        ty: TypeExpr,
        field: &'a crate::value::literal_codec::FieldSchema,
    },
    Variant {
        ty: TypeExpr,
        variant: &'a crate::value::literal_codec::VariantSchema,
    },
}

fn text<'a>(config: &'a ConfigValue, key: &str) -> Result<&'a str, ContractError> {
    config
        .get(key)
        .and_then(|value| value.as_str())
        .ok_or(ContractError::NominalConfig)
}

/// Resolves `config.type` (and `field` or `variant`) against the registered schemas.
/// Unknown keys, a type that is not a registered record/enum of the right kind, or an
/// unknown member are all [`ContractError::NominalConfig`].
pub fn nominal_target<'a>(
    access: NominalAccess,
    config: &ConfigValue,
    types: &'a TypeRegistry,
) -> Result<NominalTarget<'a>, ContractError> {
    let member = match access {
        NominalAccess::MakeRecord => None,
        NominalAccess::GetField | NominalAccess::SetField => Some("field"),
        NominalAccess::MakeEnum | NominalAccess::EnumPayload => Some("variant"),
    };
    if config
        .keys()
        .any(|key| key != "type" && Some(key.as_str()) != member)
    {
        return Err(ContractError::NominalConfig);
    }
    let ty: TypeExpr = text(config, "type")?
        .parse()
        .map_err(|_| ContractError::NominalConfig)?;
    let TypeTerm::Named(named) = TypeTerm::parse(&ty).map_err(|_| ContractError::NominalConfig)?
    else {
        return Err(ContractError::NominalConfig);
    };
    Ok(match access {
        NominalAccess::MakeRecord | NominalAccess::GetField | NominalAccess::SetField => {
            let schema = types.record(&named).ok_or(ContractError::NominalConfig)?;
            if access == NominalAccess::MakeRecord {
                NominalTarget::Record {
                    ty,
                    fields: &schema.fields,
                }
            } else {
                let id = text(config, "field")?;
                let field = schema
                    .fields
                    .iter()
                    .find(|field| field.field_id == id)
                    .ok_or(ContractError::NominalConfig)?;
                NominalTarget::Field { ty, field }
            }
        }
        NominalAccess::MakeEnum | NominalAccess::EnumPayload => {
            let schema = types
                .enumeration(&named)
                .ok_or(ContractError::NominalConfig)?;
            let id = text(config, "variant")?;
            let variant = schema
                .variants
                .iter()
                .find(|variant| variant.variant_id == id)
                .ok_or(ContractError::NominalConfig)?;
            // Projection exists only for a payload; a payload-free variant is a Switch case.
            if access == NominalAccess::EnumPayload && variant.payload.is_none() {
                return Err(ContractError::NominalConfig);
            }
            NominalTarget::Variant { ty, variant }
        }
    })
}

fn expr(term: &TypeTerm) -> Result<TypeExpr, ContractError> {
    term.to_string()
        .parse()
        .map_err(|_| ContractError::NominalConfig)
}
fn option(term: &TypeTerm) -> Result<TypeExpr, ContractError> {
    format!("core/option@1<{term}>")
        .parse()
        .map_err(|_| ContractError::TypeComplexity)
}

/// Optional record fields travel as `option<F>`: None is a missing field, and a
/// present None of an option-typed field stays distinct by the 12 §7 boxing rule.
pub(crate) fn nominal_contract(
    definition: &NodeDefinition,
    access: NominalAccess,
    node: &NodeInstance,
    types: &TypeRegistry,
) -> Result<NodeContract, ContractError> {
    let target = nominal_target(access, &node.config, types)?;
    let mut ports = Vec::new();
    match (access, target) {
        (NominalAccess::MakeRecord, NominalTarget::Record { ty, fields }) => {
            for field in fields {
                let port_ty = if field.optional {
                    option(&field.ty)?
                } else {
                    expr(&field.ty)?
                };
                let port = field
                    .field_id
                    .as_str()
                    .parse()
                    .map_err(|_| ContractError::NominalConfig)?;
                ports.push(PortDescriptor {
                    port_id: port,
                    ..PortDescriptor::data_in("value", port_ty)
                });
            }
            ports.push(PortDescriptor::data_out(
                "result",
                ty,
                OutputSource::Computed,
            ));
        }
        (NominalAccess::SetField, NominalTarget::Field { ty, field }) => {
            ports.push(PortDescriptor::data_in("record", ty.clone()));
            let value = if field.optional {
                option(&field.ty)?
            } else {
                expr(&field.ty)?
            };
            ports.push(PortDescriptor::data_in("value", value));
            ports.push(PortDescriptor::data_out(
                "result",
                ty,
                OutputSource::Computed,
            ));
        }
        (NominalAccess::GetField, NominalTarget::Field { ty, field }) => {
            ports.push(PortDescriptor::data_in("record", ty));
            let out = if field.optional {
                option(&field.ty)?
            } else {
                expr(&field.ty)?
            };
            ports.push(PortDescriptor::data_out(
                "result",
                out,
                OutputSource::Computed,
            ));
        }
        (NominalAccess::MakeEnum, NominalTarget::Variant { ty, variant }) => {
            if let Some(payload) = &variant.payload {
                ports.push(PortDescriptor::data_in("payload", expr(payload)?));
            }
            ports.push(PortDescriptor::data_out(
                "result",
                ty,
                OutputSource::Computed,
            ));
        }
        (NominalAccess::EnumPayload, NominalTarget::Variant { ty, variant }) => {
            ports.push(PortDescriptor::data_in("value", ty));
            let payload = variant
                .payload
                .as_ref()
                .ok_or(ContractError::NominalConfig)?;
            ports.push(PortDescriptor::data_out(
                "result",
                option(payload)?,
                OutputSource::Computed,
            ));
        }
        _ => return Err(ContractError::NominalConfig),
    }
    Ok(NodeContract {
        kind: definition.kind,
        suspends: definition.suspends,
        type_params: Vec::new(),
        ports,
    })
}
