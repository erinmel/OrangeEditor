//! Checks the execution domain declared for every graph body.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::document::{BodyRef, GraphDocument};
use crate::validation::{Diagnostic, DiagnosticCode, HostOrigin, Subject, sort_diagnostics};

/// Result of checking whether each body uses the built-in control domain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainValidation {
    /// One diagnostic for each body whose domain is unsupported or malformed.
    pub diagnostics: Vec<Diagnostic>,
    /// Bodies that cannot be analyzed under the built-in control-domain rules.
    pub unsupported_bodies: BTreeSet<BodyRef>,
}

/// Checks the root graph and every function body against the currently supported domain.
///
/// The `domain` field is still stored as an opaque extension until a second domain is
/// registered. Missing metadata means `control`; the exact string `control` is accepted
/// explicitly. Every other JSON value is
/// preserved and diagnosed without copying its contents into message arguments.
pub fn validate_body_domains(document: &GraphDocument, origin: &HostOrigin) -> DomainValidation {
    let mut result = DomainValidation::default();
    inspect_body(
        BodyRef::Root,
        document.extensions().get("domain"),
        origin,
        &mut result,
    );
    for (id, function) in document.functions() {
        inspect_body(
            BodyRef::Function(*id),
            function.extensions.get("domain"),
            origin,
            &mut result,
        );
    }
    sort_diagnostics(&mut result.diagnostics);
    result
}

fn inspect_body(
    body: BodyRef,
    domain: Option<&Value>,
    origin: &HostOrigin,
    result: &mut DomainValidation,
) {
    let supported = domain.is_none_or(|value| value.as_str() == Some("control"));
    if supported {
        return;
    }

    result.unsupported_bodies.insert(body);
    let subject = match body {
        BodyRef::Root => Subject::new(None, None, None),
        BodyRef::Function(function) => Subject::new(None, None, None).with_function(function),
    };
    result.diagnostics.push(Diagnostic::new(
        DiagnosticCode::DomainUnsupported,
        subject,
        origin,
    ));
}
