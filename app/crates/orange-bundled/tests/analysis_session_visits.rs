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
    SessionWork, analyze_with_options,
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

/// Every pass's counts, in [`PassVisits`] field order (resolution, inputs, config,
/// connections, order, availability, control flow, state, zones, functions), each as
/// [nodes, edges, ports, diagnostics, bodies]. Tests compare them exactly, so that a
/// counter that stops counting, or starts counting elsewhere, fails.
type Matrix = [[usize; 5]; 10];

fn matrix(visits: &PassVisits) -> Matrix {
    [
        visits.resolution,
        visits.inputs,
        visits.config,
        visits.connections,
        visits.order,
        visits.availability,
        visits.control_flow,
        visits.state,
        visits.zones,
        visits.functions,
    ]
    .map(|pass| {
        [
            pass.nodes,
            pass.edges,
            pass.ports,
            pass.diagnostics,
            pass.bodies,
        ]
    })
}

/// Applies each edit in turn, checks it stays in place, and compares its counts exactly.
fn expect_counts(
    session: &mut AnalysisSession,
    doc: &mut GraphDocument,
    registry: &RegistrySnapshot,
    edits: Vec<(Vec<DocCommand>, Matrix)>,
) {
    for (commands, expected) in edits {
        let stats = scoped(session, doc, registry, commands);
        assert_eq!(matrix(&stats.visits), expected, "{:#?}", stats.visits);
    }
}

const NONE: [usize; 5] = [0; 5];

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

    // A literal in the middle of one chain re-checks only around that add. Resolution: the
    // edit's equation check and its literal ports, the kept-solution search, the node pass
    // over the add and its two neighbours, and the comparison; inputs: the add's three
    // ports and its wired `a`.
    let middle = 10 + (CHAINS / 2) * LENGTH + LENGTH / 2;
    let literal_edit = [
        [10, 8, 14, 0, 1],
        [1, 1, 3, 0, 1],
        NONE,
        NONE,
        NONE,
        NONE,
        NONE,
        NONE,
        NONE,
        NONE,
    ];
    expect_counts(
        &mut session,
        &mut doc,
        &registry,
        vec![(vec![set_b(middle, literal(7))], literal_edit)],
    );

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
    // Removing one chain edge walks the requirement cone behind its producer; restoring it
    // searches the data order once and runs the added edge's connection check and fan
    // scans. Each scans the published diagnostics for one on the edge: every wired `a`
    // shadows its literal with a warning, so there are 4,500, then 4,499. The zone
    // re-check looks up both endpoints' owners and finds no zone.
    expect_counts(
        &mut session,
        &mut doc,
        &registry,
        vec![
            (
                vec![unwire(middle)],
                [
                    [20, 9, 13, 0, 1],
                    [1, 0, 3, 0, 1],
                    NONE,
                    [0, 1, 2, 4500, 1],
                    [0, 0, 0, 0, 1],
                    [5, 8, 9, 0, 1],
                    NONE,
                    NONE,
                    [2, 0, 0, 0, 0],
                    NONE,
                ],
            ),
            (
                vec![wire(middle, middle - 1, "sum", middle, "a")],
                [
                    [20, 17, 13, 0, 1],
                    [1, 1, 3, 0, 1],
                    NONE,
                    [0, 4, 5, 4499, 1],
                    [0, 1, 0, 0, 1],
                    [0, 0, 1, 0, 1],
                    NONE,
                    NONE,
                    [2, 0, 0, 0, 0],
                    NONE,
                ],
            ),
        ],
    );
}

#[test]
fn an_added_node_is_configured_and_resolved_alone() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let added = DocCommand::AddNode {
        body: orange_model::BodyRef::Root,
        node: serde_json::from_value(math(90_001, "add", 1, 2)).unwrap(),
    };
    // The new add alone is resolved, its inputs and configuration checked once each, and
    // its zone owner looked up.
    expect_counts(
        &mut session,
        &mut doc,
        &registry,
        vec![(
            vec![added],
            [
                [7, 0, 5, 0, 1],
                [1, 0, 3, 0, 1],
                [1, 0, 0, 0, 1],
                NONE,
                NONE,
                NONE,
                NONE,
                NONE,
                [1, 0, 0, 0, 0],
                NONE,
            ],
        )],
    );
}

#[test]
fn closing_and_opening_a_cycle_visits_the_cycle() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let first = 10 + (CHAINS / 2) * LENGTH;
    let last = first + LENGTH - 1;
    // Closing the cycle searches the order between its endpoints, holds the edge, then
    // gathers the cycle's region, the chain's ten adds, and searches its port graph for the
    // cycle. Removing the held edge releases it, so the order is acyclic again without a
    // search.
    expect_counts(
        &mut session,
        &mut doc,
        &registry,
        vec![
            (
                vec![wire(90_000, last, "sum", first, "a")],
                [
                    [20, 17, 13, 0, 1],
                    [1, 1, 3, 0, 1],
                    NONE,
                    [0, 4, 5, 4500, 1],
                    [118, 247, 68, 0, 2],
                    [10, 20, 21, 0, 1],
                    NONE,
                    NONE,
                    [2, 0, 0, 0, 0],
                    NONE,
                ],
            ),
            (
                vec![unwire(90_000)],
                [
                    [20, 9, 13, 0, 1],
                    [1, 0, 3, 0, 1],
                    NONE,
                    [0, 1, 2, 4502, 1],
                    [0, 0, 0, 0, 2],
                    [0, 0, 1, 0, 1],
                    NONE,
                    NONE,
                    [2, 0, 0, 0, 0],
                    NONE,
                ],
            ),
        ],
    );
}

#[test]
fn an_exec_edge_edit_visits_its_activation_and_the_state_pass_reruns_in_full() {
    let (mut doc, registry) = activations();
    let mut session = session(&doc, &registry);
    let (nodes, _, bodies) = whole(&doc);
    // The state pass iterates the two activation regions, every body, and each body's
    // nodes twice: once for registers and once for the forks and parallel entries.
    let state = [2 * nodes + 2, 0, 0, 0, bodies];
    // The control-flow passes run over the activation: the exec walk, the region and
    // control-cycle passes and the merge with the region index.
    expect_counts(
        &mut session,
        &mut doc,
        &registry,
        vec![
            (
                vec![unwire(5)],
                [
                    [8, 0, 4, 0, 1],
                    [1, 1, 1, 0, 1],
                    NONE,
                    [0, 1, 2, 0, 1],
                    NONE,
                    [2, 3, 5, 0, 0],
                    [35, 19, 12, 0, 3],
                    state,
                    [2, 0, 0, 0, 0],
                    NONE,
                ],
            ),
            (
                vec![wire(5, 5, "then", 6, "in")],
                [
                    [8, 0, 4, 0, 1],
                    [1, 1, 1, 0, 1],
                    NONE,
                    [0, 3, 5, 1, 1],
                    NONE,
                    [3, 7, 10, 0, 0],
                    [45, 34, 20, 1, 3],
                    state,
                    [2, 0, 0, 0, 0],
                    NONE,
                ],
            ),
        ],
    );
}

#[test]
fn a_function_body_edit_visits_that_body_and_the_call_sites() {
    let (mut doc, registry) = called_function();
    let mut session = session(&doc, &registry);
    // The function check reruns the body passes over the function's five nodes and edges,
    // its call graph and its summary; looking for its call sites scans the root only up to
    // the call, not its 2,000 padding adds.
    expect_counts(
        &mut session,
        &mut doc,
        &registry,
        vec![
            (
                vec![unwire(204)],
                [
                    [14, 4, 12, 0, 1],
                    [1, 0, 3, 0, 1],
                    NONE,
                    [0, 1, 2, 1, 1],
                    [10, 6, 4, 0, 2],
                    [0, 0, 1, 0, 1],
                    NONE,
                    NONE,
                    [2, 0, 0, 0, 0],
                    [94, 68, 50, 0, 8],
                ],
            ),
            (
                vec![wire(204, 203, "value", 204, "a")],
                [
                    [14, 9, 12, 0, 1],
                    [1, 1, 3, 0, 1],
                    NONE,
                    [0, 4, 5, 0, 1],
                    [3, 6, 1, 0, 1],
                    [0, 0, 1, 0, 1],
                    NONE,
                    NONE,
                    [2, 0, 0, 0, 0],
                    [94, 83, 58, 0, 8],
                ],
            ),
        ],
    );
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
