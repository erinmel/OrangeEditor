//! What a committed transaction changed (12 §3.3), as net per-ID effects.

use std::collections::{BTreeMap, BTreeSet};

use super::patch::Patch;
use crate::document::GraphDocument;
use crate::id::{EdgeId, EventDeclId, FunctionId, NodeId, VariableId};

/// A graph-level declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeclId {
    Variable(VariableId),
    Event(EventDeclId),
    /// Nodes and edges inside a replaced or removed function body are reported through
    /// their function, not individually.
    Function(FunctionId),
}

/// Net effect on one ID space. An ID added and removed in the same transaction is absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delta<K: Ord> {
    pub added: BTreeSet<K>,
    pub removed: BTreeSet<K>,
    /// Present before and after; touched by the transaction (possibly to an equal value).
    pub changed: BTreeSet<K>,
}

impl<K: Ord> Default for Delta<K> {
    fn default() -> Self {
        Self {
            added: BTreeSet::new(),
            removed: BTreeSet::new(),
            changed: BTreeSet::new(),
        }
    }
}

impl<K: Ord> Delta<K> {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Result of one committed transaction. Region-root compile scope (`affected_roots` in
/// 12 §3.3) is derived from these deltas by the compiler once regions exist (00d/00e).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    pub from_revision: u64,
    pub to_revision: u64,
    /// True when only editor layout changed: no validation, compile or preview work (G-36).
    pub layout_only: bool,
    pub nodes: Delta<NodeId>,
    pub edges: Delta<EdgeId>,
    pub decls: Delta<DeclId>,
}

/// Records, per touched ID, whether it existed before its first patch.
#[derive(Default)]
pub(crate) struct Tracker {
    nodes: BTreeMap<NodeId, bool>,
    edges: BTreeMap<EdgeId, bool>,
    decls: BTreeMap<DeclId, bool>,
    semantic: bool,
}

impl Tracker {
    /// `inverse` is what applying the patch returned; its entry is the slot's prior value.
    pub fn record(&mut self, inverse: &Patch) {
        self.semantic |= !inverse.is_layout();
        match inverse {
            Patch::Node { id, entry, .. } => {
                self.nodes.entry(*id).or_insert(entry.is_some());
            }
            Patch::Edge { id, entry, .. } => {
                self.edges.entry(*id).or_insert(entry.is_some());
            }
            Patch::Variable { id, entry } => {
                self.decls
                    .entry(DeclId::Variable(*id))
                    .or_insert(entry.is_some());
            }
            Patch::Event { id, entry } => {
                self.decls
                    .entry(DeclId::Event(*id))
                    .or_insert(entry.is_some());
            }
            Patch::Function { id, entry } => {
                self.decls
                    .entry(DeclId::Function(*id))
                    .or_insert(entry.is_some());
            }
            Patch::Signature { .. } | Patch::Editor { .. } => {}
        }
    }

    pub fn finish(self, document: &GraphDocument, from_revision: u64) -> ChangeSet {
        ChangeSet {
            from_revision,
            to_revision: document.revision(),
            layout_only: !self.semantic,
            nodes: delta(self.nodes, |id| document.locate_node(*id).is_some()),
            edges: delta(self.edges, |id| document.locate_edge(*id).is_some()),
            decls: delta(self.decls, |id| match id {
                DeclId::Variable(id) => document.variables().contains_key(id),
                DeclId::Event(id) => document.events().contains_key(id),
                DeclId::Function(id) => document.functions().contains_key(id),
            }),
        }
    }
}

fn delta<K: Ord>(before: BTreeMap<K, bool>, exists_now: impl Fn(&K) -> bool) -> Delta<K> {
    let mut delta = Delta::default();
    for (key, existed) in before {
        match (existed, exists_now(&key)) {
            (false, true) => delta.added.insert(key),
            (true, false) => delta.removed.insert(key),
            (true, true) => delta.changed.insert(key),
            (false, false) => false,
        };
    }
    delta
}
