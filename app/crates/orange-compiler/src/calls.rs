//! Bounded, iterative specialization of graph-local pure calls. No document edits,
//! provider callbacks, synthetic UUIDs, or persisted inference caches are involved.
use crate::compile::{CompileOptions, Incoming, charge, key, port_type};
use crate::{CompileError, PureCallSite};
use orange_model::GraphDocument;
use orange_model::catalog::{RegistrySnapshot, TypeArgs};
use orange_model::document::{GraphBody, NodeKind, PortDirection, PortKind, SignatureSnapshot};
use orange_model::id::{FunctionId, NodeId};
use orange_model::signature::try_substitute_params;
use orange_model::type_term::TypeTerm;
use orange_model::validation::ValidationAnalysis;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct CallScope {
    pub(crate) signatures: BTreeMap<NodeId, SignatureSnapshot>,
    pub(crate) calls: BTreeMap<NodeId, usize>,
    lazy: BTreeMap<NodeId, crate::lazy::Lazy>,
    pub(crate) entry: Option<NodeId>,
    pub(crate) result: Option<NodeId>,
    function: Option<FunctionId>,
    parent: Option<usize>,
    depth: usize,
}
impl CallScope {
    pub(crate) fn signature<'a>(
        &'a self,
        analysis: &'a ValidationAnalysis,
        node: NodeId,
    ) -> &'a SignatureSnapshot {
        self.signatures
            .get(&node)
            .unwrap_or(&analysis.report().resolved[&node])
    }
}
pub(crate) struct Calls<'a> {
    pub(crate) scopes: Vec<CallScope>,
    pub(crate) incoming: Incoming<'a>,
    pub(crate) sites: Vec<PureCallSite>,
}

fn bind(pattern: &TypeTerm, actual: &TypeTerm, args: &mut TypeArgs) -> Result<(), CompileError> {
    match (pattern, actual) {
        (TypeTerm::Param(name), _) => {
            let ty = actual
                .to_string()
                .parse()
                .map_err(|_| CompileError::ValidationIncomplete)?;
            if args.get(name.as_str()).is_some_and(|old| old != &ty) {
                return Err(CompileError::ValidationIncomplete);
            }
            args.insert(name.clone(), ty);
            Ok(())
        }
        (TypeTerm::Named(a), TypeTerm::Named(b))
            if a.namespace == b.namespace
                && a.name == b.name
                && a.major == b.major
                && a.args.len() == b.args.len() =>
        {
            for (a, b) in a.args.iter().zip(&b.args) {
                bind(a, b, args)?;
            }
            Ok(())
        }
        _ => Err(CompileError::ValidationIncomplete),
    }
}

impl<'a> Calls<'a> {
    pub(crate) fn prepare(
        document: &'a GraphDocument,
        snapshot: &RegistrySnapshot,
        analysis: &ValidationAnalysis,
        options: CompileOptions<'_>,
    ) -> Result<Self, CompileError> {
        let mut out = Self {
            scopes: vec![CallScope::default()],
            incoming: BTreeMap::new(),
            sites: vec![],
        };
        if !document
            .root()
            .nodes
            .values()
            .any(|n| n.definition.id.as_str() == "core/call")
        {
            return Ok(out);
        }
        let mut items = document
            .root()
            .nodes
            .len()
            .checked_add(document.root().edges.len())
            .and_then(|n| n.checked_add(document.exports().len()))
            .ok_or(CompileError::ResourceLimit)?;
        let mut operations = document.root().nodes.len();
        let mut slots = 0;
        let mut cursor = 0;
        // Every potential invocation is admitted, including unselected lazy arms.
        while cursor < out.scopes.len() {
            options.checkpoint()?;
            let body: &GraphBody = match out.scopes[cursor].function {
                Some(id) => &document.functions()[&id].body,
                None => document.root(),
            };
            let connected: BTreeSet<_> = body
                .edges
                .values()
                .map(|e| (e.to.node, e.to.port.clone()))
                .collect();
            for node in body.nodes.values() {
                options.checkpoint()?;
                let sig = out.scopes[cursor].signature(analysis, node.id);
                if cursor != 0 {
                    let marker = matches!(
                        node.definition.id.as_str(),
                        "core/function-entry" | "core/function-outputs"
                    );
                    if !marker && sig.kind != NodeKind::Pure {
                        return Err(CompileError::UnsupportedNode { node: node.id });
                    }
                    let lazy = out.scopes[cursor].lazy.get(&node.id);
                    if node.definition.id.is_core()
                        && !marker
                        && lazy.is_none()
                        && !matches!(node.definition.id.as_str(), "core/call" | "core/convert")
                    {
                        return Err(CompileError::UnsupportedNode { node: node.id });
                    }
                    for port in &sig.ports {
                        if port.kind != PortKind::Data
                            || port.is_optional()
                            || port.variadic.is_some()
                            || (port.is_lazy() && lazy.is_none())
                        {
                            return Err(CompileError::UnsupportedPort {
                                node: node.id,
                                port: key(port),
                            });
                        }
                        port_type(node.id, port)?;
                    }
                }
                // Conservative upper bound, charged before any callback/scratch output.
                for port in &sig.ports {
                    if port.direction == PortDirection::Output
                        || !connected.contains(&(node.id, key(port)))
                    {
                        charge(
                            &mut slots,
                            1,
                            options.limits.max_slots.min(u32::MAX as usize),
                        )?;
                    }
                }
                if node.definition.id.as_str() != "core/call" {
                    continue;
                }
                if sig.kind != NodeKind::Pure {
                    return Err(CompileError::UnsupportedNode { node: node.id });
                }
                let interface = orange_model::catalog::call_interface(document, node, snapshot)
                    .map_err(|_| CompileError::ValidationIncomplete)?
                    .ok_or(CompileError::ValidationIncomplete)?;
                let function = &document.functions()[&interface.function];
                // Defensive recursion check independent of semantic analysis, iterative.
                let depth = out.scopes[cursor]
                    .depth
                    .checked_add(1)
                    .filter(|d| *d <= options.limits.max_call_depth)
                    .ok_or(CompileError::ResourceLimit)?;
                let mut ancestor = Some(cursor);
                while let Some(index) = ancestor {
                    if out.scopes[index].function == Some(function.id) {
                        return Err(CompileError::ValidationIncomplete);
                    }
                    ancestor = out.scopes[index].parent;
                }
                let mut args = TypeArgs::new();
                for declared in &interface.ports {
                    let actual = sig
                        .ports
                        .iter()
                        .find(|p| {
                            p.port_id == declared.port_id && p.direction == declared.direction
                        })
                        .ok_or(CompileError::ValidationIncomplete)?;
                    let a = TypeTerm::parse(
                        declared
                            .ty
                            .as_ref()
                            .ok_or(CompileError::ValidationIncomplete)?,
                    )
                    .map_err(|_| CompileError::ValidationIncomplete)?;
                    let b = TypeTerm::parse(
                        actual
                            .ty
                            .as_ref()
                            .ok_or(CompileError::ValidationIncomplete)?,
                    )
                    .map_err(|_| CompileError::ValidationIncomplete)?;
                    bind(&a, &b, &mut args)?;
                }
                // Executable evidence currently exposes fresh port types, not an
                // authenticated type-argument map for erased/phantom parameters.
                // Never recover such arguments from the saved inference cache.
                if function
                    .type_params
                    .iter()
                    .any(|p| !args.contains_key(&p.name))
                {
                    return Err(CompileError::UnsupportedNode { node: node.id });
                }
                charge(
                    &mut operations,
                    function.body.nodes.len().max(1),
                    options.limits.max_fold_operations,
                )?;
                charge(
                    &mut items,
                    function.body.nodes.len(),
                    options.limits.max_graph_items,
                )?;
                charge(
                    &mut items,
                    function.body.edges.len(),
                    options.limits.max_graph_items,
                )?;
                let mut scope = CallScope {
                    function: Some(function.id),
                    parent: Some(cursor),
                    depth,
                    ..CallScope::default()
                };
                for child in function.body.nodes.values() {
                    options.checkpoint()?;
                    let original = analysis
                        .report()
                        .resolved
                        .get(&child.id)
                        .ok_or(CompileError::ValidationIncomplete)?;
                    charge(
                        &mut items,
                        original.ports.len(),
                        options.limits.max_graph_items,
                    )?;
                    let mut sig = original.clone();
                    if child.definition.id.as_str() == "core/function-entry" {
                        scope.entry = Some(child.id);
                        // The marker's implicit exec output is structural, not a data argument.
                        sig.ports.retain(|p| p.kind == PortKind::Data);
                    }
                    if child.definition.id.as_str() == "core/function-outputs" {
                        scope.result = Some(child.id);
                    }
                    for port in &mut sig.ports {
                        if let Some(ty) = &port.ty {
                            port.ty = Some(
                                try_substitute_params(ty, &args)
                                    .map_err(|_| CompileError::ValidationIncomplete)?,
                            );
                        }
                    }
                    if !child.definition.id.is_core() {
                        let contract = snapshot
                            .contract(document, child)
                            .map_err(|_| CompileError::ValidationIncomplete)?
                            .ok_or(CompileError::ValidationIncomplete)?;
                        let mut concrete = TypeArgs::new();
                        for declared in &contract.ports {
                            let Some(pattern) = &declared.ty else {
                                continue;
                            };
                            let actual = sig
                                .ports
                                .iter()
                                .find(|p| {
                                    p.port_id == declared.port_id
                                        && p.direction == declared.direction
                                })
                                .and_then(|p| p.ty.as_ref())
                                .ok_or(CompileError::ValidationIncomplete)?;
                            bind(
                                &TypeTerm::parse(pattern)
                                    .map_err(|_| CompileError::ValidationIncomplete)?,
                                &TypeTerm::parse(actual)
                                    .map_err(|_| CompileError::ValidationIncomplete)?,
                                &mut concrete,
                            )?;
                        }
                        let provider = child
                            .provider
                            .as_ref()
                            .and_then(|p| orange_model::catalog::ProviderKey::of(document, p));
                        if snapshot
                            .specializations(&child.definition, provider.as_ref())
                            .is_some_and(|supported| !supported.contains(&concrete))
                        {
                            return Err(CompileError::UnsupportedNode { node: child.id });
                        }
                    }
                    if let Some(lazy) = crate::lazy::Lazy::resolve(child, &sig)? {
                        scope.lazy.insert(child.id, lazy);
                    }
                    scope.signatures.insert(child.id, sig);
                }
                if !function.outputs.is_empty() && scope.result.is_none() {
                    return Err(CompileError::ValidationIncomplete);
                }
                let index = out.scopes.len();
                out.scopes[cursor].calls.insert(node.id, index);
                out.scopes.push(scope);
                out.sites.push(PureCallSite {
                    call: node.id,
                    function: function.id,
                    parent: cursor.checked_sub(1),
                });
                for edge in function.body.edges.values() {
                    out.incoming
                        .insert((edge.to.node, edge.to.port.clone()), edge);
                }
            }
            cursor += 1;
        }
        if slots
            .checked_mul(16)
            .is_none_or(|n| n > options.limits.max_constant_bytes)
        {
            return Err(CompileError::ResourceLimit);
        }
        Ok(out)
    }
    pub(crate) fn lazy(
        &self,
        root: &BTreeMap<NodeId, crate::lazy::Lazy>,
        scope: usize,
        node: NodeId,
    ) -> Option<crate::lazy::Lazy> {
        if scope == 0 {
            root.get(&node).copied()
        } else {
            self.scopes[scope].lazy.get(&node).copied()
        }
    }
    pub(crate) fn signature<'b>(
        &'b self,
        analysis: &'b ValidationAnalysis,
        scope: usize,
        node: NodeId,
    ) -> &'b SignatureSnapshot {
        self.scopes[scope].signature(analysis, node)
    }
    pub(crate) fn incoming<'b>(&'b self, root: &'b Incoming<'a>, scope: usize) -> &'b Incoming<'a> {
        if scope == 0 { root } else { &self.incoming }
    }
}
