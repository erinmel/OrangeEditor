//! Read-only validation of literal and required data inputs against resolved metadata.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{RegistrySnapshot, rigid_params};
use crate::document::{GraphDocument, PortDescriptor, PortDirection, PortKind, SignatureSnapshot};
use crate::id::{NodeId, PortKey, TypeExpr};
use crate::type_term::TypeTerm;
use crate::types::{TypeRegistry, TypeResolveError};
use crate::value::literal_codec;

use super::{Diagnostic, DiagnosticCode, HostOrigin, Severity, Subject, sort_diagnostics};

/// Validates the active value contract for data inputs with current resolved metadata.
/// Nodes absent from `resolved` are skipped because their primary metadata resolution
/// failed elsewhere; stored snapshots are intentionally not used as a fallback.
pub fn validate_inputs(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let (types, origin) = (registry.types().as_ref(), registry.origin());
    let mut diagnostics = Vec::new();
    let mut connected = BTreeSet::<(NodeId, PortKey)>::new();
    let mut nodes = BTreeMap::new();
    for body in document.bodies() {
        for node in body.nodes.values() {
            nodes.insert(node.id, node);
        }
        for edge in body.edges.values() {
            connected.insert((edge.to.node, edge.to.port.clone()));
        }
    }

    for (node_id, node) in nodes {
        let Some(snapshot) = resolved.get(&node_id) else {
            continue;
        };
        let rigid: Vec<String> = rigid_params(document, node_id)
            .iter()
            .map(|param| param.name.to_string())
            .collect();
        let mut valid_keys = BTreeSet::new();
        let mut candidates = Vec::new();

        for descriptor in &snapshot.ports {
            if let Some(bounds) = descriptor.variadic {
                let count = node
                    .variadic
                    .get(&descriptor.port_id)
                    .map_or(0, |state| state.order.len());
                if count < usize::from(bounds.min)
                    || bounds.max.is_some_and(|max| count > usize::from(max))
                {
                    diagnostics.push(diagnostic(
                        DiagnosticCode::VariadicCount,
                        Severity::Error,
                        node_id,
                        PortKey::new(descriptor.port_id.clone(), None),
                        origin,
                    ));
                }
            }
            if descriptor.direction != PortDirection::Input || descriptor.kind != PortKind::Data {
                continue;
            }
            if is_variadic(descriptor) {
                if let Some(state) = node.variadic.get(&descriptor.port_id) {
                    for instance in &state.order {
                        let key = PortKey::new(descriptor.port_id.clone(), Some(*instance));
                        valid_keys.insert(key.clone());
                        candidates.push((key, descriptor));
                    }
                }
            } else {
                let key = PortKey::new(descriptor.port_id.clone(), None);
                valid_keys.insert(key.clone());
                candidates.push((key, descriptor));
            }
        }

        // Persisted values on removed, non-input, or otherwise undeclared keys are
        // preserved by the document model and reported here without inspecting payloads.
        for key in node.inputs.keys() {
            if !valid_keys.contains(key) {
                diagnostics.push(diagnostic(
                    DiagnosticCode::PortUnknown,
                    Severity::Error,
                    node_id,
                    key.clone(),
                    origin,
                ));
            }
        }

        for (key, descriptor) in candidates {
            let Some(expected) = descriptor.ty.as_ref() else {
                diagnostics.push(diagnostic(
                    DiagnosticCode::LiteralType,
                    Severity::Error,
                    node_id,
                    key,
                    origin,
                ));
                continue;
            };
            let expected_term = match validate_expected_type(expected, types, &rigid) {
                Ok(term) => term,
                Err(code) => {
                    diagnostics.push(diagnostic(code, Severity::Error, node_id, key, origin));
                    continue;
                }
            };

            if connected.contains(&(node_id, key.clone())) {
                if node.inputs.contains_key(&key) {
                    diagnostics.push(diagnostic(
                        DiagnosticCode::LiteralShadowed,
                        Severity::Warning,
                        node_id,
                        key,
                        origin,
                    ));
                }
                continue;
            }

            let literal = node.inputs.get(&key).or(descriptor.default.as_ref());
            let Some(literal) = literal else {
                if descriptor.is_optional() && expected_term.core("option").is_some() {
                    continue;
                }
                diagnostics.push(diagnostic(
                    DiagnosticCode::InputRequired,
                    Severity::Error,
                    node_id,
                    key,
                    origin,
                ));
                continue;
            };

            if literal.ty != *expected {
                diagnostics.push(diagnostic(
                    DiagnosticCode::LiteralType,
                    Severity::Error,
                    node_id,
                    key,
                    origin,
                ));
                continue;
            }
            if expected_term.core("handle").is_some() {
                diagnostics.push(diagnostic(
                    DiagnosticCode::HandleLiteral,
                    Severity::Error,
                    node_id,
                    key,
                    origin,
                ));
                continue;
            }
            if literal_codec::decode(literal, types).is_err() {
                diagnostics.push(diagnostic(
                    DiagnosticCode::LiteralType,
                    Severity::Error,
                    node_id,
                    key,
                    origin,
                ));
            }
        }
    }

    sort_diagnostics(&mut diagnostics);
    diagnostics
}

fn is_variadic(port: &PortDescriptor) -> bool {
    port.variadic.is_some()
}

fn validate_expected_type(
    ty: &TypeExpr,
    types: &TypeRegistry,
    rigid: &[String],
) -> Result<TypeTerm, DiagnosticCode> {
    let term = TypeTerm::parse(ty).map_err(|error| match error {
        crate::error::IdError::TypeTooDeep { .. } => DiagnosticCode::TypeComplexity,
        _ => DiagnosticCode::TypeMismatch,
    })?;
    types
        .validate_scoped(&term, rigid)
        .map_err(|error| match error {
            TypeResolveError::TooDeep { .. } | TypeResolveError::DerivationTooDeep { .. } => {
                DiagnosticCode::TypeComplexity
            }
            _ => DiagnosticCode::TypeMismatch,
        })?;
    Ok(term)
}

fn diagnostic(
    code: DiagnosticCode,
    severity: Severity,
    node: NodeId,
    port: PortKey,
    origin: &HostOrigin,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(code, Subject::new(Some(node), Some(port), None), origin);
    diagnostic.severity = severity;
    diagnostic
}
