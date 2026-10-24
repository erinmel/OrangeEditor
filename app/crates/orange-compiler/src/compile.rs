use crate::*;
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::literal::{Literal, LiteralPayload};
use orange_model::document::{NodeKind, OutputSource, PortDescriptor, PortDirection, PortKind};
use orange_model::id::{NodeId, PortKey};
use orange_model::validation::{Diagnostic, DiagnosticCode, Subject, ValidationAnalysis};
use orange_model::value::literal_codec;
use orange_model::{GraphDocument, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Admission and retained-value budgets, not hard process memory or callback CPU bounds.
#[derive(Debug, Clone, Copy)]
pub struct CompileLimits {
    pub max_graph_items: usize,
    pub max_slots: usize,
    pub max_fold_operations: usize,
    pub max_constant_bytes: usize,
}
impl Default for CompileLimits {
    fn default() -> Self {
        Self {
            max_graph_items: 100_000,
            max_slots: 200_000,
            max_fold_operations: 20_000,
            max_constant_bytes: 16 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, Default)]
pub struct CompileOptions<'a> {
    pub limits: CompileLimits,
    pub cancel: Option<&'a AtomicBool>,
}
impl CompileOptions<'_> {
    fn checkpoint(self) -> Result<(), CompileError> {
        if self
            .cancel
            .is_some_and(|signal| signal.load(Ordering::Relaxed))
        {
            Err(CompileError::Cancelled)
        } else {
            Ok(())
        }
    }
}

fn key(port: &PortDescriptor) -> PortKey {
    PortKey::new(port.port_id.clone(), None)
}
fn port_type(node: NodeId, port: &PortDescriptor) -> Result<PlanType, CompileError> {
    port.ty
        .as_ref()
        .and_then(|ty| PlanType::parse(ty.as_str()))
        .ok_or_else(|| CompileError::UnsupportedPort {
            node,
            port: key(port),
        })
}
fn charge(used: &mut usize, amount: usize, limit: usize) -> Result<(), CompileError> {
    *used = used
        .checked_add(amount)
        .filter(|sum| *sum <= limit)
        .ok_or(CompileError::ResourceLimit)?;
    Ok(())
}
fn value_bytes(value: &Value) -> usize {
    match value {
        Value::Json(text) => text.as_str().len().saturating_add(16),
        _ => 16,
    }
}
fn push_slot(
    plan: &mut PurePlan,
    ty: PlanType,
    source: SlotSource,
    value: FoldedValue,
    options: CompileOptions<'_>,
    bytes: &mut usize,
) -> Result<SlotIx, CompileError> {
    if plan.constants.len() >= options.limits.max_slots {
        return Err(CompileError::ResourceLimit);
    }
    let index = u32::try_from(plan.constants.len()).map_err(|_| CompileError::ResourceLimit)?;
    let size = match &value {
        FoldedValue::Value(v) => value_bytes(v),
        FoldedValue::Fault(_) => 64,
    };
    charge(bytes, size, options.limits.max_constant_bytes)?;
    plan.types.push(ty);
    plan.sources.push(source);
    plan.constants.push(value);
    Ok(SlotIx(index))
}
fn decode(
    literal: &Literal,
    ty: PlanType,
    snapshot: &RegistrySnapshot,
    remaining: usize,
) -> Result<Option<Value>, CompileError> {
    // Bound payload processing before decoding. Composite and optional types are outside E1a.
    let size = match &literal.payload {
        LiteralPayload::Bits(bits) => bits.len(),
        LiteralPayload::Value(serde_json::Value::String(text)) => text.len(),
        LiteralPayload::Value(serde_json::Value::Bool(_) | serde_json::Value::Number(_)) => 16,
        _ => return Ok(None),
    };
    if size.saturating_add(16) > remaining {
        return Err(CompileError::ResourceLimit);
    }
    if PlanType::parse(literal.ty.as_str()) != Some(ty) {
        return Ok(None);
    }
    Ok(literal_codec::decode(literal, snapshot.types().as_ref())
        .ok()
        .filter(|v| ty.accepts(v)))
}
fn valid_fault(fault: KernelFault) -> bool {
    fn identifier(text: &str) -> bool {
        let Some((namespace, name)) = text.split_once('/') else {
            return false;
        };
        !namespace.is_empty()
            && !name.is_empty()
            && !name.contains('/')
            && text.len() <= 256
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
    }
    identifier(fault.code)
        && identifier(fault.message_key)
        && !fault.message.is_empty()
        && fault.message.len() <= 512
        && !fault.message.chars().any(char::is_control)
}

/// Compile eager pure scalar/JSON root exports in persisted export and declared input order.
/// Semantic analysis must be fresh; no resolver or executable callback runs before this check.
/// Only reachable pure nodes fold, once each. Errors remain values with producer provenance.
pub fn compile_pure(
    document: &GraphDocument,
    snapshot: Arc<RegistrySnapshot>,
    analysis: &ValidationAnalysis,
    resolver: &impl PureKernelResolver,
    options: CompileOptions<'_>,
) -> Result<PurePlan, CompileError> {
    options.checkpoint()?;
    if analysis.document_state() != document.state_id() {
        return Err(CompileError::StaleDocument);
    }
    if analysis.registry_generation() != snapshot.generation() {
        return Err(CompileError::StaleRegistry);
    }
    if !analysis.is_ready_for_lowering(document, &snapshot) {
        return Err(CompileError::ValidationIncomplete);
    }
    let body = document.root();
    let mut items = 0;
    for count in [body.nodes.len(), body.edges.len(), document.exports().len()] {
        charge(&mut items, count, options.limits.max_graph_items)?;
    }
    // Admit all root nodes: no silently omitted side effects or unsupported port semantics.
    let mut ports = 0;
    for node in body.nodes.keys() {
        options.checkpoint()?;
        let sig = analysis
            .report()
            .resolved
            .get(node)
            .ok_or(CompileError::UnsupportedNode { node: *node })?;
        if sig.kind != NodeKind::Pure {
            return Err(CompileError::UnsupportedNode { node: *node });
        }
        charge(&mut ports, sig.ports.len(), options.limits.max_graph_items)?;
        for port in &sig.ports {
            if port.kind != PortKind::Data
                || port.is_lazy()
                || port.is_optional()
                || port.variadic.is_some()
                || (port.direction == PortDirection::Output
                    && port.source != Some(OutputSource::Computed))
            {
                return Err(CompileError::UnsupportedPort {
                    node: *node,
                    port: key(port),
                });
            }
            port_type(*node, port)?;
        }
    }
    let incoming: BTreeMap<_, _> = body
        .edges
        .values()
        .map(|edge| ((edge.to.node, edge.to.port.clone()), edge))
        .collect();
    // Iterative DFS avoids stack growth with graph depth. Active/finished sets also defend
    // against internal validation regressions. Reversing pushes preserves declaration order.
    let mut done = BTreeSet::new();
    let mut active = BTreeSet::new();
    let mut order = Vec::new();
    let mut stack = Vec::new();
    for export in document.exports() {
        stack.push((export.node, false));
        while let Some((node, exit)) = stack.pop() {
            options.checkpoint()?;
            if exit {
                active.remove(&node);
                done.insert(node);
                order.push(node);
                continue;
            }
            if done.contains(&node) {
                continue;
            }
            if !active.insert(node) {
                return Err(CompileError::ValidationIncomplete);
            }
            if active.len().saturating_add(done.len()) > options.limits.max_fold_operations {
                return Err(CompileError::ResourceLimit);
            }
            stack.push((node, true));
            let sig = analysis
                .report()
                .resolved
                .get(&node)
                .ok_or(CompileError::UnsupportedNode { node })?;
            for port in sig
                .ports
                .iter()
                .rev()
                .filter(|p| p.direction == PortDirection::Input)
            {
                if let Some(edge) = incoming.get(&(node, key(port))) {
                    stack.push((edge.from.node, false));
                }
            }
        }
    }
    // Preflight dense storage before resolver callbacks and scratch-output allocation.
    let mut slots = 0;
    for node in &order {
        options.checkpoint()?;
        for port in &analysis.report().resolved[node].ports {
            if port.direction == PortDirection::Output
                || !incoming.contains_key(&(*node, key(port)))
            {
                charge(
                    &mut slots,
                    1,
                    options.limits.max_slots.min(u32::MAX as usize),
                )?;
            }
        }
    }
    if slots
        .checked_mul(16)
        .is_none_or(|minimum| minimum > options.limits.max_constant_bytes)
    {
        return Err(CompileError::ResourceLimit);
    }
    let mut plan = PurePlan {
        snapshot: snapshot.clone(),
        graph: document.graph_id(),
        revision: document.revision(),
        state: document.state_id(),
        steps: vec![],
        types: vec![],
        sources: vec![],
        constants: vec![],
        exports: vec![],
        diagnostics: vec![],
    };
    let mut outputs = BTreeMap::new();
    let mut bytes = 0;
    for export in document.exports() {
        charge(
            &mut bytes,
            export.id.len(),
            options.limits.max_constant_bytes,
        )?;
    }
    let mut input_values = Vec::new();
    let mut output_values = Vec::new();
    for node_id in order {
        options.checkpoint()?;
        let node = &body.nodes[&node_id];
        let sig = &analysis.report().resolved[&node_id];
        let input_ports: Vec<_> = sig
            .ports
            .iter()
            .filter(|p| p.direction == PortDirection::Input)
            .collect();
        let output_ports: Vec<_> = sig
            .ports
            .iter()
            .filter(|p| p.direction == PortDirection::Output)
            .collect();
        let binding = resolver
            .resolve(document, node, sig, snapshot.clone())
            .map_err(|error| match error {
                ResolveError::Unavailable => CompileError::KernelUnavailable { node: node_id },
                ResolveError::Contract => CompileError::KernelContract { node: node_id },
            })?;
        if !binding
            .inputs
            .iter()
            .cloned()
            .eq(input_ports.iter().map(|p| key(p)))
            || !binding
                .outputs
                .iter()
                .cloned()
                .eq(output_ports.iter().map(|p| key(p)))
        {
            return Err(CompileError::KernelContract { node: node_id });
        }
        let mut inputs = Vec::new();
        for port in input_ports {
            options.checkpoint()?;
            let port_key = key(port);
            let (slot, edge) = if let Some(edge) = incoming.get(&(node_id, port_key.clone())) {
                let slot = outputs
                    .get(&(edge.from.node, edge.from.port.clone()))
                    .copied()
                    .ok_or(CompileError::ValidationIncomplete)?;
                (slot, Some(edge.id))
            } else {
                let literal = node
                    .inputs
                    .get(&port_key)
                    .or(port.default.as_ref())
                    .ok_or_else(|| CompileError::InvalidLiteral {
                        node: node_id,
                        port: port_key.clone(),
                    })?;
                let ty = port_type(node_id, port)?;
                let value = decode(
                    literal,
                    ty,
                    &snapshot,
                    options.limits.max_constant_bytes.saturating_sub(bytes),
                )?
                .ok_or_else(|| CompileError::InvalidLiteral {
                    node: node_id,
                    port: port_key.clone(),
                })?;
                let slot = push_slot(
                    &mut plan,
                    ty,
                    SlotSource {
                        node: node_id,
                        port: port_key.clone(),
                    },
                    FoldedValue::Value(value),
                    options,
                    &mut bytes,
                )?;
                (slot, None)
            };
            inputs.push(InputSource {
                port: port_key,
                slot,
                edge,
            });
        }
        let inherited = inputs
            .iter()
            .find_map(|input| match &plan.constants[input.slot.index()] {
                FoldedValue::Fault(fault) => Some(fault.clone()),
                _ => None,
            });
        input_values.clear();
        output_values.clear();
        output_values.resize(output_ports.len(), Value::Unit);
        let fault = if let Some(fault) = inherited {
            Some(fault)
        } else {
            for input in &inputs {
                if let FoldedValue::Value(value) = &plan.constants[input.slot.index()] {
                    input_values.push(value.clone());
                }
            }
            options.checkpoint()?;
            match binding.kernel.fold(&input_values, &mut output_values) {
                Ok(()) => None,
                Err(fault) => {
                    if !valid_fault(fault) {
                        return Err(CompileError::KernelContract { node: node_id });
                    }
                    plan.diagnostics.push(Diagnostic::new(
                        DiagnosticCode::ConstError,
                        Subject::new(Some(node_id), None, None),
                        snapshot.origin(),
                    ));
                    Some(ConstantFault {
                        source: node_id,
                        fault,
                    })
                }
            }
        };
        let mut slots = Vec::new();
        for (port, value) in output_ports.into_iter().zip(output_values.drain(..)) {
            let ty = port_type(node_id, port)?;
            let value = if let Some(fault) = &fault {
                FoldedValue::Fault(fault.clone())
            } else if ty.accepts(&value) {
                FoldedValue::Value(value)
            } else {
                return Err(CompileError::KernelContract { node: node_id });
            };
            let port = key(port);
            let slot = push_slot(
                &mut plan,
                ty,
                SlotSource {
                    node: node_id,
                    port: port.clone(),
                },
                value,
                options,
                &mut bytes,
            )?;
            outputs.insert((node_id, port), slot);
            slots.push(slot);
        }
        let index =
            NodeIx(u32::try_from(plan.steps.len()).map_err(|_| CompileError::ResourceLimit)?);
        plan.steps.push(PureStep {
            index,
            node: node_id,
            inputs: inputs.into(),
            outputs: slots.into(),
            binding,
        });
    }
    for export in document.exports() {
        options.checkpoint()?;
        let slot = outputs
            .get(&(export.node, export.port.clone()))
            .copied()
            .ok_or(CompileError::ValidationIncomplete)?;
        plan.exports.push(Export {
            id: export.id.clone(),
            slot,
        });
    }
    orange_model::validation::sort_diagnostics(&mut plan.diagnostics);
    options.checkpoint()?;
    Ok(plan)
}
