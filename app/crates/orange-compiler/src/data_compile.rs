use crate::compile::{
    Admission, ConstantFolder, FAULT_CHARGE, Incoming, admit, binding_error, charge, decode,
    is_state_read, key, matches_abi, port_type, value_bytes,
};
use crate::lazy::Lazy;
use crate::*;
use orange_model::GraphDocument;
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeKind, PortDescriptor, PortDirection, PortKind};
use orange_model::id::{NodeId, PortKey};
use orange_model::validation::ValidationAnalysis;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Copy)]
enum SelectionMode {
    Dynamic,
    Static(Option<&'static str>),
}
impl SelectionMode {
    /// Whether a lazy alternative port can be selected in this mode.
    fn admits(&self, port: &orange_model::id::PortId) -> bool {
        match self {
            Self::Dynamic => true,
            Self::Static(selected) => Some(port.as_str()) == *selected,
        }
    }
}
struct Selection {
    selector: DataInput,
    mode: SelectionMode,
}
enum Task {
    ReturnCall {
        node: NodeId,
        child: usize,
        inputs: Vec<DataInput>,
    },
    Enter(NodeId),
    Choose(NodeId),
    Finish {
        node: NodeId,
        selection: Option<Selection>,
    },
}
#[derive(Clone, Copy)]
struct ProducedOutput {
    slot: SlotIx,
    producer: Option<NodeIx>,
}

struct Lowering<'a, R: EntropyKernelResolver> {
    scope: usize,
    zones: crate::zones::Zones,
    calls: Arc<crate::calls::Calls<'a>>,
    document: &'a GraphDocument,
    analysis: &'a ValidationAnalysis,
    resolver: &'a R,
    options: CompileOptions<'a>,
    incoming: &'a Incoming<'a>,
    lazy: &'a BTreeMap<NodeId, Lazy>,
    folder: ConstantFolder<'a, R>,
    plan: DataPlan,
    completed: BTreeSet<(usize, NodeId)>,
    outputs: BTreeMap<(usize, NodeId, PortKey), ProducedOutput>,
    work: Vec<(usize, Task)>,
    bytes: usize,
    seeded: BTreeMap<usize, SlotIx>,
    host_nodes: &'a std::collections::BTreeSet<(usize, NodeId)>,
}
impl<R: EntropyKernelResolver> Lowering<'_, R> {
    fn push(
        &mut self,
        node: NodeId,
        port: &PortDescriptor,
        value: Option<FoldedValue>,
    ) -> Result<SlotIx, CompileError> {
        if self.plan.initial.len() >= self.options.limits.max_slots {
            return Err(CompileError::ResourceLimit);
        }
        let index =
            u32::try_from(self.plan.initial.len()).map_err(|_| CompileError::ResourceLimit)?;
        let ty = port_type(node, port)?;
        if let Some(value) = &value {
            let size = match value {
                FoldedValue::Value(value) => value_bytes(&ty, value)?,
                FoldedValue::Fault(_) => FAULT_CHARGE,
            };
            charge(
                &mut self.bytes,
                size,
                self.options.limits.max_constant_bytes,
            )?;
        }
        self.plan.types.push(ty);
        self.plan.sources.push(SlotSource {
            node,
            port: key(port),
        });
        self.plan.slot_calls.push(self.scope.checked_sub(1));
        self.plan.initial.push(value);
        Ok(SlotIx(index))
    }
    fn input(&mut self, node: NodeId, port: &PortDescriptor) -> Result<DataInput, CompileError> {
        self.options.checkpoint()?;
        let port_key = key(port);
        if let Some(edge) = self
            .calls
            .incoming(self.incoming, self.scope)
            .get(&(node, port_key.clone()))
        {
            let ProducedOutput { slot, producer } = *self
                .outputs
                .get(&(self.scope, edge.from.node, edge.from.port.clone()))
                .ok_or(CompileError::ValidationIncomplete)?;
            return Ok(DataInput {
                source: InputSource {
                    port: port_key,
                    slot,
                    edge: Some(edge.id),
                },
                producer,
            });
        }
        let literal = self.calls.body(self.document, self.scope).nodes[&node]
            .inputs
            .get(&port_key)
            .or(port.default.as_ref())
            .ok_or_else(|| CompileError::InvalidLiteral {
                node,
                port: port_key.clone(),
            })?;
        let ty = port_type(node, port)?;
        let value = decode(
            literal,
            &ty,
            &self.plan.snapshot,
            self.options
                .limits
                .max_constant_bytes
                .saturating_sub(self.bytes),
        )?
        .ok_or_else(|| CompileError::InvalidLiteral {
            node,
            port: port_key.clone(),
        })?;
        let slot = self.push(node, port, Some(FoldedValue::Value(value)))?;
        Ok(DataInput {
            source: InputSource {
                port: port_key,
                slot,
                edge: None,
            },
            producer: None,
        })
    }
    fn dependencies(
        &self,
        node: NodeId,
        ports: impl IntoIterator<Item = PortKey>,
        work: &mut Vec<(usize, Task)>,
    ) {
        for port in ports {
            if let Some(edge) = self
                .calls
                .incoming(self.incoming, self.scope)
                .get(&(node, port))
            {
                work.push((self.scope, Task::Enter(edge.from.node)));
            }
        }
    }
    fn lower(&mut self, scope: usize, root: NodeId) -> Result<(), CompileError> {
        let mut work = std::mem::take(&mut self.work);
        work.push((scope, Task::Enter(root)));
        while let Some((scope, task)) = work.pop() {
            self.scope = scope;
            self.options.checkpoint()?;
            match task {
                Task::ReturnCall {
                    node,
                    child,
                    inputs,
                } => self.return_call(node, child, inputs)?,
                Task::Enter(node) => self.enter(node, &mut work)?,
                Task::Choose(node) => self.choose(node, &mut work)?,
                Task::Finish { node, selection } => self.finish(node, selection, &mut work)?,
            }
        }
        self.work = work;
        Ok(())
    }

    fn next_index(&self) -> Result<NodeIx, CompileError> {
        u32::try_from(self.plan.nodes.len())
            .map(NodeIx)
            .map_err(|_| CompileError::ResourceLimit)
    }

    /// Allocate `port`'s slot in the current scope as an output of instruction `index`.
    fn produce(
        &mut self,
        node: NodeId,
        port: &PortDescriptor,
        index: NodeIx,
        value: Option<FoldedValue>,
    ) -> Result<SlotIx, CompileError> {
        let slot = self.push(node, port, value)?;
        self.outputs.insert(
            (self.scope, node, key(port)),
            ProducedOutput {
                slot,
                producer: Some(index),
            },
        );
        Ok(slot)
    }

    /// Every output port of `node` as an output of instruction `index`, without values.
    fn produce_all(&mut self, node: NodeId, index: NodeIx) -> Result<Box<[SlotIx]>, CompileError> {
        let calls = self.calls.clone();
        let sig = calls.signature(self.analysis, self.scope, node);
        let mut outputs = Vec::new();
        for port in sig
            .ports
            .iter()
            .filter(|p| p.direction == PortDirection::Output)
        {
            outputs.push(self.produce(node, port, index, None)?);
        }
        Ok(outputs.into())
    }

    /// Finish `node` after demanding every input it declares.
    fn demand_inputs(&self, node: NodeId, work: &mut Vec<(usize, Task)>) {
        work.push((
            self.scope,
            Task::Finish {
                node,
                selection: None,
            },
        ));
        let sig = self.calls.signature(self.analysis, self.scope, node);
        self.dependencies(
            node,
            sig.ports
                .iter()
                .rev()
                .filter(|p| p.direction == PortDirection::Input)
                .map(key),
            work,
        );
    }

    /// The call node after its callee's result is lowered in the child scope.
    fn return_call(
        &mut self,
        node: NodeId,
        child: usize,
        mut inputs: Vec<DataInput>,
    ) -> Result<(), CompileError> {
        let (analysis, calls, scope) = (self.analysis, self.calls.clone(), self.scope);
        let result = calls
            .result(child)
            .ok_or(CompileError::ValidationIncomplete)?;
        let arguments = inputs.len();
        self.scope = child;
        for port in &calls.signature(analysis, child, result).ports {
            inputs.push(self.input(result, port)?);
        }
        self.scope = scope;
        let index = self.next_index()?;
        let outputs = self.produce_all(node, index)?;
        self.plan.nodes.push(DataNode {
            zone: self.zones.owner(analysis, scope, node),
            call: scope.checked_sub(1),
            index,
            node,
            stability: DataStability::Epoch,
            inputs: inputs.into(),
            outputs,
            operation: DataOperation::Call { arguments },
        });
        self.completed.insert((scope, node));
        Ok(())
    }

    fn enter(&mut self, node: NodeId, work: &mut Vec<(usize, Task)>) -> Result<(), CompileError> {
        let (analysis, calls, scope) = (self.analysis, self.calls.clone(), self.scope);
        if self.completed.contains(&(scope, node)) {
            return Ok(());
        }
        let sig = calls.signature(analysis, scope, node);
        if self.host_nodes.contains(&(scope, node)) {
            // Each data output of a host-owned node is its own epoch input.
            let mut ports: Vec<_> = sig
                .ports
                .iter()
                .filter(|p| p.kind == PortKind::Data && p.direction == PortDirection::Output)
                .cloned()
                .collect();
            if sig.implicit_ports().contains(&"error")
                && !ports.iter().any(|p| p.port_id.as_str() == "error")
            {
                ports.push(PortDescriptor::data_out(
                    "error",
                    "core/error@1".parse().expect("fixed type"),
                    orange_model::document::OutputSource::Execution,
                ));
            }
            for port in &ports {
                let index = self.next_index()?;
                let slot = self.produce(node, port, index, None)?;
                self.plan.host_inputs.push(index);
                self.plan.nodes.push(DataNode {
                    zone: None,
                    call: scope.checked_sub(1),
                    index,
                    node,
                    stability: DataStability::Epoch,
                    inputs: Box::new([]),
                    outputs: Box::new([slot]),
                    operation: DataOperation::Input,
                });
                self.completed.insert((scope, node));
            }
            return Ok(());
        }
        if crate::zones::marker(analysis, node) == Some(false) {
            // A zone begin: iteration parameters published by its running zone.
            let zone = self
                .zones
                .owner(analysis, scope, node)
                .ok_or(CompileError::ValidationIncomplete)?;
            let index = self.next_index()?;
            let outputs = self.produce_all(node, index)?;
            self.zones.layouts[zone.index()].parameters = Some(index);
            self.plan.nodes.push(DataNode {
                zone: Some(zone),
                call: scope.checked_sub(1),
                index,
                node,
                stability: DataStability::Epoch,
                inputs: Box::new([]),
                outputs,
                operation: DataOperation::Iteration(zone),
            });
            self.completed.insert((scope, node));
            return Ok(());
        }
        if let Some(zone) = self.zones.end(scope, node) {
            // A zone end strictly demands its body inputs and its begin's prefix.
            self.demand_inputs(node, work);
            let begin = self.zones.layouts[zone.index()].begin;
            work.push((scope, Task::Enter(begin)));
            self.dependencies(
                begin,
                calls
                    .signature(analysis, scope, begin)
                    .ports
                    .iter()
                    .rev()
                    .filter(|p| p.direction == PortDirection::Input)
                    .map(key),
                work,
            );
        } else if calls.lazy(self.lazy, scope, node).is_some() {
            work.push((scope, Task::Choose(node)));
            self.dependencies(node, [key(&sig.ports[0])], work);
        } else {
            self.demand_inputs(node, work);
        }
        Ok(())
    }

    /// After a lazy node's selector is lowered, demand only the alternatives it can take.
    fn choose(&mut self, node: NodeId, work: &mut Vec<(usize, Task)>) -> Result<(), CompileError> {
        let (analysis, calls, scope) = (self.analysis, self.calls.clone(), self.scope);
        let sig = calls.signature(analysis, scope, node);
        let selector = self.input(node, &sig.ports[0])?;
        let mode = match &self.plan.initial[selector.source.slot.index()] {
            Some(FoldedValue::Fault(_)) => SelectionMode::Static(None),
            Some(FoldedValue::Value(value)) => SelectionMode::Static(
                calls
                    .lazy(self.lazy, scope, node)
                    .ok_or(CompileError::ValidationIncomplete)?
                    .selected(value)
                    .ok_or(CompileError::KernelContract { node })?,
            ),
            None => SelectionMode::Dynamic,
        };
        let alternatives: Vec<_> = sig
            .ports
            .iter()
            .rev()
            .filter(|p| p.direction == PortDirection::Input && p.port_id != sig.ports[0].port_id)
            .filter(|p| mode.admits(&p.port_id))
            .map(key)
            .collect();
        work.push((
            scope,
            Task::Finish {
                node,
                selection: Some(Selection { selector, mode }),
            },
        ));
        self.dependencies(node, alternatives, work);
        Ok(())
    }

    /// Emit `node_id`'s instruction once its demanded inputs are lowered, or expand a
    /// call into its child scope.
    fn finish(
        &mut self,
        node_id: NodeId,
        selection: Option<Selection>,
        work: &mut Vec<(usize, Task)>,
    ) -> Result<(), CompileError> {
        let (analysis, calls, scope) = (self.analysis, self.calls.clone(), self.scope);
        let document = self.document;
        let sig = calls.signature(analysis, scope, node_id);
        let inputs_ports: Vec<_> = sig
            .ports
            .iter()
            .filter(|p| p.direction == PortDirection::Input)
            .filter(|p| {
                selection.as_ref().is_none_or(|selection| {
                    p.port_id == *selection.selector.source.port.port()
                        || selection.mode.admits(&p.port_id)
                })
            })
            .collect();
        let output_ports: Vec<_> = sig
            .ports
            .iter()
            .filter(|p| p.direction == PortDirection::Output)
            .collect();
        let zone_end = self.zones.end(scope, node_id);
        let mut inputs = Vec::new();
        if let Some(zone) = zone_end {
            let begin = self.zones.layouts[zone.index()].begin;
            for port in calls
                .signature(analysis, scope, begin)
                .ports
                .iter()
                .filter(|p| p.direction == PortDirection::Input)
            {
                inputs.push(self.input(begin, port)?);
            }
        }
        let prefix = inputs.len();
        for port in &inputs_ports {
            inputs.push(
                if let Some(selection) = selection
                    .as_ref()
                    .filter(|s| s.selector.source.port.port() == &port.port_id)
                {
                    selection.selector.clone()
                } else {
                    self.input(node_id, port)?
                },
            );
        }
        let child = calls.scopes[scope].calls.get(&node_id).copied();
        let constant = sig.kind == NodeKind::Pure
            && zone_end.is_none()
            && child.is_none_or(|scope| !self.zones.contains(scope))
            && inputs
                .iter()
                .all(|i| self.plan.initial[i.source.slot.index()].is_some());
        if !constant && let Some(child) = child {
            return self.expand_call(node_id, child, inputs, work);
        }
        let node = &calls.body(document, scope).nodes[&node_id];
        let operation = if let Some(zone) = zone_end {
            DataOperation::Zone { zone, prefix }
        } else if constant {
            self.folder.pull_at(scope, node_id)?;
            DataOperation::Constant
        } else if let Some(lazy) = calls.lazy(self.lazy, scope, node_id) {
            match lazy {
                Lazy::Select => {
                    let selected = selection
                        .as_ref()
                        .is_some_and(|s| matches!(s.mode, SelectionMode::Static(_)));
                    DataOperation::Select {
                        when_true: 1,
                        when_false: if selected { 1 } else { 2 },
                    }
                }
                Lazy::And => DataOperation::And,
                Lazy::Or => DataOperation::Or,
                Lazy::Coalesce => DataOperation::Coalesce,
                Lazy::Try => DataOperation::Try {
                    boxed: port_type(node_id, &sig.ports[0])?.unboxed_error(),
                },
            }
        } else if is_state_read(&self.plan.snapshot, node) {
            DataOperation::Input
        } else {
            let abi = |inputs: &[PortKey], outputs: &[PortKey]| {
                matches_abi(inputs, outputs, &inputs_ports, &output_ports)
                    .then_some(())
                    .ok_or(CompileError::KernelContract { node: node_id })
            };
            let snapshot = self.plan.snapshot.clone();
            if sig.kind == NodeKind::Pure {
                let binding = self
                    .resolver
                    .resolve(document, node, sig, snapshot)
                    .map_err(|error| binding_error(node_id, error))?;
                abi(&binding.inputs, &binding.outputs)?;
                DataOperation::Pure(binding)
            } else {
                let binding = self
                    .resolver
                    .resolve_entropy(document, node, sig, snapshot)
                    .map_err(|error| binding_error(node_id, error))?;
                abi(&binding.inputs, &binding.outputs)?;
                DataOperation::Entropy(binding)
            }
        };
        let index = self.next_index()?;
        let mut outputs = Vec::new();
        for port in output_ports {
            let value = if constant {
                Some(
                    self.folder
                        .folded_at(scope, node_id, &key(port))
                        .ok_or(CompileError::ValidationIncomplete)?
                        .clone(),
                )
            } else {
                None
            };
            outputs.push(self.produce(node_id, port, index, value)?);
        }
        let input = matches!(operation, DataOperation::Input);
        if input {
            self.plan.host_inputs.push(index);
        }
        self.plan.nodes.push(DataNode {
            zone: if input {
                None
            } else {
                self.zones.owner(analysis, scope, node_id)
            },
            call: scope.checked_sub(1),
            index,
            node: node_id,
            stability: if constant {
                DataStability::Const
            } else {
                DataStability::Epoch
            },
            inputs: inputs.into(),
            outputs: outputs.into(),
            operation,
        });
        self.completed.insert((scope, node_id));
        Ok(())
    }

    /// Bind a non-constant call's arguments as its child entry's outputs, then lower its
    /// result in the child scope before the call itself returns.
    fn expand_call(
        &mut self,
        node: NodeId,
        child: usize,
        inputs: Vec<DataInput>,
        work: &mut Vec<(usize, Task)>,
    ) -> Result<(), CompileError> {
        let (analysis, calls, scope) = (self.analysis, self.calls.clone(), self.scope);
        if let Some(entry) = calls.entry(child) {
            for input in &inputs {
                self.options.checkpoint()?;
                let slot = input.source.slot;
                if let Some(value) = self.plan.initial[slot.index()].as_ref() {
                    let constant = match self.seeded.get(&slot.index()) {
                        Some(constant) => *constant,
                        None => {
                            let constant = self.folder.seed_constant(
                                self.plan.types[slot.index()].clone(),
                                self.plan.sources[slot.index()].clone(),
                                value.clone(),
                                self.plan.slot_calls[slot.index()],
                            )?;
                            self.seeded.insert(slot.index(), constant);
                            constant
                        }
                    };
                    self.folder
                        .alias_constant(child, entry, input.source.port.clone(), constant);
                }
                self.outputs.insert(
                    (child, entry, input.source.port.clone()),
                    ProducedOutput {
                        slot: input.source.slot,
                        producer: input.producer,
                    },
                );
            }
            self.completed.insert((child, entry));
        }
        let result = calls
            .result(child)
            .ok_or(CompileError::ValidationIncomplete)?;
        work.push((
            scope,
            Task::ReturnCall {
                node,
                child,
                inputs,
            },
        ));
        self.scope = child;
        self.dependencies(
            result,
            calls
                .signature(analysis, child, result)
                .ports
                .iter()
                .rev()
                .map(key),
            work,
        );
        Ok(())
    }
}
/// Compile fresh pure/seeded-entropy computed exports and graph-local Pure/seeded-volatile calls.
/// Static selectors prune
/// inactive alternatives; epoch-dependent selectors retain both possible arms.
/// Eligible constant islands in those possible arms may fold during compilation.
pub fn compile_data(
    document: &GraphDocument,
    snapshot: Arc<RegistrySnapshot>,
    analysis: &ValidationAnalysis,
    resolver: &impl EntropyKernelResolver,
    options: CompileOptions<'_>,
) -> Result<DataPlan, CompileError> {
    let admission = admit(document, &snapshot, analysis, options, true)?;
    let calls = Arc::new(crate::calls::Calls::prepare(
        document,
        &snapshot,
        analysis,
        options,
        crate::calls::CallProfile::HostData,
    )?);
    let roots: Vec<_> = document
        .exports()
        .iter()
        .map(|export| DemandRoot {
            scope: 0,
            id: export.id.clone(),
            node: export.node,
            port: export.port.clone(),
            input: false,
        })
        .collect();
    lower_roots(
        document, snapshot, analysis, resolver, options, admission, &roots, calls,
    )
}

pub(crate) struct DemandRoot {
    pub(crate) scope: usize,
    pub(crate) id: String,
    pub(crate) node: NodeId,
    pub(crate) port: PortKey,
    pub(crate) input: bool,
}

#[allow(clippy::too_many_arguments)] // Private original-document lowering seam.
pub(crate) fn lower_roots(
    document: &GraphDocument,
    snapshot: Arc<RegistrySnapshot>,
    analysis: &ValidationAnalysis,
    resolver: &impl EntropyKernelResolver,
    options: CompileOptions<'_>,
    admission: Admission<'_>,
    roots: &[DemandRoot],
    calls: Arc<crate::calls::Calls<'_>>,
) -> Result<DataPlan, CompileError> {
    let mut bytes = calls.signature_bytes;
    let zones = crate::zones::Zones::prepare(document, analysis, &calls, options, &mut bytes)?;
    let folder = ConstantFolder::new(
        document,
        analysis,
        snapshot.clone(),
        resolver,
        options,
        &admission.incoming,
        &admission.lazy_nodes,
        calls,
    )?;
    let mut state = Lowering {
        scope: 0,
        zones,
        calls: folder.calls.clone(),
        document,
        analysis,
        resolver,
        options,
        incoming: &admission.incoming,
        lazy: &admission.lazy_nodes,
        plan: DataPlan {
            zones: vec![],
            snapshot,
            graph: document.graph_id(),
            revision: document.revision(),
            state: document.semantic_state_id(),
            calls: folder.calls.sites.clone(),
            slot_calls: vec![],
            nodes: vec![],
            types: vec![],
            sources: vec![],
            initial: vec![],
            exports: vec![],
            export_nodes: vec![],
            host_inputs: vec![],
            diagnostics: vec![],
            resources: DataResources {
                slots: 0,
                nodes: 0,
                max_inputs: 0,
                max_outputs: 0,
                work_items: 0,
                entropy_nodes: 0,
            },
        },
        folder,
        completed: BTreeSet::new(),
        outputs: BTreeMap::new(),
        work: Vec::new(),
        bytes,
        seeded: BTreeMap::new(),
        host_nodes: &admission.host_nodes,
    };
    for export in roots {
        charge(
            &mut state.bytes,
            export.id.len(),
            options.limits.max_constant_bytes,
        )?;
    }
    let calls = state.calls.clone();
    for export in roots {
        options.checkpoint()?;
        let scope = export.scope;
        state.scope = scope;
        let (slot, producer) = if export.input {
            if let Some(edge) = calls
                .incoming(&admission.incoming, scope)
                .get(&(export.node, export.port.clone()))
            {
                state.lower(scope, edge.from.node)?;
                let ProducedOutput { slot, producer } =
                    state.outputs[&(scope, edge.from.node, edge.from.port.clone())];
                (slot, producer.ok_or(CompileError::ValidationIncomplete)?)
            } else {
                let port = calls
                    .signature(analysis, scope, export.node)
                    .ports
                    .iter()
                    .find(|p| key(p) == export.port)
                    .ok_or(CompileError::ValidationIncomplete)?;
                let input = state.input(export.node, port)?;
                let index = NodeIx(
                    u32::try_from(state.plan.nodes.len())
                        .map_err(|_| CompileError::ResourceLimit)?,
                );
                state.plan.nodes.push(DataNode {
                    zone: None,
                    call: scope.checked_sub(1),
                    index,
                    node: export.node,
                    stability: DataStability::Const,
                    inputs: Box::new([]),
                    outputs: Box::new([input.source.slot]),
                    operation: DataOperation::Constant,
                });
                (input.source.slot, index)
            }
        } else {
            state.lower(scope, export.node)?;
            let ProducedOutput { slot, producer } = *state
                .outputs
                .get(&(scope, export.node, export.port.clone()))
                .ok_or(CompileError::ValidationIncomplete)?;
            (slot, producer.ok_or(CompileError::ValidationIncomplete)?)
        };
        state.plan.exports.push(Export {
            id: export.id.clone(),
            slot,
        });
        state.plan.export_nodes.push(producer);
    }
    state.plan.diagnostics = state.folder.diagnostics().to_vec();
    orange_model::validation::sort_diagnostics(&mut state.plan.diagnostics);
    let mut resources = DataResources {
        slots: state.plan.initial.len(),
        nodes: state.plan.nodes.len(),
        max_inputs: 0,
        max_outputs: 0,
        work_items: 1,
        entropy_nodes: 0,
    };
    for node in &state.plan.nodes {
        options.checkpoint()?;
        resources.max_inputs = resources.max_inputs.max(node.inputs.len());
        resources.max_outputs = resources.max_outputs.max(node.outputs.len());
        resources.entropy_nodes = resources
            .entropy_nodes
            .checked_add(usize::from(matches!(
                node.operation,
                DataOperation::Entropy(_)
            )))
            .ok_or(CompileError::ResourceLimit)?;
        let additions = node
            .inputs
            .iter()
            .filter(|i| i.producer.is_some())
            .count()
            .checked_add(2)
            .ok_or(CompileError::ResourceLimit)?;
        resources.work_items = resources
            .work_items
            .checked_add(additions)
            .ok_or(CompileError::ResourceLimit)?;
    }
    state.plan.resources = resources;
    state.plan.zones = state.zones.layouts;
    options.checkpoint()?;
    Ok(state.plan)
}
