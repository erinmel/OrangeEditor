//! Validate declaration types independently of whether a node references them.

use crate::IdError;
use crate::catalog::RegistrySnapshot;
use crate::document::GraphDocument;
use crate::id::TypeExpr;
use crate::type_term::TypeTerm;
use crate::types::TypeResolveError;

use super::{Diagnostic, DiagnosticCode, Subject, sort_diagnostics};

/// Checks all variable and event types, including unused declarations without initializers.
/// Declarations have no generic parameter scope. Messages never contain declaration text.
pub fn validate_declaration_types(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let declarations = document
        .variables()
        .values()
        .map(|variable| (&variable.ty, Subject::default().with_variable(variable.id)))
        .chain(
            document
                .events()
                .values()
                .map(|event| (&event.payload, Subject::default().with_event(event.id))),
        );
    let mut diagnostics = Vec::new();
    for (ty, subject) in declarations {
        if let Some(code) = type_error(ty, registry) {
            diagnostics.push(Diagnostic::new(code, subject, registry.origin()));
        }
    }
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

fn type_error(ty: &TypeExpr, registry: &RegistrySnapshot) -> Option<DiagnosticCode> {
    let term = match TypeTerm::parse(ty) {
        Ok(term) => term,
        Err(IdError::TypeTooDeep { .. }) => return Some(DiagnosticCode::TypeComplexity),
        Err(_) => return Some(DiagnosticCode::DeclarationType),
    };
    match registry.types().validate(&term) {
        Ok(()) => None,
        Err(TypeResolveError::TooDeep { .. } | TypeResolveError::DerivationTooDeep { .. }) => {
            Some(DiagnosticCode::TypeComplexity)
        }
        Err(_) => Some(DiagnosticCode::DeclarationType),
    }
}
