//! Trusted native services: nodes the host process implements itself, and the
//! connections they use.
//!
//! This is the tier for work that needs the operating system: sockets, files, devices.
//! A [`ServicePackage`] is compiled into the application, not installed, and its nodes
//! are asynchronous: a run issues a request and a worker thread performs it, so no
//! service code runs on a flow's own thread.
//!
//! A connection is declared in a document or shared through the host. An adapter turns
//! its settings into a [`Session`]; nodes that declare they use a connection receive the
//! session the node's `connection` config field names.
use orange_kernel::{BindingRequest, EffectBinding, KernelFault, ResolveError};
use orange_model::Value;
use orange_model::catalog::{NodeDefinition, ProviderKey};
use orange_model::document::{DefinitionRef, NodeKind, PortDirection, PortKind};
use orange_model::id::{ConnectionId, PortKey};
use orange_runtime::EffectLease;
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Provider ID of every node a service package implements.
pub const SERVICE_PROVIDER: &str = "native";

/// A live connection. The adapter that opened it defines what it holds; its nodes get
/// it back through [`Session::as_any`]. Dropping the last reference closes it.
pub trait Session: std::fmt::Debug + Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
}

/// Opens sessions of one adapter kind from declared settings.
///
/// `open` runs on the thread that opens a document and must not block on the network:
/// an adapter that needs a handshake performs it on first use, from a worker.
pub trait ConnectionAdapter: std::fmt::Debug + Send + Sync + 'static {
    fn open(&self, settings: &Map<String, Json>) -> Result<Arc<dyn Session>, String>;
}

/// One execution of a service node, on a worker thread.
pub struct Call<'a> {
    /// The node's data inputs in declared order.
    pub inputs: &'a [Value],
    /// The session of the connection the node names, when its definition uses one.
    pub session: Option<&'a dyn Session>,
    /// Cancelled when the run no longer wants the result; long work should poll it.
    pub lease: &'a EffectLease,
}
impl std::fmt::Debug for Call<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Call")
            .field("inputs", &self.inputs.len())
            .field("session", &self.session.is_some())
            .finish_non_exhaustive()
    }
}

/// The implementation of one asynchronous service node. It returns the node's data
/// outputs in declared order, or a fault that takes the node's `failed` route.
pub trait ServiceNode: Send + Sync + 'static {
    fn run(&self, call: &Call<'_>) -> Result<Vec<Value>, KernelFault>;
}

/// A set of native nodes and connection adapters built into the application.
pub struct ServicePackage {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) nodes: Vec<(NodeDefinition, Arc<dyn ServiceNode>)>,
    pub(crate) adapters: Vec<(String, Arc<dyn ConnectionAdapter>)>,
}
impl std::fmt::Debug for ServicePackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServicePackage")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("nodes", &self.nodes.len())
            .field("adapters", &self.adapters.len())
            .finish()
    }
}
impl ServicePackage {
    /// `id` is the package namespace its definitions and adapter kinds live in.
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            nodes: Vec::new(),
            adapters: Vec::new(),
        }
    }
    /// Adds an asynchronous node from its definition file text.
    pub fn node(
        mut self,
        definition: &str,
        node: impl ServiceNode,
    ) -> Result<Self, orange_model::catalog::DefinitionError> {
        let definition = NodeDefinition::from_json(definition)?;
        self.nodes.push((definition, Arc::new(node)));
        Ok(self)
    }
    /// Adds the adapter for connections whose `adapter` is `kind`.
    pub fn adapter(mut self, kind: impl Into<String>, adapter: impl ConnectionAdapter) -> Self {
        self.adapters.push((kind.into(), Arc::new(adapter)));
        self
    }
}

/// What a run's effect request carries for a service node.
pub(crate) struct ServiceEffect {
    pub node: Arc<dyn ServiceNode>,
    pub connection: Option<ConnectionId>,
}

/// Every service node the host has, by exact provider and definition.
#[derive(Default, Clone)]
pub(crate) struct Services {
    nodes: BTreeMap<(ProviderKey, DefinitionRef), (Arc<dyn ServiceNode>, bool)>,
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
        for (definition, node) in &package.nodes {
            let uses_connection = !definition.references.is_empty();
            let _ = self.nodes.insert(
                (provider.clone(), definition.definition.clone()),
                (Arc::clone(node), uses_connection),
            );
        }
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
        Ok(EffectBinding {
            route: 0,
            context: Arc::new(ServiceEffect {
                node: Arc::clone(node),
                connection,
            }),
            inputs: inputs.into(),
            outputs: outputs.into(),
            action: None,
        })
    }
}
