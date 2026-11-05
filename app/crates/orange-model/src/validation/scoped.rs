//! Scoped session updates (V12b). A change that edits only literals, type arguments or type
//! pins of root-body nodes, or only adds or removes root-body data edges, re-checks what it
//! can reach and patches the retained report, provided every result it re-derives is
//! unchanged apart from input diagnostics. Anything else runs the full pipeline (16 §5).
//!
//! Literals are read by admission, inference and the input check alone, pins by admission
//! and inference, and persisted type arguments by admission only. Inference reads literals
//! and pins through a node's generic component, and resolution also reads whether each data
//! neighbour of a component resolves. So re-resolving the edited nodes, their neighbours
//! and those nodes' components, with every neighbour of that set available, reproduces the
//! whole-document result for all of them; no other node can read the edit. A data edge is
//! also read by the connection checks, data cycles, availability and export cones, which
//! [`edge_update`] checks for the edge alone.
use std::collections::{BTreeMap, BTreeSet};

use super::acceptance::Accepted;
use super::inputs::node_input_diagnostics;
use super::limits::Admitted;
use super::nodes::{CallEvidence, Inference, validate_node_ids};
use super::regions::{RegionKind, RegionRoot};
use super::scope::{NodeSubset, Scope};
use super::session::ScopeFallback;
use super::visits::{PassVisits, VisitCounts};
use super::{
    AnalysisOptions, Diagnostic, LeafEffects, Severity, ValidationAnalysis, sort_diagnostics,
};
use crate::catalog::{RegistrySnapshot, TypeArgs};
use crate::document::{
    BodyRef, Edge, GraphDocument, NodeKind, PortDirection, PortKind, SignatureSnapshot,
};
use crate::edit::{ChangeSet, NodeEdit};
use crate::id::NodeId;
use crate::index::DocIndex;

/// What the last full analysis kept for scoped updates.
#[derive(Clone)]
pub(super) struct Retained {
    pub admitted: Admitted,
    /// The callee acceptance the final node resolution used.
    pub calls: Accepted,
    /// Node resolution diagnostics per subject node, sorted.
    pub node_diagnostics: BTreeMap<NodeId, Vec<Diagnostic>>,
    /// Nodes node resolution left incomplete.
    pub node_incomplete: BTreeSet<NodeId>,
    /// Input diagnostics per node, sorted; nodes without any are absent.
    pub inputs: BTreeMap<NodeId, Vec<Diagnostic>>,
    /// Configuration diagnostics per node, likewise.
    pub config: BTreeMap<NodeId, Vec<Diagnostic>>,
    /// Function-pass and conformance diagnostics per function, sorted. They stay in `base`;
    /// a scoped edit inside a function body must leave its function's unchanged.
    pub functions: BTreeMap<crate::id::FunctionId, Vec<Diagnostic>>,
    /// Every other published diagnostic, sorted and deduplicated.
    pub base: Vec<Diagnostic>,
    /// Availability, region and control-cycle diagnostics, sorted and deduplicated: what a
    /// scoped exec edit recomputes for the exec-connected nodes it reaches.
    pub flow: Vec<Diagnostic>,
    /// State diagnostics, sorted and deduplicated; a scoped exec edit reruns the pass.
    pub state: Vec<Diagnostic>,
    /// No node or function was incomplete. Every dependency-unavailable diagnostic comes
    /// from an incomplete node, so then no error can suppress one, and a node may gain or
    /// lose input, configuration or node errors without changing any other diagnostic.
    pub complete: bool,
    /// The report was complete, every zone marker belonged to a checked pair, and it had
    /// errors at most in the per-node node, input and configuration lists: the condition
    /// under which a structure edit can be checked locally. Scoped updates keep it.
    pub clean: bool,
    /// The report would be clean but for incomplete nodes and their dependency-unavailable
    /// errors, with every function complete. A structure edit can still be checked locally
    /// when nothing it reads or reruns touches an incomplete node or its neighbours.
    pub isolable: bool,
    /// Every zone marker belonged to a checked pair, so each zone body is closed: no data
    /// edge leaves it except into its end (V12c). Scoped edits keep it, as they never touch
    /// a marker.
    pub zones_checked: bool,
    /// Each checked zone's body by its end marker, nested zones' nodes included, as the
    /// region pass finds it ([`zone_bodies`]). Scoped structure edits keep it (V12f).
    pub zone_bodies: BTreeMap<NodeId, BTreeSet<NodeId>>,
    /// No node may suspend, so stale-capture checks cannot read data edges. Data edges and
    /// nodes are checked locally only then; exec edges recompute the state pass anyway.
    pub quiet: bool,
    /// Each body's data dependency order, kept by scoped structure edits (V12d). The root's
    /// is built with the report, and holds the edges its cycles run through when the report
    /// is complete; a function body's is built by the first structure edit there. `None`
    /// when the body's nodes form a data cycle that the order does not hold.
    pub orders: BTreeMap<BodyRef, Option<super::order::TopoOrder>>,
    /// The root body's data-cycle diagnostics, sorted. A root structure edit in a complete
    /// report recomputes them from its order's held edges, so they block no scoped edit.
    pub cycles: Vec<Diagnostic>,
    /// Each generic node's solved type arguments, as the full analysis would solve them for
    /// the current document: scoped updates replace the entries of the nodes they re-solve
    /// and drop removed nodes'. Complete only while the report is.
    pub args: BTreeMap<NodeId, TypeArgs>,
    /// Every published port type is short enough that no term the unifier builds from it
    /// can exceed its expansion budget ([`types_bounded`]). Added nodes keep it current.
    pub bounded: bool,
    /// Expanding (unowned pure or volatile) nodes whose lazy cone is known to carry no
    /// availability requirement (V12c): noted by the full availability pass and by
    /// [`super::availability::data_edge_requires`], which then need not walk them again.
    /// A structure edit whose data edges may bring a requirement clears it, removed nodes
    /// leave it, and removed edges only shrink cones.
    pub free: BTreeSet<NodeId>,
}

/// Whether no port of `signatures` has a type longer than the unifier's per-term budget of
/// expanded type nodes. A ground type has at most as many nodes as bytes, and solving any
/// equation order applies a term no larger than its ground solution, so while a component's
/// solution is published this bounds every term a solver of its equations builds.
pub(super) fn types_bounded<'s>(
    signatures: impl IntoIterator<Item = &'s SignatureSnapshot>,
) -> bool {
    signatures.into_iter().all(|signature| {
        signature.ports.iter().all(|port| {
            port.ty.as_ref().is_none_or(|ty| {
                ty.as_str().len() <= crate::inference::unify::MAX_EXPANDED_TYPE_NODES
            })
        })
    })
}

/// `diagnostics` grouped by subject node and sorted, or `None` if one has no node.
pub(super) fn by_node(diagnostics: &[Diagnostic]) -> Option<BTreeMap<NodeId, Vec<Diagnostic>>> {
    let mut nodes = BTreeMap::<NodeId, Vec<Diagnostic>>::new();
    for diagnostic in diagnostics {
        nodes
            .entry(diagnostic.subject.node?)
            .or_default()
            .push(diagnostic.clone());
    }
    for list in nodes.values_mut() {
        sort_diagnostics(list);
    }
    Some(nodes)
}

/// A proven scoped update: each edited node's old and new input diagnostics, then its old
/// and new configuration diagnostics when its configuration changed, and the work it did.
pub(super) struct Scoped {
    pub inputs: Vec<(Vec<Diagnostic>, Vec<Diagnostic>)>,
    /// Nodes re-resolved, including neighbours read only for availability.
    pub resolved: usize,
    pub components: usize,
    /// Nodes whose input check ran.
    pub checked_inputs: usize,
    /// Nodes whose configuration check ran.
    pub checked_config: usize,
    /// Nodes added or removed, whose signatures the update published or withdrew.
    pub changed_nodes: BTreeSet<NodeId>,
    /// What each pass visited.
    pub visits: PassVisits,
}

/// Re-checks a literal, type argument or pin edit against `retained`, which it updates only
/// on success. An error means the edit is not provably local, and why; the caller must run
/// the full analysis.
pub(super) fn node_update(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    options: &AnalysisOptions<'_>,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &mut Retained,
    edit: &NodeEdit,
) -> Result<Scoped, ScopeFallback> {
    use ScopeFallback as Why;
    options.checkpoint().map_err(|_| Why::Cancelled)?;
    let mut visits = PassVisits::ZERO;
    let mut removed = Admitted::default();
    let mut added = Admitted::default();
    let mut bodies = Vec::new();
    for (id, before) in &edit.before {
        // A function-body node is scoped too (V12e): with its signature and error status
        // unchanged, its function's blocked set, summary and acceptance are unchanged, so
        // no caller can observe the edit.
        let body = document.locate_node(*id).ok_or(Why::MissingEntity)?;
        if !bodies.contains(&body) {
            bodies.push(body);
            // Resolution and the input check each run once over the nodes of every body
            // the edit touches.
            visits.resolution.bodies += 1;
            visits.inputs.bodies += 1;
        }
        let node = document.node(*id).ok_or(Why::MissingEntity)?;
        // Structural passes read configuration of core intrinsics only (Fork, Switch,
        // While, zones, variables, registers); a package node's reaches admission, its
        // configuration check and its contract.
        if (before.config != node.config || before.config_version != node.config_version)
            && node.provider.is_none()
        {
            return Err(Why::IntrinsicConfig);
        }
        removed = removed
            .add(Admitted::of_node_fields(
                &before.config,
                &before.inputs,
                &before.type_args,
                &before.type_pins,
            ))
            .ok_or(Why::Admission)?;
        added = added
            .add(Admitted::of_node_fields(
                &node.config,
                &node.inputs,
                &node.type_args,
                &node.type_pins,
            ))
            .ok_or(Why::Admission)?;
    }
    let admitted = retained
        .admitted
        .replace(removed, added)
        .filter(|admitted| options.fits(*admitted))
        .ok_or(Why::Admission)?;
    let touched: BTreeSet<NodeId> = edit.before.keys().copied().collect();
    let seeds = equation_changes(document, registry, index, edit, &mut visits.resolution);
    let resolution = resolution_unchanged(
        document,
        registry,
        index,
        analysis,
        retained,
        &touched,
        &BTreeSet::new(),
        None,
        seeds.as_ref(),
        &mut visits.resolution,
    )?;
    let inputs = recheck_inputs(
        document,
        registry,
        index,
        analysis,
        retained,
        &touched,
        &mut visits.inputs,
    )?;
    let mut config = Vec::new();
    let mut schemas = super::config::ConfigSchemas::default();
    for (id, before) in &edit.before {
        let node = document.node(*id).ok_or(Why::MissingEntity)?;
        if before.config == node.config && before.config_version == node.config_version {
            continue;
        }
        let body = document.locate_node(*id).ok_or(Why::MissingEntity)?;
        // One configuration check of one node in its body.
        visits.config.nodes += 1;
        visits.config.bodies += 1;
        let list: Vec<Diagnostic> =
            super::config::validate_node_config(body, node, registry, &mut schemas)
                .into_iter()
                .collect();
        if strict(document, retained, *id)
            && has_error(&list) != has_error(retained.config.get(id).map_or(&[], Vec::as_slice))
        {
            return Err(Why::ErrorStatus);
        }
        config.push((*id, list));
    }
    options.checkpoint().map_err(|_| Why::Cancelled)?;
    resolution.args.apply(retained);
    let mut scoped = commit(
        retained,
        admitted,
        inputs,
        resolution.resolved,
        resolution.components,
    );
    scoped.checked_config = config.len();
    scoped.visits = visits;
    for (id, list) in config {
        let old = if list.is_empty() {
            retained.config.remove(&id)
        } else {
            retained.config.insert(id, list.clone())
        };
        scoped.inputs.push((old.unwrap_or_default(), list));
    }
    Ok(scoped)
}

/// The edited nodes whose inference equations may have changed, or `None` when one's
/// contract may have (V12f). A node's equations read its contract, its pins and the types
/// of its literals on ports without an incoming edge; a node edit changes no edge, and a
/// literal's value is never read. A configuration change counts only when the contract
/// built from the old configuration differs from the current one.
fn equation_changes(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    edit: &NodeEdit,
    visits: &mut VisitCounts,
) -> Option<BTreeSet<NodeId>> {
    let mut changed = BTreeSet::new();
    for (id, before) in &edit.before {
        visits.nodes += 1;
        let node = document.node(*id)?;
        if before.config != node.config || before.config_version != node.config_version {
            let mut old = node.clone();
            old.config = before.config.clone();
            old.config_version = before.config_version;
            if !matches!(
                (registry.contract(document, &old), registry.contract(document, node)),
                (Ok(old), Ok(new)) if old == new
            ) {
                return None;
            }
        }
        let literals = before.inputs.keys().chain(node.inputs.keys()).any(|key| {
            active_type(index, *id, &before.inputs, key, visits)
                != active_type(index, *id, &node.inputs, key, visits)
        });
        if literals || before.type_pins != node.type_pins {
            changed.insert(*id);
        }
    }
    Some(changed)
}

/// The type of `node`'s literal on input `key` among `inputs`, if no edge shadows it.
fn active_type<'a>(
    index: &DocIndex,
    node: NodeId,
    inputs: &'a BTreeMap<crate::id::PortKey, crate::document::literal::Literal>,
    key: &crate::id::PortKey,
    visits: &mut VisitCounts,
) -> Option<&'a crate::id::TypeExpr> {
    let port = crate::document::PortRef {
        node,
        port: key.clone(),
    };
    visits.ports += 1;
    if index.incoming(&port).next().is_some() {
        visits.edges += 1;
        return None;
    }
    inputs.get(key).map(|literal| &literal.ty)
}

/// Whether `node` must keep its error status: in an incomplete report an error can suppress
/// dependency diagnostics, and in a function body an error blocks the function's acceptance,
/// which its callers read.
fn strict(document: &GraphDocument, retained: &Retained, node: NodeId) -> bool {
    !retained.complete || document.locate_node(node) != Some(BodyRef::Root)
}

/// Whether `retained` holds an error that keeps structure edits from being checked locally.
/// Errors in the per-node lists are allowed: the session replaces those lists itself. So are
/// a function's own function-pass and conformance errors in a complete report: a called
/// function with such an error would leave its call sites incomplete, so the function is
/// uncalled and nothing but its own diagnostics reads its acceptance. The root body's data
/// cycles are allowed too: no other pass reads them, and root structure edits recompute
/// them, or run in full where they cannot (V12d).
pub(super) fn blocking_error(retained: &Retained, incomplete: &BTreeSet<NodeId>) -> bool {
    let own = |item: &&Diagnostic| {
        item.subject.function.is_some_and(|function| {
            retained
                .functions
                .get(&function)
                .is_some_and(|list| list.contains(item))
        })
    };
    retained
        .base
        .iter()
        .filter(|item| !own(item))
        .chain(&retained.flow)
        .chain(&retained.state)
        .filter(|item| {
            item.code != super::DiagnosticCode::CycleData
                || retained
                    .cycles
                    .binary_search_by(|probe| super::diagnostic::diagnostic_order(probe, item))
                    .is_err()
        })
        .filter(|item| {
            // An incomplete node's own dependency diagnostic, which an isolated edit cannot
            // reach.
            item.code != super::DiagnosticCode::DependencyUnavailable
                || item
                    .subject
                    .node
                    .is_none_or(|node| !incomplete.contains(&node))
        })
        .any(|item| item.severity == Severity::Error)
}

/// Whether no node in `nodes` is incomplete or joined by an edge to an incomplete node.
fn isolated<'a>(
    document: &GraphDocument,
    index: &DocIndex,
    incomplete: &BTreeSet<NodeId>,
    mut nodes: impl Iterator<Item = &'a NodeId>,
    visits: &mut VisitCounts,
) -> bool {
    nodes.all(|node| {
        visits.nodes += 1;
        !incomplete.contains(node)
            && index.incident(*node).all(|edge| {
                visits.edges += 1;
                document.edge(edge).is_none_or(|edge| {
                    !incomplete.contains(&edge.from.node) && !incomplete.contains(&edge.to.node)
                })
            })
    })
}

/// Every function that calls `function`, directly or through other functions, callee first.
fn transitive_callers(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    function: crate::id::FunctionId,
    visits: &mut VisitCounts,
) -> Vec<crate::id::FunctionId> {
    let mut analysis = super::functions::FunctionAnalysis::default();
    let graph = super::functions::call_graph(document, registry, &mut analysis, visits);
    let mut callers = BTreeSet::new();
    let mut stack = vec![function];
    while let Some(callee) = stack.pop() {
        for (caller, calls) in &graph {
            // Each function's call sites, read for one callee.
            visits.nodes += 1;
            if calls.values().any(|target| *target == callee) && callers.insert(*caller) {
                stack.push(*caller);
            }
        }
    }
    let (_, cyclic) = super::functions::find_cycles(&graph);
    super::functions::callee_first_order(&graph, &cyclic)
        .into_iter()
        .filter(|id| *id != function && callers.contains(id))
        .collect()
}

/// Whether any Call node in the document targets `function`.
fn is_called(
    document: &GraphDocument,
    function: crate::id::FunctionId,
    visits: &mut VisitCounts,
) -> bool {
    let target = function.to_string();
    document.bodies().any(|body| {
        visits.bodies += 1;
        body.nodes.values().any(|node| {
            visits.nodes += 1;
            node.definition.id.as_str() == crate::catalog::intrinsic_id::CALL
                && node
                    .config
                    .get("function")
                    .and_then(serde_json::Value::as_str)
                    == Some(target.as_str())
        })
    })
}

fn has_error(list: &[Diagnostic]) -> bool {
    list.iter().any(|d| d.severity == Severity::Error)
}

/// Re-checks an edit that only added or removed whole nodes and edges, in one body of a
/// clean report; in a function body, the function's acceptance, summary and diagnostics must
/// hold as well. Every added or removed node must be a pure package node whose ports are
/// all typed data ports and that no export names; such a node is read only by its own
/// resolution, inputs and configuration, its edges and admission. Every edge must join two
/// data ports; it reaches only node resolution, the connection checks of an added edge and
/// its ports, its target's inputs, data cycles, availability and export cones, and each of
/// those is checked for the edge alone. A removed edge or node in a clean report can only
/// take requirements and cycles away. The zones whose bodies or markers the edit touches
/// are re-checked alone ([`zone_update`]).
#[expect(
    clippy::too_many_arguments,
    reason = "an exec edit reruns the state pass, which reads the host's leaf effects"
)]
pub(super) fn structure_update(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    options: &AnalysisOptions<'_>,
    index: &DocIndex,
    analysis: &mut ValidationAnalysis,
    retained: &mut Retained,
    change: &ChangeSet,
    effects: &BTreeMap<NodeId, LeafEffects>,
) -> Result<Scoped, ScopeFallback> {
    use ScopeFallback as Why;
    options.checkpoint().map_err(|_| Why::Cancelled)?;
    let edit = change.structure_edit().ok_or(Why::UnscopedChange)?;
    if !retained.clean && !retained.isolable {
        return Err(Why::ReportNotClean);
    }
    let mut visits = PassVisits::ZERO;
    // The edit stays in one body: the root, or one function's body (V12e).
    let mut homes = Vec::new();
    for id in &change.nodes().added {
        homes.push(document.locate_node(*id).ok_or(Why::MissingEntity)?);
    }
    for id in &change.edges().added {
        homes.push(document.locate_edge(*id).ok_or(Why::MissingEntity)?);
    }
    homes.extend(edit.removed_nodes.values().map(|(body, _)| *body));
    homes.extend(edit.removed_edges.values().map(|(body, _)| *body));
    homes.dedup();
    let [home] = homes[..] else {
        return Err(Why::MultipleBodies);
    };
    // A complete report's root body keeps its data cycles with its order (V12d); an edit
    // that would leave a cycle anywhere else runs in full.
    let hold = home == BodyRef::Root && retained.complete;
    let root_cycles = home == BodyRef::Root && !retained.cycles.is_empty();
    if root_cycles && (!hold || !matches!(retained.orders.get(&home), Some(Some(_)))) {
        return Err(Why::DataCycle);
    }
    let root = document.body(home).ok_or(Why::MissingEntity)?;
    let added_nodes = &change.nodes().added;
    let exported = |id: &NodeId| document.exports().iter().any(|export| export.node == *id);
    let mut removed_charge = Admitted::default();
    let mut added_charge = Admitted::default();
    // Zone markers pair through configuration and data reach, which only the whole region
    // pass checks; every other node is checked here or by a control-flow rerun.
    let zoned = |node: &crate::document::NodeInstance| {
        super::zones::ZoneKind::marker(node.definition.id.as_str()).is_some()
    };
    // Intrinsics and nodes with exec or untyped ports reach the control-flow passes.
    let mut flow_nodes = false;
    for id in added_nodes {
        let node = root.nodes.get(id).ok_or(Why::MissingEntity)?;
        if exported(id) || zoned(node) {
            return Err(Why::NodeKind);
        }
        flow_nodes |= node.provider.is_none();
        added_charge = added_charge
            .add(Admitted::of_node(document, registry, node))
            .ok_or(Why::Admission)?;
    }
    for (id, (body, node)) in &edit.removed_nodes {
        if *body != home {
            return Err(Why::MultipleBodies);
        }
        if exported(id) || zoned(node) {
            return Err(Why::NodeKind);
        }
        flow_nodes |= node.provider.is_none()
            || analysis
                .report()
                .resolved
                .get(id)
                .is_none_or(|signature| !pure_data(signature));
        removed_charge = removed_charge
            .add(Admitted::of_node(document, registry, node))
            .ok_or(Why::Admission)?;
    }
    let added_edges = &change.edges().added;
    let mut edges: Vec<(&Edge, bool)> =
        Vec::with_capacity(added_edges.len() + edit.removed_edges.len());
    for id in added_edges {
        edges.push((root.edges.get(id).ok_or(Why::MissingEntity)?, true));
    }
    for (_, edge) in edit.removed_edges.values() {
        edges.push((edge, false));
    }
    let mut touched: BTreeSet<NodeId> = added_nodes.clone();
    let mut targets: BTreeSet<NodeId> = added_nodes.clone();
    for (edge, is_added) in &edges {
        for end in [edge.from.node, edge.to.node] {
            if root.nodes.contains_key(&end) {
                touched.insert(end);
            } else if *is_added {
                return Err(Why::MultipleBodies);
            }
        }
        if root.nodes.contains_key(&edge.to.node) {
            targets.insert(edge.to.node);
        }
        let charge = Admitted::of_edge(edge);
        if *is_added {
            added_charge = added_charge.add(charge).ok_or(Why::Admission)?;
        } else {
            removed_charge = removed_charge.add(charge).ok_or(Why::Admission)?;
        }
    }
    let admitted = retained
        .admitted
        .replace(removed_charge, added_charge)
        .filter(|admitted| options.fits(*admitted))
        .ok_or(Why::Admission)?;
    visits.resolution.bodies += 1;
    let Resolution {
        resolved,
        components,
        fresh,
        reach,
        args,
    } = resolution_unchanged(
        document,
        registry,
        index,
        analysis,
        retained,
        &touched,
        added_nodes,
        (!retained.complete).then_some(&analysis.report().incomplete_nodes),
        Some(&touched),
        &mut visits.resolution,
    )?;
    if fresh.values().any(|(signature, _)| !typed_data(signature)) {
        return Err(Why::NodeKind);
    }
    flow_nodes |= fresh.values().any(|(signature, _)| !pure_data(signature));

    // Stage the new node set's signatures so the edge and input checks see it; a failed
    // check restores the published ones before the caller runs the full analysis.
    let staged: Vec<(NodeId, SignatureSnapshot)> = edit
        .removed_nodes
        .keys()
        .filter_map(|id| analysis.resolved_mut().remove(id).map(|old| (*id, old)))
        .collect();
    for (id, (signature, _)) in &fresh {
        analysis.resolved_mut().insert(*id, signature.clone());
    }
    let zone_change = {
        let ports = super::ports::PortResolver::new(document, &analysis.report().resolved);
        zone_update(
            document,
            &ports,
            index,
            &analysis.report().regions.zones,
            &retained.zone_bodies,
            touched.iter().chain(edit.removed_nodes.keys()),
            added_nodes,
            &mut visits.zones,
        )
    };
    // The full analysis builds the root body's order. A function body's first structure
    // edit builds its own from the edited document, which then already accounts for the
    // edit; later edits bring an order up to date.
    let built = !retained.orders.contains_key(&home);
    if built {
        let ports = super::ports::PortResolver::new(document, &analysis.report().resolved);
        let zones = &analysis.report().regions.zones;
        let order =
            super::order::TopoOrder::build(document, &ports, home, zones, hold, &mut visits.order);
        retained.orders.insert(home, order);
    }
    let mut order = retained.orders.get_mut(&home).and_then(Option::as_mut);
    if let Some(order) = order.as_deref_mut().filter(|_| !built) {
        for id in edit.removed_nodes.keys() {
            order.remove(*id);
        }
        for (_, edge) in edit.removed_edges.values() {
            order.release(edge.id);
        }
        for id in added_nodes {
            order.push(*id);
        }
    }
    let zones = &analysis.report().regions.zones;
    // Every walk below keeps `free` true of the edited document, except that a data edge
    // bringing a requirement into a cone is reported as requiring one: then it is cleared.
    // A failed update leaves it empty, which is always true, for the full analysis to
    // replace.
    let mut free = std::mem::take(&mut retained.free);
    for id in edit.removed_nodes.keys() {
        free.remove(id);
    }
    let kinds = classify_edges(
        document,
        registry,
        index,
        analysis,
        &edges,
        order.as_deref_mut(),
        zones,
        &mut free,
        hold,
        &mut visits,
    );
    // With an edge held, or cycles before the edit, the root body's cycles are recomputed
    // from the order; otherwise the body is acyclic before and after.
    let cyclic = root_cycles
        || order
            .as_deref()
            .is_some_and(super::order::TopoOrder::is_cyclic);
    let cycles = match kinds {
        Ok(_) if cyclic => {
            let removed = !edit.removed_edges.is_empty() || !edit.removed_nodes.is_empty();
            cycle_update(
                document,
                registry,
                index,
                analysis,
                order,
                removed,
                &mut visits.order,
            )
            .map(Some)
            .ok_or(Why::DataCycle)
        }
        Ok(_) => Ok(None),
        Err(why) => Err(why),
    };
    let kinds = cycles.as_ref().map_err(|why| *why).and(kinds);
    if kinds.is_ok_and(|kinds| kinds.requires) {
        free.clear();
    }
    // Without a control-flow rerun, data edges and pure nodes need the quiet condition, for
    // the stale-capture check, and no availability requirement. With one, which exec edges
    // and flow nodes force, the rerun covers both: state reruns in full and availability
    // reruns for every consumer the edit's data edges can reach.
    let local = kinds.and_then(|kinds| {
        let structural = kinds.data || !added_nodes.is_empty() || !edit.removed_nodes.is_empty();
        if kinds.flow || flow_nodes || ((!structural || retained.quiet) && !kinds.requires) {
            Ok(kinds)
        } else {
            Err(Why::Availability)
        }
    });
    let mut function_updates = Vec::new();
    let mut state_override = None;
    let checked = zone_change
        .as_ref()
        .ok_or(Why::Zone)
        .and(local)
        .and_then(|kinds| {
            visits.inputs.bodies += usize::from(!targets.is_empty());
            let inputs = recheck_inputs(
                document,
                registry,
                index,
                analysis,
                retained,
                &targets,
                &mut visits.inputs,
            )?;
            let flow = if kinds.flow || flow_nodes {
                let ports = super::ports::PortResolver::new(document, &analysis.report().resolved);
                let mut seeds: BTreeSet<NodeId> = added_nodes.clone();
                seeds.extend(edit.removed_nodes.keys().copied());
                for (edge, _) in &edges {
                    seeds.insert(edge.from.node);
                    seeds.insert(edge.to.node);
                    if root.nodes.contains_key(&edge.to.node) && expands(&ports, edge.to.node) {
                        // Consumers that read the edge through the target's lazy cone.
                        seeds.extend(lazy_readers(
                            document,
                            &ports,
                            index,
                            edge.to.node,
                            &mut visits.availability,
                        ));
                    }
                }
                drop(ports);
                Some(
                    recompute_flow(
                        document,
                        registry,
                        index,
                        analysis,
                        retained,
                        effects,
                        &seeds,
                        &mut free,
                        &mut visits,
                    )
                    .ok_or(Why::ControlFlow)?,
                )
            } else {
                None
            };
            Ok((inputs, flow))
        })
        .and_then(|(inputs, flow)| {
            let mut schemas = super::config::ConfigSchemas::default();
            let mut config = Vec::new();
            for id in added_nodes {
                visits.config.nodes += 1;
                visits.config.bodies += 1;
                let list: Vec<Diagnostic> = super::config::validate_node_config(
                    home,
                    document.node(*id).ok_or(Why::MissingEntity)?,
                    registry,
                    &mut schemas,
                )
                .into_iter()
                .collect();
                config.push((*id, list));
            }
            // Inside a function body, the edit is local only while the function's
            // acceptance, summary and own diagnostics, which its callers read, hold.
            if let BodyRef::Function(function) = home {
                let report = analysis.report();
                let check = super::acceptance::check_function(
                    document,
                    registry,
                    effects,
                    index,
                    &retained.calls,
                    &report.functions,
                    function,
                    options,
                    &mut visits.functions,
                )
                .map_err(|_| Why::Cancelled)?;
                // Call nodes read a function's acceptance, so a called function must keep
                // it. Callers' summaries and the state pass read its summary; a changed one
                // is propagated to every transitive caller, each of which must keep its own
                // acceptance, and the state pass reruns with the new summaries.
                let empty = Vec::new();
                let accepted = check.accepted != retained.calls.get(&function).copied();
                let summary = check.summary.as_ref() != report.functions.get(&function);
                let called = is_called(document, function, &mut visits.functions);
                if check.incomplete || (accepted && called) {
                    return Err(Why::Function);
                }
                if summary && called {
                    let mut summaries = report.functions.clone();
                    match &check.summary {
                        Some(new) => summaries.insert(function, new.clone()),
                        None => summaries.remove(&function),
                    };
                    for caller in
                        transitive_callers(document, registry, function, &mut visits.functions)
                    {
                        let check = super::acceptance::check_function(
                            document,
                            registry,
                            effects,
                            index,
                            &retained.calls,
                            &summaries,
                            caller,
                            options,
                            &mut visits.functions,
                        )
                        .map_err(|_| Why::Cancelled)?;
                        if check.incomplete
                            || check.accepted != retained.calls.get(&caller).copied()
                        {
                            return Err(Why::Function);
                        }
                        let changed = check.summary.as_ref() != report.functions.get(&caller)
                            || &check.diagnostics
                                != retained.functions.get(&caller).unwrap_or(&empty);
                        match &check.summary {
                            Some(new) => summaries.insert(caller, new.clone()),
                            None => summaries.remove(&caller),
                        };
                        if changed {
                            function_updates.push((caller, check));
                        }
                    }
                    let regions = flow
                        .as_ref()
                        .map_or(&report.regions, |update: &FlowUpdate| &update.regions);
                    let mut state = super::state::validate_state_counted(
                        document,
                        registry,
                        &report.resolved,
                        regions,
                        &summaries,
                        effects,
                        &mut visits.state,
                    );
                    if has_error(&state) {
                        return Err(Why::State);
                    }
                    sort_diagnostics(&mut state);
                    state.dedup();
                    state_override = Some(state);
                }
                if accepted
                    || summary
                    || &check.diagnostics != retained.functions.get(&function).unwrap_or(&empty)
                {
                    function_updates.insert(0, (function, check));
                }
            }
            options.checkpoint().map_err(|_| Why::Cancelled)?;
            Ok((inputs, flow, config))
        });
    // In an incomplete report, every node the edit reached must keep clear of the
    // incomplete nodes, whose own diagnostics this update does not recompute.
    let checked = checked.and_then(|(inputs, flow, config)| {
        let clear = retained.complete || {
            let incomplete = &analysis.report().incomplete_nodes;
            !edit
                .removed_nodes
                .keys()
                .any(|node| incomplete.contains(node))
                && isolated(
                    document,
                    index,
                    incomplete,
                    reach
                        .iter()
                        .chain(&touched)
                        .chain(&targets)
                        .chain(flow.iter().flat_map(|update| &update.component))
                        .chain(zone_change.iter().flat_map(|change| &change.walked)),
                    &mut visits.resolution,
                )
        };
        if clear {
            Ok((inputs, flow, config))
        } else {
            Err(Why::Incomplete)
        }
    });
    let ((inputs, flow, config), zone_change) =
        match checked.and_then(|checked| Ok((checked, zone_change.ok_or(Why::Zone)?))) {
            Ok(checked) => checked,
            Err(why) => {
                for id in fresh.keys() {
                    analysis.resolved_mut().remove(id);
                }
                analysis.resolved_mut().extend(staged);
                return Err(why);
            }
        };

    args.apply(retained);
    retained.bounded &= types_bounded(fresh.values().map(|(signature, _)| signature));
    retained.free = free;
    let mut scoped = commit(retained, admitted, inputs, resolved, components);
    scoped.checked_config = config.len();
    scoped.visits = visits;
    if let Ok(Some(CycleUpdate { diagnostics, paths })) = cycles {
        // The root cycles' diagnostics live in `base`; swap them there, then publish. Their
        // witnesses replace the root body's, which are those not in a function body.
        let old = std::mem::replace(&mut retained.cycles, diagnostics.clone());
        swap_base(&mut retained.base, &old, &diagnostics);
        analysis.replace_cycle_paths(
            |path| {
                path.first().is_some_and(|port| {
                    matches!(document.locate_node(port.node), Some(BodyRef::Function(_)))
                })
            },
            paths,
        );
        scoped.inputs.push((old, diagnostics));
    }
    for (function, check) in function_updates {
        match check.accepted {
            Some(accepted) => retained.calls.insert(function, accepted),
            None => retained.calls.remove(&function),
        };
        analysis.set_summary(function, check.summary);
        // The function's diagnostics live in `base`; swap them there, then publish.
        let old = retained.functions.remove(&function).unwrap_or_default();
        swap_base(&mut retained.base, &old, &check.diagnostics);
        if !check.diagnostics.is_empty() {
            retained
                .functions
                .insert(function, check.diagnostics.clone());
        }
        scoped.inputs.push((old, check.diagnostics));
        // Only errors outside the function lists could change these flags.
        retained.clean = retained.complete
            && !blocking_error(retained, &BTreeSet::new())
            && retained.zones_checked;
        retained.isolable =
            !blocking_error(retained, &analysis.report().incomplete_nodes) && retained.isolable;
    }
    let mut state = state_override;
    if let Some(update) = flow {
        analysis.set_regions(update.regions);
        if update.component.is_empty() {
            let old = std::mem::replace(&mut retained.flow, update.flow.clone());
            scoped.inputs.push((old, update.flow));
        } else {
            // Only the component's diagnostics change, so only they are compared and
            // patched; every other kept item stays in place and in order.
            let component = &update.component;
            let (old, mut kept): (Vec<Diagnostic>, Vec<Diagnostic>) =
                std::mem::take(&mut retained.flow)
                    .into_iter()
                    .partition(|item| {
                        item.subject
                            .node
                            .is_some_and(|node| component.contains(&node))
                    });
            kept.extend(update.flow.iter().cloned());
            sort_diagnostics(&mut kept);
            retained.flow = kept;
            scoped.inputs.push((old, update.flow));
        }
        state.get_or_insert(update.state);
    }
    // After the flow update, whose region index may carry the zones from before the edit.
    zone_change.apply(analysis.zones_mut(), &mut retained.zone_bodies);
    if let Some(state) = state {
        let old = std::mem::replace(&mut retained.state, state.clone());
        scoped.inputs.push((old, state));
    }
    for (id, list) in config {
        if !list.is_empty() {
            retained.config.insert(id, list.clone());
            scoped.inputs.push((Vec::new(), list));
        }
    }
    for (id, (_, list)) in fresh {
        if !list.is_empty() {
            retained.node_diagnostics.insert(id, list.clone());
            scoped.inputs.push((Vec::new(), list));
        }
    }
    for id in edit.removed_nodes.keys() {
        retained.node_incomplete.remove(id);
        retained.args.remove(id);
        for map in [
            &mut retained.node_diagnostics,
            &mut retained.inputs,
            &mut retained.config,
        ] {
            if let Some(old) = map.remove(id) {
                scoped.inputs.push((old, Vec::new()));
            }
        }
    }
    scoped.changed_nodes = added_nodes
        .iter()
        .chain(edit.removed_nodes.keys())
        .copied()
        .collect();
    Ok(scoped)
}

/// Replaces `old` with `new` in the sorted `base`.
fn swap_base(base: &mut Vec<Diagnostic>, old: &[Diagnostic], new: &[Diagnostic]) {
    for item in old {
        if let Ok(at) =
            base.binary_search_by(|probe| super::diagnostic::diagnostic_order(probe, item))
        {
            base.remove(at);
        }
    }
    for item in new {
        if let Err(at) =
            base.binary_search_by(|probe| super::diagnostic::diagnostic_order(probe, item))
        {
            base.insert(at, item.clone());
        }
    }
}

/// The root body's data cycles after a structure edit: their sorted diagnostics, and
/// their witnesses.
struct CycleUpdate {
    diagnostics: Vec<Diagnostic>,
    paths: Vec<Vec<crate::document::PortRef>>,
}

/// Recomputes the root body's data cycles after a structure edit that held an edge, or
/// in a body that had cycles, against the staged signatures (V12d, 16 §5.3). After an edit
/// that removed edges or nodes, held edges that no longer close a cycle are placed. Every
/// cycle then runs through a held edge, so the cycles are those of the data-cycle pass
/// over the nodes [`TopoOrder::cyclic_region`](super::order::TopoOrder::cyclic_region)
/// finds, which it builds and chooses as it does for the whole body. `None` past that
/// region's budget, and in a document with exports while the order holds an edge: an
/// export's cone may then reach the edit's edges against the order, which the per-edge
/// export check left alone.
fn cycle_update(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    order: Option<&mut super::order::TopoOrder>,
    removed: bool,
    visits: &mut VisitCounts,
) -> Option<CycleUpdate> {
    let order = order?;
    if order.is_cyclic() && !document.exports().is_empty() {
        return None;
    }
    visits.bodies += 1;
    let ports = super::ports::PortResolver::new(document, &analysis.report().resolved);
    if removed && order.is_cyclic() {
        order.retry(document, &ports, index, visits);
    }
    let mut update = CycleUpdate {
        diagnostics: Vec::new(),
        paths: Vec::new(),
    };
    if !order.is_cyclic() {
        return Some(update);
    }
    let region = order.cyclic_region(document, &ports, index, visits)?;
    let mut edges = BTreeMap::new();
    for node in &region {
        visits.nodes += 1;
        for id in index.incident(*node) {
            visits.edges += 1;
            if let Some(edge) = document
                .edge(id)
                .filter(|edge| region.contains(&edge.from.node) && region.contains(&edge.to.node))
            {
                edges.insert(id, edge);
            }
        }
    }
    // Choosing the region's zones scans every zone pair of the order.
    visits.edges += order.zone_pairs();
    let found = super::data_cycles::cycles_among(
        &ports,
        document.root(),
        &region,
        order.zones_among(&region),
        edges.into_values(),
        registry.origin(),
        visits,
    )?;
    for (diagnostic, path) in found {
        update.diagnostics.push(diagnostic);
        update.paths.push(path);
    }
    sort_diagnostics(&mut update.diagnostics);
    Some(update)
}

/// Each checked zone's body by its end marker, from the innermost owners the region pass
/// keeps: a node belongs to its owner's body and to every enclosing zone's, except that a
/// begin is owned by its own zone but lies only in the enclosing bodies.
pub(super) fn zone_bodies(zones: &super::zones::ZoneIndex) -> BTreeMap<NodeId, BTreeSet<NodeId>> {
    let mut bodies: BTreeMap<NodeId, BTreeSet<NodeId>> = zones
        .pairs
        .keys()
        .map(|end| (*end, BTreeSet::new()))
        .collect();
    for (node, owner) in &zones.owners {
        let mut zone = zones.pairs.get(owner);
        if let Some(pair) = zone.filter(|pair| pair.begin == *node) {
            zone = pair.parent.and_then(|end| zones.pairs.get(&end));
        }
        while let Some(pair) = zone {
            bodies.entry(pair.end).or_default().insert(*node);
            zone = pair.parent.and_then(|end| zones.pairs.get(&end));
        }
    }
    bodies
}

/// How many nodes outside a zone's body a structure edit may bring into its begin's reach
/// before only the whole region pass decides the zone (V12f).
const ZONE_PULL_BUDGET: usize = 1024;

/// The zone bodies a structure edit changed, by end marker, and every node its re-check
/// walked.
struct ZoneChange {
    bodies: Vec<(NodeId, BTreeSet<NodeId>)>,
    walked: Vec<NodeId>,
}

impl ZoneChange {
    /// Moves the nodes that left or joined each changed body, which encloses and sits in
    /// no other zone, so each is its own innermost owner.
    fn apply(
        self,
        zones: &mut super::zones::ZoneIndex,
        bodies: &mut BTreeMap<NodeId, BTreeSet<NodeId>>,
    ) {
        for (end, body) in self.bodies {
            let old = bodies.insert(end, body.clone()).unwrap_or_default();
            for node in old.difference(&body) {
                zones.owners.remove(node);
            }
            for node in body.difference(&old) {
                zones.owners.insert(*node, end);
            }
        }
    }
}

/// Re-checks every zone a structure edit reaches, against the staged signatures, as the
/// region pass checks it (V12c, V12f), or `None` when only the whole pass can decide.
///
/// A zone body is what its begin reaches through data edges, stopping at its end, among
/// the nodes that reach the end (11 §8.4). With every marker in a checked pair, no data
/// edge leaves a body except into its end, so everything the begin reaches is the body or
/// the end. An edit none of whose touched (edge endpoints and added nodes) or removed nodes
/// lies in a body, or is its begin or end, changes no zone: an added edge whose source is
/// outside cannot be reached from any begin, so it adds nothing the begin reaches; a
/// removed edge with an outside source was never on such a path, and every path from a
/// body node to the end stays inside the body. So the zones reached are those whose body
/// holds a touched or removed node, with every zone enclosing them, and those whose begin
/// or end is touched.
///
/// For each, the begin's reach is walked again. A reached node that was neither in the
/// body nor added is an escape, which the closure check below finds, or an existing node
/// pulled into the body, as when an undo restores the begin's edge; the walk gives up on
/// one owned by a zone or a marker, and after [`ZONE_PULL_BUDGET`] of them. The new body
/// is the part of that reach which reaches the end within it, since every node on a path
/// from a reached node to the end is reached too. It must be closed,
/// pure, keep each marker inside with its partner and hold no export, as the region pass
/// requires; then every other zone keeps its body, and no marker moves into or out of one.
/// Paired types read the markers' signatures only, which the node resolution kept, and
/// the data-cycle pass's begin-to-end dependencies, which the body order carries, read
/// the pairs alone. A body that changed must enclose and sit in no other zone, so that
/// its nodes' innermost owner is simply it or none.
#[expect(
    clippy::too_many_arguments,
    reason = "the session's visit counters are charged with the walks' reads"
)]
fn zone_update<'a>(
    document: &GraphDocument,
    ports: &super::ports::PortResolver<'_>,
    index: &DocIndex,
    zones: &super::zones::ZoneIndex,
    bodies: &BTreeMap<NodeId, BTreeSet<NodeId>>,
    nodes: impl Iterator<Item = &'a NodeId>,
    added: &BTreeSet<NodeId>,
    visits: &mut VisitCounts,
) -> Option<ZoneChange> {
    let mut reached = BTreeSet::new();
    for node in nodes {
        // Each touched or removed node's zone owner is looked up.
        visits.nodes += 1;
        if zones.pairs.contains_key(node) {
            reached.insert(*node);
        }
        let mut zone = zones.owners.get(node).copied();
        while let Some(end) = zone {
            if !reached.insert(end) {
                break;
            }
            zone = zones.pairs.get(&end)?.parent;
        }
    }
    let marker = |node: &NodeId| {
        zones.pairs.contains_key(node) || zones.owner(*node).is_some_and(|pair| pair.begin == *node)
    };
    let mut change = ZoneChange {
        bodies: Vec::new(),
        walked: Vec::new(),
    };
    visits.bodies += usize::from(!reached.is_empty());
    for end in reached {
        let pair = zones.pairs.get(&end)?;
        let old = bodies.get(&end)?;
        let begin = pair.begin;
        let mut after = BTreeSet::from([begin]);
        let mut edges = Vec::new();
        let mut stack = vec![begin];
        let mut pulled = 0;
        while let Some(node) = stack.pop() {
            if node == end {
                continue;
            }
            visits.nodes += 1;
            for id in index.incident(node) {
                visits.edges += 1;
                let Some(edge) = document.edge(id).filter(|edge| edge.from.node == node) else {
                    continue;
                };
                visits.ports += 2;
                let data = matches!(
                    (ports.port(&edge.from), ports.port(&edge.to)),
                    (Ok(from), Ok(to)) if from.kind == PortKind::Data
                        && to.kind == PortKind::Data
                        && from.direction == PortDirection::Output
                        && to.direction == PortDirection::Input
                );
                if !data {
                    continue;
                }
                let target = edge.to.node;
                edges.push((node, target));
                if !after.insert(target) {
                    continue;
                }
                if target != end && !old.contains(&target) && !added.contains(&target) {
                    // Another zone's node or marker, this begin, or past the budget.
                    pulled += 1;
                    if zones.owners.contains_key(&target)
                        || zones.pairs.contains_key(&target)
                        || pulled > ZONE_PULL_BUDGET
                    {
                        return None;
                    }
                }
                stack.push(target);
            }
        }
        let mut sources: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        for (from, to) in &edges {
            sources.entry(*to).or_default().push(*from);
        }
        let mut before = BTreeSet::from([end]);
        let mut stack = vec![end];
        while let Some(node) = stack.pop() {
            if node == begin {
                continue;
            }
            visits.nodes += 1;
            for from in sources.get(&node).into_iter().flatten() {
                visits.edges += 1;
                if before.insert(*from) {
                    stack.push(*from);
                }
            }
        }
        let body: BTreeSet<NodeId> = after
            .intersection(&before)
            .filter(|node| **node != begin && **node != end)
            .copied()
            .collect();
        let closed = edges.iter().all(|(from, to)| {
            visits.edges += 1;
            (*from != begin && !body.contains(from)) || *to == end || body.contains(to)
        });
        let pure = body.iter().all(|node| {
            visits.nodes += 1;
            ports.get(node).is_none_or(|signature| {
                matches!(signature.kind, NodeKind::Pure | NodeKind::Volatile)
            })
        });
        let nested = body.iter().all(|node| {
            visits.nodes += 1;
            zones
                .pairs
                .get(node)
                .is_none_or(|other| body.contains(&other.begin))
                && zones
                    .owner(*node)
                    .filter(|other| other.begin == *node)
                    .is_none_or(|other| body.contains(&other.end))
        });
        let exported = pair.function.is_none()
            && document
                .exports()
                .iter()
                .any(|export| export.node == begin || body.contains(&export.node));
        if !closed || !pure || !nested || exported {
            return None;
        }
        change.walked.extend(after);
        if body != *old {
            if pair.parent.is_some() || old.iter().chain(&body).any(marker) {
                return None;
            }
            change.bodies.push((end, body));
        }
    }
    Some(change)
}

/// A pure node whose ports are all typed data ports, outputs with a source: the data
/// cycle pass needs nothing else of it, and no control or region pass sees it.
fn pure_data(signature: &SignatureSnapshot) -> bool {
    signature.kind == NodeKind::Pure
        && signature
            .ports
            .iter()
            .all(|port| port.kind == PortKind::Data)
        && typed_data(signature)
}

/// Every data port is typed and every data output has a source, as the data cycle pass
/// requires of a node it does not mark incomplete.
fn typed_data(signature: &SignatureSnapshot) -> bool {
    signature
        .ports
        .iter()
        .filter(|port| port.kind == PortKind::Data)
        .all(|port| {
            port.ty.is_some() && (port.direction == PortDirection::Input || port.source.is_some())
        })
}

/// What the edges of a structure edit can reach.
#[derive(Default, Clone, Copy)]
struct EdgeKinds {
    /// A data edge: only its target's inputs, once its own checks pass.
    data: bool,
    /// An exec or event edge: the control-flow passes too.
    flow: bool,
    /// A data edge whose producer or cone carries an availability requirement, so only a
    /// control-flow rerun can account for it.
    requires: bool,
}

/// Classifies each edge with the staged signatures, or `None` when one may change more than
/// its target's inputs and, for exec edges, the control-flow passes. With the body's data
/// order, an added data edge is checked for cycles within its order interval and for
/// exports by its producer's cone; without one, by walking everything it feeds. With
/// `hold`, an edge that closes a cycle is held by the order, for [`cycle_update`] to
/// diagnose, and the export check is left to it while the order holds an edge.
#[expect(
    clippy::too_many_arguments,
    reason = "the availability check also reads and extends the known requirement-free cones"
)]
fn classify_edges(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    edges: &[(&Edge, bool)],
    mut order: Option<&mut super::order::TopoOrder>,
    zones: &super::zones::ZoneIndex,
    free: &mut BTreeSet<NodeId>,
    hold: bool,
    visits: &mut PassVisits,
) -> Result<EdgeKinds, ScopeFallback> {
    use ScopeFallback as Why;
    let report = analysis.report();
    let ports = super::ports::PortResolver::new(document, &report.resolved);
    let origin = registry.origin();
    let mut kinds = EdgeKinds::default();
    let mut why = Why::EdgeKind;
    let (mut read_order, mut read_cones) = (false, false);
    let local = edges.iter().all(|(edge, is_added)| {
        visits.connections.edges += 1;
        // Whether the edge carries a published diagnostic: a scan of the report up to the
        // first one that does.
        let scanned = report
            .diagnostics
            .iter()
            .position(|diagnostic| diagnostic.subject.edge == Some(edge.id));
        visits.connections.diagnostics += scanned.map_or(report.diagnostics.len(), |at| at + 1);
        let published = scanned.is_some();
        if !is_added
            && (document.node(edge.from.node).is_none() || document.node(edge.to.node).is_none())
        {
            // The edge went with a removed node, which was pure and clean.
            kinds.data = true;
            why = Why::EdgeDiagnostics;
            return !published;
        }
        visits.connections.ports += 2;
        let exec = matches!(
            (ports.port(&edge.from), ports.port(&edge.to)),
            (Ok(from), Ok(to)) if matches!(from.kind, PortKind::Exec | PortKind::Event)
                && to.kind == PortKind::Exec
                && from.direction == PortDirection::Output
                && to.direction == PortDirection::Input
        );
        if exec {
            kinds.flow = true;
            return if *is_added {
                why = Why::ConnectionCheck;
                super::connections::added_edge_is_clean(
                    document,
                    &ports,
                    index,
                    edge,
                    origin,
                    &mut visits.connections,
                )
            } else {
                why = Why::EdgeDiagnostics;
                !published
            };
        }
        kinds.data = true;
        let data = matches!(
            (ports.port(&edge.from), ports.port(&edge.to)),
            (Ok(from), Ok(to)) if from.kind == PortKind::Data
                && to.kind == PortKind::Data
                && from.direction == PortDirection::Output
                && to.direction == PortDirection::Input
        );
        if !data {
            why = Why::EdgeKind;
            return false;
        }
        read_cones = true;
        kinds.requires |= super::availability::data_edge_requires(
            document,
            &ports,
            &report.regions,
            index,
            &edge.from,
            free,
            &mut visits.availability,
        );
        read_order = true;
        if let Some(order) = order.as_deref_mut() {
            if !*is_added {
                // A removed edge only shrinks cones, and closes no cycle.
                why = Why::EdgeDiagnostics;
                return !published;
            }
            if !order.insert(
                document,
                &ports,
                index,
                edge.from.node,
                edge.to.node,
                &mut visits.order,
            ) {
                if !hold {
                    why = Why::DataCycle;
                    return false;
                }
                order.hold(edge.id, edge.from.node, edge.to.node);
            }
            // Before the edge, no export's cone crossed an activation, and the edge adds
            // its producer's cone to those of the exports it can reach, which lie at or
            // after its target in the order while it holds no edge. The availability
            // requirement already covers producers reached through pure nodes; this covers
            // any other computed output.
            let at = order.position(edge.to.node);
            let reaches_export = !order.is_cyclic()
                && document
                    .exports()
                    .iter()
                    .any(|export| order.position(export.node) >= at);
            if reaches_export
                && super::exports::edge_crosses_activation(
                    document,
                    &ports,
                    index,
                    edge,
                    zones,
                    &mut visits.order,
                )
            {
                why = Why::Export;
                return false;
            }
            why = Why::ConnectionCheck;
            return super::connections::added_edge_is_clean(
                document,
                &ports,
                index,
                edge,
                origin,
                &mut visits.connections,
            );
        }
        let downstream = data_downstream(
            document,
            &ports,
            index,
            edge.to.node,
            zones,
            &mut visits.order,
        );
        if document
            .exports()
            .iter()
            .any(|export| downstream.contains(&export.node))
        {
            why = Why::Export;
            return false;
        }
        if *is_added {
            // Acyclic before, so a new cycle must lead from the target back to the source.
            if downstream.contains(&edge.from.node) {
                why = Why::DataCycle;
                return false;
            }
            why = Why::ConnectionCheck;
            super::connections::added_edge_is_clean(
                document,
                &ports,
                index,
                edge,
                origin,
                &mut visits.connections,
            )
        } else {
            why = Why::EdgeDiagnostics;
            !published
        }
    });
    // Each of these passes ran in the edit's one body.
    visits.connections.bodies += usize::from(!edges.is_empty());
    visits.availability.bodies += usize::from(read_cones);
    visits.order.bodies += usize::from(read_order);
    if local { Ok(kinds) } else { Err(why) }
}

/// New control-flow results after an exec edit.
struct FlowUpdate {
    /// The new availability, region and control-cycle diagnostics of the component's
    /// nodes, which replace theirs, or of the whole document when `component` is empty;
    /// sorted and deduplicated.
    flow: Vec<Diagnostic>,
    state: Vec<Diagnostic>,
    regions: super::regions::RegionIndex,
    /// The exec-connected nodes the passes reran for; empty after a whole-document rerun,
    /// which only a complete report can pass.
    component: BTreeSet<NodeId>,
}

/// Reruns the region, control-cycle and availability passes for the exec-connected nodes
/// around `seeds` (V12c), and the state pass in full, with the clean connection result an
/// edge edit that passed [`classify_edges`] has. Region ownership outside that set cannot
/// change, so its region index entries and diagnostics are replaced alone. When a node
/// whose ownership changed feeds an owned consumer elsewhere through data, that
/// consumer's availability could change too, and the whole document is rerun instead.
/// `None` when the passes find an error, an incomplete node or a control cycle.
///
/// Availability checks only the set's consumers that read a requirement, directly or
/// through the lazy cone behind a data input, as `free` and
/// [`super::availability::data_edge_requires`] tell; any other consumer has no
/// availability diagnostic in any region.
#[expect(
    clippy::too_many_arguments,
    reason = "availability also reads and extends the known requirement-free cones"
)]
fn recompute_flow(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &Retained,
    effects: &BTreeMap<NodeId, LeafEffects>,
    seeds: &BTreeSet<NodeId>,
    free: &mut BTreeSet<NodeId>,
    visits: &mut PassVisits,
) -> Option<FlowUpdate> {
    let report = analysis.report();
    let ports = super::ports::PortResolver::new(document, &report.resolved);
    let connections = super::connections::ConnectionValidation::default();
    let component = exec_component(document, &ports, index, seeds, &mut visits.control_flow);
    // Once the set is most of the body, gathering it through the index costs more than the
    // whole-document passes, which give the same result.
    if component.len() > document.root().nodes.len() / 2 {
        return whole_flow(document, registry, analysis, effects, &ports, free, visits);
    }
    // Exec edges never leave the component, so its own outgoing edges are all the region
    // and control-cycle passes read.
    let outgoing = NodeSubset {
        nodes: &component,
        consumers: &component,
        edges: {
            let mut ids = BTreeSet::new();
            for &node in &component {
                ids.extend(index.incident(node).filter(|id| {
                    visits.control_flow.edges += 1;
                    document
                        .edge(*id)
                        .is_some_and(|edge| edge.from.node == node)
                }));
            }
            ids.into_iter().filter_map(|id| document.edge(id)).collect()
        },
    };
    // The region and control-cycle passes count what they iterate as they go.
    let counted = std::cell::Cell::default();
    let tally = super::scope::Tally::new(&counted);
    let regions = super::regions::validate_regions_among(
        document,
        &ports,
        &connections,
        registry,
        &outgoing,
        tally,
    );
    let control = super::control_cycles::validate_control_cycles_among(
        document,
        &ports,
        &connections,
        registry,
        &outgoing,
        tally,
    );
    visits.control_flow += counted.get();
    if !regions.incomplete_nodes.is_empty()
        || !control.incomplete_nodes.is_empty()
        || !control.cycle_paths.is_empty()
    {
        return None;
    }
    let old = &report.regions;
    // Merging scans the document's whole region index: its regions, owners and
    // unreachable nodes.
    let merging = &mut visits.control_flow;
    let mut merged = old.clone();
    merged.regions.retain(|root, _| {
        merging.nodes += 1;
        !component.contains(&root.node)
    });
    merged.owners.retain(|node, _| {
        merging.nodes += 1;
        !component.contains(node)
    });
    merged.unreachable.retain(|node| {
        merging.nodes += 1;
        !component.contains(node)
    });
    merged.regions.extend(regions.index.regions);
    merged.owners.extend(regions.index.owners);
    merged.unreachable.extend(regions.index.unreachable);

    // What availability reads of a producer's ownership: its region and that region's
    // lineage, with each one's kind and loop nesting, but not its other members.
    /// One region of a lineage: its root, kind, enclosing loop and loop depth.
    type Lineage<'i> = (
        &'i RegionRoot,
        Option<(RegionKind, &'i Option<RegionRoot>, usize)>,
    );
    fn region_of<'i>(
        index: &'i super::regions::RegionIndex,
        node: &NodeId,
        visits: &mut VisitCounts,
    ) -> Vec<Lineage<'i>> {
        let mut lineage = Vec::new();
        let mut current = index.owners.get(node);
        while let Some(root) = current {
            visits.nodes += 1;
            let region = index.regions.get(root);
            lineage.push((
                root,
                region.map(|region| (region.kind, &region.enclosing_loop, region.loop_depth)),
            ));
            current = region.and_then(|region| region.parent.as_ref());
        }
        lineage
    }
    let moved: Vec<NodeId> = component
        .iter()
        .filter(|node| {
            region_of(old, node, &mut visits.control_flow)
                != region_of(&merged, node, &mut visits.control_flow)
        })
        .copied()
        .collect();
    let crosses = moved.iter().any(|node| {
        lazy_readers(document, &ports, index, *node, &mut visits.availability)
            .iter()
            .any(|consumer| !component.contains(consumer) && merged.owners.contains_key(consumer))
    });
    if crosses {
        return whole_flow(document, registry, analysis, effects, &ports, free, visits);
    }

    // Pure and volatile nodes are never owned, so lazy cones do not depend on regions;
    // should one's ownership change anyway, forget which cones are known to be free.
    if moved.iter().any(|node| {
        matches!(
            ports.get(node).map(|signature| signature.kind),
            Some(NodeKind::Pure | NodeKind::Volatile)
        )
    }) {
        free.clear();
    }
    // Availability reports only on owned consumers, and only for values whose producer, or
    // an edge in the lazy cone behind it, carries a requirement. Every other node of the
    // set has no availability diagnostic, before or after the edit.
    let consumers: BTreeSet<NodeId> = component
        .iter()
        .copied()
        .filter(|node| {
            merged.owners.contains_key(node)
                && reads_requirement(
                    document,
                    &ports,
                    &merged,
                    index,
                    *node,
                    free,
                    &mut visits.availability,
                )
        })
        .collect();
    let availability = if consumers.is_empty() {
        super::availability::AvailabilityValidation::default()
    } else {
        // The set's exec edges and the data cones of those consumers.
        let cones = super::availability::subset_edges(
            document,
            &ports,
            &merged,
            index,
            &component,
            &consumers,
            &mut visits.availability,
        );
        let subset = NodeSubset {
            nodes: &component,
            consumers: &consumers,
            edges: cones,
        };
        let counted = std::cell::Cell::default();
        let availability = super::availability::validate_availability_among(
            document,
            &ports,
            &connections,
            &merged,
            registry,
            &subset,
            super::scope::Tally::new(&counted),
        );
        visits.availability += counted.get();
        availability
    };
    let state = super::state::validate_state_counted(
        document,
        registry,
        &report.resolved,
        &merged,
        &report.functions,
        effects,
        &mut visits.state,
    );
    let mut fresh = regions.diagnostics;
    fresh.extend(control.diagnostics);
    fresh.extend(availability.diagnostics);
    let local = |item: &Diagnostic| {
        item.subject
            .node
            .is_some_and(|node| component.contains(&node))
    };
    let unattached = retained
        .flow
        .iter()
        .position(|item| item.subject.node.is_none());
    visits.control_flow.diagnostics += unattached.map_or(retained.flow.len(), |at| at + 1);
    if !fresh.iter().all(local) || unattached.is_some() || has_error(&fresh) || has_error(&state) {
        return whole_flow(document, registry, analysis, effects, &ports, free, visits);
    }
    let mut flow = fresh;
    sort_diagnostics(&mut flow);
    flow.dedup();
    let mut state = state;
    sort_diagnostics(&mut state);
    state.dedup();
    Some(FlowUpdate {
        flow,
        state,
        regions: merged,
        component,
    })
}

/// The control-flow passes over the whole document, for an exec edit whose effect may
/// leave its exec-connected nodes.
fn whole_flow(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    analysis: &ValidationAnalysis,
    effects: &BTreeMap<NodeId, LeafEffects>,
    ports: &super::ports::PortResolver<'_>,
    free: &mut BTreeSet<NodeId>,
    visits: &mut PassVisits,
) -> Option<FlowUpdate> {
    let report = analysis.report();
    let connections = super::connections::ConnectionValidation::default();
    // The whole-document passes count what they iterate as they go.
    let (flow_counted, available_counted) =
        (std::cell::Cell::default(), std::cell::Cell::default());
    let tally = super::scope::Tally::new(&flow_counted);
    let regions = super::regions::validate_regions_in(
        document,
        ports,
        &connections,
        registry,
        Scope::All,
        tally,
    );
    let control = super::control_cycles::validate_control_cycles_in(
        document,
        ports,
        &connections,
        registry,
        Scope::All,
        tally,
    );
    visits.control_flow += flow_counted.get();
    if !regions.incomplete_nodes.is_empty()
        || !control.incomplete_nodes.is_empty()
        || !control.cycle_paths.is_empty()
    {
        return None;
    }
    let mut flow = super::availability::validate_availability_noting_free(
        document,
        ports,
        &connections,
        &regions.index,
        registry,
        free,
        super::scope::Tally::new(&available_counted),
    )
    .diagnostics;
    visits.availability += available_counted.get();
    let mut state = super::state::validate_state_counted(
        document,
        registry,
        &report.resolved,
        &regions.index,
        &report.functions,
        effects,
        &mut visits.state,
    );
    flow.extend(regions.diagnostics);
    flow.extend(control.diagnostics);
    if has_error(&flow) || has_error(&state) {
        return None;
    }
    sort_diagnostics(&mut flow);
    flow.dedup();
    sort_diagnostics(&mut state);
    state.dedup();
    Some(FlowUpdate {
        flow,
        state,
        regions: regions.index,
        component: BTreeSet::new(),
    })
}

/// Whether availability may find a requirement on `node`'s values: some data input whose
/// producer's output, or an edge in the lazy cone behind it, carries one, or whose ports
/// do not resolve.
fn reads_requirement(
    document: &GraphDocument,
    ports: &super::ports::PortResolver<'_>,
    regions: &super::regions::RegionIndex,
    index: &DocIndex,
    node: NodeId,
    free: &mut BTreeSet<NodeId>,
    visits: &mut VisitCounts,
) -> bool {
    visits.nodes += 1;
    index.incident(node).any(|id| {
        visits.edges += 1;
        let Some(edge) = document.edge(id).filter(|edge| edge.to.node == node) else {
            return false;
        };
        visits.ports += 2;
        match (ports.port(&edge.from), ports.port(&edge.to)) {
            (Ok(from), Ok(to)) if from.kind != PortKind::Data || to.kind != PortKind::Data => false,
            _ => super::availability::data_edge_requires(
                document, ports, regions, index, &edge.from, free, visits,
            ),
        }
    })
}

/// The root-body nodes joined to `seeds` by exec or event edges, in either direction.
fn exec_component(
    document: &GraphDocument,
    ports: &super::ports::PortResolver<'_>,
    index: &DocIndex,
    seeds: &BTreeSet<NodeId>,
    visits: &mut VisitCounts,
) -> BTreeSet<NodeId> {
    visits.bodies += 1;
    let mut seen = BTreeSet::new();
    let mut stack: Vec<NodeId> = seeds.iter().copied().collect();
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        visits.nodes += 1;
        for id in index.incident(node) {
            visits.edges += 1;
            let Some(edge) = document.edge(id) else {
                continue;
            };
            visits.ports += 2;
            let exec = matches!(
                (ports.port(&edge.from), ports.port(&edge.to)),
                (Ok(from), Ok(to)) if matches!(from.kind, PortKind::Exec | PortKind::Event)
                    && to.kind == PortKind::Exec
            );
            if exec {
                stack.push(if edge.from.node == node {
                    edge.to.node
                } else {
                    edge.from.node
                });
            }
        }
    }
    seen
}

/// What [`resolution_unchanged`] proved: how many nodes it resolved, how many components it
/// solved, the added nodes' results, every node it resolved and the arguments to keep.
struct Resolution {
    resolved: usize,
    components: usize,
    fresh: Fresh,
    reach: Vec<NodeId>,
    args: ArgUpdate,
}

/// The re-solved nodes' new arguments, stored in [`Retained::args`] once an update commits.
#[derive(Default)]
struct ArgUpdate {
    nodes: BTreeSet<NodeId>,
    args: BTreeMap<NodeId, TypeArgs>,
}

impl ArgUpdate {
    fn apply(self, retained: &mut Retained) {
        let Self { nodes, mut args } = self;
        for id in nodes {
            match args.remove(&id) {
                Some(args) => retained.args.insert(id, args),
                None => retained.args.remove(&id),
            };
        }
    }
}

/// Re-resolves `touched`, their neighbours and those nodes' generic components, with every
/// neighbour of that set available, and confirms each result equals the retained one.
/// Nodes in `added` are new: each must resolve without errors, and its results are returned.
/// With `incomplete`, every node the resolution reads must keep clear of those nodes.
/// With `seeds`, the edit kept every remaining node's contract and changed only equations
/// over the variables of generic nodes among `seeds`, which hold every added node and
/// every remaining endpoint of a removed edge, and of removed nodes: then a kept solution
/// ([`kept_resolution`]) is tried before the components are solved.
#[expect(
    clippy::too_many_arguments,
    reason = "structure edits in incomplete reports also pass the nodes to keep clear of"
)]
fn resolution_unchanged(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &Retained,
    touched: &BTreeSet<NodeId>,
    added: &BTreeSet<NodeId>,
    incomplete: Option<&BTreeSet<NodeId>>,
    seeds: Option<&BTreeSet<NodeId>>,
    visits: &mut VisitCounts,
) -> Result<Resolution, ScopeFallback> {
    if let Some(seeds) = seeds
        && retained.complete
        && retained.bounded
        && let Some(kept) = kept_resolution(
            document, registry, index, analysis, retained, touched, added, seeds, visits,
        )
    {
        return Ok(kept);
    }
    let near = with_neighbours(document, index, touched, visits);
    let components = crate::inference::component::component_nodes(
        document,
        index,
        near.iter().copied(),
        registry,
        visits,
    );
    let mut affected = near;
    for component in &components {
        affected.extend(component.iter().copied());
    }
    let ids: Vec<NodeId> = with_neighbours(document, index, &affected, visits)
        .into_iter()
        .collect();
    // In an incomplete report, give up before solving if the set reaches an incomplete node.
    if incomplete
        .is_some_and(|incomplete| !isolated(document, index, incomplete, ids.iter(), visits))
    {
        return Err(ScopeFallback::Incomplete);
    }
    let calls = CallEvidence {
        accepted: retained.calls.clone(),
        pipeline: true,
    };
    let mut args = BTreeMap::new();
    let nodes = validate_node_ids(
        document,
        registry,
        &calls,
        &ids,
        index,
        Inference::Solve(None),
        Some(&mut args),
        visits,
    );
    // Grouping the resolution diagnostics by node reads each of them once.
    visits.diagnostics += nodes.diagnostics.len();
    let fresh = by_node(&nodes.diagnostics).ok_or(ScopeFallback::Resolution)?;
    let fresh_added = compare(analysis, retained, &nodes, &fresh, &affected, added, visits)
        .ok_or(ScopeFallback::Resolution)?;
    Ok(Resolution {
        resolved: ids.len(),
        components: components.len(),
        fresh: fresh_added,
        reach: ids,
        args: ArgUpdate {
            nodes: affected,
            args,
        },
    })
}

/// Resolution of an edit that changed only equations over the variables of `touched`
/// nodes, in a complete report, without solving their components (V12f). When
/// [`solution_kept`](crate::inference::component::solution_kept) proves every component's
/// solution unchanged, every node keeps its arguments. A node's resolution reads its own
/// fields, its contract, its arguments and, for a generic node, whether every member of
/// its component is eligible with its data neighbours eligible; it reads no edge
/// otherwise. Eligibility reads a node's own fields, so it changes only for edited nodes,
/// which are touched, and which data neighbours a node has changes only for the endpoints
/// of changed edges, which are touched too. A touched node that loses its eligibility
/// resolves differently itself, so the comparison of the touched nodes fails; while every
/// one keeps it, only a touched generic node's own data neighbours can block its
/// component. So resolving the touched nodes, the generic ones with their kept arguments
/// and their neighbours available, decides every changed result. A non-generic endpoint,
/// such as a Get Variable read by many clusters, is therefore resolved but not expanded.
///
/// Whole nodes may come and go (V12f): every `added` node is touched and takes the
/// arguments `solution_kept` checked for it, and must resolve without errors. A removed
/// node only takes away a member or neighbour, which cannot block a component; its
/// remaining neighbours are touched, as endpoints of its removed edges. An added node's
/// published types must stay within the unifier's budget, as [`Retained::bounded`]
/// requires of every published type before a kept solution is trusted.
/// `None` leaves the components to be solved.
#[expect(
    clippy::too_many_arguments,
    reason = "structure edits also pass their added nodes"
)]
fn kept_resolution(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &Retained,
    touched: &BTreeSet<NodeId>,
    added: &BTreeSet<NodeId>,
    seeds: &BTreeSet<NodeId>,
    visits: &mut VisitCounts,
) -> Option<Resolution> {
    use crate::inference::component::{KEPT_SOLUTION_BUDGET, solution_kept};
    let added_args = solution_kept(
        document,
        index,
        registry,
        seeds,
        added,
        |id| retained.args.get(&id),
        KEPT_SOLUTION_BUDGET,
        visits,
    )?;
    // In a complete report every generic node has arguments, and no other node does.
    let args_of = |id: &NodeId| added_args.get(id).or_else(|| retained.args.get(id));
    let near: BTreeSet<NodeId> = touched
        .iter()
        .copied()
        .filter(|id| args_of(id).is_some())
        .collect();
    let ids: Vec<NodeId> = with_neighbours(document, index, &near, visits)
        .into_iter()
        .chain(touched.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let type_args: BTreeMap<NodeId, TypeArgs> = near
        .iter()
        .filter_map(|id| Some((*id, args_of(id)?.clone())))
        .collect();
    let known = crate::inference::ComponentInference {
        nodes: type_args.keys().copied().collect(),
        type_args,
        issues: Vec::new(),
    };
    let calls = CallEvidence {
        accepted: retained.calls.clone(),
        pipeline: true,
    };
    let nodes = validate_node_ids(
        document,
        registry,
        &calls,
        &ids,
        index,
        Inference::Known(known),
        None,
        visits,
    );
    // Grouping the resolution diagnostics by node reads each of them once.
    visits.diagnostics += nodes.diagnostics.len();
    let fresh = by_node(&nodes.diagnostics)?;
    let fresh = compare(analysis, retained, &nodes, &fresh, touched, added, visits)?;
    if !types_bounded(fresh.values().map(|(signature, _)| signature)) {
        return None;
    }
    Some(Resolution {
        resolved: ids.len(),
        components: 0,
        fresh,
        reach: ids,
        args: ArgUpdate {
            nodes: added.clone(),
            args: added_args,
        },
    })
}

/// Checks that every node of `nodes` but `added` resolved as retained, and that each added
/// node resolved without errors; returns the added nodes' results. Each node compared, its
/// signature's ports and its diagnostics are added to `visits`.
fn compare(
    analysis: &ValidationAnalysis,
    retained: &Retained,
    resolution: &super::NodeValidation,
    fresh: &BTreeMap<NodeId, Vec<Diagnostic>>,
    nodes: &BTreeSet<NodeId>,
    added: &BTreeSet<NodeId>,
    visits: &mut VisitCounts,
) -> Option<Fresh> {
    let report = analysis.report();
    let empty = Vec::new();
    let mut fresh_added = Fresh::new();
    for id in nodes {
        visits.nodes += 1;
        visits.ports += resolution
            .resolved
            .get(id)
            .map_or(0, |signature| signature.ports.len());
        visits.diagnostics += fresh.get(id).map_or(0, Vec::len);
        if added.contains(id) {
            // A new node has nothing to compare with: it must resolve without errors.
            let list = fresh.get(id).cloned().unwrap_or_default();
            if resolution.incomplete_nodes.contains(id) || has_error(&list) {
                return None;
            }
            fresh_added.insert(*id, (resolution.resolved.get(id)?.clone(), list));
            continue;
        }
        if resolution.resolved.get(id) != report.resolved.get(id)
            || resolution.incomplete_nodes.contains(id) != retained.node_incomplete.contains(id)
            || fresh.get(id).unwrap_or(&empty)
                != retained.node_diagnostics.get(id).unwrap_or(&empty)
        {
            return None;
        }
    }
    if added.iter().any(|id| !fresh_added.contains_key(id)) {
        return None;
    }
    Some(fresh_added)
}

/// Each added node's new signature and node diagnostics.
type Fresh = BTreeMap<NodeId, (SignatureSnapshot, Vec<Diagnostic>)>;

/// The current input diagnostics of `nodes`, whose signatures are unchanged. A node that
/// [`strict`] binds runs in full when it gains or loses an input error.
fn recheck_inputs(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &Retained,
    nodes: &BTreeSet<NodeId>,
    visits: &mut VisitCounts,
) -> Result<Vec<(NodeId, Vec<Diagnostic>)>, ScopeFallback> {
    let report = analysis.report();
    let empty = Vec::new();
    let mut inputs = Vec::with_capacity(nodes.len());
    for id in nodes {
        let node = document.node(*id).ok_or(ScopeFallback::MissingEntity)?;
        visits.nodes += 1;
        let mut list = match report.resolved.get(id) {
            Some(snapshot) => {
                node_input_diagnostics(document, node, snapshot, registry, index, visits)
            }
            None => Vec::new(),
        };
        sort_diagnostics(&mut list);
        if strict(document, retained, *id)
            && has_error(&list) != has_error(retained.inputs.get(id).unwrap_or(&empty))
        {
            return Err(ScopeFallback::ErrorStatus);
        }
        inputs.push((*id, list));
    }
    Ok(inputs)
}

/// Stores a proven update in `retained` and returns each node's old and new inputs.
fn commit(
    retained: &mut Retained,
    admitted: Admitted,
    inputs: Vec<(NodeId, Vec<Diagnostic>)>,
    resolved: usize,
    components: usize,
) -> Scoped {
    retained.admitted = admitted;
    let inputs: Vec<_> = inputs
        .into_iter()
        .map(|(id, list)| {
            let old = if list.is_empty() {
                retained.inputs.remove(&id)
            } else {
                retained.inputs.insert(id, list.clone())
            };
            (old.unwrap_or_default(), list)
        })
        .collect();
    Scoped {
        checked_inputs: inputs.len(),
        checked_config: 0,
        inputs,
        resolved,
        components,
        changed_nodes: BTreeSet::new(),
        visits: PassVisits::ZERO,
    }
}

/// `node` and every node it feeds through data edges, a zone's begin feeding its end as
/// the data-cycle pass has it.
fn data_downstream(
    document: &GraphDocument,
    ports: &super::ports::PortResolver<'_>,
    index: &DocIndex,
    node: NodeId,
    zones: &super::zones::ZoneIndex,
    visits: &mut VisitCounts,
) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        visits.nodes += 1;
        if let Some(pair) = zones.owner(node).filter(|pair| pair.begin == node) {
            stack.push(pair.end);
        }
        for id in index.incident(node) {
            visits.edges += 1;
            let Some(edge) = document.edge(id).filter(|edge| edge.from.node == node) else {
                continue;
            };
            visits.ports += 1;
            if matches!(ports.port(&edge.from), Ok(port) if port.kind == PortKind::Data) {
                stack.push(edge.to.node);
            }
        }
    }
    seen
}

/// True for a pure or volatile node, which is never owned, so its consumers read its inputs
/// lazily (as availability's expanding producers).
fn expands(ports: &super::ports::PortResolver<'_>, node: NodeId) -> bool {
    matches!(
        ports.get(&node).map(|signature| signature.kind),
        Some(NodeKind::Pure | NodeKind::Volatile)
    )
}

/// The nodes whose availability may read `node`'s outputs: those it feeds through data
/// edges, directly or through expanding nodes. A non-expanding node stores its outputs, so
/// its consumers read its region and not what lies behind it.
fn lazy_readers(
    document: &GraphDocument,
    ports: &super::ports::PortResolver<'_>,
    index: &DocIndex,
    node: NodeId,
    visits: &mut VisitCounts,
) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![node];
    while let Some(producer) = stack.pop() {
        visits.nodes += 1;
        for id in index.incident(producer) {
            visits.edges += 1;
            let Some(edge) = document.edge(id).filter(|edge| edge.from.node == producer) else {
                continue;
            };
            visits.ports += 1;
            if matches!(ports.port(&edge.from), Ok(port) if port.kind == PortKind::Data)
                && seen.insert(edge.to.node)
                && expands(ports, edge.to.node)
            {
                stack.push(edge.to.node);
            }
        }
    }
    seen
}

/// `nodes` and the other endpoint of every edge incident to one of them.
fn with_neighbours(
    document: &GraphDocument,
    index: &DocIndex,
    nodes: &BTreeSet<NodeId>,
    visits: &mut VisitCounts,
) -> BTreeSet<NodeId> {
    let mut out = nodes.clone();
    for &node in nodes {
        visits.nodes += 1;
        for edge in index.incident(node) {
            visits.edges += 1;
            if let Some(edge) = document.edge(edge) {
                out.insert(edge.from.node);
                out.insert(edge.to.node);
            }
        }
    }
    out
}
