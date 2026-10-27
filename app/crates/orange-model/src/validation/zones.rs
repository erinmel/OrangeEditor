//! Shared intrinsic pairing and the checked, compact lexical zone directory.

use crate::catalog::intrinsic_id as ids;
use std::collections::{BTreeMap, BTreeSet};

use crate::document::GraphBody;
use crate::id::{FunctionId, NodeId};

/// Language-owned pure iteration families. Packages cannot introduce new families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ZoneKind {
    Repeat,
    Map,
    Fold,
    Filter,
}

impl ZoneKind {
    /// Exact intrinsic identity and whether it is the end marker.
    pub(crate) fn marker(definition: &str) -> Option<(Self, bool)> {
        Some(match definition {
            ids::REPEAT_BEGIN => (Self::Repeat, false),
            ids::REPEAT_END => (Self::Repeat, true),
            ids::MAP_BEGIN => (Self::Map, false),
            ids::MAP_END => (Self::Map, true),
            ids::FOLD_BEGIN => (Self::Fold, false),
            ids::FOLD_END => (Self::Fold, true),
            ids::FILTER_BEGIN => (Self::Filter, false),
            ids::FILTER_END => (Self::Filter, true),
            _ => return None,
        })
    }
}

/// One structurally checked zone. Pair identity is local to its source body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneRegion {
    pub kind: ZoneKind,
    pub begin: NodeId,
    pub end: NodeId,
    pub function: Option<FunctionId>,
    /// End marker of the immediately enclosing zone, if any.
    pub parent: Option<NodeId>,
}

/// Compact lexical ownership from the region pass, without duplicated nested body sets.
/// A body with a zone structure or paired-type error publishes no zone entries. This directory alone
/// does not certify types, dependency cycles, metadata completeness or compile readiness.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ZoneIndex {
    pub(crate) pairs: BTreeMap<NodeId, ZoneRegion>,
    pub(crate) owners: BTreeMap<NodeId, NodeId>,
}

impl ZoneIndex {
    /// Zone identified by its end marker.
    pub fn get(&self, end: NodeId) -> Option<&ZoneRegion> {
        self.pairs.get(&end)
    }

    /// All checked pairs in deterministic end-marker order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &ZoneRegion> {
        self.pairs.values()
    }

    /// Innermost zone whose iteration owns this node. Begin projections belong to
    /// their own zone; an end result belongs to the enclosing zone, if any.
    /// Captures outside all bodies have no owner.
    pub fn owner(&self, node: NodeId) -> Option<&ZoneRegion> {
        self.get(*self.owners.get(&node)?)
    }
}

pub(crate) struct Pairing {
    pub(crate) pairs: Vec<ZoneRegion>,
    pub(crate) faults: BTreeSet<NodeId>,
}

/// Pair only exact intrinsic families in one body. Both the region and dependency
/// passes use this projection; malformed pairing is diagnosed by the region pass.
pub(crate) fn pair_zones(body: &GraphBody, function: Option<FunctionId>) -> Pairing {
    let mut groups = BTreeMap::<(ZoneKind, &str), (Vec<NodeId>, Vec<NodeId>)>::new();
    let mut faults = BTreeSet::new();
    for node in body.nodes.values() {
        let Some((kind, end)) = ZoneKind::marker(node.definition.id.as_str()) else {
            continue;
        };
        match node.config.get("zone").and_then(serde_json::Value::as_str) {
            Some(zone) if !zone.is_empty() => {
                let group = groups.entry((kind, zone)).or_default();
                if end { &mut group.1 } else { &mut group.0 }.push(node.id);
            }
            _ => {
                faults.insert(node.id);
            }
        }
    }
    let mut pairs = Vec::new();
    for ((kind, _), (begins, ends)) in groups {
        if let ([begin], [end]) = (&begins[..], &ends[..]) {
            pairs.push(ZoneRegion {
                kind,
                begin: *begin,
                end: *end,
                function,
                parent: None,
            });
        } else {
            faults.extend(begins.into_iter().chain(ends));
        }
    }
    Pairing { pairs, faults }
}
