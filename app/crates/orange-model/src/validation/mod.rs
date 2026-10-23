//! Read-only connection, node-resolution, input and function analysis (WU-00d.1–3).
//!
//! [`validate_nodes`] resolves current declarative contracts and exact provider evidence;
//! its projections feed [`validate_inputs`]. [`validate_connections`] also works offline
//! from stored signatures, preserving orphan edges. Each pass returns stable diagnostics
//! without changing document content, revision or state identity. The embedding host
//! supplies the immutable catalogue origin; these passes do not load package code.
//!
//! [`analyze_functions`] derives graph-local kind, suspension and run-write summaries
//! from fresh resolved metadata. Contextual Get/Set ports are declaration-owned. Call/Update ports, regions,
//! data/control cycles and availability remain later slices. Passing these individual
//! analyses is not compile approval.

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
