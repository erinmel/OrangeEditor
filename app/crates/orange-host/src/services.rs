//! How the host runs the native service packages an application builds in. The types a
//! package implements are in `orange-sdk`.
use orange_kernel::{BindingRequest, EffectBinding, ResolveError};
use orange_model::catalog::ProviderKey;
use orange_model::document::{DefinitionRef, NodeKind, PortDirection, PortKind};
use orange_model::id::{ConnectionId, PortKey};
use orange_sdk::{ServiceNode, ServicePackage, ServiceSource};
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a run's effect request carries for a service node.
pub(crate) struct ServiceEffect {
    pub node: Arc<dyn ServiceNode>,
    pub connection: Option<ConnectionId>,
    /// The item type, when the node's one data output is a stream.
    pub stream: Option<orange_model::id::TypeExpr>,
}

/// Every service node the host has, by exact provider and definition.
#[derive(Default, Clone)]
pub(crate) struct Services {
    nodes: BTreeMap<(ProviderKey, DefinitionRef), (Arc<dyn ServiceNode>, bool)>,
    sources: BTreeMap<(ProviderKey, DefinitionRef), (Arc<dyn ServiceSource>, bool)>,
}
impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field("nodes", &self.nodes.len())
            .finish()
    }
}
impl Services {
    pub fn add(&mut self, provider: &ProviderKey, package: &ServicePackage) {
        for (definition, node) in package.nodes() {
            let uses_connection = !definition.references.is_empty();
            let _ = self.nodes.insert(
                (provider.clone(), definition.definition.clone()),
                (Arc::clone(node), uses_connection),
            );
        }
        for (definition, source) in package.sources() {
            let uses_connection = !definition.references.is_empty();
            let _ = self.sources.insert(
                (provider.clone(), definition.definition.clone()),
                (Arc::clone(source), uses_connection),
            );
        }
    }

    /// The source of an entry node bound to `provider`, and whether it uses a connection.
    pub fn source(
        &self,
        provider: &ProviderKey,
        definition: &DefinitionRef,
    ) -> Option<(Arc<dyn ServiceSource>, bool)> {
        self.sources
            .get(&(provider.clone(), definition.clone()))
            .cloned()
    }

    /// Binds one async service node. Exec ports carry no value; any other non-data port
    /// or an unresolved type is refused.
    pub fn resolve(&self, request: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
        let provider = request.provider().ok_or(ResolveError::Unavailable)?;
        let (node, uses_connection) = self
            .nodes
            .get(&(provider.clone(), request.definition().clone()))
            .ok_or(ResolveError::Unavailable)?;
        if request.signature().kind != NodeKind::Async {
            return Err(ResolveError::Contract);
        }
        let (mut inputs, mut outputs) = (Vec::new(), Vec::new());
        let mut stream = None;
        for port in &request.signature().ports {
            match port.kind {
                PortKind::Data => {}
                PortKind::Exec => continue,
                _ => return Err(ResolveError::Contract),
            }
            let ty = port.ty.as_ref().ok_or(ResolveError::Contract)?;
            if ty.as_str().contains('$') {
                return Err(ResolveError::UnsupportedType);
            }
            let key: PortKey = port
                .port_id
                .as_str()
                .parse()
                .map_err(|_| ResolveError::Contract)?;
            if port.direction == PortDirection::Input {
                inputs.push(key);
            } else {
                if let Some(item) = ty
                    .as_str()
                    .strip_prefix("core/stream@1<")
                    .and_then(|rest| rest.strip_suffix('>'))
                {
                    stream = Some(item.parse().map_err(|_| ResolveError::Contract)?);
                }
                outputs.push(key);
            }
        }
        let connection = if *uses_connection {
            let text = request
                .config()
                .get("connection")
                .and_then(Json::as_str)
                .ok_or(ResolveError::Contract)?;
            Some(text.parse().map_err(|_| ResolveError::Contract)?)
        } else {
            None
        };
        // A stream is a node's only result.
        if stream.is_some() && outputs.len() != 1 {
            return Err(ResolveError::Contract);
        }
        Ok(EffectBinding {
            route: 0,
            context: Arc::new(ServiceEffect {
                node: Arc::clone(node),
                connection,
                stream,
            }),
            inputs: inputs.into(),
            outputs: outputs.into(),
            action: None,
        })
    }
}
