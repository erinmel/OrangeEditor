//! Read-only validation of a document against one registry snapshot.
//!
//! Hosts call [`analyze`] (or [`analyze_with_effects`] when they hold fresh leaf effects)
//! and receive a [`ValidationAnalysis`] with honest pass coverage and freshness evidence.
//! Its [`ValidationReport`] contains sorted, de-duplicated diagnostics, the resolved
//! signatures, incomplete nodes and functions, and function summaries. It first runs
//! [`validate_body_domains`]; unsupported domains stop the control passes with only domain
//! coverage. For supported documents it runs
//! [`validate_nodes`] once and feeds that resolution to [`validate_inputs`],
//! [`validate_connections`], [`validate_data_cycles`] and [`analyze_functions`]. Connections fall back to stored
//! signatures only for package nodes that did not resolve, preserving orphan edges.
//! `validate` supplies no leaf effects, so a package action leaf makes its function
//! incomplete rather than assumed effect-free. No pass changes document
//! content, revision or state identity, and none loads package code; the host supplies the
//! immutable catalogue origin.
//!
//! The individual passes stay public for hosts that run them separately. Contextual variable
//! and shared-write ports are declaration-owned. Call/interface ports and control cycles remain later slices; [`validate_availability`] checks value availability from the region index, and a valid report is not compile approval.

mod availability;
mod conformance;
mod connections;
mod control_cycles;
mod data_cycles;
mod diagnostic;
mod domains;
mod functions;
mod inputs;
mod nodes;
pub(crate) mod ports;
mod regions;
mod report;
mod variables;

pub use diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, IdentifierArg, MessageArg, MessageArgError,
    OriginError, Severity, Subject, sort_diagnostics,
};

pub use availability::{AvailabilityValidation, validate_availability};
pub use connections::{ConnectionValidation, validate_connections};
pub use control_cycles::{ControlCycleValidation, validate_control_cycles};
pub use data_cycles::{DataCycleValidation, validate_data_cycles};
pub use domains::{DomainValidation, validate_body_domains};
pub use inputs::validate_inputs;
pub use nodes::{NodeValidation, validate_nodes};
pub use regions::{
    Region, RegionIndex, RegionKind, RegionRoot, RegionValidation, validate_regions,
};
pub use report::{
    ValidationAnalysis, ValidationCheck, ValidationReport, analyze, analyze_with_effects,
    required_checks, validate, validate_with_effects,
};

pub use functions::{
    FunctionAnalysis, FunctionEvidence, FunctionSummary, LeafEffects, VariableWrite,
    WriteOperation, analyze_functions,
};
