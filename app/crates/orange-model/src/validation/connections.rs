//! Structural validation of persisted graph edges (11 §§4.2–4.3).

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, PortDescriptor, PortKind, PortRef, SignatureSnapshot};
use crate::id::{EdgeId, NodeId};
use crate::index::DocIndex;
use crate::type_term::TypeTerm;

use super::diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, Severity, Subject, sort_diagnostics,
};
use super::ports::{PortLookupError, lookup_port};

/// Result of validating the graph's connections using currently available metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionValidation {
    pub diagnostics: Vec<Diagnostic>,
    /// Nodes whose signatures or unresolved generic types prevented a complete check.
    pub incomplete_nodes: BTreeSet<NodeId>,
}

impl ConnectionValidation {
    /// True only when validation completed and produced no error diagnostics.
    pub fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != Severity::Error)
    }
}

/// Validates stored edges against the fresh node resolution `resolved` (from
/// [`validate_nodes`](super::validate_nodes)) without changing the document. A package
/// node that did not resolve is checked against its stored snapshot (11 §4.5).
pub fn validate_connections(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> ConnectionValidation {
    let origin = registry.origin();
    let index = DocIndex::build(document);
    let mut report = ConnectionValidation::default();
    let mut diagnostics = Vec::new();
    let mut edges: Vec<_> = document
        .bodies_with_refs()
        .flat_map(|(body_ref, body)| body.edges.values().map(move |edge| (body_ref, edge)))
        .collect();
    edges.sort_by_key(|(_, edge)| edge.id);

    // Cardinality counts persisted edges even when their opposite endpoint is malformed.
    let mut incoming: BTreeMap<(NodeId, crate::id::PortKey), Vec<EdgeId>> = BTreeMap::new();
    let mut outgoing: BTreeMap<(NodeId, crate::id::PortKey), Vec<EdgeId>> = BTreeMap::new();
    for (_, edge) in &edges {
        incoming
            .entry((edge.to.node, edge.to.port.clone()))
            .or_default()
            .push(edge.id);
        outgoing
            .entry((edge.from.node, edge.from.port.clone()))
            .or_default()
            .push(edge.id);
    }

    let mut scoped_out = BTreeSet::new();
    for (stored_body, edge) in &edges {
        let from_body = index.body_of(edge.from.node);
        let to_body = index.body_of(edge.to.node);
        if matches!((from_body, to_body), (Some(from), Some(to)) if from != to)
            || (from_body.is_some_and(|body| body != *stored_body)
                && to_body.is_some_and(|body| body != *stored_body))
        {
            scoped_out.insert(edge.id);
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::ScopeEdge,
                subject(&edge.from, Some(edge.id)),
                origin,
            ));
            continue;
        }

        let from = resolve_endpoint(
            document,
            resolved,
            &edge.from,
            edge.id,
            origin,
            &mut report,
            &mut diagnostics,
        );
        let to = resolve_endpoint(
            document,
            resolved,
            &edge.to,
            edge.id,
            origin,
            &mut report,
            &mut diagnostics,
        );
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };

        if from.direction != crate::document::PortDirection::Output
            || to.direction != crate::document::PortDirection::Input
        {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::PortDirection,
                subject(&edge.from, Some(edge.id)),
                origin,
            ));
            continue;
        }

        let matrix_ok = matches!(
            (from.kind, to.kind),
            (PortKind::Data, PortKind::Data)
                | (PortKind::Exec, PortKind::Exec)
                | (PortKind::Event, PortKind::Exec)
        );
        if !matrix_ok {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::KindMismatch,
                subject(&edge.from, Some(edge.id)),
                origin,
            ));
            continue;
        }

        if from.kind == PortKind::Data {
            match (&from.ty, &to.ty) {
                (Some(from_ty), Some(to_ty)) => {
                    let from_term = TypeTerm::parse(from_ty);
                    let to_term = TypeTerm::parse(to_ty);
                    match (from_term, to_term) {
                        (Ok(from_term), Ok(to_term))
                            if from_term.contains_param() || to_term.contains_param() =>
                        {
                            report.incomplete_nodes.insert(edge.from.node);
                            report.incomplete_nodes.insert(edge.to.node);
                        }
                        (Ok(_), Ok(_)) if from_ty != to_ty => diagnostics.push(Diagnostic::new(
                            DiagnosticCode::TypeMismatch,
                            subject(&edge.to, Some(edge.id)),
                            origin,
                        )),
                        (Ok(_), Ok(_)) => {}
                        _ => {
                            report.incomplete_nodes.insert(edge.from.node);
                            report.incomplete_nodes.insert(edge.to.node);
                        }
                    }
                }
                _ => {
                    report.incomplete_nodes.insert(edge.from.node);
                    report.incomplete_nodes.insert(edge.to.node);
                }
            }
        }
    }

    for ((node, port), edge_ids) in incoming {
        let port_ref = PortRef { node, port };
        let edge_ids: Vec<_> = edge_ids
            .into_iter()
            .filter(|edge| !scoped_out.contains(edge))
            .collect();
        if edge_ids.len() <= 1 {
            continue;
        }
        let Ok(port) = lookup_port(document, resolved, &port_ref) else {
            continue;
        };
        if port.direction == crate::document::PortDirection::Input && port.kind == PortKind::Data {
            let edge = edge_ids[0];
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticCode::FaninData,
                    subject(&port_ref, Some(edge)),
                    origin,
                )
                .with_count("count", edge_ids.len() as u64),
            );
        }
    }

    for ((node, port), edge_ids) in outgoing {
        let port_ref = PortRef { node, port };
        let edge_ids: Vec<_> = edge_ids
            .into_iter()
            .filter(|edge| !scoped_out.contains(edge))
            .collect();
        if edge_ids.len() <= 1 {
            continue;
        }
        let Ok(port) = lookup_port(document, resolved, &port_ref) else {
            continue;
        };
        let code = if port.direction != crate::document::PortDirection::Output {
            None
        } else {
            match port.kind {
                PortKind::Exec => Some(DiagnosticCode::FanoutExec),
                PortKind::Event => Some(DiagnosticCode::FanoutEvent),
                PortKind::Data => match stream_status(&port) {
                    Some(true) => Some(DiagnosticCode::StreamFanout),
                    Some(false) => None,
                    None => {
                        report.incomplete_nodes.insert(port_ref.node);
                        None
                    }
                },
            }
        };
        if let Some(code) = code {
            diagnostics.push(
                Diagnostic::new(code, subject(&port_ref, Some(edge_ids[0])), origin)
                    .with_count("count", edge_ids.len() as u64),
            );
        }
    }

    // Edges crossing body boundaries are intentionally not checked a second time.
    sort_diagnostics(&mut diagnostics);
    report.diagnostics = diagnostics;
    report
}

fn resolve_endpoint(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    endpoint: &PortRef,
    edge: EdgeId,
    origin: &HostOrigin,
    report: &mut ConnectionValidation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<PortDescriptor> {
    match lookup_port(document, resolved, endpoint) {
        Ok(port) => Some(port),
        Err(PortLookupError::MissingNode | PortLookupError::UnknownPort) => {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::PortUnknown,
                subject(endpoint, Some(edge)),
                origin,
            ));
            None
        }
        Err(PortLookupError::MissingMetadata) => {
            report.incomplete_nodes.insert(endpoint.node);
            None
        }
    }
}

fn stream_status(port: &PortDescriptor) -> Option<bool> {
    let term = TypeTerm::parse(port.ty.as_ref()?).ok()?;
    if term.contains_param() {
        None
    } else {
        Some(term.core("stream").is_some())
    }
}

fn subject(endpoint: &PortRef, edge: Option<EdgeId>) -> Subject {
    Subject::new(Some(endpoint.node), Some(endpoint.port.clone()), edge)
}
