//! The OrangeEditor plugin SDK: what a Rust crate implements to add nodes, triggers and
//! connection kinds to a host application.
//!
//! A plugin builds a [`ServicePackage`]: definition files for its nodes, a
//! [`ServiceNode`] or [`ServiceSource`] behind each, and a [`ConnectionAdapter`] per
//! connection kind. The application passes the package to its host. This crate has no
//! package store, compiler front end or WebAssembly engine, so a plugin links only the
//! graph model, the value types and the run's effect lease.
//!
//! This is the trusted tier for work that needs the operating system: sockets, files,
//! devices. Nodes are asynchronous: a run issues a request and a worker thread performs
//! it, so no plugin code runs on a flow's own thread.
//!
//! A connection is declared in a document or shared through the host. An adapter turns
//! its settings into a [`Session`]; nodes that declare they use a connection receive the
//! session the node's `connection` config field names.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub use orange_kernel::{FaultText, KernelFault};
pub use orange_model::Value;
pub use orange_model::catalog::{DefinitionError, NodeDefinition, Presentation, TextRef};
pub use orange_model::document::DefinitionRef;
pub use orange_runtime::EffectLease;
use serde_json::{Map, Value as Json};
use std::any::Any;
use std::sync::Arc;

/// Provider ID of every node a service package implements.
pub const SERVICE_PROVIDER: &str = "native";

/// A live connection. The adapter that opened it defines what it holds; its nodes get
/// it back through [`Session::as_any`]. Dropping the last reference closes it.
pub trait Session: std::fmt::Debug + Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    /// Called each time a flow that uses the session has started all of its triggers,
    /// so more than once for a session several flows share. A session that receives
    /// should not begin before the first call: what arrives earlier has no trigger to
    /// reach and would be lost. The default does nothing.
    fn start(&self) {}
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

/// Why an [`Emitter`] did not deliver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitError {
    /// The flow's mailbox is full. The source decides whether to retry or drop.
    Busy,
    /// The flow has ended; the source should stop.
    Closed,
}

/// Fires one entry node of one flow. Cloneable and usable from any thread; firing never
/// blocks. What a paused or busy flow does with a firing is the entry's own policy.
#[derive(Clone)]
pub struct Emitter(Arc<Fire>);
type Fire = dyn Fn(Option<Value>) -> Result<(), EmitError> + Send + Sync;
impl std::fmt::Debug for Emitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emitter").finish_non_exhaustive()
    }
}
impl Emitter {
    /// Wraps the host's way of firing one entry node. Hosts call this; a plugin only
    /// receives emitters.
    pub fn new(
        fire: impl Fn(Option<Value>) -> Result<(), EmitError> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(fire))
    }
    /// Fires the entry with its payload, or `None` for an entry that declares none.
    pub fn emit(&self, payload: Option<Value>) -> Result<(), EmitError> {
        (self.0)(payload)
    }
}

/// The implementation of one entry node: something outside the graph that fires it.
///
/// `start` is called once per entry node when its flow opens and must not block. It
/// returns a guard; dropping the guard stops the source for that node.
pub trait ServiceSource: Send + Sync + 'static {
    fn start(
        &self,
        session: Option<Arc<dyn Session>>,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String>;
}

/// A set of native nodes and connection adapters built into the application.
pub struct ServicePackage {
    id: String,
    version: String,
    nodes: Vec<(NodeDefinition, Arc<dyn ServiceNode>)>,
    sources: Vec<(NodeDefinition, Arc<dyn ServiceSource>)>,
    adapters: Vec<(String, Arc<dyn ConnectionAdapter>)>,
    presentations: Vec<(DefinitionRef, Presentation)>,
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
            sources: Vec::new(),
            adapters: Vec::new(),
            presentations: Vec::new(),
        }
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    /// The asynchronous nodes, each with its definition.
    pub fn nodes(&self) -> &[(NodeDefinition, Arc<dyn ServiceNode>)] {
        &self.nodes
    }
    /// The entry nodes, each with its definition.
    pub fn sources(&self) -> &[(NodeDefinition, Arc<dyn ServiceSource>)] {
        &self.sources
    }
    /// The connection adapters by adapter kind.
    pub fn adapters(&self) -> &[(String, Arc<dyn ConnectionAdapter>)] {
        &self.adapters
    }
    /// What each definition file says about showing its node.
    pub fn presentations(&self) -> &[(DefinitionRef, Presentation)] {
        &self.presentations
    }
    /// Adds an asynchronous node from its definition file text.
    pub fn node(
        mut self,
        definition: &str,
        node: impl ServiceNode,
    ) -> Result<Self, DefinitionError> {
        let (definition, presentation) = NodeDefinition::described(definition)?;
        self.presentations
            .push((definition.definition.clone(), presentation));
        self.nodes.push((definition, Arc::new(node)));
        Ok(self)
    }
    /// Adds an entry node from its definition file text. The definition's kind is
    /// `entry`; it has a `then` outcome and at most one data output, `payload`.
    pub fn source(
        mut self,
        definition: &str,
        source: impl ServiceSource,
    ) -> Result<Self, DefinitionError> {
        let (definition, presentation) = NodeDefinition::described(definition)?;
        self.presentations
            .push((definition.definition.clone(), presentation));
        self.sources.push((definition, Arc::new(source)));
        Ok(self)
    }
    /// Adds the adapter for connections whose `adapter` is `kind`.
    pub fn adapter(mut self, kind: impl Into<String>, adapter: impl ConnectionAdapter) -> Self {
        self.adapters.push((kind.into(), Arc::new(adapter)));
        self
    }
}
