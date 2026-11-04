//! New nodes bound to the registry's new-binding default (12 §8, ADR-07).
//!
//! A host selects which provider new bindings of a definition use; existing locks never
//! follow that selection. [`new_node_edit`] builds what an editor commits when a user places
//! a node: the selected provider's [`DocCommand::PutPackageLock`], the node's projected
//! [`DocCommand::PutSignature`] and the [`DocCommand::AddNode`] naming both. It reads the
//! document and registry but never edits. Callers apply the commands with
//! [`GraphDocument::apply`], or with [`apply_inferred`](crate::inference::apply_inferred) to
//! persist inferred `type_args` in the same undoable transaction.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::catalog::{ContractError, ProviderKey, RegistrySnapshot};
use crate::document::literal::Literal;
use crate::document::{
    BodyRef, ConfigValue, DefinitionRef, Extensions, GraphDocument, NodeInstance, PackageLockEntry,
    ProviderBinding, TypeParamName,
};
use crate::edit::DocCommand;
use crate::id::{NodeId, PortId, PortKey, TypeExpr};
use crate::signature::{
    SubstitutionFailure, project_with_suspension, semantic_key, try_substitute_params,
};

/// A node to place. The definition's registered config version is used.
#[derive(Debug, Clone, PartialEq)]
pub struct NewNode {
    pub id: NodeId,
    pub body: BodyRef,
    pub definition: DefinitionRef,
    pub config: ConfigValue,
    /// Explicit type choices (11 §5.5); the signature substitutes them.
    pub type_pins: BTreeMap<TypeParamName, TypeExpr>,
    /// Input literals.
    pub inputs: BTreeMap<PortKey, Literal>,
}

impl NewNode {
    /// A root-body node of `definition` with an empty config, no pins and no literals.
    pub fn new(id: NodeId, definition: DefinitionRef) -> Self {
        Self {
            id,
            body: BodyRef::Root,
            definition,
            config: ConfigValue::new(),
            type_pins: BTreeMap::new(),
            inputs: BTreeMap::new(),
        }
    }
}

/// The commands that add one default-bound node, and the provider it was bound to.
#[derive(Debug, Clone, PartialEq)]
pub struct NewNodeEdit {
    /// The registry's new-binding default for the definition when the edit was built.
    pub provider: ProviderKey,
    /// `PutPackageLock`, `PutSignature`, then `AddNode`.
    pub commands: Vec<DocCommand>,
}

/// Why no default-bound node can be built. The document is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NewNodeError {
    /// The registry has no such definition version.
    #[error("definition `{}@{}` is not registered", .0.id, .0.version)]
    UnknownDefinition(DefinitionRef),
    /// The host selected no provider for new bindings of the definition, as after an
    /// install (no last-installed-wins) or after the selected provider was removed.
    #[error("no provider is selected for new bindings of `{}@{}`", .0.id, .0.version)]
    NoDefault(DefinitionRef),
    /// The definition's ports depend on document declarations that do not admit them.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// A pin names a type parameter the node's contract does not declare. Pin keys are
    /// unvalidated caller text, so the error does not retain or print the key; compare the
    /// pins with the contract's type parameters to find it.
    #[error("a type pin names no declared type parameter")]
    UndeclaredPin,
    /// A pinned type could not be substituted into a declared port type. Only the port and
    /// the failure kind are kept; no type text is retained (13 §2).
    #[error("port `{port}` cannot take the pinned types ({failure:?})")]
    Substitution {
        port: PortId,
        failure: SubstitutionFailure,
    },
}

/// Builds the edit that adds `node` bound to `registry`'s new-binding default for its
/// definition.
///
/// The package lock records the provider's exact version and digest; applying it fails with
/// `TxError::PackageLockConflict` when the document already locks that version to another
/// digest. The signature is projected from the node's contract with its pins substituted;
/// unpinned parameters stay `$name` until inference resolves them. A pin for a parameter the
/// contract does not declare is [`NewNodeError::UndeclaredPin`], and a pin that cannot be
/// substituted into a port type is [`NewNodeError::Substitution`].
pub fn new_node_edit(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    node: NewNode,
) -> Result<NewNodeEdit, NewNodeError> {
    let NewNode {
        id,
        body,
        definition,
        config,
        type_pins,
        inputs,
    } = node;
    let registered = registry
        .definition(&definition)
        .ok_or_else(|| NewNodeError::UnknownDefinition(definition.clone()))?;
    let provider = registry
        .new_binding_default(&definition)
        .ok_or_else(|| NewNodeError::NoDefault(definition.clone()))?
        .clone();
    let mut instance = NodeInstance {
        id,
        definition: definition.clone(),
        config_version: registered.config_version,
        config,
        type_args: BTreeMap::new(),
        type_pins,
        provider: Some(ProviderBinding {
            package: provider.package.clone(),
            version: provider.version.clone(),
            id: provider.id.clone(),
            extensions: Extensions::new(),
        }),
        signature: None,
        inputs,
        variadic: BTreeMap::new(),
        activation: None,
        extensions: Extensions::new(),
    };
    let contract = registry
        .contract(document, &instance)?
        .ok_or(NewNodeError::UnknownDefinition(definition))?;
    if instance
        .type_pins
        .keys()
        .any(|pin| !contract.type_params.iter().any(|param| &param.name == pin))
    {
        return Err(NewNodeError::UndeclaredPin);
    }
    let mut ports = contract.ports.clone();
    // Without pins there is nothing to substitute; malformed declared types stay for the
    // validator to report, as for any unpinned node.
    for port in ports.iter_mut().filter(|_| !instance.type_pins.is_empty()) {
        if let Some(ty) = &port.ty {
            let resolved = try_substitute_params(ty, &instance.type_pins).map_err(|error| {
                NewNodeError::Substitution {
                    port: port.port_id.clone(),
                    failure: error.failure(),
                }
            })?;
            port.ty = Some(resolved);
        }
    }
    let snapshot =
        project_with_suspension(contract.kind, contract.suspends, ports, &BTreeMap::new());
    let key = semantic_key(&snapshot);
    instance.signature = Some(key.clone());
    let commands = vec![
        DocCommand::PutPackageLock(PackageLockEntry {
            id: provider.package.clone(),
            version: provider.version.clone(),
            content_digest: provider.content_digest.clone(),
            extensions: Extensions::new(),
        }),
        DocCommand::PutSignature { key, snapshot },
        DocCommand::AddNode {
            body,
            node: instance,
        },
    ];
    Ok(NewNodeEdit { provider, commands })
}
