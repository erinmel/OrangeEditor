//! The single validation entry point composing every pass.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, SignatureSnapshot};
use crate::id::{FunctionId, NodeId};

use super::{
    Diagnostic, FunctionEvidence, FunctionSummary, LeafEffects, Severity, analyze_functions,
    sort_diagnostics, validate_connections, validate_inputs, validate_nodes,
};

/// Combined result of every validation pass over one document and registry snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationReport {
    /// All pass diagnostics, sorted by [`sort_diagnostics`]; identical records emitted by
    /// more than one pass appear once.
    pub diagnostics: Vec<Diagnostic>,
    /// Current semantic projections for nodes that resolved completely.
    pub resolved: BTreeMap<NodeId, SignatureSnapshot>,
    /// Nodes whose resolution or connection checks could not complete.
    pub incomplete_nodes: BTreeSet<NodeId>,
    /// Summaries of functions whose analysis completed.
    pub functions: BTreeMap<FunctionId, FunctionSummary>,
    /// Functions whose summary could not be established.
    pub incomplete_functions: BTreeSet<FunctionId>,
}

impl ValidationReport {
    /// True only when nothing is incomplete and no error was diagnosed.
    pub fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty()
            && self.incomplete_functions.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != Severity::Error)
    }
}

/// Validates `document` against `registry` without supplying leaf effects.
///
/// A package leaf that is not pure or volatile has no known effects here, so its
/// function stays incomplete (`E-FUNCTION-METADATA-UNAVAILABLE`, 11 §14); missing leaf
/// facts are never treated as "no writes". Use [`validate_with_effects`] to supply them.
pub fn validate(document: &GraphDocument, registry: &RegistrySnapshot) -> ValidationReport {
    validate_with_effects(document, registry, BTreeMap::new())
}

/// Validates `document` against `registry` with host-supplied leaf effects.
///
/// Runs node resolution, input, connection and function passes on the same resolution
/// and never changes the document. `leaf_effects` must be fresh for this exact document,
/// config, provider and lock snapshot; nodes without an entry make their function
/// incomplete (11 §14). Diagnostics are sorted and exact duplicates removed.
pub fn validate_with_effects(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: BTreeMap<NodeId, LeafEffects>,
) -> ValidationReport {
    let nodes = validate_nodes(document, registry);
    let connections = validate_connections(document, registry);
    let evidence = FunctionEvidence::from_nodes(document, &nodes, leaf_effects);
    let functions = analyze_functions(document, &evidence, registry);

    let mut diagnostics = nodes.diagnostics;
    diagnostics.extend(validate_inputs(document, &nodes.resolved, registry));
    diagnostics.extend(connections.diagnostics);
    diagnostics.extend(functions.diagnostics);
    sort_diagnostics(&mut diagnostics);
    diagnostics.dedup();

    let mut incomplete_nodes = nodes.incomplete_nodes;
    incomplete_nodes.extend(connections.incomplete_nodes);
    ValidationReport {
        diagnostics,
        resolved: nodes.resolved,
        incomplete_nodes,
        functions: functions.summaries,
        incomplete_functions: functions.incomplete_functions,
    }
}
