//! Stream storage and handle persistence rules for declarations and exports (11 §11.1, §12).
//!
//! Streams and handles are affine, host-owned resources. A stream may be stored nowhere
//! (`E-STREAM-STORE`); a handle may not live in a workspace or persistent variable, be an
//! initializer literal or be exported (`E-HANDLE-LITERAL`, `E-HANDLE-PERSIST`). The strictest
//! reading applies: a resource type nested anywhere inside the declared type counts. Registers
//! have no registered contract yet, so a stream stored in a register is not checked here.

use std::collections::BTreeMap;

use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, PortRef, SignatureSnapshot, VarScope};
use crate::id::{NodeId, TypeExpr};
use crate::type_term::TypeTerm;

use super::diagnostic::{Diagnostic, DiagnosticCode, Subject, sort_diagnostics};
use super::ports::lookup_port;

#[derive(Default, Clone, Copy)]
struct Resources {
    handle: bool,
    stream: bool,
}

/// Finds `core/handle@1` and `core/stream@1` anywhere in the type, iteratively.
fn resources(ty: &TypeExpr) -> Option<Resources> {
    let mut found = Resources::default();
    let mut stack = vec![TypeTerm::parse(ty).ok()?];
    while let Some(term) = stack.pop() {
        found.handle |= term.core("handle").is_some();
        found.stream |= term.core("stream").is_some();
        if let TypeTerm::Named(named) = term {
            stack.extend(named.args);
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
    let origin = registry.origin();
    let mut diagnostics = Vec::new();
    for (id, decl) in document.variables() {
        let Some(found) = resources(&decl.ty) else {
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
        let Ok(descriptor) = lookup_port(document, resolved, &port) else {
            continue;
        };
        let Some(found) = descriptor.ty.as_ref().and_then(resources) else {
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
