//! Stream storage and handle persistence rules for declarations and exports (11 §11.1, §12).
//!
//! Streams and handles are affine, host-owned resources. A stream may be stored nowhere
//! (`E-STREAM-STORE`); a handle may not live in a workspace or persistent variable, be an
//! initializer literal or be exported (`E-HANDLE-LITERAL`, `E-HANDLE-PERSIST`). The strictest
//! reading applies: resources in constructor arguments, nominal record fields and enum
//! payloads count. Register storage checks share this traversal through `validate_state`.

use std::collections::{BTreeMap, HashSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, PortRef, SignatureSnapshot, VarScope};
use crate::id::{NodeId, TypeExpr};
use crate::type_term::TypeTerm;
use crate::types::TypeRegistry;
use crate::value::literal_codec::Schemas;

use super::diagnostic::{Diagnostic, DiagnosticCode, Subject, sort_diagnostics};
use super::ports::PortResolver;

#[derive(Default, Clone, Copy)]
pub(super) struct Resources {
    pub(super) handle: bool,
    pub(super) stream: bool,
}

/// Includes nominal record fields and enum payloads, not just constructor arguments.
/// Each schema is expanded once so recursive nominal types terminate without losing siblings.
pub(super) fn resources(ty: &TypeExpr, types: &TypeRegistry) -> Option<Resources> {
    let mut found = Resources::default();
    let term = TypeTerm::parse(ty).ok()?;
    let mut stack = vec![&term];
    let mut expanded = HashSet::new();
    while let Some(term) = stack.pop() {
        found.handle |= term.core("handle").is_some();
        found.stream |= term.core("stream").is_some();
        if let TypeTerm::Named(named) = term {
            stack.extend(&named.args);
            if expanded.insert(named) {
                if let Some(schema) = types.record(named) {
                    stack.extend(schema.fields.iter().map(|field| &field.ty));
                }
                if let Some(schema) = types.enumeration(named) {
                    stack.extend(
                        schema
                            .variants
                            .iter()
                            .filter_map(|variant| variant.payload.as_ref()),
                    );
                }
            }
        }
    }
    Some(found)
}

/// Checks variable declarations and exports. Read-only; unparsable types are diagnosed by the
/// type passes and skipped here.
pub fn validate_streams(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let ports = PortResolver::new(document, resolved);
    validate_streams_with_ports(document, &ports, registry)
}

pub(crate) fn validate_streams_with_ports(
    document: &GraphDocument,
    resolved: &PortResolver<'_>,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let origin = registry.origin();
    let mut diagnostics = Vec::new();
    for (id, decl) in document.variables() {
        let Some(found) = resources(&decl.ty, registry.types()) else {
            continue;
        };
        let subject = Subject::default().with_variable(*id);
        if found.stream {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::StreamStore,
                subject.clone(),
                origin,
            ));
        }
        if found.handle && (decl.persistent || decl.scope == VarScope::Workspace) {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::HandlePersist,
                subject.clone(),
                origin,
            ));
        }
        if (found.handle || found.stream) && decl.init.is_some() {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::HandleLiteral,
                subject,
                origin,
            ));
        }
    }
    for export in document.exports() {
        let port = PortRef {
            node: export.node,
            port: export.port.clone(),
        };
        let Ok(descriptor) = resolved.port(&port) else {
            continue;
        };
        let Some(found) = descriptor
            .ty
            .as_ref()
            .and_then(|ty| resources(ty, registry.types()))
        else {
            continue;
        };
        let subject = Subject::new(Some(export.node), Some(export.port.clone()), None);
        if found.stream {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::StreamStore,
                subject.clone(),
                origin,
            ));
        }
        if found.handle {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::HandlePersist,
                subject,
                origin,
            ));
        }
    }
    sort_diagnostics(&mut diagnostics);
    diagnostics
}
