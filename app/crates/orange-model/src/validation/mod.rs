//! Read-only validation of a document against one registry snapshot.
//!
//! Hosts call [`analyze`] (or [`analyze_with_effects`] when they hold fresh leaf effects)
//! and receive a [`ValidationAnalysis`] with honest pass coverage and freshness evidence.
//! Its [`ValidationReport`] contains sorted, de-duplicated diagnostics, the resolved
//! signatures, incomplete nodes and functions, and function summaries. It first runs
//! [`validate_body_domains`]; unsupported domains stop the control passes with only domain
//! coverage. For supported documents it first accepts graph-local functions callee first, so
//! calls resolve against accepted callees, then runs [`validate_nodes`] once and feeds that
//! resolution to every later pass. Connections fall back to stored signatures only for
//! package nodes that did not resolve, preserving orphan edges.
//! `validate` supplies no leaf effects, so a package action leaf makes its function
//! incomplete rather than assumed effect-free. No pass changes document
//! content, revision or state identity, and none loads package code; the host supplies the
//! immutable catalogue origin.
//!
//! The individual passes stay public for hosts that run them separately. Contextual variable
//! and shared-write ports are declaration-owned. A valid report is not compile approval: see
//! [`ValidationAnalysis::is_ready_for_compilation`].

mod acceptance;
mod availability;
mod config;
mod conformance;
mod connections;
mod control_cycles;
mod data_cycles;
mod declarations;
mod diagnostic;
mod domains;
mod dominators;
mod exports;
mod functions;
mod inputs;
mod limits;
mod nodes;
pub(crate) mod ports;
mod regions;
mod report;
mod scope;
mod state;
mod streams;
mod variables;

pub use diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, IdentifierArg, MessageArg, MessageArgError,
    OriginError, Severity, Subject, sort_diagnostics,
};

pub use availability::{AvailabilityValidation, validate_availability};
pub use config::{
    CONFIG_SCHEMA_DIALECT, config_satisfies_schema, config_schema_is_supported, validate_config,
};
pub use connections::{ConnectionValidation, validate_connections};
pub use control_cycles::{ControlCycleValidation, validate_control_cycles};
pub use data_cycles::{DataCycleValidation, validate_data_cycles};
pub use declarations::validate_declaration_types;
pub use domains::{DomainValidation, validate_body_domains};
pub use exports::validate_exports;
pub use inputs::validate_inputs;
pub use limits::{AnalysisLimits, AnalysisOptions};
pub use nodes::{NodeValidation, validate_nodes};
pub use regions::{
    Region, RegionIndex, RegionKind, RegionRoot, RegionValidation, validate_regions,
};
pub use report::{
    ValidationAnalysis, ValidationCheck, ValidationReport, analyze, analyze_with_effects,
    analyze_with_options, required_checks, validate, validate_with_effects,
};
pub use state::validate_state;
pub use streams::validate_streams;

pub use functions::{
    FunctionAnalysis, FunctionEvidence, FunctionSummary, LeafEffects, VariableWrite,
    WriteOperation, analyze_functions,
};
