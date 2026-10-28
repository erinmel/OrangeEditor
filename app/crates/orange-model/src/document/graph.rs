//! Graph storage, body lookup and content identity. Editing is owned by crate::edit.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Map, Value as Json};

use super::literal::Literal;
use super::{
    Edge, EventDecl, Extensions, FunctionDecl, NodeInstance, PortRef, SignatureSnapshot,
    VariableDecl,
};
use crate::id::{
    EdgeId, EventDeclId, FunctionId, GraphId, NodeId, PortKey, SignatureKey, TypeExpr, VariableId,
};

/// A fresh content-state identity; see [`GraphDocument::state_id`].
pub(crate) fn fresh_state_id() -> u64 {
    crate::id::process_unique()
}

/// The format v1 document, without the `"format"` envelope tag, which the codec owns.
///
/// Serde deserialization is checked: it applies the same version, cross-body identity and
/// lock checks as [`from_json`](crate::from_json), and accepts an optional `"format"` tag
/// only when it names this format. Duplicate-key detection and import quotas need the
/// text, so they stay with the bounded JSON importer.
#[derive(Debug, Clone)]
pub struct GraphDocument {
    pub(crate) format_version: u32,
    pub(crate) graph_id: GraphId,
    pub(crate) revision: u64,
    /// Identifies the content within this session; see [`GraphDocument::state_id`].
    pub(crate) state_id: u64,
    pub(crate) semantic_state_id: u64,
    pub(crate) package_lock: Vec<PackageLockEntry>,
    pub(crate) type_lock: Vec<TypeLockEntry>,
    pub(crate) variables: IndexMap<VariableId, VariableDecl>,
    pub(crate) events: IndexMap<EventDeclId, EventDecl>,
    pub(crate) signatures: BTreeMap<SignatureKey, SignatureSnapshot>,
    /// Top-level body, persisted as the document's own `nodes` and `edges`.
    pub(crate) root: GraphBody,
    pub(crate) functions: IndexMap<FunctionId, FunctionDecl>,
    pub(crate) exports: Vec<GraphExport>,
    /// Layout only; never read by the compiler. Typed by the editor work, opaque here.
    pub(crate) editor: Map<String, Json>,
    pub(crate) extensions: Extensions,
    /// The body holding each node: built on the first lookup, then kept current by every
    /// patch, so lookups stay constant-time across edits. Derived state, never compared.
    pub(crate) node_bodies: OnceLock<HashMap<NodeId, BodyRef>>,
}

/// The wire layout of [`GraphDocument`], the one place its Serde attributes live. Its
/// generated functions are private, so the unchecked deserializer stays inside the crate;
/// Serde checks at compile time that the field list matches the document.
#[derive(Serialize, Deserialize)]
#[serde(remote = "GraphDocument")]
pub(crate) struct DocumentWire {
    format_version: u32,
    graph_id: GraphId,
    revision: u64,
    #[serde(skip, default = "fresh_state_id")]
    state_id: u64,
    #[serde(skip, default = "fresh_state_id")]
    semantic_state_id: u64,
    #[serde(default)]
    package_lock: Vec<PackageLockEntry>,
    #[serde(default)]
    type_lock: Vec<TypeLockEntry>,
    #[serde(default, with = "crate::document::keyed")]
    variables: IndexMap<VariableId, VariableDecl>,
    #[serde(default, with = "crate::document::keyed")]
    events: IndexMap<EventDeclId, EventDecl>,
    #[serde(default)]
    signatures: BTreeMap<SignatureKey, SignatureSnapshot>,
    #[serde(flatten)]
    root: GraphBody,
    #[serde(default, with = "crate::document::keyed")]
    functions: IndexMap<FunctionId, FunctionDecl>,
    #[serde(default)]
    exports: Vec<GraphExport>,
    #[serde(default)]
    editor: Map<String, Json>,
    #[serde(flatten)]
    extensions: Extensions,
    #[serde(skip)]
    node_bodies: OnceLock<HashMap<NodeId, BodyRef>>,
}

impl Serialize for GraphDocument {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        DocumentWire::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for GraphDocument {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let document = DocumentWire::deserialize(deserializer)?;
        super::codec::admit(document).map_err(de::Error::custom)
    }
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
            semantic_state_id: fresh_state_id(),
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
            node_bodies: OnceLock::new(),
        }
    }

    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    pub fn graph_id(&self) -> GraphId {
        self.graph_id
    }

    pub fn type_lock(&self) -> &[TypeLockEntry] {
        &self.type_lock
    }

    pub fn exports(&self) -> &[GraphExport] {
        &self.exports
    }

    /// Unknown top-level fields, preserved on save. Read-only: no edit command changes them,
    /// so content and [`GraphDocument::state_id`] stay in step.
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
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

    /// Identity of executable content. Layout edits preserve it; semantic edits replace
    /// it, undo/redo restore it, clones share it and each load gets a fresh identity.
    pub fn semantic_state_id(&self) -> u64 {
        self.semantic_state_id
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

    /// Exact package versions and content digests the document's provider bindings use,
    /// one entry per package version. Changed only by `PutPackageLock`.
    pub fn package_lock(&self) -> &[PackageLockEntry] {
        &self.package_lock
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

    /// The body holding `node`; constant-time after the first lookup.
    pub fn locate_node(&self, node: NodeId) -> Option<BodyRef> {
        self.node_bodies
            .get_or_init(|| {
                self.bodies_with_refs()
                    .flat_map(|(body_ref, body)| body.nodes.keys().map(move |id| (*id, body_ref)))
                    .collect()
            })
            .get(&node)
            .copied()
    }

    /// Records that a patch put `nodes` into `body` (`present`) or took them out of it.
    pub(crate) fn track_nodes(
        &mut self,
        body: BodyRef,
        nodes: impl IntoIterator<Item = NodeId>,
        present: bool,
    ) {
        let Some(map) = self.node_bodies.get_mut() else {
            return;
        };
        for node in nodes {
            if present {
                map.insert(node, body);
            } else {
                map.remove(&node);
            }
        }
    }

    pub fn locate_edge(&self, edge: EdgeId) -> Option<BodyRef> {
        self.bodies_with_refs()
            .find(|(_, body)| body.edges.contains_key(&edge))
            .map(|(body, _)| body)
    }

    pub fn node(&self, node: NodeId) -> Option<&NodeInstance> {
        self.body(self.locate_node(node)?)?.nodes.get(&node)
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
