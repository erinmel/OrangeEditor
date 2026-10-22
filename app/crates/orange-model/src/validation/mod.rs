//! Read-only connection, node-resolution and input validation (WU-00d.1–2).
//!
//! [`validate_nodes`] resolves current declarative contracts and exact provider evidence;
//! its projections feed [`validate_inputs`]. [`validate_connections`] also works offline
//! from stored signatures, preserving orphan edges. Each pass returns stable diagnostics
//! without changing document content, revision or state identity. The embedding host
//! supplies the immutable catalogue origin; these passes do not load package code.
//!
//! Results describe these passes only. Region, cycle, availability and function analysis
//! are later 00d slices; a successful connection or node pass is not compile approval.

mod connections;
mod diagnostic;
mod inputs;
mod nodes;
mod ports;

pub use diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, IdentifierArg, MessageArg, MessageArgError,
    OriginError, Severity, Subject, sort_diagnostics,
};

pub use connections::{ConnectionValidation, validate_connections};
pub use inputs::validate_inputs;
pub use nodes::{NodeValidation, ProviderCatalog, validate_nodes};
