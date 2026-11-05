//! NFR-15 visit counters: a scoped edit reports the work its passes iterate, which stays
//! within its cone rather than the graph, and an edit that falls back to the full analysis
//! says why (16 §3).
#[path = "support/data_graph.rs"]
#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{Edge, PortRef};
use orange_model::validation::{
    AnalysisOptions, AnalysisSession, EditStats, PassVisits, RebuildReason, ScopeFallback,
    SessionWork, VisitCounts, analyze_with_options,
};
use orange_model::{DocCommand, GraphDocument, LayoutCommand};
use serde_json::{Value as Json, json};
use support::*;

const CHAINS: u32 = 500;
const LENGTH: u32 = 10;
/// Unrelated adds that pad the control-flow and function fixtures.
const PAD: u32 = 2000;

type Registry = std::sync::Arc<RegistrySnapshot>;

/// `CHAINS` independent chains of `LENGTH` adds, each `a` but the first wired to the add
/// before it.
fn chains() -> (GraphDocument, Registry) {
    let (package, registry) = catalog();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for chain in 0..CHAINS {
        let first = 10 + chain * LENGTH;
        nodes.push(math(first, "add", 0, 1));
        for link in 1..LENGTH {
            let n = first + link;
            nodes.push(math(n, "add", 0, 1));
            edges.push(edge(n, n - 1, "sum", n, "a"));
        }
    }
    (graph(&package, nodes, edges, vec![]), registry)
}

/// Unrelated adds numbered from 10,000.
fn padding() -> impl Iterator<Item = Json> {
    (10_000..10_000 + PAD).map(|n| math(n, "add", 0, 1))
}

/// Two On Start activations of two Sets each, every Set fed by an add, among `PAD` adds.
fn activations() -> (GraphDocument, Registry) {
    let (package, registry) = catalog();
    let variable = id(100);
    let start = |n: u32, order: u32| {
        json!({"id": id(n), "definition_id": "core/on-start", "definition_version": 1,
            "config_version": 1, "config": {}, "activation": {"mode": "queue", "capacity": 4,
            "overflow": "drop-newest", "on_fault": "continue", "order": order}})
    };
    let set = |n: u32| {
        json!({"id": id(n), "definition_id": "core/set-variable", "definition_version": 1,
            "config_version": 1, "config": {"variable": variable}})
    };
    let nodes = vec![
        start(1, 0),
        set(2),
        set(3),
        start(4, 1),
        set(5),
        set(6),
        math(10, "add", 1, 2),
        math(11, "add", 3, 4),
        math(12, "add", 5, 6),
        math(13, "add", 7, 8),
    ]
    .into_iter()
    .chain(padding())
    .collect();
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(4, 4, "then", 5, "in"),
        edge(5, 5, "then", 6, "in"),
        edge(20, 10, "sum", 2, "value"),
        edge(21, 11, "sum", 3, "value"),
        edge(22, 12, "sum", 5, "value"),
        edge(23, 13, "sum", 6, "value"),
    ];
    let base = graph(&package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!([{"id": variable, "name": "total", "type": "core/u64@1",
        "scope": "run", "init": {"type": "core/u64@1", "value": "0"}}]);
    (orange_model::from_json(&raw.to_string()).unwrap(), registry)
}

/// An On Start activation calling a five-node function, among `PAD` adds.
fn called_function() -> (GraphDocument, Registry) {
    let (package, registry) = catalog();
    let (total, local) = (id(100), id(101));
    let core = |n: u32, name: &str, config: Json| {
        json!({"id": id(n), "definition_id": format!("core/{name}"), "definition_version": 1,
            "config_version": 1, "config": config})
    };
    let port = |name: &str, direction: &str| json!({"port_id": name, "direction": direction, "kind": "data", "type": "core/u64@1"});
    let mut start = core(1, "on-start", json!({}));
    start["activation"] = json!({"mode": "queue", "capacity": 4, "overflow": "drop-newest",
        "on_fault": "continue", "order": 0});
    let nodes = vec![
        start,
        core(2, "get-variable", json!({"variable": total})),
        core(3, "call", json!({"function": id(200)})),
        core(4, "set-variable", json!({"variable": total})),
    ]
    .into_iter()
    .chain(padding())
    .collect();
    let edges = vec![
        edge(1, 1, "then", 3, "in"),
        edge(2, 2, "value", 3, "value"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 3, "result", 4, "value"),
    ];
    let base = graph(&package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    let mut add = math(204, "add", 0, 1);
    add["provider"] = json!({"package": "org.orange.math", "version": "0.0.0", "id": "native-v1"});
    raw["functions"] = json!([{"id": id(200), "name": "increment", "kind": "action",
        "inputs": [port("value", "input")], "outputs": [port("result", "output")],
        "nodes": [core(201, "function-entry", json!({})),
            core(202, "set-variable", json!({"variable": local})),
            core(203, "get-variable", json!({"variable": local})),
            add, core(205, "return", json!({}))],
        "edges": [edge(201, 201, "then", 202, "in"), edge(202, 201, "value", 202, "value"),
            edge(203, 202, "then", 205, "in"), edge(204, 203, "value", 204, "a"),
            edge(205, 204, "sum", 205, "result")]}]);
    raw["variables"] = json!([
        {"id": total, "name": "total", "type": "core/u64@1", "scope": "run",
            "init": {"type": "core/u64@1", "value": "0"}},
        {"id": local, "name": "local", "type": "core/u64@1", "scope": "local",
            "init": {"type": "core/u64@1", "value": "0"}},
    ]);
    (orange_model::from_json(&raw.to_string()).unwrap(), registry)
}

fn set_b(node: u32, literal: Json) -> DocCommand {
    DocCommand::SetLiteral {
        port: PortRef {
            node: id(node).parse().unwrap(),
            port: "b".parse().unwrap(),
        },
        literal: Some(serde_json::from_value(literal).unwrap()),
    }
}

fn wire(n: u32, from: u32, output: &str, to: u32, input: &str) -> DocCommand {
    DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge(n, from, output, to, input)).unwrap(),
    }
}

fn unwire(n: u32) -> DocCommand {
    DocCommand::Disconnect {
        edge: serde_json::from_value::<Edge>(edge(n, 0, "sum", 0, "a"))
            .unwrap()
            .id,
    }
}

fn session(doc: &GraphDocument, registry: &RegistrySnapshot) -> AnalysisSession {
    let session = AnalysisSession::new(doc, registry, BTreeMap::new(), AnalysisOptions::default());
    assert!(session.analysis().report().is_valid());
    session
}

/// Applies `commands` and updates the session, which must stay in place and equal the full
/// analysis; returns the update's stats.
fn scoped(
    session: &mut AnalysisSession,
    doc: &mut GraphDocument,
    registry: &RegistrySnapshot,
    commands: Vec<DocCommand>,
) -> EditStats {
    let options = AnalysisOptions::default();
    let committed = doc.apply(commands).unwrap();
    let stats = *session.update(doc, registry, &committed.change_set, options);
    assert!(stats.is_scoped(), "{stats:?}");
    assert_eq!(stats.fallback, None);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(doc, registry, BTreeMap::new(), options)
    );
    eprintln!("{:#?}", stats.visits);
    stats
}

/// Every pass but those listed visited nothing.
fn only(visits: &PassVisits, passes: &[&str]) {
    let all = [
        ("resolution", visits.resolution),
        ("inputs", visits.inputs),
        ("config", visits.config),
        ("connections", visits.connections),
        ("order", visits.order),
        ("availability", visits.availability),
        ("control_flow", visits.control_flow),
        ("state", visits.state),
        ("zones", visits.zones),
        ("functions", visits.functions),
    ];
    for (name, pass) in all {
        assert_eq!(
            passes.contains(&name),
            !pass.is_zero(),
            "{name}: {visits:#?}"
        );
    }
}

/// A pass's counts as [nodes, edges, ports, diagnostics, bodies].
fn counts(visits: VisitCounts) -> [usize; 5] {
    [
        visits.nodes,
        visits.edges,
        visits.ports,
        visits.diagnostics,
        visits.bodies,
    ]
}

/// Every body, node and edge of `doc`.
fn whole(doc: &GraphDocument) -> (usize, usize, usize) {
    let mut sizes = (0, 0, 0);
    for body in doc.bodies() {
        sizes.0 += body.nodes.len();
        sizes.1 += body.edges.len();
        sizes.2 += 1;
    }
    sizes
}

#[test]
fn a_scoped_literal_edit_visits_its_cone_and_a_fallback_says_why() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let total = (CHAINS * LENGTH) as usize;

    // Creation charges every pass the whole document.
    let created = *session.stats();
    assert_eq!(created.work, SessionWork::Rebuild(RebuildReason::Created));
    assert_eq!(created.fallback, None);
    assert_eq!(created.visits.resolution.nodes, total);
    assert_eq!(created.visits.resolution.bodies, 1);

    // A literal in the middle of one chain re-checks only around that add.
    let middle = 10 + (CHAINS / 2) * LENGTH + LENGTH / 2;
    let stats = scoped(
        &mut session,
        &mut doc,
        &registry,
        vec![set_b(middle, literal(7))],
    );
    only(&stats.visits, &["resolution", "inputs"]);
    let visits = stats.visits.total();
    assert_eq!(visits.bodies, 2, "resolution and inputs, in the root");
    assert!(visits.nodes <= 2 * LENGTH as usize, "{visits:?}");
    assert!(visits.edges <= 2 * LENGTH as usize, "{visits:?}");
    // Exact, so that a counter that stops counting fails here. Resolution: the edit's
    // equation check and its literal ports, the kept-solution search, the node pass over
    // the add and its two neighbours, and the comparison; inputs: the add's three ports
    // and its wired `a`.
    assert_eq!(counts(stats.visits.resolution), [10, 8, 14, 0, 1]);
    assert_eq!(counts(stats.visits.inputs), [1, 1, 3, 0, 1]);

    // An i64 literal conflicts with the chain's u64 values: the inference result changes,
    // which only the full analysis publishes.
    let committed = doc
        .apply([set_b(middle, json!({"type": "core/i64@1", "value": "1"}))])
        .unwrap();
    let stats = *session.update(&doc, &registry, &committed.change_set, options);
    assert_eq!(stats.work, SessionWork::Recompute);
    assert_eq!(stats.fallback, Some(ScopeFallback::Resolution));
    assert_eq!(stats.visits.resolution.nodes, total);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );

    // A layout-only edit visits nothing.
    let moved = doc
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["nodes".into(), id(middle)],
            value: Some(json!({"x": 40})),
        })])
        .unwrap();
    let stats = *session.update(&doc, &registry, &moved.change_set, options);
    assert_eq!(stats.work, SessionWork::None);
    assert_eq!(stats.visits, PassVisits::ZERO);
    assert_eq!(stats.fallback, None);
}

#[test]
fn a_data_edge_edit_visits_its_cone() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let middle = 10 + (CHAINS / 2) * LENGTH + LENGTH / 2;
    // Exact counts per pass, [nodes, edges, ports, diagnostics, bodies], for the removal
    // and then the restoration of one chain edge. Restoring searches the data order once
    // and runs the added edge's connection check and fan scans; removing walks the
    // requirement cone behind the edge's producer.
    let expected = [
        [
            [20, 9, 13, 0, 1],
            [1, 0, 3, 0, 1],
            [0, 1, 2, 4500, 1],
            [0, 0, 0, 0, 1],
            [5, 8, 9, 0, 1],
            [2, 0, 0, 0, 0],
        ],
        [
            [20, 17, 13, 0, 1],
            [1, 1, 3, 0, 1],
            [0, 4, 5, 4499, 1],
            [0, 1, 0, 0, 1],
            [0, 0, 1, 0, 1],
            [2, 0, 0, 0, 0],
        ],
    ];
    for (commands, expected) in [
        vec![unwire(middle)],
        vec![wire(middle, middle - 1, "sum", middle, "a")],
    ]
    .into_iter()
    .zip(expected)
    {
        let published = session.diagnostics().len();
        let stats = scoped(&mut session, &mut doc, &registry, commands);
        // The zone re-check looks up the endpoints' owners and finds no zone.
        only(
            &stats.visits,
            &[
                "resolution",
                "inputs",
                "connections",
                "order",
                "availability",
                "zones",
            ],
        );
        assert_eq!(stats.visits.zones.bodies, 0);
        // Whether the edge carries a diagnostic is a scan of every published diagnostic:
        // here each wired `a` shadows its literal with a warning.
        assert_eq!(stats.visits.connections.diagnostics, published);
        let visits: VisitCounts = stats.visits.total();
        assert!(visits.nodes <= 3 * LENGTH as usize, "{visits:?}");
        assert!(visits.edges <= 4 * LENGTH as usize, "{visits:?}");
        let v = stats.visits;
        let actual = [
            v.resolution,
            v.inputs,
            v.connections,
            v.order,
            v.availability,
            v.zones,
        ]
        .map(counts);
        assert_eq!(actual, expected, "{v:#?}");
    }
}

#[test]
fn an_added_node_is_configured_and_resolved_alone() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let added = DocCommand::AddNode {
        body: orange_model::BodyRef::Root,
        node: serde_json::from_value(math(90_001, "add", 1, 2)).unwrap(),
    };
    let stats = scoped(&mut session, &mut doc, &registry, vec![added]);
    // One configuration check of the new add, in the root.
    assert_eq!(counts(stats.visits.config), [1, 0, 0, 0, 1]);
    assert!(stats.visits.resolution.nodes >= 1 && stats.visits.resolution.nodes <= 10);
    assert!(stats.visits.total().nodes <= 20, "{:?}", stats.visits);
}

#[test]
fn closing_and_opening_a_cycle_visits_the_cycle() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let first = 10 + (CHAINS / 2) * LENGTH;
    let last = first + LENGTH - 1;
    // Closing the cycle searches the order between its endpoints, then gathers and
    // diagnoses the cycle's region: the chain's adds.
    let stats = scoped(
        &mut session,
        &mut doc,
        &registry,
        vec![wire(90_000, last, "sum", first, "a")],
    );
    // The data-cycle search runs over the region's port vertices, a few per add.
    let visits = stats.visits.total();
    assert!(visits.nodes <= 20 * LENGTH as usize, "{visits:?}");
    assert!(visits.edges <= 40 * LENGTH as usize, "{visits:?}");
    // The order search and the cycle region each reach every add of the chain.
    assert!(
        stats.visits.order.nodes >= 2 * LENGTH as usize,
        "{:?}",
        stats.visits
    );
    assert!(stats.visits.order.edges >= 2 * LENGTH as usize);
    // Removing the held edge releases it: the order is acyclic again without a search.
    let stats = scoped(&mut session, &mut doc, &registry, vec![unwire(90_000)]);
    let visits = stats.visits.total();
    assert!(visits.nodes <= 3 * LENGTH as usize, "{visits:?}");
    assert_eq!(stats.visits.order.nodes, 0);
}

#[test]
fn an_exec_edge_edit_visits_its_activation_and_the_state_pass_reruns_in_full() {
    let (mut doc, registry) = activations();
    let mut session = session(&doc, &registry);
    let (nodes, _, bodies) = whole(&doc);
    // The control-flow passes, exactly: the exec walk, the region and control-cycle passes
    // over the activation and the merge with the region index.
    let expected = [[35, 18, 12, 0, 3], [45, 33, 20, 1, 3]];
    for (commands, expected) in [vec![unwire(5)], vec![wire(5, 5, "then", 6, "in")]]
        .into_iter()
        .zip(expected)
    {
        let stats = scoped(&mut session, &mut doc, &registry, commands);
        // The state pass iterates the two activation regions, every body, and each body's
        // nodes twice: once for registers and once for the forks and parallel entries.
        let state = stats.visits.state;
        assert_eq!(counts(state), [2 * nodes + 2, 0, 0, 0, bodies]);
        assert_eq!(counts(stats.visits.control_flow), expected);
        // Everything else stays within the activation and the region index.
        let mut rest = stats.visits;
        rest.state = VisitCounts::ZERO;
        let rest = rest.total();
        assert!(rest.nodes <= 70, "{rest:?}");
        assert!(rest.edges <= 60, "{rest:?}");
    }
}

#[test]
fn a_function_body_edit_visits_that_body_and_the_call_sites() {
    let (mut doc, registry) = called_function();
    let mut session = session(&doc, &registry);
    for commands in [vec![unwire(204)], vec![wire(204, 203, "value", 204, "a")]] {
        let stats = scoped(&mut session, &mut doc, &registry, commands);
        // The function check reruns the body passes over its five nodes and edges: node
        // resolution's preflight and projection, data cycles, the region pass's three
        // loops, inputs and the summary each iterate the nodes, and the connection pass
        // twice, data cycles, regions and control cycles the edges. Looking for its call
        // sites scans the root only up to the call, not its 2,000 padding adds.
        let functions = stats.visits.functions;
        assert!(functions.nodes >= 8 * 5, "{functions:?}");
        assert!(functions.edges >= 5 * 5, "{functions:?}");
        assert!(functions.bodies >= 3, "{functions:?}");
        assert!(functions.nodes <= 200, "{functions:?}");
        let mut rest = stats.visits;
        rest.functions = VisitCounts::ZERO;
        let rest = rest.total();
        assert!(rest.nodes <= 40, "{rest:?}");
        assert!(rest.edges <= 40, "{rest:?}");
    }
}

#[test]
fn an_edit_the_journal_does_not_scope_reports_it() {
    let (mut doc, registry) = chains();
    let options = AnalysisOptions::default();
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options);
    // A literal edit together with a new variable declaration is neither a node-field
    // edit nor a structure edit.
    let variable = serde_json::from_value(json!({
        "id": "40000000-0000-4000-8000-000000000001", "name": "total", "type": "core/u64@1",
        "scope": "run"
    }))
    .unwrap();
    let committed = doc
        .apply([set_b(10, literal(3)), DocCommand::UpsertVariable(variable)])
        .unwrap();
    let stats = *session.update(&doc, &registry, &committed.change_set, options);
    assert_eq!(stats.work, SessionWork::Recompute);
    assert_eq!(stats.fallback, Some(ScopeFallback::UnscopedChange));
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
}
