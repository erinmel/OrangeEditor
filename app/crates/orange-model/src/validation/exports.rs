//! Graph export validation (11 §7.2 consumption sites, §12 streams; 12 `exports`).
//!
//! An export evaluates one root-body data output for preview or the CLI. The pass is
//! read-only and uses only fresh resolved signatures, never stored snapshots.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{
    GraphDocument, OutputSource, PortDirection, PortKind, PortRef, SignatureSnapshot,
};
use crate::id::{NodeId, TypeExpr};
use crate::type_term::TypeTerm;
use crate::types::{CapabilityRequest, Constraint};

use super::diagnostic::{Diagnostic, DiagnosticCode, Subject, sort_diagnostics};
use super::ports::{PortLookupError, PortResolver};

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
    let ports = PortResolver::new(document, resolved);
    let zones = super::regions::root_zones(document, &ports, registry.origin());
    validate_exports_with_ports(document, &ports, registry, &zones)
}

pub(crate) fn validate_exports_with_ports(
    document: &GraphDocument,
    resolved: &PortResolver<'_>,
    registry: &RegistrySnapshot,
    zones: &super::zones::ZoneIndex,
) -> Vec<Diagnostic> {
    if document.exports().is_empty() {
        return Vec::new();
    }
    let origin = registry.origin();
    let mut diagnostics = Vec::new();
    let mut seen = BTreeSet::new();
    // Inspect each edge once, rather than scanning every edge at every step of each cone.
    // State outputs stop a pull: their write inputs do not determine the stored value.
    let mut incoming: BTreeMap<NodeId, Vec<(NodeId, OutputSource)>> = BTreeMap::new();
    for edge in document.root().edges.values() {
        if !document.root().nodes.contains_key(&edge.from.node)
            || !document.root().nodes.contains_key(&edge.to.node)
        {
            continue;
        }
        let (Ok(from), Ok(to)) = (resolved.port(&edge.from), resolved.port(&edge.to)) else {
            continue;
        };
        if from.kind == PortKind::Data
            && to.kind == PortKind::Data
            && from.direction == PortDirection::Output
            && to.direction == PortDirection::Input
            && let Some(source) = from.source
        {
            incoming
                .entry(edge.to.node)
                .or_default()
                .push((edge.from.node, source));
        }
    }
    // A result consumes its begin prefix even when no body edge mentions it.
    for pair in zones.iter() {
        if let Some(prefix) = incoming.get(&pair.begin).cloned() {
            incoming.entry(pair.end).or_default().extend(prefix);
        }
    }
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
        let port = match resolved.port(&endpoint) {
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
        if zones.owner(export.node).is_some()
            || crosses_activation(&incoming, export.node, port.source, zones)
        {
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
    let resource = ty.and_then(|ty| super::streams::resources(ty, registry.types()));
    !resource.is_some_and(|found| found.handle || found.stream)
        && !term.contains_param()
        && registry
            .types()
            .supports_term(&term, CapabilityRequest::Constraint(Constraint::Serialize))
            .is_ok_and(|witness| witness.is_some())
}

/// Whether the exported value, or anything in its pure/volatile cone, belongs to an
/// activation (payload, iteration or execution-sourced output).
fn crosses_activation(
    incoming: &BTreeMap<NodeId, Vec<(NodeId, OutputSource)>>,
    node: NodeId,
    source: Option<OutputSource>,
    zones: &super::zones::ZoneIndex,
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
    if source != Some(OutputSource::Computed) {
        return false;
    }
    let mut visited = BTreeSet::from([node]);
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        for &(producer, source) in incoming.get(&node).into_iter().flatten() {
            if source == OutputSource::Iteration
                && zones
                    .owner(producer)
                    .is_some_and(|pair| pair.begin == producer)
            {
                continue;
            }
            if scoped(Some(source)) {
                return true;
            }
            if source == OutputSource::Computed && visited.insert(producer) {
                stack.push(producer);
            }
        }
    }
    false
}
