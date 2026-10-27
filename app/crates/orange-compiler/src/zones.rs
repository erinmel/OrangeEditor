//! Scoped dense lowering of the model's checked lexical zone directory.

use crate::calls::Calls;
use crate::compile::{CompileOptions, charge};
use crate::{CompileError, NodeIx};
use orange_model::GraphDocument;
use orange_model::id::NodeId;
use orange_model::validation::{ValidationAnalysis, ZoneKind};
use std::collections::BTreeMap;

/// Plan-local zone identity. Source UUIDs are retained separately for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ZoneIx(pub(crate) u32);

impl ZoneIx {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One zone in one specialized call scope. Unused admitted zones have no lowered
/// parameter instruction; runtime work is created only by an actual end demand.
#[derive(Debug, Clone)]
pub struct DataZone {
    pub(crate) kind: ZoneKind,
    pub(crate) begin: NodeId,
    pub(crate) end: NodeId,
    pub(crate) call: Option<usize>,
    pub(crate) parent: Option<ZoneIx>,
    pub(crate) parameters: Option<NodeIx>,
}

impl DataZone {
    pub fn kind(&self) -> ZoneKind {
        self.kind
    }
    pub fn begin(&self) -> NodeId {
        self.begin
    }
    pub fn end(&self) -> NodeId {
        self.end
    }
    pub fn call_instance(&self) -> Option<usize> {
        self.call
    }
    pub fn parent(&self) -> Option<ZoneIx> {
        self.parent
    }
    pub fn parameters(&self) -> Option<NodeIx> {
        self.parameters
    }
}

/// True is an end, false is a begin; only fresh checked intrinsic pairs count.
pub(crate) fn marker(analysis: &ValidationAnalysis, node: NodeId) -> Option<bool> {
    let zones = &analysis.report().regions.zones;
    if zones.get(node).is_some() {
        Some(true)
    } else {
        zones.owner(node).filter(|p| p.begin == node).map(|_| false)
    }
}

pub(crate) struct Zones {
    pub(crate) layouts: Vec<DataZone>,
    ends: BTreeMap<(usize, NodeId), ZoneIx>,
    inherited: Vec<Option<ZoneIx>>,
    contains: Vec<bool>,
}

impl Zones {
    /// Resolve all source identities before any provider binding. Calls are in
    /// parent-before-child order; lexical zone parents need not be in UUID order.
    pub(crate) fn prepare(
        document: &GraphDocument,
        analysis: &ValidationAnalysis,
        calls: &Calls<'_>,
        options: CompileOptions<'_>,
        bytes: &mut usize,
    ) -> Result<Self, CompileError> {
        let index = &analysis.report().regions.zones;
        charge(
            bytes,
            calls
                .scopes
                .len()
                .checked_mul(std::mem::size_of::<Option<ZoneIx>>() + 1)
                .ok_or(CompileError::ResourceLimit)?,
            options.limits.max_constant_bytes,
        )?;
        let mut result = Self {
            layouts: Vec::new(),
            ends: BTreeMap::new(),
            inherited: vec![None; calls.scopes.len()],
            contains: vec![false; calls.scopes.len()],
        };
        for scope in 0..calls.scopes.len() {
            options.checkpoint()?;
            let inherited = if scope == 0 {
                None
            } else {
                let site = &calls.sites[scope - 1];
                result.owner(analysis, site.parent.map_or(0, |p| p + 1), site.call)
            };
            result.inherited[scope] = inherited;
            let start = result.layouts.len();
            for node in calls.body(document, scope).nodes.keys() {
                options.checkpoint()?;
                let Some(pair) = index.get(*node) else {
                    continue;
                };
                if result.layouts.len() >= options.limits.max_graph_items {
                    return Err(CompileError::ResourceLimit);
                }
                charge(
                    bytes,
                    std::mem::size_of::<DataZone>() + 48,
                    options.limits.max_constant_bytes,
                )?;
                let ix = ZoneIx(
                    u32::try_from(result.layouts.len()).map_err(|_| CompileError::ResourceLimit)?,
                );
                result.ends.insert((scope, pair.end), ix);
                result.layouts.push(DataZone {
                    kind: pair.kind,
                    begin: pair.begin,
                    end: pair.end,
                    call: scope.checked_sub(1),
                    parent: None,
                    parameters: None,
                });
                result.contains[scope] = true;
            }
            for i in start..result.layouts.len() {
                let pair = index
                    .get(result.layouts[i].end)
                    .ok_or(CompileError::ValidationIncomplete)?;
                result.layouts[i].parent = match pair.parent {
                    Some(parent) => Some(
                        *result
                            .ends
                            .get(&(scope, parent))
                            .ok_or(CompileError::ValidationIncomplete)?,
                    ),
                    None => inherited,
                };
            }
        }
        // Constant calls containing any transitive zone must lower as programs,
        // rather than asking the scalar folder to unroll their entire result.
        for scope in (1..calls.scopes.len()).rev() {
            if result.contains[scope] {
                let parent = calls.sites[scope - 1].parent.map_or(0, |p| p + 1);
                result.contains[parent] = true;
            }
        }
        Ok(result)
    }

    pub(crate) fn end(&self, scope: usize, node: NodeId) -> Option<ZoneIx> {
        self.ends.get(&(scope, node)).copied()
    }

    pub(crate) fn owner(
        &self,
        analysis: &ValidationAnalysis,
        scope: usize,
        node: NodeId,
    ) -> Option<ZoneIx> {
        analysis
            .report()
            .regions
            .zones
            .owner(node)
            .and_then(|p| self.end(scope, p.end))
            .or(self.inherited[scope])
    }

    pub(crate) fn contains(&self, scope: usize) -> bool {
        self.contains[scope]
    }
}
