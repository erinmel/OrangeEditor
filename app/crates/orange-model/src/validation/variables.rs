//! Variable identity lookups shared by validation passes.

use super::diagnostic::DiagnosticCode;
use crate::catalog::{ContractError, ReferenceKind, RegistrySnapshot};
use crate::document::NodeInstance;
use crate::edit::DeclId;
use crate::id::VariableId;

pub(super) fn variable_reference(
    registry: &RegistrySnapshot,
    node: &NodeInstance,
) -> Result<VariableId, DiagnosticCode> {
    match registry.reference(node, ReferenceKind::Variable) {
        Ok(DeclId::Variable(id)) => Ok(id),
        _ => Err(DiagnosticCode::VariableUnknown),
    }
}

pub(super) fn contract_code(error: ContractError) -> DiagnosticCode {
    match error {
        ContractError::VariableUnknown => DiagnosticCode::VariableUnknown,
        ContractError::VariableScope => DiagnosticCode::VariableScope,
    }
}
