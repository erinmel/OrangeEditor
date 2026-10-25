use crate::compile::{
    Admission, ConstantFolder, Incoming, admit, charge, decode, is_state_read, key, port_type,
    value_bytes,
};
use crate::lazy::Lazy;
use crate::*;
use orange_model::GraphDocument;
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeKind, PortDescriptor, PortDirection, PortKind};
use orange_model::id::{NodeId, PortKey};
use orange_model::validation::ValidationAnalysis;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum SelectionMode {
    Dynamic,
    Static(Option<&'static str>),
}
struct Selection {
    selector: DataInput,
    mode: SelectionMode,
}
enum Task {
    Enter(NodeId),
    Choose(NodeId),
    Finish {
        node: NodeId,
        selection: Option<Selection>,
    },
}
struct Lowering<'a, R: EntropyKernelResolver> {
    document: &'a GraphDocument,
    analysis: &'a ValidationAnalysis,
    resolver: &'a R,
    options: CompileOptions<'a>,
    incoming: &'a Incoming<'a>,
    lazy: &'a BTreeMap<NodeId, Lazy>,
    folder: ConstantFolder<'a, R>,
    plan: DataPlan,
    completed: BTreeMap<NodeId, NodeIx>,
    outputs: BTreeMap<(NodeId, PortKey), SlotIx>,
    writers: BTreeMap<(NodeId, PortKey), NodeIx>,
    bytes: usize,
    host_nodes: &'a std::collections::BTreeSet<NodeId>,
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
        if let Some(value) = &value {
            let size = match value {
                FoldedValue::Value(value) => value_bytes(value),
                FoldedValue::Fault(_) => 64,
            };
            charge(
                &mut self.bytes,
                size,
                self.options.limits.max_constant_bytes,
            )?;
        }
        self.plan.types.push(port_type(node, port)?);
        self.plan.sources.push(SlotSource {
            node,
            port: key(port),
        });
        self.plan.initial.push(value);
        Ok(SlotIx(index))
    }
    fn input(&mut self, node: NodeId, port: &PortDescriptor) -> Result<DataInput, CompileError> {
        self.options.checkpoint()?;
        let port_key = key(port);
        if let Some(edge) = self.incoming.get(&(node, port_key.clone())) {
            let slot = *self
                .outputs
                .get(&(edge.from.node, edge.from.port.clone()))
                .ok_or(CompileError::ValidationIncomplete)?;
            let producer = *self
                .completed
                .get(&edge.from.node)
                .ok_or(CompileError::ValidationIncomplete)?;
            return Ok(DataInput {
                source: InputSource {
                    port: port_key,
                    slot,
                    edge: Some(edge.id),
                },
                producer: Some(producer),
            });
        }
        let literal = self.document.root().nodes[&node]
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
        work: &mut Vec<Task>,
    ) {
        for port in ports {
            if let Some(edge) = self.incoming.get(&(node, port)) {
                work.push(Task::Enter(edge.from.node));
            }
        }
    }
    fn lower(&mut self, root: NodeId) -> Result<(), CompileError> {
        let analysis = self.analysis;
        let document = self.document;
        let mut work = vec![Task::Enter(root)];
        while let Some(task) = work.pop() {
            self.options.checkpoint()?;
            let (node_id, selection) = match task {
                Task::Enter(node) => {
                    if self.completed.contains_key(&node) {
                        continue;
                    }
                    let sig = &analysis.report().resolved[&node];
                    if self.host_nodes.contains(&node) {
                        for port in sig.ports.iter().filter(|p| {
                            p.kind == PortKind::Data && p.direction == PortDirection::Output
                        }) {
                            let slot = self.push(node, port, None)?;
                            let index = NodeIx(
                                u32::try_from(self.plan.nodes.len())
                                    .map_err(|_| CompileError::ResourceLimit)?,
                            );
                            self.outputs.insert((node, key(port)), slot);
                            self.writers.insert((node, key(port)), index);
                            self.plan.host_inputs.push(index);
                            self.plan.nodes.push(DataNode {
                                index,
                                node,
                                stability: DataStability::Epoch,
                                inputs: Box::new([]),
                                outputs: Box::new([slot]),
                                operation: DataOperation::Input,
                            });
                            self.completed.insert(node, index);
                        }
                        continue;
                    }
                    if self.lazy.contains_key(&node) {
                        work.push(Task::Choose(node));
                        self.dependencies(node, [key(&sig.ports[0])], &mut work);
                    } else {
                        work.push(Task::Finish {
                            node,
                            selection: None,
                        });
                        self.dependencies(
                            node,
                            sig.ports
                                .iter()
                                .rev()
                                .filter(|p| p.direction == PortDirection::Input)
                                .map(key),
                            &mut work,
                        );
                    }
                    continue;
                }
                Task::Choose(node) => {
                    let sig = &analysis.report().resolved[&node];
                    let selector = self.input(node, &sig.ports[0])?;
                    let mode = match &self.plan.initial[selector.source.slot.index()] {
                        Some(FoldedValue::Fault(_)) => SelectionMode::Static(None),
                        Some(FoldedValue::Value(value)) => SelectionMode::Static(
                            self.lazy[&node]
                                .selected(value)
                                .ok_or(CompileError::KernelContract { node })?,
                        ),
                        None => SelectionMode::Dynamic,
                    };
                    let alternatives: Vec<_> = sig
                        .ports
                        .iter()
                        .rev()
                        .filter(|p| {
                            p.direction == PortDirection::Input && p.port_id != sig.ports[0].port_id
                        })
                        .filter(|p| match &mode {
                            SelectionMode::Dynamic => true,
                            SelectionMode::Static(selected) => {
                                Some(p.port_id.as_str()) == *selected
                            }
                        })
                        .map(key)
                        .collect();
                    work.push(Task::Finish {
                        node,
                        selection: Some(Selection { selector, mode }),
                    });
                    self.dependencies(node, alternatives, &mut work);
                    continue;
                }
                Task::Finish { node, selection } => (node, selection),
            };
            let sig = &analysis.report().resolved[&node_id];
            let inputs_ports: Vec<_> = sig
                .ports
                .iter()
                .filter(|p| p.direction == PortDirection::Input)
                .filter(|p| {
                    selection.as_ref().is_none_or(|selection| {
                        p.port_id == *selection.selector.source.port.port()
                            || match &selection.mode {
                                SelectionMode::Dynamic => true,
                                SelectionMode::Static(selected) => {
                                    Some(p.port_id.as_str()) == *selected
                                }
                            }
                    })
                })
                .collect();
            let output_ports: Vec<_> = sig
                .ports
                .iter()
                .filter(|p| p.direction == PortDirection::Output)
                .collect();
            let mut inputs = Vec::new();
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
            let constant = sig.kind == NodeKind::Pure
                && inputs
                    .iter()
                    .all(|i| self.plan.initial[i.source.slot.index()].is_some());
            let node = &document.root().nodes[&node_id];
            let operation = if constant {
                self.folder.pull(node_id)?;
                DataOperation::Constant
            } else if let Some(lazy) = self.lazy.get(&node_id) {
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
            } else if sig.kind == NodeKind::Pure {
                let binding = self
                    .resolver
                    .resolve(document, node, sig, self.plan.snapshot.clone())
                    .map_err(|error| binding_error(node_id, error))?;
                if !matches_abi(
                    &binding.inputs,
                    &binding.outputs,
                    &inputs_ports,
                    &output_ports,
                ) {
                    return Err(CompileError::KernelContract { node: node_id });
                }
                DataOperation::Pure(binding)
            } else {
                let binding = self
                    .resolver
                    .resolve_entropy(document, node, sig, self.plan.snapshot.clone())
                    .map_err(|error| binding_error(node_id, error))?;
                if !matches_abi(
                    &binding.inputs,
                    &binding.outputs,
                    &inputs_ports,
                    &output_ports,
                ) {
                    return Err(CompileError::KernelContract { node: node_id });
                }
                DataOperation::Entropy(binding)
            };
            let index = NodeIx(
                u32::try_from(self.plan.nodes.len()).map_err(|_| CompileError::ResourceLimit)?,
            );
            let mut outputs = Vec::new();
            for port in output_ports {
                let value = if constant {
                    Some(
                        self.folder
                            .folded(node_id, &key(port))
                            .ok_or(CompileError::ValidationIncomplete)?
                            .clone(),
                    )
                } else {
                    None
                };
                let slot = self.push(node_id, port, value)?;
                self.outputs.insert((node_id, key(port)), slot);
                self.writers.insert((node_id, key(port)), index);
                outputs.push(slot);
            }
            if matches!(operation, DataOperation::Input) {
                self.plan.host_inputs.push(index);
            }
            self.plan.nodes.push(DataNode {
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
            self.completed.insert(node_id, index);
        }
        Ok(())
    }
}
fn matches_abi(
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
fn binding_error(node: NodeId, error: ResolveError) -> CompileError {
    match error {
        ResolveError::Unavailable => CompileError::KernelUnavailable { node },
        ResolveError::Contract => CompileError::KernelContract { node },
    }
}

/// Compile fresh root pure/seeded-entropy computed exports. Static selectors prune
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
    let roots: Vec<_> = document
        .exports()
        .iter()
        .map(|export| DemandRoot {
            id: export.id.clone(),
            node: export.node,
            port: export.port.clone(),
            input: false,
        })
        .collect();
    lower_roots(
        document, snapshot, analysis, resolver, options, admission, &roots,
    )
}

pub(crate) struct DemandRoot {
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
) -> Result<DataPlan, CompileError> {
    let folder = ConstantFolder::new(
        document,
        analysis,
        snapshot.clone(),
        resolver,
        options,
        &admission.incoming,
        &admission.lazy_nodes,
    )?;
    let mut state = Lowering {
        document,
        analysis,
        resolver,
        options,
        incoming: &admission.incoming,
        lazy: &admission.lazy_nodes,
        folder,
        plan: DataPlan {
            snapshot,
            graph: document.graph_id(),
            revision: document.revision(),
            state: document.state_id(),
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
        completed: BTreeMap::new(),
        outputs: BTreeMap::new(),
        writers: BTreeMap::new(),
        bytes: 0,
        host_nodes: &admission.host_nodes,
    };
    for export in roots {
        charge(
            &mut state.bytes,
            export.id.len(),
            options.limits.max_constant_bytes,
        )?;
    }
    for export in roots {
        options.checkpoint()?;
        let (slot, producer) = if export.input {
            if let Some(edge) = admission.incoming.get(&(export.node, export.port.clone())) {
                state.lower(edge.from.node)?;
                let slot = state.outputs[&(edge.from.node, edge.from.port.clone())];
                let producer = state.writers[&(edge.from.node, edge.from.port.clone())];
                (slot, producer)
            } else {
                let port = analysis.report().resolved[&export.node]
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
            state.lower(export.node)?;
            let slot = *state
                .outputs
                .get(&(export.node, export.port.clone()))
                .ok_or(CompileError::ValidationIncomplete)?;
            let producer = state.writers[&(export.node, export.port.clone())];
            (slot, producer)
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
    options.checkpoint()?;
    Ok(state.plan)
}
