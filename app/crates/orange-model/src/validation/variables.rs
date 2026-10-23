//! Contextual variable identity and declaration-owned port contracts.

use super::diagnostic::DiagnosticCode;
use crate::catalog::NodeContract;
use crate::document::{
    GraphDocument, NodeInstance, NodeKind, OutputSource, PortDescriptor, Suspends, VarScope,
};
use crate::id::VariableId;

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
) -> Option<Result<NodeContract, DiagnosticCode>> {
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
        let ty = declaration.ty.clone();
        let port = if write {
            PortDescriptor::data_in("value", ty)
        } else {
            PortDescriptor::data_out("value", ty, OutputSource::State)
        };
        Ok(NodeContract {
            kind: if write {
                NodeKind::Action
            } else {
                NodeKind::Volatile
            },
            suspends: Some(Suspends::Never),
            type_params: Vec::new(),
            ports: vec![port],
        })
    })())
}
