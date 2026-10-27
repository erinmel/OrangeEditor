//! Iterative call admission. Immutable specialized bodies are shared; invocation
//! output maps and memoization remain separate in the folder.
use crate::compile::{CompileOptions, Incoming, admit_data_ports, charge, key};
use crate::{CompileError, PureCallSite};
use orange_model::GraphDocument;
use orange_model::catalog::{Ports, ReferenceKind, RegistrySnapshot, TypeArgs};
use orange_model::document::{GraphBody, NodeKind, OutputSource, SignatureSnapshot};
use orange_model::edit::DeclId;
use orange_model::id::{FunctionId, NodeId};
use orange_model::signature::try_substitute_params;
use orange_model::type_term::TypeTerm;
use orange_model::validation::ValidationAnalysis;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallProfile {
    Pure,
    HostData,
    Control,
}

#[derive(Default)]
struct PreparedBody<'a> {
    signatures: BTreeMap<NodeId, Cow<'a, SignatureSnapshot>>,
    lazy: BTreeMap<NodeId, crate::lazy::Lazy>,
    entry: Option<NodeId>,
    result: Option<NodeId>,
}
#[derive(Default)]
pub(crate) struct CallScope<'a> {
    pub(crate) control: bool,
    body: Option<Arc<PreparedBody<'a>>>,
    pub(crate) calls: BTreeMap<NodeId, usize>,
    function: Option<FunctionId>,
    parent: Option<usize>,
    depth: usize,
}
pub(crate) struct Calls<'a> {
    pub(crate) scopes: Vec<CallScope<'a>>,
    pub(crate) incoming: Incoming<'a>,
    pub(crate) sites: Vec<PureCallSite>,
    pub(crate) signature_bytes: usize,
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
// Charge owned payload/metadata before cloning a specialized signature. This is
// conservative retained-copy accounting, not an allocator/RSS measurement.
pub(crate) fn charge_json(
    value: &serde_json::Value,
    used: &mut usize,
    options: CompileOptions<'_>,
) -> Result<(), CompileError> {
    let mut work = vec![value];
    while let Some(value) = work.pop() {
        options.checkpoint()?;
        charge(used, 16, options.limits.max_constant_bytes)?;
        match value {
            serde_json::Value::String(s) => {
                charge(used, s.len(), options.limits.max_constant_bytes)?
            }
            serde_json::Value::Array(values) => work.extend(values),
            serde_json::Value::Object(values) => {
                for (key, value) in values {
                    charge(used, key.len(), options.limits.max_constant_bytes)?;
                    work.push(value);
                }
            }
            _ => {}
        }
    }
    Ok(())
}
fn specialize<'a>(
    original: &'a SignatureSnapshot,
    args: &TypeArgs,
    used: &mut usize,
    options: CompileOptions<'_>,
) -> Result<Cow<'a, SignatureSnapshot>, CompileError> {
    if args.is_empty() {
        return Ok(Cow::Borrowed(original));
    }
    let mut types = Vec::with_capacity(original.ports.len());
    let mut changed = false;
    for port in &original.ports {
        options.checkpoint()?;
        let ty = port
            .ty
            .as_ref()
            .map(|ty| try_substitute_params(ty, args))
            .transpose()
            .map_err(|_| CompileError::ValidationIncomplete)?;
        changed |= ty != port.ty;
        types.push(ty);
    }
    if !changed {
        return Ok(Cow::Borrowed(original));
    }
    for (key, value) in &original.extensions {
        charge(used, key.len(), options.limits.max_constant_bytes)?;
        charge_json(value, used, options)?;
    }
    for (port, ty) in original.ports.iter().zip(&types) {
        options.checkpoint()?;
        charge(
            used,
            64usize
                .saturating_add(port.port_id.as_str().len())
                .saturating_add(ty.as_ref().map_or(0, |ty| ty.as_str().len())),
            options.limits.max_constant_bytes,
        )?;
        if let Some(default) = &port.default {
            match &default.payload {
                orange_model::document::literal::LiteralPayload::Value(value) => {
                    charge_json(value, used, options)?
                }
                orange_model::document::literal::LiteralPayload::Bits(bits) => {
                    charge(used, bits.len(), options.limits.max_constant_bytes)?
                }
            }
            charge(
                used,
                default.display.as_ref().map_or(0, String::len),
                options.limits.max_constant_bytes,
            )?;
        }
        for (key, value) in &port.extensions {
            charge(used, key.len(), options.limits.max_constant_bytes)?;
            charge_json(value, used, options)?;
        }
    }
    let mut signature = original.clone();
    for (port, ty) in signature.ports.iter_mut().zip(types) {
        port.ty = ty;
    }
    Ok(Cow::Owned(signature))
}
impl<'a> Calls<'a> {
    pub(crate) fn prepare(
        document: &'a GraphDocument,
        snapshot: &RegistrySnapshot,
        analysis: &'a ValidationAnalysis,
        options: CompileOptions<'_>,
        profile: CallProfile,
    ) -> Result<Self, CompileError> {
        let mut out = Self {
            scopes: vec![CallScope::default()],
            incoming: BTreeMap::new(),
            sites: vec![],
            signature_bytes: 0,
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
        let mut cache = BTreeMap::<(FunctionId, TypeArgs), Arc<PreparedBody<'a>>>::new();
        let mut cursor = 0;
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
                let sig = out.signature(analysis, cursor, node.id);
                if cursor != 0 {
                    let effect = profile == CallProfile::Control && out.scopes[cursor].control;
                    let control =
                        effect && !matches!(sig.kind, NodeKind::Pure | NodeKind::Volatile);
                    let marker = matches!(
                        node.definition.id.as_str(),
                        "core/function-entry" | "core/function-outputs" | "core/return"
                    );
                    let state = profile != CallProfile::Pure
                        && sig.kind == NodeKind::Volatile
                        && crate::compile::is_state_read(snapshot, node);
                    if state {
                        crate::compile::admit_state_read(node.id, sig)?;
                    }
                    let seeded = profile != CallProfile::Pure
                        && sig.kind == NodeKind::Volatile
                        && (node.definition.id.as_str() == "core/call"
                            || crate::compile::is_seeded_entropy(snapshot, node));
                    if !marker && sig.kind != NodeKind::Pure && !seeded && !state && !control {
                        return Err(CompileError::UnsupportedNode { node: node.id });
                    }
                    let lazy = out.lazy(&BTreeMap::new(), cursor, node.id).is_some();
                    let zone = (profile != CallProfile::Pure)
                        .then(|| crate::zones::marker(analysis, node.id))
                        .flatten();
                    if node.definition.id.is_core()
                        && !marker
                        && !lazy
                        && !state
                        && !control
                        && zone.is_none()
                        && !matches!(node.definition.id.as_str(), "core/call" | "core/convert")
                    {
                        return Err(CompileError::UnsupportedNode { node: node.id });
                    }
                    if !effect
                        || (!marker && matches!(sig.kind, NodeKind::Pure | NodeKind::Volatile))
                    {
                        admit_data_ports(
                            node.id,
                            sig,
                            lazy,
                            if state {
                                OutputSource::State
                            } else if zone == Some(false) {
                                OutputSource::Iteration
                            } else {
                                OutputSource::Computed
                            },
                            options,
                        )?;
                    }
                }
                for port in &sig.ports {
                    options.checkpoint()?;
                    if port.direction == orange_model::document::PortDirection::Output
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
                if !(sig.kind == NodeKind::Pure
                    || (profile != CallProfile::Pure && sig.kind == NodeKind::Volatile)
                    || (profile == CallProfile::Control
                        && matches!(sig.kind, NodeKind::Action | NodeKind::Async)))
                    || node.definition.version != 1
                    || node.config_version != 1
                    || node.provider.is_some()
                {
                    return Err(CompileError::UnsupportedNode { node: node.id });
                }
                // Fresh whole-document analysis has already accepted this exact local
                // declaration. Read its typed reference and borrow its interface.
                let target = match snapshot.reference(node, ReferenceKind::Function) {
                    Ok(DeclId::Function(id)) => id,
                    _ => return Err(CompileError::ValidationIncomplete),
                };
                let function = &document.functions()[&target];
                let depth = out.scopes[cursor]
                    .depth
                    .checked_add(1)
                    .filter(|d| *d <= options.limits.max_call_depth)
                    .ok_or(CompileError::ResourceLimit)?;
                let mut ancestor = Some(cursor);
                while let Some(index) = ancestor {
                    options.checkpoint()?;
                    if out.scopes[index].function == Some(target) {
                        return Err(CompileError::ValidationIncomplete);
                    }
                    ancestor = out.scopes[index].parent;
                }
                let declared = function.inputs.iter().chain(&function.outputs);
                if sig.ports.len() != function.inputs.len() + function.outputs.len() {
                    return Err(CompileError::ValidationIncomplete);
                }
                let mut args = TypeArgs::new();
                for (declared, actual) in declared.zip(&sig.ports) {
                    options.checkpoint()?;
                    if declared.port_id != actual.port_id || declared.direction != actual.direction
                    {
                        return Err(CompileError::ValidationIncomplete);
                    }
                    bind(
                        &TypeTerm::parse(
                            declared
                                .ty
                                .as_ref()
                                .ok_or(CompileError::ValidationIncomplete)?,
                        )
                        .map_err(|_| CompileError::ValidationIncomplete)?,
                        &TypeTerm::parse(
                            actual
                                .ty
                                .as_ref()
                                .ok_or(CompileError::ValidationIncomplete)?,
                        )
                        .map_err(|_| CompileError::ValidationIncomplete)?,
                        &mut args,
                    )?;
                }
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
                // Charge each potential invocation, even when metadata is shared.
                for child in function.body.nodes.keys() {
                    options.checkpoint()?;
                    charge(
                        &mut items,
                        analysis.report().resolved[child].ports.len(),
                        options.limits.max_graph_items,
                    )?;
                }
                let control = matches!(sig.kind, NodeKind::Action | NodeKind::Async);
                let cache_key = (target, args);
                let prepared = if let Some(body) = cache.get(&cache_key) {
                    body.clone()
                } else {
                    let mut prepared = PreparedBody::default();
                    for child in function.body.nodes.values() {
                        options.checkpoint()?;
                        let original = analysis
                            .report()
                            .resolved
                            .get(&child.id)
                            .ok_or(CompileError::ValidationIncomplete)?;
                        let sig =
                            specialize(original, &cache_key.1, &mut out.signature_bytes, options)?;
                        if child.definition.id.as_str() == "core/function-entry" {
                            prepared.entry = Some(child.id);
                        }
                        if child.definition.id.as_str() == "core/function-outputs" {
                            prepared.result = Some(child.id);
                        }
                        if !child.definition.id.is_core() {
                            let Ports::Declared(declared) = &snapshot
                                .definition(&child.definition)
                                .ok_or(CompileError::ValidationIncomplete)?
                                .ports
                            else {
                                return Err(CompileError::UnsupportedNode { node: child.id });
                            };
                            if declared.len() != sig.ports.len() {
                                return Err(CompileError::ValidationIncomplete);
                            }
                            let mut concrete = TypeArgs::new();
                            for (declared, actual) in declared.iter().zip(&sig.ports) {
                                options.checkpoint()?;
                                if declared.port_id != actual.port_id
                                    || declared.direction != actual.direction
                                {
                                    return Err(CompileError::ValidationIncomplete);
                                }
                                if let Some(pattern) = &declared.ty {
                                    bind(
                                        &TypeTerm::parse(pattern)
                                            .map_err(|_| CompileError::ValidationIncomplete)?,
                                        &TypeTerm::parse(
                                            actual
                                                .ty
                                                .as_ref()
                                                .ok_or(CompileError::ValidationIncomplete)?,
                                        )
                                        .map_err(|_| CompileError::ValidationIncomplete)?,
                                        &mut concrete,
                                    )?;
                                }
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
                            prepared.lazy.insert(child.id, lazy);
                        }
                        prepared.signatures.insert(child.id, sig);
                    }
                    if !control && !function.outputs.is_empty() && prepared.result.is_none() {
                        return Err(CompileError::ValidationIncomplete);
                    }
                    for edge in function.body.edges.values() {
                        out.incoming
                            .insert((edge.to.node, edge.to.port.clone()), edge);
                    }
                    let prepared = Arc::new(prepared);
                    cache.insert(cache_key, prepared.clone());
                    prepared
                };
                let index = out.scopes.len();
                out.scopes[cursor].calls.insert(node.id, index);
                out.scopes.push(CallScope {
                    control,
                    body: Some(prepared),
                    function: Some(target),
                    parent: Some(cursor),
                    depth,
                    calls: BTreeMap::new(),
                });
                out.sites.push(PureCallSite {
                    call: node.id,
                    function: target,
                    parent: cursor.checked_sub(1),
                });
            }
            cursor += 1;
        }
        if slots
            .checked_mul(16)
            .and_then(|n| n.checked_add(out.signature_bytes))
            .is_none_or(|n| n > options.limits.max_constant_bytes)
        {
            return Err(CompileError::ResourceLimit);
        }
        Ok(out)
    }
    pub(crate) fn body<'b>(&self, document: &'b GraphDocument, scope: usize) -> &'b GraphBody {
        match self.scopes[scope].function {
            Some(id) => &document.functions()[&id].body,
            None => document.root(),
        }
    }
    pub(crate) fn entry(&self, scope: usize) -> Option<NodeId> {
        self.scopes[scope].body.as_ref().and_then(|b| b.entry)
    }
    pub(crate) fn result(&self, scope: usize) -> Option<NodeId> {
        self.scopes[scope].body.as_ref().and_then(|b| b.result)
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
            self.scopes[scope].body.as_ref()?.lazy.get(&node).copied()
        }
    }
    pub(crate) fn signature<'b>(
        &'b self,
        analysis: &'b ValidationAnalysis,
        scope: usize,
        node: NodeId,
    ) -> &'b SignatureSnapshot {
        match &self.scopes[scope].body {
            Some(body) => body.signatures[&node].as_ref(),
            None => &analysis.report().resolved[&node],
        }
    }
    pub(crate) fn incoming<'b>(&'b self, root: &'b Incoming<'a>, scope: usize) -> &'b Incoming<'a> {
        if scope == 0 { root } else { &self.incoming }
    }
}
