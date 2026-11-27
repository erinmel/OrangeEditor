//! Function body/interface conformance and return coverage (11 §14).
//!
//! A function summary says what a body does; conformance says the body honours its declared
//! interface, so callers may rely on that interface. Marker ports come from the declaration
//! (see `nodes::marker_contract`), so port, direction, kind, type and required-output
//! checks are the ordinary connection and input passes. This pass adds what they cannot see:
//! marker multiplicity against the inferred kind, return coverage, and whether anything in
//! the body is still unresolved or erroneous.

use crate::catalog::intrinsic_id as ids;
use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{RegistrySnapshot, intrinsic, project_interface};
use crate::document::{
    FunctionDecl, GraphDocument, NodeInstance, NodeKind, PortDirection, PortKind, SignatureSnapshot,
};
use crate::id::{FunctionId, NodeId, PortKey};

use super::diagnostic::{Diagnostic, DiagnosticCode, Subject};
use super::functions::FunctionSummary;
use super::ports::lookup_port;

#[derive(Debug, Default)]
pub(crate) struct Conformance {
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Functions whose complete summary may back an executable call contract.
    pub(crate) conformant: BTreeSet<FunctionId>,
}

/// Facts about the rest of the validation run that block a function from being accepted.
pub(crate) struct BodyStatus<'a> {
    pub(crate) regions: &'a super::regions::RegionIndex,
    pub(crate) resolved: &'a BTreeMap<NodeId, SignatureSnapshot>,
    /// Nodes that are unresolved, incomplete or carry an error diagnostic.
    pub(crate) blocked: &'a BTreeSet<NodeId>,
}

pub(crate) fn check_conformance(
    document: &GraphDocument,
    summaries: &BTreeMap<FunctionId, FunctionSummary>,
    status: &BodyStatus<'_>,
    registry: &RegistrySnapshot,
) -> Conformance {
    let mut result = Conformance::default();
    for (id, summary) in summaries {
        let Some(function) = document.functions().get(id) else {
            continue;
        };
        let mut check = Check {
            document,
            function,
            id: *id,
            status,
            registry,
            diagnostics: Vec::new(),
            provable: true,
        };
        check.run(summary.kind);
        let clean = check.diagnostics.is_empty()
            && check.provable
            && function
                .body
                .nodes
                .keys()
                .all(|node| !status.blocked.contains(node));
        result.diagnostics.append(&mut check.diagnostics);
        if clean {
            result.conformant.insert(*id);
        }
    }
    result
}

type ExecTargets = BTreeMap<NodeId, BTreeMap<PortKey, Vec<(NodeId, PortKey)>>>;
struct ReturnFlow {
    edges: ExecTargets,
    next: BTreeMap<super::regions::RegionRoot, Option<PortKey>>,
}
impl ReturnFlow {
    fn new(body: &crate::document::GraphBody, edges: ExecTargets) -> Self {
        let mut next = BTreeMap::new();
        for node in body
            .nodes
            .values()
            .filter(|node| node.definition.id.as_str() == ids::SEQUENCE)
        {
            let Some(order) = node.variadic.get(&crate::port_id!("then")) else {
                continue;
            };
            let mut continuation = None;
            for &key in order.order.iter().rev() {
                let port = PortKey::new(crate::port_id!("then"), Some(key));
                next.insert(
                    super::regions::RegionRoot {
                        node: node.id,
                        port: port.clone(),
                    },
                    continuation.clone(),
                );
                if edges
                    .get(&node.id)
                    .is_some_and(|ports| ports.contains_key(&port))
                {
                    continuation = Some(port);
                }
            }
        }
        Self { edges, next }
    }
}

struct Check<'a> {
    document: &'a GraphDocument,
    function: &'a FunctionDecl,
    id: FunctionId,
    status: &'a BodyStatus<'a>,
    registry: &'a RegistrySnapshot,
    diagnostics: Vec<Diagnostic>,
    /// False when a fact needed to prove coverage is unresolved elsewhere.
    provable: bool,
}

impl Check<'_> {
    fn emit(&mut self, code: DiagnosticCode, node: Option<NodeId>) {
        self.diagnostics.push(Diagnostic::new(
            code,
            Subject::new(node, None, None).with_function(self.id),
            self.registry.origin(),
        ));
    }

    fn markers(&self, definition: &str) -> Vec<NodeId> {
        self.function
            .body
            .nodes
            .values()
            .filter(|node: &&NodeInstance| {
                node.definition.id.as_str() == definition
                    && node.provider.is_none()
                    && node.definition.version == 1
                    && node.config_version == 1
            })
            .map(|node| node.id)
            .collect()
    }

    fn run(&mut self, kind: NodeKind) {
        if project_interface(self.function).is_err() {
            self.emit(DiagnosticCode::FunctionInterface, None);
            return;
        }
        let entries = self.markers(ids::FUNCTION_ENTRY);
        let returns = self.markers(ids::RETURN);
        let outputs = self.markers(ids::FUNCTION_OUTPUTS);
        let has_outputs = !self.function.outputs.is_empty();
        for extra in entries.iter().skip(1) {
            self.emit(DiagnosticCode::FunctionBodyMarker, Some(*extra));
        }

        if matches!(kind, NodeKind::Pure | NodeKind::Volatile) {
            // A data-only function yields its result through exactly one outputs node.
            for node in &returns {
                self.emit(DiagnosticCode::FunctionBodyMarker, Some(*node));
            }
            for extra in outputs.iter().skip(1) {
                self.emit(DiagnosticCode::FunctionBodyMarker, Some(*extra));
            }
            if has_outputs && outputs.is_empty() {
                self.emit(DiagnosticCode::FunctionBodyMarker, None);
            }
            return;
        }

        for node in &outputs {
            self.emit(DiagnosticCode::FunctionBodyMarker, Some(*node));
        }
        if has_outputs {
            self.check_return_coverage(entries.first().copied(), &returns);
        }
    }

    /// Follow natural region completion into the next Sequence step, or the enclosing
    /// region. Loop/Fork bodies complete their owner rather than requiring a Return.
    fn finish_path(
        &mut self,
        mut node: NodeId,
        origin: NodeId,
        port: Option<PortKey>,
        flow: &ReturnFlow,
        stack: &mut Vec<(NodeId, Option<PortKey>)>,
        flagged: &mut BTreeSet<(NodeId, Option<PortKey>)>,
    ) {
        loop {
            let Some(root) = self.status.regions.owners.get(&node) else {
                self.provable = false;
                return;
            };
            let Some(region) = self.status.regions.regions.get(root) else {
                self.provable = false;
                return;
            };
            match region.kind {
                super::regions::RegionKind::Entry => {
                    if flagged.insert((origin, port.clone())) {
                        self.diagnostics.push(Diagnostic::new(
                            DiagnosticCode::MissingReturn,
                            Subject::new(Some(origin), port, None).with_function(self.id),
                            self.registry.origin(),
                        ));
                    }
                    return;
                }
                super::regions::RegionKind::LoopBody | super::regions::RegionKind::Fork => return,
                super::regions::RegionKind::Sequence => {
                    let Some(next) = flow.next.get(root) else {
                        self.provable = false;
                        return;
                    };
                    if let Some(targets) = next.as_ref().and_then(|port| {
                        flow.edges.get(&root.node).and_then(|ports| ports.get(port))
                    }) {
                        stack.extend(
                            targets
                                .iter()
                                .map(|(node, port)| (*node, Some(port.clone()))),
                        );
                        return;
                    }
                    node = root.node;
                }
            }
        }
    }

    /// Every normal completion path must return results. Missing continuations may
    /// finish an owned Sequence step; silent state-control suppression does too.
    fn check_return_coverage(&mut self, entry: Option<NodeId>, returns: &[NodeId]) {
        let Some(entry) = entry.filter(|_| !returns.is_empty()) else {
            self.emit(DiagnosticCode::MissingReturn, None);
            return;
        };
        let returns: BTreeSet<_> = returns.iter().copied().collect();
        let mut mutable_gates = BTreeSet::new();
        let mut edges = ExecTargets::new();
        for edge in self.function.body.edges.values() {
            if matches!(edge.to.port.port().as_str(), "close" | "toggle") {
                mutable_gates.insert(edge.to.node);
            }
            match lookup_port(self.document, self.status.resolved, &edge.from) {
                Ok(port) if port.kind == PortKind::Exec => {
                    edges
                        .entry(edge.from.node)
                        .or_default()
                        .entry(edge.from.port.clone())
                        .or_default()
                        .push((edge.to.node, edge.to.port.clone()));
                }
                Ok(_) => {}
                Err(_) => self.provable = false,
            }
        }
        let flow = ReturnFlow::new(&self.function.body, edges);
        let mut seen = BTreeSet::new();
        let mut flagged = BTreeSet::new();
        let mut stack = vec![(entry, None)];
        while let Some((node_id, input)) = stack.pop() {
            if !seen.insert((node_id, input.clone())) || returns.contains(&node_id) {
                continue;
            }
            let (Some(node), Some(snapshot)) = (
                self.function.body.nodes.get(&node_id),
                self.status.resolved.get(&node_id),
            ) else {
                self.provable = false;
                continue;
            };
            let definition = node.definition.id.as_str();
            if definition == ids::SEQUENCE {
                let Some(order) = node.variadic.get(&crate::port_id!("then")) else {
                    self.provable = false;
                    continue;
                };
                let target = order.order.iter().find_map(|&key| {
                    flow.edges.get(&node_id).and_then(|ports| {
                        ports.get(&PortKey::new(crate::port_id!("then"), Some(key)))
                    })
                });
                if let Some(targets) = target {
                    stack.extend(
                        targets
                            .iter()
                            .map(|(node, port)| (*node, Some(port.clone()))),
                    );
                } else {
                    self.finish_path(node_id, node_id, None, &flow, &mut stack, &mut flagged);
                }
                continue;
            }
            let command = input
                .as_ref()
                .map(|port| port.port().as_str())
                .unwrap_or("in");
            let command_only = (definition == ids::GATE
                && matches!(command, "open" | "close" | "toggle"))
                || (definition == ids::DO_ONCE && command == "reset");
            let immutable_open_gate = definition == ids::GATE
                && node.config.get("start_open").and_then(|v| v.as_bool()) == Some(true)
                && !mutable_gates.contains(&node_id);
            if command_only
                || definition == ids::DO_ONCE
                || (definition == ids::GATE && !immutable_open_gate)
            {
                self.finish_path(node_id, node_id, None, &flow, &mut stack, &mut flagged);
                if command_only {
                    continue;
                }
            }
            if intrinsic(definition).is_some_and(|rule| rule.terminator) {
                continue;
            }
            let mut ports = BTreeSet::new();
            for port in snapshot
                .ports
                .iter()
                .filter(|p| p.kind == PortKind::Exec && p.direction == PortDirection::Output)
            {
                if port.variadic.is_some() {
                    if let Some(order) = node.variadic.get(&port.port_id) {
                        ports.extend(
                            order
                                .order
                                .iter()
                                .map(|key| PortKey::new(port.port_id.clone(), Some(*key))),
                        );
                    }
                } else {
                    ports.insert(PortKey::new(port.port_id.clone(), None));
                }
            }
            ports.extend(
                snapshot
                    .implicit_ports()
                    .into_iter()
                    .filter(|name| matches!(*name, "then" | "body" | "completed"))
                    .map(crate::id::fixed_id),
            );
            for port in ports {
                if matches!(port.port().as_str(), "failed" | "cancelled")
                    || (definition == ids::FORK && port.port().as_str() == "branch")
                    || (snapshot.kind == NodeKind::Iterator && port.port().as_str() == "body")
                {
                    continue;
                }
                if let Some(targets) = flow.edges.get(&node_id).and_then(|ports| ports.get(&port)) {
                    stack.extend(
                        targets
                            .iter()
                            .map(|(node, port)| (*node, Some(port.clone()))),
                    );
                } else {
                    self.finish_path(
                        node_id,
                        node_id,
                        Some(port),
                        &flow,
                        &mut stack,
                        &mut flagged,
                    );
                }
            }
        }
    }
}
