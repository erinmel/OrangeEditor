//! What a committed transaction changed (12 §3.3), as net per-ID effects.

use std::collections::{BTreeMap, BTreeSet};

use std::sync::Arc;

use super::patch::Patch;
use crate::document::literal::Literal;
use crate::document::{BodyRef, GraphDocument, NodeInstance};
use crate::id::{EdgeId, NodeId, PortKey};

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
    /// Present when the only semantic change is to the literals of existing nodes.
    literals: Option<Arc<LiteralEdit>>,
}

/// Journal evidence that a transaction changed only node literals: each changed node
/// kept its body and every other field. Holds each node's literals before the change.
#[derive(Debug, PartialEq)]
pub(crate) struct LiteralEdit {
    pub before: BTreeMap<NodeId, BTreeMap<PortKey, Literal>>,
}

// Literal payloads are JSON text values, which never hold NaN, so equality is total.
impl Eq for LiteralEdit {}

impl ChangeSet {
    /// The prior literals when this change edited only node literals.
    pub(crate) fn literal_edit(&self) -> Option<&LiteralEdit> {
        self.literals.as_deref()
    }
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
            literals: None,
        }
    }
}

/// Records, per touched ID, whether it existed before its first patch.
pub(crate) struct Tracker {
    nodes: BTreeMap<NodeId, bool>,
    edges: BTreeMap<EdgeId, bool>,
    decls: BTreeMap<DeclId, bool>,
    semantic: bool,
    global_dirty: bool,
    /// Each touched node's first prior value, kept while the transaction may still be a
    /// literal edit; `None` once it cannot be.
    prior: Option<PriorNodes>,
}

/// Each touched node's body and value before its first patch; `None` for an added node.
type PriorNodes = BTreeMap<NodeId, Option<(BodyRef, Box<NodeInstance>)>>;

/// More touched nodes than this are not tracked as a literal edit, so a large paste
/// or deletion never clones its nodes for it.
const LITERAL_EDIT_NODES: usize = 64;

impl Default for Tracker {
    fn default() -> Self {
        Self {
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            decls: BTreeMap::new(),
            semantic: false,
            global_dirty: false,
            prior: Some(BTreeMap::new()),
        }
    }
}

impl Tracker {
    /// `inverse` is what applying the patch returned; its entry is the slot's prior value.
    pub fn record(&mut self, inverse: &Patch) {
        self.semantic |= !inverse.is_layout();
        if !matches!(inverse, Patch::Node { .. } | Patch::Editor { .. }) {
            self.prior = None;
        }
        match inverse {
            Patch::Node { body, id, entry } => {
                self.nodes.entry(*id).or_insert(entry.is_some());
                if let Some(prior) = &mut self.prior
                    && !prior.contains_key(id)
                {
                    if prior.len() < LITERAL_EDIT_NODES {
                        prior.insert(*id, entry.as_ref().map(|e| (*body, e.value.clone())));
                    } else {
                        self.prior = None;
                    }
                }
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

    /// The prior literals when every semantic change replaced only literals of nodes
    /// that exist before and after in the same body, and some literal differs.
    fn literal_edit(&self, document: &GraphDocument) -> Option<LiteralEdit> {
        let prior = self.prior.as_ref()?;
        if !self.semantic || self.global_dirty || prior.is_empty() {
            return None;
        }
        let mut changed = false;
        let mut before = BTreeMap::new();
        for (id, entry) in prior {
            let (body, old) = entry.as_ref()?;
            if document.locate_node(*id) != Some(*body) {
                return None;
            }
            let now = document.body(*body)?.nodes.get(id)?;
            let mut same = (**old).clone();
            same.inputs.clone_from(&now.inputs);
            if same != *now {
                return None;
            }
            changed |= old.inputs != now.inputs;
            before.insert(*id, old.inputs.clone());
        }
        changed.then_some(LiteralEdit { before })
    }

    pub fn finish(
        self,
        document: &GraphDocument,
        from_revision: u64,
        from_state: u64,
        from_semantic_state: u64,
        committed: bool,
    ) -> ChangeSet {
        let literals = self.literal_edit(document).map(Arc::new);
        ChangeSet {
            literals,
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
