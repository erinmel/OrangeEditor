//! Graph export validation (11 §7.2 consumption sites, §12 streams; 12 `exports`).
//!
//! An export evaluates one root-body data output for preview or the CLI. The pass is
//! read-only and uses only fresh resolved signatures, never stored snapshots.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{
    GraphDocument, GraphExport, OutputSource, PortDirection, PortKind, PortRef, SignatureSnapshot,
};
use crate::id::{NodeId, TypeExpr};
use crate::type_term::TypeTerm;
use crate::types::{CapabilityRequest, Constraint};

use super::diagnostic::{Diagnostic, DiagnosticCode, Subject, sort_diagnostics};
use super::ports::{PortLookupError, lookup_port};

/// Validates `GraphDocument::exports` against fresh resolved signatures.
///
/// Rules: the export id is unique (`E-EXPORT-DUPLICATE`, every later use); it names a data
/// output of a root-body node (`E-EXPORT-TARGET`); the port type is concrete and has
/// `serialize` evidence, so handles, streams and containers of them are rejected
/// (`E-EXPORT-TYPE`); and the value's cone reads no activation-scoped producer
/// (`E-CROSS-ACTIVATION`, G-35). Constant, literal, state and volatile cones are allowed.
/// A target whose node did not resolve reports `E-DEPENDENCY-UNAVAILABLE`; it is never
/// checked against a stored snapshot.
pub fn validate_exports(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let origin = registry.origin();
    let mut diagnostics = Vec::new();
    let mut seen = BTreeSet::new();
    for export in document.exports() {
        let subject = Subject::new(Some(export.node), Some(export.port.clone()), None);
        let mut report = |code| diagnostics.push(Diagnostic::new(code, subject.clone(), origin));
        if !seen.insert(export.id.as_str()) {
            report(DiagnosticCode::ExportDuplicate);
        }
        if !document.root().nodes.contains_key(&export.node) {
            report(DiagnosticCode::ExportTarget);
            continue;
        }
        if !resolved.contains_key(&export.node) {
            report(DiagnosticCode::DependencyUnavailable);
            continue;
        }
        let endpoint = PortRef {
            node: export.node,
            port: export.port.clone(),
        };
        let port = match lookup_port(document, resolved, &endpoint) {
            Ok(port) if port.direction == PortDirection::Output && port.kind == PortKind::Data => {
                port
            }
            Ok(_) | Err(PortLookupError::UnknownPort | PortLookupError::MissingNode) => {
                report(DiagnosticCode::ExportTarget);
                continue;
            }
            Err(PortLookupError::MissingMetadata) => {
                report(DiagnosticCode::DependencyUnavailable);
                continue;
            }
        };
        if !serializable(registry, port.ty.as_ref()) {
            report(DiagnosticCode::ExportType);
        }
        if crosses_activation(document, resolved, export, port.source) {
            report(DiagnosticCode::CrossActivation);
        }
    }
    sort_diagnostics(&mut diagnostics);
    diagnostics.dedup();
    diagnostics
}

fn serializable(registry: &RegistrySnapshot, ty: Option<&TypeExpr>) -> bool {
    let Some(Ok(term)) = ty.map(TypeTerm::parse) else {
        return false;
    };
    !term.contains_param()
        && registry
            .types()
            .supports_term(&term, CapabilityRequest::Constraint(Constraint::Serialize))
            .is_ok_and(|witness| witness.is_some())
}

/// Whether the exported value, or anything in its pure/volatile cone, belongs to an
/// activation (payload, iteration or execution-sourced output).
fn crosses_activation(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    export: &GraphExport,
    source: Option<OutputSource>,
) -> bool {
    let scoped = |source: Option<OutputSource>| {
        matches!(
            source,
            Some(OutputSource::Payload | OutputSource::Iteration | OutputSource::Execution)
        )
    };
    if scoped(source) {
        return true;
    }
    let mut visited = BTreeSet::from([export.node]);
    let mut stack = vec![export.node];
    while let Some(node) = stack.pop() {
        for edge in document.root().edges.values().filter(|e| e.to.node == node) {
            let (Ok(from), Ok(to)) = (
                lookup_port(document, resolved, &edge.from),
                lookup_port(document, resolved, &edge.to),
            ) else {
                continue;
            };
            if from.kind != PortKind::Data || to.kind != PortKind::Data {
                continue;
            }
            if scoped(from.source) {
                return true;
            }
            if visited.insert(edge.from.node) {
                stack.push(edge.from.node);
            }
        }
    }
    false
}
