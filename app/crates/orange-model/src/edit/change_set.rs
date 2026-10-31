//! What a committed transaction changed (12 §3.3), as net per-ID effects.

use std::collections::{BTreeMap, BTreeSet};

use std::sync::Arc;

use super::patch::Patch;
use crate::document::literal::Literal;
use crate::document::{BodyRef, ConfigValue, Edge, GraphDocument, NodeInstance, TypeParamName};
use crate::id::{EdgeId, NodeId, PortKey, TypeExpr};

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
    literals: Option<Arc<NodeEdit>>,
    /// Present when the only semantic change adds or removes whole nodes and edges.
    structure: Option<Arc<StructureEdit>>,
}

/// Journal evidence that a transaction only added or removed whole nodes and edges: no
/// declaration or shared metadata changed, and no node or edge was replaced in place.
/// Holds each removed node and edge with its body; the added ones are the change set's.
#[derive(Debug, PartialEq)]
pub(crate) struct StructureEdit {
    pub removed_nodes: BTreeMap<NodeId, (BodyRef, Box<NodeInstance>)>,
    pub removed_edges: BTreeMap<EdgeId, (BodyRef, Edge)>,
}

// Node and edge payloads hold JSON values, which never hold NaN, so equality is total.
impl Eq for StructureEdit {}

/// Journal evidence that a transaction changed only node literals, type arguments, type
/// pins or configuration: each changed node kept its body and every other field. Holds
/// those fields of each node before the change.
#[derive(Debug, PartialEq)]
pub(crate) struct NodeEdit {
    pub before: BTreeMap<NodeId, NodeFields>,
}

// Literal payloads are JSON text values, which never hold NaN, so equality is total.
impl Eq for NodeEdit {}

/// The node fields a [`NodeEdit`] may change.
#[derive(Debug, PartialEq)]
pub(crate) struct NodeFields {
    pub inputs: BTreeMap<PortKey, Literal>,
    pub type_args: BTreeMap<TypeParamName, TypeExpr>,
    pub type_pins: BTreeMap<TypeParamName, TypeExpr>,
    pub config_version: u32,
    pub config: ConfigValue,
}

impl NodeFields {
    fn of(node: &NodeInstance) -> Self {
        Self {
            inputs: node.inputs.clone(),
            type_args: node.type_args.clone(),
            type_pins: node.type_pins.clone(),
            config_version: node.config_version,
            config: node.config.clone(),
        }
    }
}

impl ChangeSet {
    /// The prior literals when this change edited only node literals.
    pub(crate) fn node_edit(&self) -> Option<&NodeEdit> {
        self.literals.as_deref()
    }
    /// The removed nodes and edges when this change only added or removed whole nodes and
    /// edges; the added ones are `nodes().added` and `edges().added`.
    pub(crate) fn structure_edit(&self) -> Option<&StructureEdit> {
        self.structure.as_deref()
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
            structure: None,
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
    /// Each touched edge's first prior value, kept while the transaction may still be an
    /// edge edit; `None` once it cannot be.
    prior_edges: Option<BTreeMap<EdgeId, Option<(BodyRef, Edge)>>>,
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
            prior_edges: Some(BTreeMap::new()),
        }
    }
}

impl Tracker {
    /// `inverse` is what applying the patch returned; its entry is the slot's prior value.
    pub fn record(&mut self, inverse: &Patch) {
        self.semantic |= !inverse.is_layout();
        if !matches!(
            inverse,
            Patch::Node { .. } | Patch::Edge { .. } | Patch::Editor { .. }
        ) {
            self.prior = None;
            self.prior_edges = None;
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
            Patch::Edge { body, id, entry } => {
                self.edges.entry(*id).or_insert(entry.is_some());
                if let Some(prior) = &mut self.prior_edges
                    && !prior.contains_key(id)
                {
                    if prior.len() < LITERAL_EDIT_NODES {
                        prior.insert(*id, entry.as_ref().map(|e| (*body, e.value.clone())));
                    } else {
                        self.prior_edges = None;
                    }
                }
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

    /// The removed edges when every semantic change added or removed whole edges.
    fn structure_edit(&self, document: &GraphDocument) -> Option<StructureEdit> {
        let (nodes, edges) = (self.prior.as_ref()?, self.prior_edges.as_ref()?);
        if !self.semantic || self.global_dirty || nodes.is_empty() && edges.is_empty() {
            return None;
        }
        let mut removed_nodes = BTreeMap::new();
        for (id, before) in nodes {
            match (before, document.locate_node(*id)) {
                (None, Some(_)) => {}
                (Some(before), None) => {
                    removed_nodes.insert(*id, before.clone());
                }
                // Replaced in place, or added and removed again.
                _ => return None,
            }
        }
        let mut removed_edges = BTreeMap::new();
        for (id, before) in edges {
            match (before, document.locate_edge(*id)) {
                (None, Some(_)) => {}
                (Some(before), None) => {
                    removed_edges.insert(*id, before.clone());
                }
                _ => return None,
            }
        }
        Some(StructureEdit {
            removed_nodes,
            removed_edges,
        })
    }

    /// The prior fields when every semantic change replaced only literals, type arguments,
    /// pins or configuration of nodes that exist before and after in the same body. A
    /// replacement by equal values, such as setting a literal to its current value, is a
    /// node edit too, which a session re-checks in place.
    fn node_edit(&self, document: &GraphDocument) -> Option<NodeEdit> {
        let prior = self.prior.as_ref()?;
        if !self.semantic
            || self.global_dirty
            || prior.is_empty()
            || self
                .prior_edges
                .as_ref()
                .is_none_or(|edges| !edges.is_empty())
        {
            return None;
        }
        let mut before = BTreeMap::new();
        for (id, entry) in prior {
            let (body, old) = entry.as_ref()?;
            if document.locate_node(*id) != Some(*body) {
                return None;
            }
            let now = document.body(*body)?.nodes.get(id)?;
            let mut same = (**old).clone();
            same.inputs.clone_from(&now.inputs);
            same.type_args.clone_from(&now.type_args);
            same.type_pins.clone_from(&now.type_pins);
            same.config_version = now.config_version;
            same.config.clone_from(&now.config);
            if same != *now {
                return None;
            }
            before.insert(*id, NodeFields::of(old));
        }
        Some(NodeEdit { before })
    }

    pub fn finish(
        self,
        document: &GraphDocument,
        from_revision: u64,
        from_state: u64,
        from_semantic_state: u64,
        committed: bool,
    ) -> ChangeSet {
        let literals = self.node_edit(document).map(Arc::new);
        let structure = self.structure_edit(document).map(Arc::new);
        ChangeSet {
            literals,
            structure,
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
