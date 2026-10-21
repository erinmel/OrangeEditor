//! Graph storage, body lookup and content identity. Editing is owned by crate::edit.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

use super::literal::Literal;
use super::{
    Edge, EventDecl, Extensions, FunctionDecl, NodeInstance, PortRef, SignatureSnapshot,
    VariableDecl,
};
use crate::id::{
    EdgeId, EventDeclId, FunctionId, GraphId, NodeId, PortKey, SignatureKey, TypeExpr, VariableId,
};

static NEXT_STATE_ID: AtomicU64 = AtomicU64::new(1);

/// A process-unique content-state identity. `fetch_add` works on every supported toolchain
/// (`fetch_update` is deprecated on 1.99, and `try_update` may postdate the 1.97.1 minimum).
pub(crate) fn fresh_state_id() -> u64 {
    let id = NEXT_STATE_ID.fetch_add(1, Ordering::Relaxed);
    assert_ne!(
        id,
        u64::MAX,
        "document content-state identity space exhausted"
    );
    id
}

/// The format v1 document, without the `"format"` envelope tag, which the codec owns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphDocument {
    pub format_version: u32,
    pub graph_id: GraphId,
    pub(crate) revision: u64,
    /// Identifies the content within this session; see [`GraphDocument::state_id`].
    #[serde(skip, default = "fresh_state_id")]
    pub(crate) state_id: u64,
    #[serde(default)]
    pub package_lock: Vec<PackageLockEntry>,
    #[serde(default)]
    pub type_lock: Vec<TypeLockEntry>,
    #[serde(default, with = "crate::document::keyed")]
    pub(crate) variables: IndexMap<VariableId, VariableDecl>,
    #[serde(default, with = "crate::document::keyed")]
    pub(crate) events: IndexMap<EventDeclId, EventDecl>,
    #[serde(default)]
    pub(crate) signatures: BTreeMap<SignatureKey, SignatureSnapshot>,
    /// Top-level body, persisted as the document's own `nodes` and `edges`.
    #[serde(flatten)]
    pub(crate) root: GraphBody,
    #[serde(default, with = "crate::document::keyed")]
    pub(crate) functions: IndexMap<FunctionId, FunctionDecl>,
    #[serde(default)]
    pub exports: Vec<GraphExport>,
    /// Layout only; never read by the compiler. Typed by the editor work, opaque here.
    #[serde(default)]
    pub(crate) editor: Map<String, Json>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl PartialEq for GraphDocument {
    fn eq(&self, other: &Self) -> bool {
        self.format_version == other.format_version
            && self.graph_id == other.graph_id
            && self.revision == other.revision
            && self.package_lock == other.package_lock
            && self.type_lock == other.type_lock
            && self.variables == other.variables
            && self.events == other.events
            && self.signatures == other.signatures
            && self.root == other.root
            && self.functions == other.functions
            && self.exports == other.exports
            && self.editor == other.editor
            && self.extensions == other.extensions
    }
}

/// Which body of a document holds a node or edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BodyRef {
    Root,
    Function(FunctionId),
}

impl GraphDocument {
    /// An empty format v1 document at revision 0.
    pub fn new(graph_id: GraphId) -> Self {
        Self {
            format_version: 1,
            graph_id,
            revision: 0,
            state_id: fresh_state_id(),
            package_lock: Vec::new(),
            type_lock: Vec::new(),
            variables: IndexMap::new(),
            events: IndexMap::new(),
            signatures: BTreeMap::new(),
            root: GraphBody::default(),
            functions: IndexMap::new(),
            exports: Vec::new(),
            editor: Map::new(),
            extensions: Extensions::new(),
        }
    }

    /// +1 per committed transaction, undo and redo included.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Opaque identity of the current content state, used to bind undo/redo inverses.
    /// Clones preserve it; each new edit gets a unique identity, while undo/redo restore
    /// the identity of the content they return to. Not persisted: each load gets a new one.
    pub fn state_id(&self) -> u64 {
        self.state_id
    }

    pub fn root(&self) -> &GraphBody {
        &self.root
    }

    pub fn variables(&self) -> &IndexMap<VariableId, VariableDecl> {
        &self.variables
    }

    pub fn events(&self) -> &IndexMap<EventDeclId, EventDecl> {
        &self.events
    }

    pub fn functions(&self) -> &IndexMap<FunctionId, FunctionDecl> {
        &self.functions
    }

    pub fn signatures(&self) -> &BTreeMap<SignatureKey, SignatureSnapshot> {
        &self.signatures
    }

    pub fn editor(&self) -> &Map<String, Json> {
        &self.editor
    }

    /// The root body followed by every function body.
    pub fn bodies(&self) -> impl Iterator<Item = &GraphBody> {
        self.bodies_with_refs().map(|(_, body)| body)
    }

    /// Bodies with their references, root first.
    pub fn bodies_with_refs(&self) -> impl Iterator<Item = (BodyRef, &GraphBody)> {
        let functions = self
            .functions
            .iter()
            .map(|(id, function)| (BodyRef::Function(*id), &function.body));
        std::iter::once((BodyRef::Root, &self.root)).chain(functions)
    }

    pub fn body(&self, body: BodyRef) -> Option<&GraphBody> {
        match body {
            BodyRef::Root => Some(&self.root),
            BodyRef::Function(id) => self.functions.get(&id).map(|function| &function.body),
        }
    }

    /// The body holding `node`. Linear in the number of bodies; use [`crate::DocIndex`]
    /// for repeated lookups over the same document.
    pub fn locate_node(&self, node: NodeId) -> Option<BodyRef> {
        self.bodies_with_refs()
            .find(|(_, body)| body.nodes.contains_key(&node))
            .map(|(body, _)| body)
    }

    pub fn locate_edge(&self, edge: EdgeId) -> Option<BodyRef> {
        self.bodies_with_refs()
            .find(|(_, body)| body.edges.contains_key(&edge))
            .map(|(body, _)| body)
    }

    pub fn node(&self, node: NodeId) -> Option<&NodeInstance> {
        self.bodies().find_map(|body| body.nodes.get(&node))
    }

    pub fn edge(&self, edge: EdgeId) -> Option<&Edge> {
        self.bodies().find_map(|body| body.edges.get(&edge))
    }

    /// The instance literal the compiler would use for `port`: `None` when there is none or
    /// when a connected edge shadows it (11 §4.4). A shadowed literal stays in the document.
    pub fn active_literal(&self, port: &PortRef) -> Option<&Literal> {
        let body = self.body(self.locate_node(port.node)?)?;
        let literal = body.nodes.get(&port.node)?.inputs.get(&port.port)?;
        let shadowed = body.edges.values().any(|edge| edge.to == *port);
        (!shadowed).then_some(literal)
    }
}

/// Nodes and edges of one body. Edges never cross bodies.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphBody {
    #[serde(default, with = "crate::document::keyed")]
    pub nodes: IndexMap<NodeId, NodeInstance>,
    #[serde(default, with = "crate::document::keyed")]
    pub edges: IndexMap<EdgeId, Edge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageLockEntry {
    pub id: String,
    pub version: String,
    pub content_digest: String,
    #[serde(flatten)]
    pub extensions: Extensions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeLockEntry {
    #[serde(rename = "type")]
    pub ty: TypeExpr,
    pub codec: String,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// An output evaluated for preview or the CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphExport {
    pub id: String,
    pub node: NodeId,
    pub port: PortKey,
    #[serde(flatten)]
    pub extensions: Extensions,
}
