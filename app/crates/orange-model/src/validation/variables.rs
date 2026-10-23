//! Contextual variable identity and declaration-owned port contracts.

use super::diagnostic::DiagnosticCode;
use crate::document::{
    GraphDocument, NodeInstance, NodeKind, OutputSource, PortDirection, PortKind, ResolvedPort,
    Suspends, VarScope,
};
use crate::id::VariableId;
use crate::inference::GenericDefinition;

pub(super) fn variable_reference(node: &NodeInstance) -> Result<VariableId, DiagnosticCode> {
    node.config
        .get("variable")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse().ok())
        .ok_or(DiagnosticCode::VariableUnknown)
}

pub(super) fn variable_contract(
    document: &GraphDocument,
    node: &NodeInstance,
) -> Option<Result<GenericDefinition, DiagnosticCode>> {
    let write = match node.definition.id.as_str() {
        "core/get-variable" => false,
        "core/set-variable" => true,
        _ => return None,
    };
    Some((|| {
        let id = variable_reference(node)?;
        let declaration = document
            .variables()
            .get(&id)
            .ok_or(DiagnosticCode::VariableUnknown)?;
        if write && declaration.scope == VarScope::Workspace {
            return Err(DiagnosticCode::VariableScope);
        }
        Ok(GenericDefinition {
            kind: if write {
                NodeKind::Action
            } else {
                NodeKind::Volatile
            },
            suspends: Some(Suspends::Never),
            type_params: Vec::new(),
            ports: vec![ResolvedPort {
                port_id: "value".parse().expect("fixed language port ID"),
                direction: if write {
                    PortDirection::Input
                } else {
                    PortDirection::Output
                },
                kind: PortKind::Data,
                ty: Some(declaration.ty.clone()),
                source: (!write).then_some(OutputSource::State),
                available_on: None,
                default: None,
                sensitivity: None,
                extensions: Default::default(),
            }],
        })
    })())
}
