//! Read-only validation of a document against one registry snapshot.
//!
//! Hosts call [`validate`] (or [`validate_with_effects`] when they hold fresh leaf effects)
//! and receive one [`ValidationReport`]: sorted, de-duplicated diagnostics, the resolved
//! signatures, incomplete nodes and functions, and function summaries. It runs
//! [`validate_nodes`] once and feeds that resolution to [`validate_inputs`],
//! [`validate_connections`] and [`analyze_functions`]. Connections fall back to stored
//! signatures only for package nodes that did not resolve, preserving orphan edges.
//! `validate` supplies no leaf effects, so a package action leaf makes its function
//! incomplete rather than assumed effect-free. No pass changes document
//! content, revision or state identity, and none loads package code; the host supplies the
//! immutable catalogue origin.
//!
//! The individual passes stay public for hosts that run them separately. Contextual Get/Set
//! ports are declaration-owned. Call/Update ports, regions, data/control cycles and
//! availability remain later slices, and a valid report is not compile approval.

mod connections;
mod diagnostic;
mod functions;
mod inputs;
mod nodes;
pub(crate) mod ports;
mod report;
mod variables;

pub use diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, IdentifierArg, MessageArg, MessageArgError,
    OriginError, Severity, Subject, sort_diagnostics,
};

pub use connections::{ConnectionValidation, validate_connections};
pub use inputs::validate_inputs;
pub use nodes::{NodeValidation, validate_nodes};
pub use report::{ValidationReport, validate, validate_with_effects};

pub use functions::{
    FunctionAnalysis, FunctionEvidence, FunctionSummary, LeafEffects, VariableWrite,
    WriteOperation, analyze_functions,
};
