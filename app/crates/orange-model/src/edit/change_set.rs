//! What a committed transaction changed (12 §3.3), as net per-ID effects.

use std::collections::{BTreeMap, BTreeSet};

use super::patch::Patch;
use crate::document::GraphDocument;
use crate::id::{EdgeId, NodeId};

pub use crate::id::DeclId;

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

impl<K: Ord + Clone> Delta<K> {
    /// The effect of undoing this delta.
    pub(crate) fn reversed(&self) -> Self {
        Self {
            added: self.removed.clone(),
            removed: self.added.clone(),
            changed: self.changed.clone(),
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
///
/// Only the transaction journal constructs this evidence. Readers may inspect or clone
/// it, but cannot forge a layout edit or change the affected IDs.
///
/// ```compile_fail
/// fn forge(mut change: orange_model::ChangeSet) {
///     change.layout_only = true;
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    pub(crate) from_revision: u64,
    pub(crate) to_revision: u64,
    from_state: u64,
    to_state: u64,
    from_semantic_state: u64,
    to_semantic_state: u64,
    committed: bool,
    global_dirty: bool,
    /// True when only editor layout changed: no validation, compile or preview work (G-36).
    pub(crate) layout_only: bool,
    pub(crate) nodes: Delta<NodeId>,
    pub(crate) edges: Delta<EdgeId>,
    pub(crate) decls: Delta<DeclId>,
}

impl ChangeSet {
    pub fn from_revision(&self) -> u64 {
        self.from_revision
    }
    pub fn to_revision(&self) -> u64 {
        self.to_revision
    }
    /// Whole-content identity before this transaction.
    pub fn from_state(&self) -> u64 {
        self.from_state
    }
    /// Whole-content identity after this transaction.
    pub fn to_state(&self) -> u64 {
        self.to_state
    }
    pub fn from_semantic_state(&self) -> u64 {
        self.from_semantic_state
    }
    pub fn to_semantic_state(&self) -> u64 {
        self.to_semantic_state
    }
    /// False for private rolled-back trial edits; they never establish new analysis evidence.
    pub fn is_committed(&self) -> bool {
        self.committed
    }
    /// Shared signatures or package locks changed. Consumers without those reverse
    /// dependencies must rebuild instead of treating empty ID deltas as no work.
    pub fn global_dirty(&self) -> bool {
        self.global_dirty
    }
    pub fn layout_only(&self) -> bool {
        self.layout_only
    }
    pub fn nodes(&self) -> &Delta<NodeId> {
        &self.nodes
    }
    pub fn edges(&self) -> &Delta<EdgeId> {
        &self.edges
    }
    pub fn decls(&self) -> &Delta<DeclId> {
        &self.decls
    }

    /// The change set of undoing this one.
    pub(crate) fn reversed(&self) -> Self {
        Self {
            from_revision: self.to_revision,
            to_revision: self.from_revision,
            from_state: self.to_state,
            to_state: self.from_state,
            from_semantic_state: self.to_semantic_state,
            to_semantic_state: self.from_semantic_state,
            committed: false,
            global_dirty: self.global_dirty,
            layout_only: self.layout_only,
            nodes: self.nodes.reversed(),
            edges: self.edges.reversed(),
            decls: self.decls.reversed(),
        }
    }
}

/// Records, per touched ID, whether it existed before its first patch.
#[derive(Default)]
pub(crate) struct Tracker {
    nodes: BTreeMap<NodeId, bool>,
    edges: BTreeMap<EdgeId, bool>,
    decls: BTreeMap<DeclId, bool>,
    semantic: bool,
    global_dirty: bool,
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
            Patch::Signature { .. }
            | Patch::PackageLock { .. }
            | Patch::TypeLock { .. }
            | Patch::Export { .. } => self.global_dirty = true,
            Patch::Editor { .. } => {}
        }
    }

    pub fn layout_only(&self) -> bool {
        !self.semantic
    }

    pub fn finish(
        self,
        document: &GraphDocument,
        from_revision: u64,
        from_state: u64,
        from_semantic_state: u64,
        committed: bool,
    ) -> ChangeSet {
        ChangeSet {
            from_revision,
            to_revision: document.revision(),
            from_state,
            to_state: document.state_id(),
            from_semantic_state,
            to_semantic_state: document.semantic_state_id(),
            committed,
            global_dirty: self.global_dirty,
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
