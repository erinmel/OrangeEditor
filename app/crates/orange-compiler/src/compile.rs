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
    /// Conservative number of producers in the potential export cone, including
    /// unselected lazy alternatives, not the number of callbacks actually invoked.
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
        Value::Some(inner) => value_bytes(inner).saturating_add(16),
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
    ty: &PlanType,
    snapshot: &RegistrySnapshot,
    remaining: usize,
) -> Result<Option<Value>, CompileError> {
    if PlanType::parse(literal.ty.as_str()).as_ref() != Some(ty) {
        return Ok(None);
    }
    let Some(cost) = literal_bytes(&literal.payload, ty) else {
        return Ok(None);
    };
    if cost > remaining {
        return Err(CompileError::ResourceLimit);
    }
    Ok(literal_codec::decode(literal, snapshot.types().as_ref())
        .ok()
        .filter(|v| ty.accepts(v)))
}
fn literal_bytes(payload: &LiteralPayload, ty: &PlanType) -> Option<usize> {
    // Walk only the supported option envelope and scalar leaf before decoding.
    // No payload serialization, recursive JSON traversal or unbounded work stack.
    let mut cost = 16;
    let mut expected = ty;
    let mut payload = match payload {
        LiteralPayload::Bits(bits) => return Some(bits.len().saturating_add(cost)),
        LiteralPayload::Value(value) => value,
    };
    while let PlanType::Option(inner) = expected {
        let object = payload.as_object().filter(|o| o.len() == 1)?;
        if object.get("none") == Some(&serde_json::Value::Bool(true)) {
            return Some(cost);
        }
        let some = object.get("some")?;
        if matches!(inner.as_ref(), PlanType::Option(_)) {
            cost += 16;
        }
        payload = some;
        expected = inner;
    }
    let size = match payload {
        serde_json::Value::String(text) => text.len(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => 16,
        _ => return None,
    };
    Some(size.saturating_add(cost))
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

type OutputSlots = BTreeMap<(NodeId, PortKey), SlotIx>;
type Incoming<'a> = BTreeMap<(NodeId, PortKey), &'a orange_model::document::Edge>;
struct InputContext<'a> {
    document: &'a GraphDocument,
    snapshot: &'a RegistrySnapshot,
    incoming: &'a Incoming<'a>,
    options: CompileOptions<'a>,
}
impl InputContext<'_> {
    fn source(
        &self,
        node: NodeId,
        port: &PortDescriptor,
        plan: &mut PurePlan,
        outputs: &OutputSlots,
        bytes: &mut usize,
    ) -> Result<InputSource, CompileError> {
        self.options.checkpoint()?;
        let port_key = key(port);
        let (slot, edge) = if let Some(edge) = self.incoming.get(&(node, port_key.clone())) {
            let slot = outputs
                .get(&(edge.from.node, edge.from.port.clone()))
                .copied()
                .ok_or(CompileError::ValidationIncomplete)?;
            (slot, Some(edge.id))
        } else {
            let instance = &self.document.root().nodes[&node];
            let bad = || CompileError::InvalidLiteral {
                node,
                port: port_key.clone(),
            };
            let literal = instance
                .inputs
                .get(&port_key)
                .or(port.default.as_ref())
                .ok_or_else(bad)?;
            let ty = port_type(node, port)?;
            let value = decode(
                literal,
                &ty,
                self.snapshot,
                self.options
                    .limits
                    .max_constant_bytes
                    .saturating_sub(*bytes),
            )?
            .ok_or_else(bad)?;
            let slot = push_slot(
                plan,
                ty,
                SlotSource {
                    node,
                    port: port_key.clone(),
                },
                FoldedValue::Value(value),
                self.options,
                bytes,
            )?;
            (slot, None)
        };
        Ok(InputSource {
            port: port_key,
            slot,
            edge,
        })
    }
}
struct Decision {
    selector: InputSource,
    selected: Option<&'static str>,
}
enum Work {
    Enter(NodeId),
    Choose(NodeId),
    Finish {
        node: NodeId,
        decision: Option<Decision>,
    },
}

/// Compile pure scalar/JSON/option root exports in persisted export and declared input order.
/// Select/And/Or/Coalesce visit only selected inputs; other supported nodes remain eager.
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
    let mut lazy_nodes = BTreeMap::new();
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
        if let Some(kind) = crate::lazy::Lazy::resolve(&body.nodes[node], sig)? {
            lazy_nodes.insert(*node, kind);
        }
        charge(&mut ports, sig.ports.len(), options.limits.max_graph_items)?;
        for port in &sig.ports {
            if port.kind != PortKind::Data
                || (port.is_lazy() && !lazy_nodes.contains_key(node))
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
    // Potential-cone admission is complete; do not retain its traversal buffers while
    // building the smaller demanded plan (which may omit entire lazy alternatives).
    drop(order);
    drop(active);
    drop(done);
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
    let context = InputContext {
        document,
        snapshot: &snapshot,
        incoming: &incoming,
        options,
    };
    let mut completed = BTreeSet::new();
    let mut work: Vec<_> = document
        .exports()
        .iter()
        .rev()
        .map(|export| Work::Enter(export.node))
        .collect();
    while let Some(task) = work.pop() {
        options.checkpoint()?;
        let (node_id, decision) = match task {
            Work::Enter(node) => {
                if completed.contains(&node) {
                    continue;
                }
                let signature = &analysis.report().resolved[&node];
                if lazy_nodes.contains_key(&node) {
                    work.push(Work::Choose(node));
                    if let Some(edge) = incoming.get(&(node, key(&signature.ports[0]))) {
                        work.push(Work::Enter(edge.from.node));
                    }
                } else {
                    work.push(Work::Finish {
                        node,
                        decision: None,
                    });
                    for port in signature
                        .ports
                        .iter()
                        .rev()
                        .filter(|p| p.direction == PortDirection::Input)
                    {
                        if let Some(edge) = incoming.get(&(node, key(port))) {
                            work.push(Work::Enter(edge.from.node));
                        }
                    }
                }
                continue;
            }
            Work::Choose(node) => {
                let signature = &analysis.report().resolved[&node];
                let selector =
                    context.source(node, &signature.ports[0], &mut plan, &outputs, &mut bytes)?;
                let selected = match &plan.constants[selector.slot.index()] {
                    FoldedValue::Fault(_) => None,
                    FoldedValue::Value(value) => lazy_nodes[&node]
                        .selected(value)
                        .ok_or(CompileError::KernelContract { node })?,
                };
                work.push(Work::Finish {
                    node,
                    decision: Some(Decision { selector, selected }),
                });
                if let Some(name) = selected {
                    let port = signature
                        .ports
                        .iter()
                        .find(|p| p.port_id.as_str() == name)
                        .ok_or(CompileError::KernelContract { node })?;
                    if let Some(edge) = incoming.get(&(node, key(port))) {
                        work.push(Work::Enter(edge.from.node));
                    }
                }
                continue;
            }
            Work::Finish { node, decision } => (node, decision),
        };
        let node = &body.nodes[&node_id];
        let sig = &analysis.report().resolved[&node_id];
        let input_ports: Vec<_> = sig
            .ports
            .iter()
            .filter(|p| {
                p.direction == PortDirection::Input
                    && decision.as_ref().is_none_or(|choice| {
                        p.port_id.as_str() == choice.selector.port.port().as_str()
                            || Some(p.port_id.as_str()) == choice.selected
                    })
            })
            .collect();
        let output_ports: Vec<_> = sig
            .ports
            .iter()
            .filter(|p| p.direction == PortDirection::Output)
            .collect();
        let binding = if let Some(choice) = &decision {
            crate::lazy::binding(
                input_ports.iter().map(|p| key(p)).collect(),
                output_ports.iter().map(|p| key(p)).collect(),
                matches!(
                    plan.constants[choice.selector.slot.index()],
                    FoldedValue::Fault(_)
                ),
                matches!(lazy_nodes[&node_id], crate::lazy::Lazy::Coalesce)
                    && choice.selected.is_none(),
            )
        } else {
            resolver
                .resolve(document, node, sig, snapshot.clone())
                .map_err(|error| match error {
                    ResolveError::Unavailable => CompileError::KernelUnavailable { node: node_id },
                    ResolveError::Contract => CompileError::KernelContract { node: node_id },
                })?
        };
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
            let prepared = decision
                .as_ref()
                .filter(|choice| choice.selector.port.port() == &port.port_id);
            inputs.push(if let Some(choice) = prepared {
                choice.selector.clone()
            } else {
                context.source(node_id, port, &mut plan, &outputs, &mut bytes)?
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
        completed.insert(node_id);
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
