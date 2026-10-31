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
use super::nodes::{CallEvidence, validate_node_ids};
use super::regions::{RegionKind, RegionRoot};
use super::scope::{NodeSubset, Scope};
use super::{
    AnalysisOptions, Diagnostic, LeafEffects, Severity, ValidationAnalysis, sort_diagnostics,
};
use crate::catalog::RegistrySnapshot;
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
    /// The report was complete, had no zone, and had errors at most in the per-node node,
    /// input and configuration lists: the condition under which a structure edit can be
    /// checked locally. Scoped updates keep it.
    pub clean: bool,
    /// No node may suspend, so stale-capture checks cannot read data edges. Data edges and
    /// nodes are checked locally only then; exec edges recompute the state pass anyway.
    pub quiet: bool,
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
}

/// Re-checks a literal, type argument or pin edit against `retained`, which it updates only on success. `None`
/// means the edit is not provably local and the caller must run the full analysis.
pub(super) fn node_update(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    options: &AnalysisOptions<'_>,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &mut Retained,
    edit: &NodeEdit,
) -> Option<Scoped> {
    options.checkpoint().ok()?;
    let mut removed = Admitted::default();
    let mut added = Admitted::default();
    for (id, before) in &edit.before {
        // A function-body node is scoped too (V12e): with its signature and error status
        // unchanged, its function's blocked set, summary and acceptance are unchanged, so
        // no caller can observe the edit.
        document.locate_node(*id)?;
        let node = document.node(*id)?;
        // Structural passes read configuration of core intrinsics only (Fork, Switch,
        // While, zones, variables, registers); a package node's reaches admission, its
        // configuration check and its contract.
        if (before.config != node.config || before.config_version != node.config_version)
            && node.provider.is_none()
        {
            return None;
        }
        removed = removed.add(Admitted::of_node_fields(
            &before.config,
            &before.inputs,
            &before.type_args,
            &before.type_pins,
        ))?;
        added = added.add(Admitted::of_node_fields(
            &node.config,
            &node.inputs,
            &node.type_args,
            &node.type_pins,
        ))?;
    }
    let admitted = retained.admitted.replace(removed, added)?;
    if !options.fits(admitted) {
        return None;
    }
    let touched: BTreeSet<NodeId> = edit.before.keys().copied().collect();
    let (resolved, components, _) = resolution_unchanged(
        document,
        registry,
        index,
        analysis,
        retained,
        &touched,
        &BTreeSet::new(),
    )?;
    let inputs = recheck_inputs(document, registry, index, analysis, retained, &touched)?;
    let mut config = Vec::new();
    let mut schemas = super::config::ConfigSchemas::default();
    for (id, before) in &edit.before {
        let node = document.node(*id)?;
        if before.config == node.config && before.config_version == node.config_version {
            continue;
        }
        let body = document.locate_node(*id)?;
        let list: Vec<Diagnostic> =
            super::config::validate_node_config(body, node, registry, &mut schemas)
                .into_iter()
                .collect();
        if strict(document, retained, *id)
            && has_error(&list) != has_error(retained.config.get(id).map_or(&[], Vec::as_slice))
        {
            return None;
        }
        config.push((*id, list));
    }
    options.checkpoint().ok()?;
    let mut scoped = commit(retained, admitted, inputs, resolved, components);
    scoped.checked_config = config.len();
    for (id, list) in config {
        let old = if list.is_empty() {
            retained.config.remove(&id)
        } else {
            retained.config.insert(id, list.clone())
        };
        scoped.inputs.push((old.unwrap_or_default(), list));
    }
    Some(scoped)
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
/// uncalled and nothing but its own diagnostics reads its acceptance.
pub(super) fn blocking_error(retained: &Retained) -> bool {
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
        .any(|item| item.severity == Severity::Error)
}

/// Every function that calls `function`, directly or through other functions, callee first.
fn transitive_callers(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    function: crate::id::FunctionId,
) -> Vec<crate::id::FunctionId> {
    let mut analysis = super::functions::FunctionAnalysis::default();
    let graph = super::functions::call_graph(document, registry, &mut analysis);
    let mut callers = BTreeSet::new();
    let mut stack = vec![function];
    while let Some(callee) = stack.pop() {
        for (caller, calls) in &graph {
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
fn is_called(document: &GraphDocument, function: crate::id::FunctionId) -> bool {
    let target = function.to_string();
    document.bodies().any(|body| {
        body.nodes.values().any(|node| {
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
/// take requirements and cycles away.
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
) -> Option<Scoped> {
    options.checkpoint().ok()?;
    let edit = change.structure_edit()?;
    if !retained.clean {
        return None;
    }
    // The edit stays in one body: the root, or one function's body (V12e).
    let mut homes = Vec::new();
    for id in &change.nodes().added {
        homes.push(document.locate_node(*id)?);
    }
    for id in &change.edges().added {
        homes.push(document.locate_edge(*id)?);
    }
    homes.extend(edit.removed_nodes.values().map(|(body, _)| *body));
    homes.extend(edit.removed_edges.values().map(|(body, _)| *body));
    homes.dedup();
    let [home] = homes[..] else {
        return None;
    };
    let root = document.body(home)?;
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
        let node = root.nodes.get(id)?;
        if exported(id) || zoned(node) {
            return None;
        }
        flow_nodes |= node.provider.is_none();
        added_charge = added_charge.add(Admitted::of_node(document, registry, node))?;
    }
    for (id, (body, node)) in &edit.removed_nodes {
        if *body != home || exported(id) || zoned(node) {
            return None;
        }
        flow_nodes |= node.provider.is_none()
            || analysis
                .report()
                .resolved
                .get(id)
                .is_none_or(|signature| !pure_data(signature));
        removed_charge = removed_charge.add(Admitted::of_node(document, registry, node))?;
    }
    let added_edges = &change.edges().added;
    let mut edges: Vec<(&Edge, bool)> =
        Vec::with_capacity(added_edges.len() + edit.removed_edges.len());
    for id in added_edges {
        edges.push((root.edges.get(id)?, true));
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
                return None;
            }
        }
        if root.nodes.contains_key(&edge.to.node) {
            targets.insert(edge.to.node);
        }
        let charge = Admitted::of_edge(edge);
        if *is_added {
            added_charge = added_charge.add(charge)?;
        } else {
            removed_charge = removed_charge.add(charge)?;
        }
    }
    let admitted = retained.admitted.replace(removed_charge, added_charge)?;
    if !options.fits(admitted) {
        return None;
    }
    let (resolved, components, fresh) = resolution_unchanged(
        document,
        registry,
        index,
        analysis,
        retained,
        &touched,
        added_nodes,
    )?;
    if fresh.values().any(|(signature, _)| !typed_data(signature)) {
        return None;
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
    let kinds = classify_edges(document, registry, index, analysis, &edges);
    // Without a control-flow rerun, data edges and pure nodes need the quiet condition, for
    // the stale-capture check, and no availability requirement. With one, which exec edges
    // and flow nodes force, the rerun covers both: state reruns in full and availability
    // reruns for every consumer the edit's data edges can reach.
    let local = kinds.filter(|kinds| {
        let structural = kinds.data || !added_nodes.is_empty() || !edit.removed_nodes.is_empty();
        kinds.flow || flow_nodes || ((!structural || retained.quiet) && !kinds.requires)
    });
    let mut function_updates = Vec::new();
    let mut state_override = None;
    let checked = local
        .and_then(|kinds| {
            let inputs = recheck_inputs(document, registry, index, analysis, retained, &targets)?;
            let flow = if kinds.flow || flow_nodes {
                let ports = super::ports::PortResolver::new(document, &analysis.report().resolved);
                let mut seeds: BTreeSet<NodeId> = added_nodes.clone();
                seeds.extend(edit.removed_nodes.keys().copied());
                for (edge, _) in &edges {
                    seeds.insert(edge.from.node);
                    if root.nodes.contains_key(&edge.to.node) {
                        // Consumers the edge feeds, directly or through pure nodes.
                        seeds.extend(data_downstream(document, &ports, index, edge.to.node));
                    } else {
                        seeds.insert(edge.to.node);
                    }
                }
                drop(ports);
                Some(recompute_flow(
                    document, registry, index, analysis, retained, effects, &seeds,
                )?)
            } else {
                None
            };
            Some((inputs, flow))
        })
        .and_then(|(inputs, flow)| {
            let mut schemas = super::config::ConfigSchemas::default();
            let mut config = Vec::new();
            for id in added_nodes {
                let list: Vec<Diagnostic> = super::config::validate_node_config(
                    home,
                    document.node(*id)?,
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
                )
                .ok()?;
                // Call nodes read a function's acceptance, so a called function must keep
                // it. Callers' summaries and the state pass read its summary; a changed one
                // is propagated to every transitive caller, each of which must keep its own
                // acceptance, and the state pass reruns with the new summaries.
                let empty = Vec::new();
                let accepted = check.accepted != retained.calls.get(&function).copied();
                let summary = check.summary.as_ref() != report.functions.get(&function);
                let called = is_called(document, function);
                if check.incomplete || (accepted && called) {
                    return None;
                }
                if summary && called {
                    let mut summaries = report.functions.clone();
                    match &check.summary {
                        Some(new) => summaries.insert(function, new.clone()),
                        None => summaries.remove(&function),
                    };
                    for caller in transitive_callers(document, registry, function) {
                        let check = super::acceptance::check_function(
                            document,
                            registry,
                            effects,
                            index,
                            &retained.calls,
                            &summaries,
                            caller,
                            options,
                        )
                        .ok()?;
                        if check.incomplete
                            || check.accepted != retained.calls.get(&caller).copied()
                        {
                            return None;
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
                    let mut state = super::state::validate_state(
                        document,
                        registry,
                        &report.resolved,
                        regions,
                        &summaries,
                        effects,
                    );
                    if has_error(&state) {
                        return None;
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
            options.checkpoint().ok()?;
            Some((inputs, flow, config))
        });
    let Some((inputs, flow, config)) = checked else {
        for id in fresh.keys() {
            analysis.resolved_mut().remove(id);
        }
        analysis.resolved_mut().extend(staged);
        return None;
    };

    let mut scoped = commit(retained, admitted, inputs, resolved, components);
    scoped.checked_config = config.len();
    for (function, check) in function_updates {
        match check.accepted {
            Some(accepted) => retained.calls.insert(function, accepted),
            None => retained.calls.remove(&function),
        };
        analysis.set_summary(function, check.summary);
        // The function's diagnostics live in `base`; swap them there, then publish.
        let old = retained.functions.remove(&function).unwrap_or_default();
        for item in &old {
            if let Ok(at) = retained
                .base
                .binary_search_by(|probe| super::diagnostic::diagnostic_order(probe, item))
            {
                retained.base.remove(at);
            }
        }
        for item in &check.diagnostics {
            if let Err(at) = retained
                .base
                .binary_search_by(|probe| super::diagnostic::diagnostic_order(probe, item))
            {
                retained.base.insert(at, item.clone());
            }
        }
        if !check.diagnostics.is_empty() {
            retained
                .functions
                .insert(function, check.diagnostics.clone());
        }
        scoped.inputs.push((old, check.diagnostics));
        // The edit started clean, so only errors outside the function lists could change it.
        retained.clean = retained.complete && !blocking_error(retained);
    }
    let mut state = state_override;
    if let Some(update) = flow {
        analysis.set_regions(update.regions);
        let old = std::mem::replace(&mut retained.flow, update.flow.clone());
        scoped.inputs.push((old, update.flow));
        state.get_or_insert(update.state);
    }
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
    Some(scoped)
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
/// its target's inputs and, for exec edges, the control-flow passes.
fn classify_edges(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    edges: &[(&Edge, bool)],
) -> Option<EdgeKinds> {
    let report = analysis.report();
    let ports = super::ports::PortResolver::new(document, &report.resolved);
    let origin = registry.origin();
    let mut kinds = EdgeKinds::default();
    let local = edges.iter().all(|(edge, is_added)| {
        let published = report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.subject.edge == Some(edge.id));
        if !is_added
            && (document.node(edge.from.node).is_none() || document.node(edge.to.node).is_none())
        {
            // The edge went with a removed node, which was pure and clean.
            kinds.data = true;
            return !published;
        }
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
                super::connections::added_edge_is_clean(document, &ports, index, edge, origin)
            } else {
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
            return false;
        }
        kinds.requires |= super::availability::data_edge_requires(
            document,
            &ports,
            &report.regions,
            index,
            &edge.from,
        );
        let downstream = data_downstream(document, &ports, index, edge.to.node);
        if document
            .exports()
            .iter()
            .any(|export| downstream.contains(&export.node))
        {
            return false;
        }
        if *is_added {
            // Acyclic before, so a new cycle must lead from the target back to the source.
            !downstream.contains(&edge.from.node)
                && super::connections::added_edge_is_clean(document, &ports, index, edge, origin)
        } else {
            !published
        }
    });
    local.then_some(kinds)
}

/// New control-flow results after an exec edit.
struct FlowUpdate {
    flow: Vec<Diagnostic>,
    state: Vec<Diagnostic>,
    regions: super::regions::RegionIndex,
}

/// Reruns the region, control-cycle and availability passes for the exec-connected nodes
/// around `seeds` (V12c), and the state pass in full, with the clean connection result an
/// edge edit that passed [`classify_edges`] has. Region ownership outside that set cannot
/// change, so its region index entries and diagnostics are replaced alone. When a node
/// whose ownership changed feeds an owned consumer elsewhere through data, that
/// consumer's availability could change too, and the whole document is rerun instead.
/// `None` when the passes find an error, an incomplete node or a control cycle.
fn recompute_flow(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &Retained,
    effects: &BTreeMap<NodeId, LeafEffects>,
    seeds: &BTreeSet<NodeId>,
) -> Option<FlowUpdate> {
    let report = analysis.report();
    let ports = super::ports::PortResolver::new(document, &report.resolved);
    let connections = super::connections::ConnectionValidation::default();
    let component = exec_component(document, &ports, index, seeds);
    // Once the set is most of the body, gathering it through the index costs more than the
    // whole-document passes, which give the same result.
    if component.len() > document.root().nodes.len() / 2 {
        return whole_flow(document, registry, analysis, effects, &ports);
    }
    // Availability reads the data cones of the set's consumers. Pure and volatile nodes are
    // never owned, so the cones do not depend on the regions about to change.
    let cones =
        super::availability::subset_edges(document, &ports, &report.regions, index, &component);
    // Exec edges never leave the component, so its own outgoing edges are all the region
    // and control-cycle passes read.
    let outgoing = NodeSubset {
        nodes: &component,
        edges: {
            let mut ids = BTreeSet::new();
            for &node in &component {
                ids.extend(index.incident(node).filter(|id| {
                    document
                        .edge(*id)
                        .is_some_and(|edge| edge.from.node == node)
                }));
            }
            ids.into_iter().filter_map(|id| document.edge(id)).collect()
        },
    };
    let regions =
        super::regions::validate_regions_among(document, &ports, &connections, registry, &outgoing);
    let control = super::control_cycles::validate_control_cycles_among(
        document,
        &ports,
        &connections,
        registry,
        &outgoing,
    );
    if !regions.incomplete_nodes.is_empty()
        || !control.incomplete_nodes.is_empty()
        || !control.cycle_paths.is_empty()
    {
        return None;
    }
    let old = &report.regions;
    let mut merged = old.clone();
    merged
        .regions
        .retain(|root, _| !component.contains(&root.node));
    merged.owners.retain(|node, _| !component.contains(node));
    merged.unreachable.retain(|node| !component.contains(node));
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
    fn region_of<'i>(index: &'i super::regions::RegionIndex, node: &NodeId) -> Vec<Lineage<'i>> {
        let mut lineage = Vec::new();
        let mut current = index.owners.get(node);
        while let Some(root) = current {
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
        .filter(|node| region_of(old, node) != region_of(&merged, node))
        .copied()
        .collect();
    let crosses = moved.iter().any(|node| {
        data_downstream(document, &ports, index, *node)
            .iter()
            .any(|consumer| !component.contains(consumer) && merged.owners.contains_key(consumer))
    });
    if crosses {
        return whole_flow(document, registry, analysis, effects, &ports);
    }

    let availability = super::availability::validate_availability_among(
        document,
        &ports,
        &connections,
        &merged,
        registry,
        &NodeSubset {
            nodes: &component,
            edges: cones,
        },
    );
    let state = super::state::validate_state(
        document,
        registry,
        &report.resolved,
        &merged,
        &report.functions,
        effects,
    );
    let mut fresh = regions.diagnostics;
    fresh.extend(control.diagnostics);
    fresh.extend(availability.diagnostics);
    let local = |item: &Diagnostic| {
        item.subject
            .node
            .is_some_and(|node| component.contains(&node))
    };
    if !fresh.iter().all(local)
        || retained.flow.iter().any(|item| item.subject.node.is_none())
        || has_error(&fresh)
        || has_error(&state)
    {
        return whole_flow(document, registry, analysis, effects, &ports);
    }
    let mut flow: Vec<Diagnostic> = retained
        .flow
        .iter()
        .filter(|item| !local(item))
        .cloned()
        .collect();
    flow.extend(fresh);
    sort_diagnostics(&mut flow);
    flow.dedup();
    let mut state = state;
    sort_diagnostics(&mut state);
    state.dedup();
    Some(FlowUpdate {
        flow,
        state,
        regions: merged,
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
) -> Option<FlowUpdate> {
    let report = analysis.report();
    let connections = super::connections::ConnectionValidation::default();
    let regions =
        super::regions::validate_regions_in(document, ports, &connections, registry, Scope::All);
    let control = super::control_cycles::validate_control_cycles_in(
        document,
        ports,
        &connections,
        registry,
        Scope::All,
    );
    if !regions.incomplete_nodes.is_empty()
        || !control.incomplete_nodes.is_empty()
        || !control.cycle_paths.is_empty()
    {
        return None;
    }
    let mut flow = super::availability::validate_availability_with_ports(
        document,
        ports,
        &connections,
        &regions.index,
        registry,
    )
    .diagnostics;
    let mut state = super::state::validate_state(
        document,
        registry,
        &report.resolved,
        &regions.index,
        &report.functions,
        effects,
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
    })
}

/// The root-body nodes joined to `seeds` by exec or event edges, in either direction.
fn exec_component(
    document: &GraphDocument,
    ports: &super::ports::PortResolver<'_>,
    index: &DocIndex,
    seeds: &BTreeSet<NodeId>,
) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<NodeId> = seeds.iter().copied().collect();
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        for id in index.incident(node) {
            let Some(edge) = document.edge(id) else {
                continue;
            };
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

/// Re-resolves `touched`, their neighbours and those nodes' generic components, with every
/// neighbour of that set available, and confirms each result equals the retained one.
/// Nodes in `added` are new: each must resolve without errors, and its results are returned.
/// Returns how many nodes were resolved, how many components they span and the new nodes'
/// results.
fn resolution_unchanged(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &Retained,
    touched: &BTreeSet<NodeId>,
    added: &BTreeSet<NodeId>,
) -> Option<(usize, usize, Fresh)> {
    let near = with_neighbours(document, index, touched);
    let components = crate::inference::component::component_nodes(
        document,
        index,
        near.iter().copied(),
        registry,
    );
    let mut affected = near;
    for component in &components {
        affected.extend(component.iter().copied());
    }
    let ids: Vec<NodeId> = with_neighbours(document, index, &affected)
        .into_iter()
        .collect();
    let calls = CallEvidence {
        accepted: retained.calls.clone(),
        pipeline: true,
    };
    let nodes = validate_node_ids(document, registry, &calls, &ids, index, None);
    let fresh = by_node(&nodes.diagnostics)?;
    let report = analysis.report();
    let empty = Vec::new();
    let mut fresh_added = Fresh::new();
    for id in &affected {
        if added.contains(id) {
            // A new node has nothing to compare with: it must resolve without errors.
            let list = fresh.get(id).cloned().unwrap_or_default();
            if nodes.incomplete_nodes.contains(id) || has_error(&list) {
                return None;
            }
            fresh_added.insert(*id, (nodes.resolved.get(id)?.clone(), list));
            continue;
        }
        if nodes.resolved.get(id) != report.resolved.get(id)
            || nodes.incomplete_nodes.contains(id) != retained.node_incomplete.contains(id)
            || fresh.get(id).unwrap_or(&empty)
                != retained.node_diagnostics.get(id).unwrap_or(&empty)
        {
            return None;
        }
    }
    if added.iter().any(|id| !fresh_added.contains_key(id)) {
        return None;
    }
    Some((ids.len(), components.len(), fresh_added))
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
) -> Option<Vec<(NodeId, Vec<Diagnostic>)>> {
    let report = analysis.report();
    let empty = Vec::new();
    let mut inputs = Vec::with_capacity(nodes.len());
    for id in nodes {
        let node = document.node(*id)?;
        let mut list = match report.resolved.get(id) {
            Some(snapshot) => node_input_diagnostics(document, node, snapshot, registry, index),
            None => Vec::new(),
        };
        sort_diagnostics(&mut list);
        if strict(document, retained, *id)
            && has_error(&list) != has_error(retained.inputs.get(id).unwrap_or(&empty))
        {
            return None;
        }
        inputs.push((*id, list));
    }
    Some(inputs)
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
    }
}

/// `node` and every node it feeds through data edges.
fn data_downstream(
    document: &GraphDocument,
    ports: &super::ports::PortResolver<'_>,
    index: &DocIndex,
    node: NodeId,
) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        for id in index.incident(node) {
            let Some(edge) = document.edge(id).filter(|edge| edge.from.node == node) else {
                continue;
            };
            if matches!(ports.port(&edge.from), Ok(port) if port.kind == PortKind::Data) {
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
) -> BTreeSet<NodeId> {
    let mut out = nodes.clone();
    for &node in nodes {
        for edge in index.incident(node) {
            if let Some(edge) = document.edge(edge) {
                out.insert(edge.from.node);
                out.insert(edge.to.node);
            }
        }
    }
    out
}
