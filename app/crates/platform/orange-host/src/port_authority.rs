//! Fresh port contracts for authoring. A saved signature can anchor an old wire
//! in the view, but does not authorize a new wire or a literal edit.

use super::*;
#[cfg(test)]
use crate::intrinsic;
use orange_model::catalog::{ContractError, NodeContract, intrinsic_id};
use orange_model::document::NodeInstance;

pub(crate) struct PortResolution<'a> {
    pub(crate) contract: Option<Cow<'a, NodeContract>>,
    pub(crate) missing: Option<MissingPackage>,
}

/// Resolves the shape of the exact bound provider. An unbound draft node can
/// use a known definition while awaiting `Host::bind`.
pub(crate) fn port_resolution<'a>(
    document: &GraphDocument,
    node: &NodeInstance,
    registry: &'a RegistrySnapshot,
) -> PortResolution<'a> {
    let id = node.definition.id.as_str();
    if id == intrinsic_id::CONVERT {
        // Converter providers belong to a separate table. Its contract resolves
        // the exact package digest and converter entry together.
        let contract = registry.contract(document, node);
        let unavailable = matches!(
            contract,
            Err(ContractError::ConverterEntry
                | ContractError::ConverterProvider
                | ContractError::ConverterPackageLockMissing)
        ) && node.provider.is_some();
        return PortResolution {
            contract: contract.ok().flatten(),
            missing: unavailable.then(|| missing_package(node)),
        };
    }
    if !node.definition.id.is_core() {
        let available = match &node.provider {
            Some(binding) => ProviderKey::of(document, binding).is_some_and(|provider| {
                registry.provides(&node.definition, node.config_version, &provider)
            }),
            None => node.signature.is_none() && registry.definition(&node.definition).is_some(),
        };
        if !available {
            return PortResolution {
                contract: None,
                missing: Some(missing_package(node)),
            };
        }
    }
    let contract = if id == intrinsic_id::CALL {
        view::call_contract(document, node, registry).map(Cow::Owned)
    } else {
        registry.contract(document, node).ok().flatten()
    };
    PortResolution {
        contract,
        missing: None,
    }
}

pub(crate) fn fresh_contract<'a>(
    document: &GraphDocument,
    node: &NodeInstance,
    registry: &'a RegistrySnapshot,
) -> Option<Cow<'a, NodeContract>> {
    port_resolution(document, node, registry).contract
}

fn missing_package(node: &NodeInstance) -> MissingPackage {
    match &node.provider {
        Some(provider) => MissingPackage {
            package: provider.package.clone(),
            version: provider.version.clone(),
        },
        None => MissingPackage {
            package: node.definition.id.namespace().to_owned(),
            version: String::new(),
        },
    }
}

impl Host {
    /// Refuses edits on an absent provider before a lower layer can consult the
    /// saved signature for inference and repair.
    pub(crate) fn require_port_authority(
        &self,
        document: &GraphDocument,
        node: NodeId,
    ) -> Result<(), HostError> {
        let node = document
            .node(node)
            .ok_or_else(|| HostError::Document("the document has no such node".into()))?;
        let registry = self.registry_for(document);
        let resolved = port_resolution(document, node, &registry);
        if let Some(package) = resolved.missing {
            return Err(HostError::Document(format!(
                "the node's provider from `{}` is unavailable",
                package.package
            )));
        }
        if resolved.contract.is_none() && node.signature.is_some() {
            return Err(HostError::Document(
                "the current port contract is unavailable".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_model::DocCommand;

    #[test]
    fn reusing_an_edge_id_for_a_different_port_refreshes_warmed_cards() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        let mut draft = host.draft(&new_document().unwrap()).unwrap();
        let literal = host
            .add_node(&mut draft, intrinsic::WHOLE_NUMBER, [0.0, 0.0])
            .unwrap();
        let select = host
            .add_node(&mut draft, intrinsic::SELECT, [300.0, 0.0])
            .unwrap();
        host.connect_through(&mut draft, (literal, "result"), (select, "if_true"), None)
            .unwrap();
        let flags = |draft: &Draft| {
            let graph = host.draft_authoring_view(draft).unwrap().graph;
            let card = graph.nodes.iter().find(|card| card.id == select).unwrap();
            ["if_true", "if_false"].map(|name| {
                card.ports
                    .iter()
                    .find(|port| port.id == name)
                    .unwrap()
                    .connected
            })
        };
        assert_eq!(flags(&draft), [true, false]);
        let mut edge = draft
            .document()
            .root()
            .edges
            .values()
            .next()
            .unwrap()
            .clone();
        edge.to.port = "if_false".parse().unwrap();
        let registry = host.registry_for(draft.document());
        assert!(
            draft
                .commit(
                    &registry,
                    [
                        DocCommand::Disconnect { edge: edge.id },
                        DocCommand::Connect { edge }
                    ],
                )
                .unwrap()
        );
        assert_eq!(flags(&draft), [false, true]);
        assert!(draft.undo());
        assert_eq!(flags(&draft), [true, false]);
    }
}
