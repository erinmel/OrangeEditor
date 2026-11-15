//! Single-pass entry points for the behaviour suite. Production code runs every pass through
//! `analyze`, which shares one port resolver and one scope across them; these wrappers build
//! that context for one pass so a scenario can check its diagnostics in isolation.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, SignatureSnapshot};
use crate::id::NodeId;
use crate::index::DocIndex;

use super::availability::{AvailabilityValidation, validate_availability_with_ports};
use super::connections::{ConnectionValidation, validate_connections_in};
use super::control_cycles::{ControlCycleValidation, validate_control_cycles_in};
use super::data_cycles::{DataCycleValidation, validate_data_cycles_in};
use super::diagnostic::{Diagnostic, Severity, sort_diagnostics};
use super::exports::validate_exports_with_ports;
use super::inputs::{validate_inputs_in, validate_type_locks};
use super::ports::PortResolver;
use super::regions::{RegionIndex, RegionValidation, check_zones, validate_regions_in};
use super::scope::{Scope, Tally};
use super::streams::validate_streams_with_ports;
use super::visits::VisitCounts;
use super::{LeafEffects, ValidationReport, analyze_with_effects};

fn no_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().all(|d| d.severity != Severity::Error)
}

impl AvailabilityValidation {
    pub(crate) fn is_valid(&self) -> bool {
        no_errors(&self.diagnostics)
    }
}

impl ConnectionValidation {
    pub(crate) fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty() && no_errors(&self.diagnostics)
    }
}

impl ControlCycleValidation {
    pub(crate) fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty() && no_errors(&self.diagnostics)
    }
}

impl DataCycleValidation {
    pub(crate) fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty() && no_errors(&self.diagnostics)
    }
}

pub(crate) fn validate_availability(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    regions: &RegionIndex,
    registry: &RegistrySnapshot,
) -> AvailabilityValidation {
    let ports = PortResolver::new(document, resolved);
    validate_availability_with_ports(document, &ports, connections, regions, registry)
}

pub(crate) fn validate_connections(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> ConnectionValidation {
    let index = DocIndex::build(document);
    let ports = PortResolver::new(document, resolved);
    let mut visits = VisitCounts::default();
    validate_connections_in(
        document,
        &ports,
        registry,
        Scope::All,
        &index,
        None,
        &mut visits,
    )
}

pub(crate) fn validate_control_cycles(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    registry: &RegistrySnapshot,
) -> ControlCycleValidation {
    let ports = PortResolver::new(document, resolved);
    validate_control_cycles_in(
        document,
        &ports,
        connections,
        registry,
        Scope::All,
        Tally::NONE,
    )
}

pub(crate) fn validate_data_cycles(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    registry: &RegistrySnapshot,
) -> DataCycleValidation {
    let ports = PortResolver::new(document, resolved);
    let mut visits = VisitCounts::default();
    validate_data_cycles_in(
        document,
        &ports,
        connections,
        registry,
        Scope::All,
        &mut visits,
    )
}

pub(crate) fn validate_exports(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let ports = PortResolver::new(document, resolved);
    let mut regions = RegionValidation::default();
    check_zones(
        document,
        &ports,
        document.root(),
        None,
        &BTreeSet::new(),
        registry.origin(),
        &mut regions,
    );
    validate_exports_with_ports(document, &ports, registry, &regions.index.zones)
}

pub(crate) fn validate_inputs(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let mut visits = VisitCounts::default();
    let mut diagnostics = validate_inputs_in(
        document,
        resolved,
        registry,
        Scope::All,
        None,
        None,
        &mut visits,
    );
    diagnostics.extend(validate_type_locks(document, registry));
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

pub(crate) fn validate_regions(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    registry: &RegistrySnapshot,
) -> RegionValidation {
    let ports = PortResolver::new(document, resolved);
    validate_regions_in(
        document,
        &ports,
        connections,
        registry,
        Scope::All,
        Tally::NONE,
    )
}

pub(crate) fn validate_streams(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let ports = PortResolver::new(document, resolved);
    validate_streams_with_ports(document, &ports, registry)
}

pub(crate) fn validate_with_effects(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: BTreeMap<NodeId, LeafEffects>,
) -> ValidationReport {
    analyze_with_effects(document, registry, leaf_effects).into_report()
}
