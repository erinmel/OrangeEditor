//! Derived lookup structures over a document (12 §3.2). Never persisted.
//!
//! Kept beside the document rather than inside it, so document equality, serialization and
//! undo never involve derived data. Build once with [`DocIndex::build`], then apply each
//! committed [`ChangeSet`] with [`DocIndex::update`]. An updated index always equals a
//! fresh build of the same document.
//!
//! The rows of 12 §3.2 that need port kinds, types or regions (topological order, type
//! components, region index, diagnostics) belong to the validator (00c/00d).

use std::collections::{BTreeSet, HashMap};

use crate::catalog::instance_port;
use crate::document::{
    BodyRef, Edge, GraphBody, GraphDocument, NodeInstance, PortRef, SignatureSnapshot,
};
use crate::edit::{ChangeSet, DeclId};
use crate::id::{EdgeId, FunctionId, NodeId};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocIndex {
    /// The document content state this index describes; 0 (never a state) for a default index.
    state: u64,
    nodes: HashMap<NodeId, BodyRef>,
    edges: HashMap<EdgeId, EdgeEntry>,
    incoming: HashMap<PortRef, BTreeSet<EdgeId>>,
    outgoing: HashMap<PortRef, BTreeSet<EdgeId>>,
    incident: HashMap<NodeId, BTreeSet<EdgeId>>,
    /// Membership is retained only for function bodies. Root nodes and edges need
    /// no secondary directory because the root body is never wholesale replaced.
    function_members: HashMap<FunctionId, BodyMembers>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BodyMembers {
    nodes: BTreeSet<NodeId>,
    edges: BTreeSet<EdgeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EdgeEntry {
    body: BodyRef,
    from: PortRef,
    to: PortRef,
}

impl DocIndex {
    pub fn build(document: &GraphDocument) -> Self {
        let mut index = Self {
            state: document.state_id(),
            ..Self::default()
        };
        for (body_ref, body) in document.bodies_with_refs() {
            index.add_body(body_ref, body);
        }
        index
    }

    /// Brings the index up to date with `document` after the transaction that produced
    /// `change`. Function membership lookups are average O(1); ordered membership and
    /// incidence-set edits are O(log n) in the affected function or endpoint degree.
    /// Replacing or removing a function visits only its previous members, not unrelated
    /// root or function contents.
    pub fn update(&mut self, document: &GraphDocument, change: &ChangeSet) {
        if self.state != change.from_state() || document.state_id() != change.to_state() {
            *self = Self::build(document);
            return;
        }
        self.state = document.state_id();
        if change.layout_only {
            return;
        }
        let decls = &change.decls;
        for decl in decls
            .added
            .iter()
            .chain(&decls.removed)
            .chain(&decls.changed)
        {
            if let DeclId::Function(id) = decl {
                let body_ref = BodyRef::Function(*id);
                self.remove_body(body_ref, document);
                if let Some(body) = document.body(body_ref) {
                    self.add_body(body_ref, body);
                }
            }
        }
        let (nodes, edges) = (&change.nodes, &change.edges);
        for id in nodes.removed.iter() {
            self.remove_node(*id);
        }
        for id in nodes.added.iter().chain(&nodes.changed) {
            if let Some(body) = document.locate_node(*id) {
                self.insert_node(body, *id);
            }
        }
        for id in edges.removed.iter().chain(&edges.changed) {
            self.remove_edge(*id);
        }
        for id in edges.added.iter().chain(&edges.changed) {
            if let (Some(body), Some(edge)) = (document.locate_edge(*id), document.edge(*id)) {
                self.insert_edge(body, edge);
            }
        }
    }

    /// Whether this index describes the current content of `document`. An index goes stale
    /// when the document changes without [`DocIndex::update`], for example by an undo.
    pub fn is_current(&self, document: &GraphDocument) -> bool {
        self.state == document.state_id()
    }

    pub(crate) fn state_id(&self) -> u64 {
        self.state
    }

    /// The body holding `node`.
    pub fn body_of(&self, node: NodeId) -> Option<BodyRef> {
        self.nodes.get(&node).copied()
    }

    /// Edges ending at `port`. More than one means a fan-in violation for data inputs,
    /// which the validator reports.
    pub fn incoming(&self, port: &PortRef) -> impl Iterator<Item = EdgeId> + '_ {
        self.incoming.get(port).into_iter().flatten().copied()
    }

    pub fn outgoing(&self, port: &PortRef) -> impl Iterator<Item = EdgeId> + '_ {
        self.outgoing.get(port).into_iter().flatten().copied()
    }

    /// Every edge with `node` at either end.
    pub fn incident(&self, node: NodeId) -> impl Iterator<Item = EdgeId> + '_ {
        self.incident.get(&node).into_iter().flatten().copied()
    }

    /// Edges at `node` whose endpoint is not a port of the node under `snapshot` (11 §4.5),
    /// by the validator's rule: an undeclared port, a variadic instance missing from the
    /// node's variadic state, or a bare variadic base port. Pass the node's fresh resolved
    /// signature, such as `ValidationReport::resolved`, to match the validator. Orphaned
    /// edges are kept in the document and shown, but not compiled.
    pub fn orphan_edges(&self, node: &NodeInstance, snapshot: &SignatureSnapshot) -> Vec<EdgeId> {
        let undeclared = |end: &PortRef| {
            end.node == node.id && instance_port(node, snapshot, &end.port).is_none()
        };
        self.incident(node.id)
            .filter(|id| {
                let entry = &self.edges[id];
                undeclared(&entry.from) || undeclared(&entry.to)
            })
            .collect()
    }

    fn add_body(&mut self, body_ref: BodyRef, body: &GraphBody) {
        if let BodyRef::Function(id) = body_ref {
            self.function_members.entry(id).or_default();
        }
        for id in body.nodes.keys() {
            self.insert_node(body_ref, *id);
        }
        for edge in body.edges.values() {
            self.insert_edge(body_ref, edge);
        }
    }

    fn remove_body(&mut self, body_ref: BodyRef, document: &GraphDocument) {
        let BodyRef::Function(function) = body_ref else {
            return;
        };
        // Keep the map entry, even for an empty function, so incremental and fresh
        // builds have the same private representation. Taking the sets lets removals
        // proceed without scanning unrelated root or function membership.
        let members = self.function_members.entry(function).or_default();
        let nodes = std::mem::take(&mut members.nodes);
        let edges = std::mem::take(&mut members.edges);
        for id in nodes {
            // A transaction may move an ID into another function before this body is
            // visited. The index owner guard preserves that newer location.
            if self.nodes.get(&id) == Some(&body_ref) {
                self.remove_node(id);
            }
        }
        for id in edges {
            if self
                .edges
                .get(&id)
                .is_some_and(|entry| entry.body == body_ref)
            {
                self.remove_edge(id);
            }
        }
        if document.body(body_ref).is_none() {
            self.function_members.remove(&function);
        }
    }

    fn insert_node(&mut self, body: BodyRef, id: NodeId) {
        let previous = self.nodes.insert(id, body);
        if previous == Some(body) {
            return;
        }
        if let Some(BodyRef::Function(function)) = previous
            && let Some(members) = self.function_members.get_mut(&function)
        {
            members.nodes.remove(&id);
        }
        if let BodyRef::Function(function) = body {
            self.function_members
                .entry(function)
                .or_default()
                .nodes
                .insert(id);
        }
    }

    fn remove_node(&mut self, id: NodeId) {
        let Some(body) = self.nodes.remove(&id) else {
            return;
        };
        if let BodyRef::Function(function) = body
            && let Some(members) = self.function_members.get_mut(&function)
        {
            members.nodes.remove(&id);
        }
    }

    fn insert_edge(&mut self, body: BodyRef, edge: &Edge) {
        let id = edge.id;
        let entry = EdgeEntry {
            body,
            from: edge.from.clone(),
            to: edge.to.clone(),
        };
        if let Some(previous) = self.edges.insert(id, entry) {
            self.remove_edge_entry(id, previous);
        }
        self.incoming.entry(edge.to.clone()).or_default().insert(id);
        self.outgoing
            .entry(edge.from.clone())
            .or_default()
            .insert(id);
        self.incident.entry(edge.from.node).or_default().insert(id);
        self.incident.entry(edge.to.node).or_default().insert(id);
        if let BodyRef::Function(function) = body {
            self.function_members
                .entry(function)
                .or_default()
                .edges
                .insert(id);
        }
    }

    fn remove_edge(&mut self, id: EdgeId) {
        let Some(entry) = self.edges.remove(&id) else {
            return;
        };
        self.remove_edge_entry(id, entry);
    }

    fn remove_edge_entry(&mut self, id: EdgeId, entry: EdgeEntry) {
        if let BodyRef::Function(function) = entry.body
            && let Some(members) = self.function_members.get_mut(&function)
        {
            members.edges.remove(&id);
        }
        remove_from(&mut self.incoming, &entry.to, id);
        remove_from(&mut self.outgoing, &entry.from, id);
        remove_from(&mut self.incident, &entry.from.node, id);
        remove_from(&mut self.incident, &entry.to.node, id);
    }
}

/// Removes `id` from the set at `key`, dropping the set when it empties so the index
/// equals a fresh build.
fn remove_from<K: std::hash::Hash + Eq>(
    map: &mut HashMap<K, BTreeSet<EdgeId>>,
    key: &K,
    id: EdgeId,
) {
    if let Some(set) = map.get_mut(key) {
        set.remove(&id);
        if set.is_empty() {
            map.remove(key);
        }
    }
}
