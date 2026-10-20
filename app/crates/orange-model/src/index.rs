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

use crate::change_set::{ChangeSet, DeclId};
use crate::document::{BodyRef, Edge, GraphBody, GraphDocument, PortRef, SignatureSnapshot};
use crate::id::{EdgeId, NodeId};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocIndex {
    nodes: HashMap<NodeId, BodyRef>,
    edges: HashMap<EdgeId, EdgeEntry>,
    incoming: HashMap<PortRef, BTreeSet<EdgeId>>,
    outgoing: HashMap<PortRef, BTreeSet<EdgeId>>,
    incident: HashMap<NodeId, BTreeSet<EdgeId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EdgeEntry {
    body: BodyRef,
    from: PortRef,
    to: PortRef,
}

impl DocIndex {
    pub fn build(document: &GraphDocument) -> Self {
        let mut index = Self::default();
        for (body_ref, body) in document.bodies_with_refs() {
            index.add_body(body_ref, body);
        }
        index
    }

    /// Brings the index up to date with `document` after the transaction that produced
    /// `change`. Cost is proportional to the change, plus the size of any function body
    /// the transaction replaced or removed.
    pub fn update(&mut self, document: &GraphDocument, change: &ChangeSet) {
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
                self.remove_body(body_ref);
                if let Some(body) = document.body(body_ref) {
                    self.add_body(body_ref, body);
                }
            }
        }
        let (nodes, edges) = (&change.nodes, &change.edges);
        for id in nodes.removed.iter() {
            self.nodes.remove(id);
        }
        for id in nodes.added.iter().chain(&nodes.changed) {
            if let Some(body) = document.locate_node(*id) {
                self.nodes.insert(*id, body);
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

    /// Edges at `node` that use a port `snapshot` does not declare (11 §4.5). They are kept
    /// in the document and shown, but not compiled.
    pub fn orphan_edges(&self, node: NodeId, snapshot: &SignatureSnapshot) -> Vec<EdgeId> {
        let undeclared =
            |end: &PortRef| end.node == node && !snapshot.declares_port(end.port.port());
        self.incident(node)
            .filter(|id| {
                let entry = &self.edges[id];
                undeclared(&entry.from) || undeclared(&entry.to)
            })
            .collect()
    }

    fn add_body(&mut self, body_ref: BodyRef, body: &GraphBody) {
        for id in body.nodes.keys() {
            self.nodes.insert(*id, body_ref);
        }
        for edge in body.edges.values() {
            self.insert_edge(body_ref, edge);
        }
    }

    fn remove_body(&mut self, body_ref: BodyRef) {
        self.nodes.retain(|_, body| *body != body_ref);
        let edges: Vec<EdgeId> = self
            .edges
            .iter()
            .filter(|(_, entry)| entry.body == body_ref)
            .map(|(id, _)| *id)
            .collect();
        for id in edges {
            self.remove_edge(id);
        }
    }

    fn insert_edge(&mut self, body: BodyRef, edge: &Edge) {
        let id = edge.id;
        self.incoming.entry(edge.to.clone()).or_default().insert(id);
        self.outgoing
            .entry(edge.from.clone())
            .or_default()
            .insert(id);
        self.incident.entry(edge.from.node).or_default().insert(id);
        self.incident.entry(edge.to.node).or_default().insert(id);
        self.edges.insert(
            id,
            EdgeEntry {
                body,
                from: edge.from.clone(),
                to: edge.to.clone(),
            },
        );
    }

    fn remove_edge(&mut self, id: EdgeId) {
        let Some(entry) = self.edges.remove(&id) else {
            return;
        };
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
