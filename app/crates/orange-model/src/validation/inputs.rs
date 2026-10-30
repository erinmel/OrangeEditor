//! Read-only validation of literal and required data inputs against resolved metadata.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{RegistrySnapshot, rigid_params};
use crate::document::{
    GraphDocument, NodeInstance, PortDescriptor, PortDirection, PortKind, PortRef,
    SignatureSnapshot,
};
use crate::id::{NodeId, PortKey, TypeExpr};
use crate::index::DocIndex;
use crate::type_term::TypeTerm;
use crate::types::{TypeRegistry, TypeResolveError};
use crate::value::literal_codec;

use super::scope::Scope;
use super::{Diagnostic, DiagnosticCode, HostOrigin, Severity, Subject, sort_diagnostics};

/// Validates the active value contract for data inputs with current resolved metadata.
/// Nodes absent from `resolved` are skipped because their primary metadata resolution
/// failed elsewhere; stored snapshots are intentionally not used as a fallback.
pub fn validate_inputs(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let mut diagnostics = validate_inputs_in(document, resolved, registry, Scope::All, None, None);
    diagnostics.extend(validate_type_locks(document, registry));
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

/// Exact codec locks are prerequisites for interpreting saved values. Absence retains
/// the format-v1 structural codec default; explicit unsupported locks never fall back.
pub(crate) fn validate_type_locks(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let (types, origin) = (registry.types().as_ref(), registry.origin());
    let mut diagnostics = Vec::new();
    for lock in document.type_lock() {
        let compatible = TypeTerm::parse(&lock.ty)
            .ok()
            .and_then(|term| types.literal_codec_identity(&term))
            == Some(lock.codec.as_str());
        if !compatible {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::TypeCodec,
                Subject::default(),
                origin,
            ));
        }
    }
    diagnostics
}

/// [`validate_inputs`] over the bodies in `scope`.
pub(crate) fn validate_inputs_in(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
    scope: Scope,
    index: Option<&DocIndex>,
    mut cache: Option<&mut InputCache>,
) -> Vec<Diagnostic> {
    let mut connected = BTreeMap::<NodeId, BTreeSet<PortKey>>::new();
    let mut nodes = BTreeMap::new();
    let scoped_body = match scope {
        Scope::All => None,
        Scope::Function(id) => document.body(crate::document::BodyRef::Function(id)),
    };
    for (_, body) in scope.bodies(document) {
        for node in body.nodes.values() {
            nodes.insert(node.id, node);
        }
        if index.is_none() {
            for edge in body.edges.values() {
                connected
                    .entry(edge.to.node)
                    .or_default()
                    .insert(edge.to.port.clone());
            }
        }
    }
    let empty = BTreeSet::new();
    let mut diagnostics = Vec::new();
    for (node_id, node) in nodes {
        let Some(snapshot) = resolved.get(&node_id) else {
            continue;
        };
        let is_connected = |port: &PortRef| match index {
            Some(index) => index
                .incoming(port)
                .any(|edge| scoped_body.is_none_or(|body| body.edges.contains_key(&edge))),
            None => connected
                .get(&node_id)
                .unwrap_or(&empty)
                .contains(&port.port),
        };
        match cache.as_deref_mut() {
            Some(cache) => cache.node_inputs(
                document,
                node,
                snapshot,
                is_connected,
                registry,
                &mut diagnostics,
            ),
            None => append_node_inputs(
                document,
                node,
                snapshot,
                is_connected,
                registry,
                &mut diagnostics,
            ),
        }
    }
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

/// One node's active input contract, shared by whole-body and retained analysis.
/// Incoming keys describe wire precedence; an invalid wire still shadows its literal
/// exactly as in the full pass. Type-lock checks remain a document prerequisite.
/// One root-body node's input diagnostics, as the whole-document pass reports them.
pub(crate) fn node_input_diagnostics(
    document: &GraphDocument,
    node: &NodeInstance,
    snapshot: &SignatureSnapshot,
    registry: &RegistrySnapshot,
    index: &DocIndex,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let is_connected = |port: &PortRef| index.incoming(port).next().is_some();
    append_node_inputs(
        document,
        node,
        snapshot,
        is_connected,
        registry,
        &mut diagnostics,
    );
    diagnostics
}

fn append_node_inputs(
    document: &GraphDocument,
    node: &NodeInstance,
    snapshot: &SignatureSnapshot,
    mut is_connected: impl FnMut(&PortRef) -> bool,
    registry: &RegistrySnapshot,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let (types, origin) = (registry.types().as_ref(), registry.origin());
    let node_id = node.id;
    let rigid: Vec<String> = rigid_params(document, node_id)
        .iter()
        .map(|param| param.name.clone())
        .collect();
    let mut valid_keys = BTreeSet::new();
    let mut candidates = Vec::new();

    for (port, state) in &node.variadic {
        if !state.is_valid() {
            diagnostics.push(diagnostic(
                DiagnosticCode::VariadicIdentity,
                Severity::Error,
                node_id,
                PortKey::new(port.clone(), None),
                origin,
            ));
        }
    }

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

        let reference = PortRef {
            node: node_id,
            port: key,
        };
        let connected = is_connected(&reference);
        let key = reference.port;
        if connected {
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

/// Each node's input diagnostics from an earlier analysis of the same session (16, V12b).
/// An entry is reused only while the change journal leaves its node untouched and every
/// other input of the check is equal: the resolved signature, the enclosing function's
/// rigid type parameters and the wire answers the check asked for. Registry changes
/// clear the cache. Results are the full pass's, computed by the same function.
#[derive(Default, Clone)]
pub(crate) struct InputCache {
    entries: BTreeMap<NodeId, InputEntry>,
    pub(crate) computed: usize,
    pub(crate) hits: usize,
}

#[derive(Clone)]
struct InputEntry {
    snapshot: SignatureSnapshot,
    rigid: Vec<String>,
    /// Each port the check asked about and whether it had an incoming wire, in order.
    wires: Vec<(PortKey, bool)>,
    diagnostics: Vec<Diagnostic>,
}

impl InputCache {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn forget(&mut self, node: &NodeId) {
        self.entries.remove(node);
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    fn node_inputs(
        &mut self,
        document: &GraphDocument,
        node: &NodeInstance,
        snapshot: &SignatureSnapshot,
        mut is_connected: impl FnMut(&PortRef) -> bool,
        registry: &RegistrySnapshot,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let rigid: Vec<String> = rigid_params(document, node.id)
            .iter()
            .map(|param| param.name.clone())
            .collect();
        if let Some(entry) = self.entries.get(&node.id)
            && entry.snapshot == *snapshot
            && entry.rigid == rigid
            && entry.wires.iter().all(|(port, wired)| {
                is_connected(&PortRef {
                    node: node.id,
                    port: port.clone(),
                }) == *wired
            })
        {
            self.hits += 1;
            diagnostics.extend(entry.diagnostics.iter().cloned());
            return;
        }
        self.computed += 1;
        let mut wires = Vec::new();
        let mut own = Vec::new();
        append_node_inputs(
            document,
            node,
            snapshot,
            |port| {
                let wired = is_connected(port);
                wires.push((port.port.clone(), wired));
                wired
            },
            registry,
            &mut own,
        );
        diagnostics.extend(own.iter().cloned());
        self.entries.insert(
            node.id,
            InputEntry {
                snapshot: snapshot.clone(),
                rigid,
                wires,
                diagnostics: own,
            },
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Edge, FunctionDecl};
    use serde_json::json;

    #[test]
    fn indexed_input_precedence_matches_standalone_scope_for_invalid_wires() {
        let registry = RegistrySnapshot::core(
            HostOrigin::new("1.0.0", format!("sha256:{}", "0".repeat(64))).unwrap(),
        );
        let mut document =
            GraphDocument::new("00000000-0000-4000-8000-000000000001".parse().unwrap());
        let function: FunctionDecl = serde_json::from_value(json!({
            "id":"00000000-0000-4000-8000-000000000002", "name":"test", "kind":"pure",
            "nodes":[{"id":"00000000-0000-4000-8000-000000000003", "definition_id":"example/node",
                "definition_version":1, "config_version":1, "config":{},
                "inputs":{"value":{"type":"core/i64@1","value":"7"}}}]
        }))
        .unwrap();
        let function_id = function.id;
        let node_id = *function.body.nodes.keys().next().unwrap();
        document.functions.insert(function_id, function);
        let signature: SignatureSnapshot = serde_json::from_value(json!({"kind":"pure","ports":[{
            "port_id":"value", "direction":"input", "kind":"data", "type":"core/i64@1"
        }]}))
        .unwrap();
        let resolved = BTreeMap::from([(node_id, signature)]);
        let edge: Edge = serde_json::from_value(json!({
            "id":"00000000-0000-4000-8000-000000000004",
            "from":{"node":"00000000-0000-4000-8000-000000000099","port":"missing"},
            "to":{"node":node_id,"port":"value"}
        }))
        .unwrap();
        // A malformed wire stored in another body must not shadow this function's input.
        document.root.edges.insert(edge.id, edge.clone());
        let scope = Scope::Function(function_id);
        // One cache across both states: moving the wire must not reuse the first result.
        let mut cache = InputCache::default();
        let standalone = validate_inputs_in(&document, &resolved, &registry, scope, None, None);
        assert!(standalone.is_empty());
        assert_eq!(
            standalone,
            validate_inputs_in(
                &document,
                &resolved,
                &registry,
                scope,
                Some(&DocIndex::build(&document)),
                Some(&mut cache)
            )
        );
        // An invalid source on a wire in this body still shadows the literal.
        document.root.edges.shift_remove(&edge.id);
        document
            .functions
            .get_mut(&function_id)
            .unwrap()
            .body
            .edges
            .insert(edge.id, edge);
        let standalone = validate_inputs_in(&document, &resolved, &registry, scope, None, None);
        assert_eq!(standalone.len(), 1);
        assert_eq!(standalone[0].code, DiagnosticCode::LiteralShadowed);
        assert_eq!(
            standalone,
            validate_inputs_in(
                &document,
                &resolved,
                &registry,
                scope,
                Some(&DocIndex::build(&document)),
                Some(&mut cache)
            )
        );
        assert_eq!((cache.computed, cache.hits), (2, 0));
    }
}
