//! Function body/interface conformance and return coverage (11 §14).
//!
//! A function summary says what a body does; conformance says the body honours its declared
//! interface, so callers may rely on that interface. Marker ports come from the declaration
//! (see `nodes::marker_contract`), so port, direction, kind, type and required-output
//! checks are the ordinary connection and input passes. This pass adds what they cannot see:
//! marker multiplicity against the inferred kind, return coverage, and whether anything in
//! the body is still unresolved or erroneous.

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
        let entries = self.markers("core/function-entry");
        let returns = self.markers("core/return");
        let outputs = self.markers("core/function-outputs");
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

    /// Every exec path from the entry in an action function with outputs reaches a Return.
    ///
    /// A path ends at an exec output that has no connection. Faults (`failed`), cancellation,
    /// a loop's `body` and paths inside a loop body (which continue the loop) do not end the
    /// function. `core/fork` branches are exempt because a Return may not appear in them.
    fn check_return_coverage(&mut self, entry: Option<NodeId>, returns: &[NodeId]) {
        let Some(entry) = entry.filter(|_| !returns.is_empty()) else {
            self.emit(DiagnosticCode::MissingReturn, None);
            return;
        };
        let mut exec_out: BTreeMap<NodeId, BTreeMap<String, Vec<NodeId>>> = BTreeMap::new();
        for edge in self.function.body.edges.values() {
            match lookup_port(self.document, self.status.resolved, &edge.from) {
                Ok(port) if port.kind == PortKind::Exec => {
                    exec_out
                        .entry(edge.from.node)
                        .or_default()
                        .entry(edge.from.port.port().as_str().to_owned())
                        .or_default()
                        .push(edge.to.node);
                }
                Ok(_) => {}
                Err(_) => self.provable = false,
            }
        }

        let mut seen = BTreeSet::new();
        let mut flagged = BTreeSet::new();
        let mut stack = vec![(entry, false)];
        while let Some((node_id, in_loop)) = stack.pop() {
            if !seen.insert((node_id, in_loop)) || returns.contains(&node_id) {
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
            let terminator = intrinsic(definition).is_some_and(|rule| rule.terminator);
            let fork = definition == "core/fork";
            let is_loop = snapshot.kind == NodeKind::Iterator;
            let mut ports: BTreeSet<String> = snapshot
                .ports
                .iter()
                .filter(|port| {
                    port.kind == PortKind::Exec && port.direction == PortDirection::Output
                })
                .map(|port| port.port_id.as_str().to_owned())
                .collect();
            ports.extend(
                snapshot
                    .implicit_ports()
                    .into_iter()
                    .filter(|name| matches!(*name, "then" | "body" | "completed"))
                    .map(str::to_owned),
            );
            if terminator {
                ports.clear();
            }
            for port in ports {
                let targets = exec_out.get(&node_id).and_then(|map| map.get(&port));
                let loop_body = is_loop && port == "body";
                if targets.is_none() && !fork && !loop_body && !in_loop {
                    if flagged.insert((node_id, port.clone())) {
                        self.diagnostics.push(Diagnostic::new(
                            DiagnosticCode::MissingReturn,
                            Subject::new(
                                Some(node_id),
                                Some(PortKey::new(
                                    port.parse().expect("declared port id is valid"),
                                    None,
                                )),
                                None,
                            )
                            .with_function(self.id),
                            self.registry.origin(),
                        ));
                    }
                    continue;
                }
                for target in targets.into_iter().flatten() {
                    stack.push((*target, in_loop || loop_body));
                }
            }
        }
    }
}
