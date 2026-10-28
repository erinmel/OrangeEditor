use crate::*;
use orange_model::catalog::RegistrySnapshot;
use orange_model::catalog::intrinsic_id as ids;
use orange_model::document::literal::{Literal, LiteralPayload};
use orange_model::document::{NodeKind, OutputSource, PortDescriptor, PortDirection, PortKind};
use orange_model::id::{NodeId, PortKey};
use orange_model::validation::{Diagnostic, DiagnosticCode, Subject, ValidationAnalysis};
use orange_model::value::literal_codec;
use orange_model::value::shape::{MAP_ENTRY_CHARGE, VALUE_CHARGE, is_error_code};
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
    /// Maximum nested graph-local calls in the selected profile; preparation is iterative.
    pub max_call_depth: usize,
    /// Conservative number of producers in the potential export cone, including
    /// unselected lazy alternatives, not the number of callbacks actually invoked.
    pub max_fold_operations: usize,
    /// Nominal retained values and owned specialized-signature copies. Metadata
    /// borrowed from fresh analysis is shared and needs no owned-copy charge.
    pub max_constant_bytes: usize,
}
impl Default for CompileLimits {
    fn default() -> Self {
        Self {
            max_graph_items: 100_000,
            max_slots: 200_000,
            max_call_depth: 256,
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
    pub(crate) fn checkpoint(self) -> Result<(), CompileError> {
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

pub(crate) fn key(port: &PortDescriptor) -> PortKey {
    PortKey::new(port.port_id.clone(), None)
}
pub(crate) fn port_type(node: NodeId, port: &PortDescriptor) -> Result<ValueShape, CompileError> {
    port.ty
        .as_ref()
        .and_then(ValueShape::of)
        .ok_or_else(|| CompileError::UnsupportedPort {
            node,
            port: key(port),
        })
}
/// Shared executable port policy for root and function-body admission.
pub(crate) fn admit_data_ports(
    node: NodeId,
    sig: &orange_model::document::SignatureSnapshot,
    lazy: bool,
    source: OutputSource,
    options: CompileOptions<'_>,
) -> Result<(), CompileError> {
    for port in &sig.ports {
        options.checkpoint()?;
        if port.kind != PortKind::Data
            || (port.is_lazy() && !lazy)
            || port.is_optional()
            || port.variadic.is_some()
            || (port.direction == PortDirection::Output && port.source != Some(source))
        {
            return Err(CompileError::UnsupportedPort {
                node,
                port: key(port),
            });
        }
        port_type(node, port)?;
    }
    Ok(())
}
pub(crate) fn charge(used: &mut usize, amount: usize, limit: usize) -> Result<(), CompileError> {
    *used = used
        .checked_add(amount)
        .filter(|sum| *sum <= limit)
        .ok_or(CompileError::ResourceLimit)?;
    Ok(())
}
/// Retained bytes of one published constant fault: static code, key, message and producer.
pub(crate) const FAULT_CHARGE: usize = 64;

/// Nominal retained bytes of a constant, already checked against its shape.
pub(crate) fn value_bytes(ty: &ValueShape, value: &Value) -> Result<usize, CompileError> {
    ty.checked_charge(value, usize::MAX)
        .map_err(|_| CompileError::ValidationIncomplete)
}
/// Whether a binding's port order is exactly the demanded inputs and declared outputs.
pub(crate) fn matches_abi(
    inputs: &[PortKey],
    outputs: &[PortKey],
    expected_inputs: &[&PortDescriptor],
    expected_outputs: &[&PortDescriptor],
) -> bool {
    inputs
        .iter()
        .cloned()
        .eq(expected_inputs.iter().map(|p| key(p)))
        && outputs
            .iter()
            .cloned()
            .eq(expected_outputs.iter().map(|p| key(p)))
}
pub(crate) fn binding_error(node: NodeId, error: ResolveError) -> CompileError {
    match error {
        ResolveError::Unavailable => CompileError::KernelUnavailable { node },
        ResolveError::Contract => CompileError::KernelContract { node },
    }
}
/// The first fault among already folded slots, in order.
fn first_fault(plan: &PurePlan, slots: impl IntoIterator<Item = SlotIx>) -> Option<ConstantFault> {
    slots
        .into_iter()
        .find_map(|slot| match &plan.constants[slot.index()] {
            FoldedValue::Fault(fault) => Some(fault.clone()),
            FoldedValue::Value(_) => None,
        })
}
fn push_slot(
    plan: &mut PurePlan,
    ty: ValueShape,
    source: SlotSource,
    value: FoldedValue,
    call: Option<usize>,
    options: CompileOptions<'_>,
    bytes: &mut usize,
) -> Result<SlotIx, CompileError> {
    if plan.constants.len() >= options.limits.max_slots {
        return Err(CompileError::ResourceLimit);
    }
    let index = u32::try_from(plan.constants.len()).map_err(|_| CompileError::ResourceLimit)?;
    let size = match &value {
        FoldedValue::Value(v) => value_bytes(&ty, v)?,
        FoldedValue::Fault(_) => FAULT_CHARGE,
    };
    charge(bytes, size, options.limits.max_constant_bytes)?;
    plan.slot_calls.push(call);
    plan.types.push(ty);
    plan.sources.push(source);
    plan.constants.push(value);
    Ok(SlotIx(index))
}
pub(crate) fn decode(
    literal: &Literal,
    ty: &ValueShape,
    snapshot: &RegistrySnapshot,
    remaining: usize,
) -> Result<Option<Value>, CompileError> {
    if ValueShape::of(&literal.ty).as_ref() != Some(ty) {
        return Ok(None);
    }
    let cost = literal_bytes(&literal.payload, ty, remaining).ok_or(CompileError::ResourceLimit)?;
    if cost > remaining {
        return Err(CompileError::ResourceLimit);
    }
    Ok(literal_codec::decode(literal, snapshot.types().as_ref())
        .ok()
        .filter(|v| ty.accepts(v)))
}
fn literal_bytes(payload: &LiteralPayload, ty: &ValueShape, limit: usize) -> Option<usize> {
    fn visit(value: &serde_json::Value, ty: &ValueShape, remaining: &mut usize) -> Option<()> {
        *remaining = remaining.checked_sub(VALUE_CHARGE)?;
        match ty {
            ValueShape::Option(inner) => {
                let object = value.as_object().filter(|o| o.len() == 1)?;
                if object.get("none") == Some(&serde_json::Value::Bool(true)) {
                    return Some(());
                }
                let some = object.get("some")?;
                if !inner.unboxed_none() {
                    *remaining = remaining.checked_add(VALUE_CHARGE)?;
                }
                visit(some, inner, remaining)
            }
            ValueShape::List(inner) => {
                let items = value.as_array()?;
                if items.len().checked_mul(VALUE_CHARGE)? > *remaining {
                    return None;
                }
                for item in items {
                    visit(item, inner, remaining)?;
                }
                Some(())
            }
            ValueShape::Map(key, inner) => {
                let items = value.as_array()?;
                if items.len().checked_mul(MAP_ENTRY_CHARGE)? > *remaining {
                    return None;
                }
                for item in items {
                    let pair = item.as_array().filter(|p| p.len() == 2)?;
                    visit(&pair[0], key, remaining)?;
                    visit(&pair[1], inner, remaining)?;
                }
                Some(())
            }
            _ => {
                let bytes = match value {
                    serde_json::Value::String(s) => s.len(),
                    serde_json::Value::Bool(_) | serde_json::Value::Number(_) => VALUE_CHARGE,
                    _ => return None,
                };
                *remaining = remaining.checked_sub(bytes)?;
                Some(())
            }
        }
    }
    let mut remaining = limit;
    match payload {
        LiteralPayload::Bits(bits) => return bits.len().checked_add(VALUE_CHARGE),
        LiteralPayload::Value(value) => visit(value, ty, &mut remaining)?,
    };
    Some(limit - remaining)
}
pub(crate) fn valid_fault(fault: KernelFault) -> bool {
    is_error_code(fault.code)
        && is_error_code(fault.message_key)
        && !fault.message.is_empty()
        && fault.message.len() <= 512
        && !fault.message.chars().any(char::is_control)
}

type OutputSlots = BTreeMap<(NodeId, PortKey), SlotIx>;
pub(crate) type Incoming<'a> = BTreeMap<(NodeId, PortKey), &'a orange_model::document::Edge>;
struct InputContext<'a> {
    document: &'a GraphDocument,
    snapshot: &'a RegistrySnapshot,
    incoming: &'a Incoming<'a>,
    options: CompileOptions<'a>,
    call: Option<usize>,
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
            let instance = &self
                .document
                .node(node)
                .ok_or(CompileError::ValidationIncomplete)?;
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
                self.call,
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
    ReturnCall {
        parent: usize,
        node: NodeId,
    },
}

/// Compile pure scalar/JSON/option/result root exports in persisted export order.
/// Select/And/Or/Coalesce visit selected inputs; Try captures graph input faults.
/// Semantic analysis must be fresh; no resolver or executable callback runs before this check.
/// Only reachable pure nodes fold, once each. Errors remain values with producer provenance.
pub fn compile_pure(
    document: &GraphDocument,
    snapshot: Arc<RegistrySnapshot>,
    analysis: &ValidationAnalysis,
    resolver: &impl PureKernelResolver,
    options: CompileOptions<'_>,
) -> Result<PurePlan, CompileError> {
    let Admission {
        incoming,
        lazy_nodes,
        ..
    } = admit(document, &snapshot, analysis, options, false)?;
    let calls = Arc::new(crate::calls::Calls::prepare(
        document,
        &snapshot,
        analysis,
        options,
        crate::calls::CallProfile::Pure,
    )?);
    let mut folder = ConstantFolder::new(
        document,
        analysis,
        snapshot,
        resolver,
        options,
        &incoming,
        &lazy_nodes,
        calls,
    )?;
    for export in document.exports() {
        folder.pull(export.node)?;
    }
    for export in document.exports() {
        options.checkpoint()?;
        let slot = folder.frames[0]
            .outputs
            .get(&(export.node, export.port.clone()))
            .copied()
            .ok_or(CompileError::ValidationIncomplete)?;
        folder.plan.exports.push(Export {
            id: export.id.clone(),
            slot,
        });
    }
    orange_model::validation::sort_diagnostics(&mut folder.plan.diagnostics);
    options.checkpoint()?;
    Ok(folder.plan)
}

/// Internal demand folder shared by checked lowering paths. Callers must perform
/// fresh semantic and whole-potential-cone admission before constructing it.
pub(crate) struct ConstantFolder<'a, R: PureKernelResolver> {
    document: &'a GraphDocument,
    analysis: &'a ValidationAnalysis,
    snapshot: Arc<RegistrySnapshot>,
    resolver: &'a R,
    options: CompileOptions<'a>,
    incoming: &'a Incoming<'a>,
    lazy_nodes: &'a BTreeMap<NodeId, crate::lazy::Lazy>,
    plan: PurePlan,
    frames: Vec<FoldFrame>,
    pub(crate) calls: Arc<crate::calls::Calls<'a>>,
    bytes: usize,
    input_values: Vec<Value>,
    output_values: Vec<Value>,
    work: Vec<(usize, Work)>,
}
#[derive(Default)]
struct FoldFrame {
    outputs: OutputSlots,
    completed: BTreeSet<NodeId>,
}

impl<'a, R: PureKernelResolver> ConstantFolder<'a, R> {
    #[allow(clippy::too_many_arguments)] // Explicit admitted metadata shared by two profiles.
    pub(crate) fn new(
        document: &'a GraphDocument,
        analysis: &'a ValidationAnalysis,
        snapshot: Arc<RegistrySnapshot>,
        resolver: &'a R,
        options: CompileOptions<'a>,
        incoming: &'a Incoming<'a>,
        lazy_nodes: &'a BTreeMap<NodeId, crate::lazy::Lazy>,
        calls: Arc<crate::calls::Calls<'a>>,
    ) -> Result<Self, CompileError> {
        let frames = (0..calls.scopes.len())
            .map(|_| FoldFrame::default())
            .collect();
        let plan = PurePlan {
            snapshot: snapshot.clone(),
            graph: document.graph_id(),
            revision: document.revision(),
            state: document.semantic_state_id(),
            steps: vec![],
            types: vec![],
            sources: vec![],
            constants: vec![],
            exports: vec![],
            diagnostics: vec![],
            calls: calls.sites.clone(),
            slot_calls: vec![],
        };
        let mut bytes = calls.signature_bytes;
        for export in document.exports() {
            charge(
                &mut bytes,
                export.id.len(),
                options.limits.max_constant_bytes,
            )?;
        }
        Ok(Self {
            document,
            analysis,
            snapshot,
            resolver,
            options,
            incoming,
            lazy_nodes,
            plan,
            frames,
            calls,
            bytes,
            input_values: vec![],
            output_values: vec![],
            work: vec![],
        })
    }
    pub(crate) fn folded_at(
        &self,
        scope: usize,
        node: NodeId,
        port: &PortKey,
    ) -> Option<&FoldedValue> {
        self.frames[scope]
            .outputs
            .get(&(node, port.clone()))
            .map(|slot| &self.plan.constants[slot.index()])
    }
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.plan.diagnostics
    }
    pub(crate) fn seed_constant(
        &mut self,
        ty: ValueShape,
        source: SlotSource,
        value: FoldedValue,
        call: Option<usize>,
    ) -> Result<SlotIx, CompileError> {
        push_slot(
            &mut self.plan,
            ty,
            source,
            value,
            call,
            self.options,
            &mut self.bytes,
        )
    }
    pub(crate) fn alias_constant(
        &mut self,
        scope: usize,
        node: NodeId,
        port: PortKey,
        slot: SlotIx,
    ) {
        self.frames[scope].outputs.insert((node, port), slot);
    }
    pub(crate) fn pull(&mut self, root: NodeId) -> Result<(), CompileError> {
        self.pull_at(0, root)
    }
    pub(crate) fn pull_at(&mut self, root_scope: usize, root: NodeId) -> Result<(), CompileError> {
        let Self {
            document,
            analysis,
            snapshot,
            resolver,
            options,
            incoming,
            lazy_nodes,
            plan,
            frames,
            calls,
            bytes,
            input_values,
            output_values,
            work,
        } = self;
        work.push((root_scope, Work::Enter(root)));
        while let Some((scope, task)) = work.pop() {
            options.checkpoint()?;
            let incoming = calls.incoming(incoming, scope);
            let context = InputContext {
                document,
                snapshot,
                incoming,
                options: *options,
                call: scope.checked_sub(1),
            };
            if let Work::ReturnCall { parent, node } = task {
                let result = calls
                    .result(scope)
                    .ok_or(CompileError::ValidationIncomplete)?;
                let mut results = Vec::new();
                for port in &calls.signature(analysis, scope, result).ports {
                    results.push((
                        port,
                        context
                            .source(result, port, plan, &frames[scope].outputs, bytes)?
                            .slot,
                    ));
                }
                // The result marker gathers required inputs in declaration order.
                // Its first fault is the call's failure, shared by every result.
                let fault = first_fault(plan, results.iter().map(|(_, slot)| *slot));
                for (port, mut slot) in results {
                    if let Some(fault) = &fault {
                        slot = push_slot(
                            plan,
                            port_type(result, port)?,
                            SlotSource {
                                node: result,
                                port: key(port),
                            },
                            FoldedValue::Fault(fault.clone()),
                            scope.checked_sub(1),
                            *options,
                            bytes,
                        )?;
                    }
                    frames[parent].outputs.insert((node, key(port)), slot);
                }
                frames[parent].completed.insert(node);
                continue;
            }
            let FoldFrame { outputs, completed } = &mut frames[scope];
            let (node_id, decision) = match task {
                Work::Enter(node) => {
                    if completed.contains(&node) {
                        continue;
                    }
                    if calls.entry(scope) == Some(node) {
                        continue;
                    }
                    let signature = calls.signature(analysis, scope, node);
                    if signature.kind != NodeKind::Pure {
                        return Err(CompileError::UnsupportedNode { node });
                    }
                    if calls.lazy(lazy_nodes, scope, node).is_some() {
                        work.push((scope, Work::Choose(node)));
                        if let Some(edge) = incoming.get(&(node, key(&signature.ports[0]))) {
                            work.push((scope, Work::Enter(edge.from.node)));
                        }
                    } else {
                        work.push((
                            scope,
                            Work::Finish {
                                node,
                                decision: None,
                            },
                        ));
                        for port in signature
                            .ports
                            .iter()
                            .rev()
                            .filter(|p| p.direction == PortDirection::Input)
                        {
                            if let Some(edge) = incoming.get(&(node, key(port))) {
                                work.push((scope, Work::Enter(edge.from.node)));
                            }
                        }
                    }
                    continue;
                }
                Work::Choose(node) => {
                    let signature = calls.signature(analysis, scope, node);
                    let selector =
                        context.source(node, &signature.ports[0], plan, outputs, bytes)?;
                    let selected = match &plan.constants[selector.slot.index()] {
                        FoldedValue::Fault(_) => None,
                        FoldedValue::Value(value) => calls
                            .lazy(lazy_nodes, scope, node)
                            .ok_or(CompileError::ValidationIncomplete)?
                            .selected(value)
                            .ok_or(CompileError::KernelContract { node })?,
                    };
                    work.push((
                        scope,
                        Work::Finish {
                            node,
                            decision: Some(Decision { selector, selected }),
                        },
                    ));
                    if let Some(name) = selected {
                        let port = signature
                            .ports
                            .iter()
                            .find(|p| p.port_id.as_str() == name)
                            .ok_or(CompileError::KernelContract { node })?;
                        if let Some(edge) = incoming.get(&(node, key(port))) {
                            work.push((scope, Work::Enter(edge.from.node)));
                        }
                    }
                    continue;
                }
                Work::Finish { node, decision } => (node, decision),
                Work::ReturnCall { .. } => unreachable!("handled before frame borrowing"),
            };
            let node = document
                .node(node_id)
                .ok_or(CompileError::ValidationIncomplete)?;
            let sig = calls.signature(analysis, scope, node_id);
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
            if let Some(child) = calls.scopes[scope].calls.get(&node_id).copied() {
                let mut args = Vec::new();
                for port in &input_ports {
                    args.push((
                        key(port),
                        context.source(node_id, port, plan, outputs, bytes)?.slot,
                    ));
                }
                let fault = first_fault(plan, args.iter().map(|(_, slot)| *slot));
                if let Some(fault) = fault {
                    for port in output_ports {
                        let slot = push_slot(
                            plan,
                            port_type(node_id, port)?,
                            SlotSource {
                                node: node_id,
                                port: key(port),
                            },
                            FoldedValue::Fault(fault.clone()),
                            scope.checked_sub(1),
                            *options,
                            bytes,
                        )?;
                        outputs.insert((node_id, key(port)), slot);
                    }
                    completed.insert(node_id);
                    continue;
                }
                if let Some(entry) = calls.entry(child) {
                    for (port, slot) in args {
                        frames[child].outputs.insert((entry, port), slot);
                    }
                }
                if let Some(result) = calls.result(child) {
                    work.push((
                        child,
                        Work::ReturnCall {
                            parent: scope,
                            node: node_id,
                        },
                    ));
                    for port in calls.signature(analysis, child, result).ports.iter().rev() {
                        if let Some(edge) = calls.incoming.get(&(result, key(port))) {
                            work.push((child, Work::Enter(edge.from.node)));
                        }
                    }
                } else {
                    frames[scope].completed.insert(node_id);
                }
                continue;
            }
            let binding = if let Some(choice) = &decision {
                crate::lazy::binding(
                    input_ports.iter().map(|p| key(p)).collect(),
                    output_ports.iter().map(|p| key(p)).collect(),
                    matches!(
                        plan.constants[choice.selector.slot.index()],
                        FoldedValue::Fault(_)
                    ),
                    match calls
                        .lazy(lazy_nodes, scope, node_id)
                        .ok_or(CompileError::ValidationIncomplete)?
                    {
                        crate::lazy::Lazy::Coalesce if choice.selected.is_none() => {
                            crate::lazy::ProjectionMode::Option {
                                boxed: port_type(node_id, &sig.ports[1])?.unboxed_none(),
                            }
                        }
                        crate::lazy::Lazy::Try => crate::lazy::ProjectionMode::Result {
                            boxed: port_type(node_id, &sig.ports[0])?.unboxed_error(),
                        },
                        _ => crate::lazy::ProjectionMode::Value,
                    },
                )
            } else {
                BindingRequest::new(document, node, sig, snapshot.clone())
                    .and_then(|request| resolver.resolve_binding(&request))
                    .map_err(|error| binding_error(node_id, error))?
            };
            if !matches_abi(
                &binding.inputs,
                &binding.outputs,
                &input_ports,
                &output_ports,
            ) {
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
                    context.source(node_id, port, plan, outputs, bytes)?
                });
            }
            let inherited = first_fault(plan, inputs.iter().map(|input| input.slot));
            input_values.clear();
            output_values.clear();
            output_values.resize(output_ports.len(), Value::Unit);
            let fault = if let Some(fault) = inherited {
                if matches!(
                    calls.lazy(lazy_nodes, scope, node_id),
                    Some(crate::lazy::Lazy::Try)
                ) {
                    output_values[0] = Value::Err(Arc::new(orange_model::value::NodeError {
                        code: fault.fault.code.to_owned(),
                        node: Some(fault.source),
                    }));
                    None
                } else {
                    Some(fault)
                }
            } else {
                for input in &inputs {
                    if let FoldedValue::Value(value) = &plan.constants[input.slot.index()] {
                        input_values.push(value.clone());
                    }
                }
                options.checkpoint()?;
                match binding.kernel.fold(input_values, output_values) {
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
                } else {
                    ty.checked_charge(
                        &value,
                        options.limits.max_constant_bytes.saturating_sub(*bytes),
                    )
                    .map_err(|error| match error {
                        ValueCheckError::ResourceLimit => CompileError::ResourceLimit,
                        _ => CompileError::KernelContract { node: node_id },
                    })?;
                    FoldedValue::Value(value)
                };
                let port = key(port);
                let slot = push_slot(
                    plan,
                    ty,
                    SlotSource {
                        node: node_id,
                        port: port.clone(),
                    },
                    value,
                    scope.checked_sub(1),
                    *options,
                    bytes,
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
                call: scope.checked_sub(1),
            });
            completed.insert(node_id);
        }

        Ok(())
    }
}

pub(crate) struct Admission<'a> {
    pub(crate) incoming: Incoming<'a>,
    pub(crate) lazy_nodes: BTreeMap<NodeId, crate::lazy::Lazy>,
    pub(crate) host_nodes: BTreeSet<(usize, NodeId)>,
}
/// Fresh definition metadata says the node reads host-owned state.
pub(crate) fn is_seeded_entropy(
    snapshot: &RegistrySnapshot,
    node: &orange_model::document::NodeInstance,
) -> bool {
    snapshot
        .definition(&node.definition)
        .is_some_and(|definition| {
            definition.volatility == Some(orange_model::catalog::Volatility::Entropy)
                && definition.determinism == orange_model::catalog::Determinism::Seeded
        })
}
pub(crate) fn is_state_read(
    snapshot: &RegistrySnapshot,
    node: &orange_model::document::NodeInstance,
) -> bool {
    snapshot
        .definition(&node.definition)
        .is_some_and(|definition| {
            definition.volatility == Some(orange_model::catalog::Volatility::StateRead)
        })
}
/// State readers have the same host-input ABI in roots and called bodies.
pub(crate) fn admit_state_read(
    node: NodeId,
    sig: &orange_model::document::SignatureSnapshot,
) -> Result<(), CompileError> {
    if !matches!(sig.ports.as_slice(), [port] if port.direction == PortDirection::Output) {
        return Err(CompileError::UnsupportedNode { node });
    }
    Ok(())
}
pub(crate) fn admit<'a>(
    document: &'a GraphDocument,
    snapshot: &RegistrySnapshot,
    analysis: &ValidationAnalysis,
    options: CompileOptions<'_>,
    data_plan: bool,
) -> Result<Admission<'a>, CompileError> {
    let roots: Vec<_> = document
        .exports()
        .iter()
        .map(|export| export.node)
        .collect();
    admit_roots(
        document,
        snapshot,
        analysis,
        options,
        data_plan,
        &roots,
        &BTreeSet::new(),
    )
}

/// Control lowering supplies checked demand roots and a separately admitted set of
/// host control nodes. Neither source documents nor persisted exports are rewritten.
#[allow(clippy::too_many_arguments)] // Private checked entry shared by two compiler profiles.
pub(crate) fn admit_roots<'a>(
    document: &'a GraphDocument,
    snapshot: &RegistrySnapshot,
    analysis: &ValidationAnalysis,
    options: CompileOptions<'_>,
    data_plan: bool,
    roots: &[NodeId],
    controls: &BTreeSet<NodeId>,
) -> Result<Admission<'a>, CompileError> {
    options.checkpoint()?;
    if analysis.document_state() != document.semantic_state_id() {
        return Err(CompileError::StaleDocument);
    }
    if analysis.registry_generation() != snapshot.generation() {
        return Err(CompileError::StaleRegistry);
    }
    if !analysis.is_ready_for_lowering(document, snapshot) {
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
        if controls.contains(node) {
            charge(&mut ports, sig.ports.len(), options.limits.max_graph_items)?;
            continue;
        }
        // A data plan reads host state through a single-output input slot.
        let state_read = data_plan
            && sig.kind == NodeKind::Volatile
            && is_state_read(snapshot, &body.nodes[node]);
        if state_read {
            admit_state_read(*node, sig)?;
        }
        let call = body.nodes[node].definition.id.as_str() == ids::CALL;
        let zone = data_plan
            .then(|| crate::zones::marker(analysis, *node))
            .flatten();
        if sig.kind != NodeKind::Pure
            && !state_read
            && !(data_plan
                && sig.kind == NodeKind::Volatile
                && (call || is_seeded_entropy(snapshot, &body.nodes[node])))
        {
            return Err(CompileError::UnsupportedNode { node: *node });
        }
        if let Some(kind) = crate::lazy::Lazy::resolve(&body.nodes[node], sig)? {
            lazy_nodes.insert(*node, kind);
        }
        if body.nodes[node].definition.id.is_core()
            && body.nodes[node].definition.id.as_str() != ids::CONVERT
            && !lazy_nodes.contains_key(node)
            && !state_read
            && zone.is_none()
            && !(call
                && (sig.kind == NodeKind::Pure || (data_plan && sig.kind == NodeKind::Volatile)))
        {
            return Err(CompileError::UnsupportedNode { node: *node });
        }
        let output_source = if state_read {
            OutputSource::State
        } else if zone == Some(false) {
            OutputSource::Iteration
        } else {
            OutputSource::Computed
        };
        charge(&mut ports, sig.ports.len(), options.limits.max_graph_items)?;
        admit_data_ports(
            *node,
            sig,
            lazy_nodes.contains_key(node),
            output_source,
            options,
        )?;
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
    for root in roots {
        stack.push((*root, false));
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
            if controls.contains(&node) {
                continue;
            }
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
    Ok(Admission {
        incoming,
        lazy_nodes,
        host_nodes: controls.iter().map(|id| (0, *id)).collect(),
    })
}
