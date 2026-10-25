use std::{collections::BTreeMap, sync::Arc};

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{
    GraphDocument, NodeKind, OutputSource, PortDescriptor, SignatureSnapshot,
};
use orange_model::from_json;
use orange_model::id::{NodeId, PortId};
use orange_model::validation::{
    ConnectionValidation, DiagnosticCode, HostOrigin, validate_availability, validate_regions,
};
use serde_json::{Value, json};

const SIGNATURE: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn id(n: u8) -> String {
    format!("10000000-0000-4000-8000-{n:012}")
}

fn nid(n: u8) -> NodeId {
    id(n).parse().unwrap()
}

fn pid(text: &str) -> PortId {
    text.parse().unwrap()
}

fn registry() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap())
}

struct N {
    json: Value,
    id: NodeId,
    signature: SignatureSnapshot,
}

fn node(n: u8, definition: &str, kind: NodeKind, ports: Vec<PortDescriptor>) -> N {
    let mut json = json!({
        "id": id(n), "definition_id": definition, "definition_version": 1,
        "config_version": 1, "config": {}
    });
    if !definition.starts_with("core/") {
        json["signature"] = json!(SIGNATURE);
    }
    N {
        json,
        id: nid(n),
        signature: SignatureSnapshot {
            kind,
            suspends: None,
            ports,
            extensions: Default::default(),
        },
    }
}

fn int() -> orange_model::id::TypeExpr {
    "core/i64@1".parse().unwrap()
}

fn entry(n: u8) -> N {
    node(
        n,
        "example/entry",
        NodeKind::Entry,
        vec![
            PortDescriptor::exec_out("then"),
            PortDescriptor::data_out("payload", int(), OutputSource::Payload),
        ],
    )
}

/// Action producing `out` (available on `then` by default) and consuming `v`.
fn action(n: u8) -> N {
    node(
        n,
        "example/action",
        NodeKind::Action,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("then"),
            PortDescriptor::data_in("v", int()),
            PortDescriptor::data_out("out", int(), OutputSource::Execution),
        ],
    )
}

fn pure(n: u8) -> N {
    node(
        n,
        "example/pure",
        NodeKind::Pure,
        vec![
            PortDescriptor::data_in("v", int()),
            PortDescriptor::data_out("out", int(), OutputSource::Computed),
        ],
    )
}

/// Async node with `then`/`failed`; `out` is available on `then`, `err` on `failed`.
fn asynchronous(n: u8) -> N {
    let mut error = PortDescriptor::data_out("err", int(), OutputSource::Execution);
    error.available_on = Some(vec![pid("failed")]);
    node(
        n,
        "example/async",
        NodeKind::Async,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("then"),
            PortDescriptor::exec_out("failed"),
            PortDescriptor::data_in("v", int()),
            PortDescriptor::data_out("out", int(), OutputSource::Execution),
            error,
        ],
    )
}

fn branch(n: u8) -> N {
    node(
        n,
        "core/branch",
        NodeKind::Action,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("true"),
            PortDescriptor::exec_out("false"),
        ],
    )
}

fn looped(n: u8) -> N {
    node(
        n,
        "core/count",
        NodeKind::Iterator,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("body"),
            PortDescriptor::exec_out("completed"),
            PortDescriptor::data_out("index", int(), OutputSource::Iteration),
        ],
    )
}

fn fork(n: u8, join: &str) -> N {
    let mut fork = node(
        n,
        "core/fork",
        NodeKind::Action,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("completed"),
            PortDescriptor::exec_out("branch").variadic(1, None),
        ],
    );
    fork.json["variadic"] = json!({"branch":{"order":[0,1],"next":2}});
    fork.json["config"] = json!({"join": join});
    fork
}

fn sequence(n: u8) -> N {
    let mut node = node(
        n,
        "core/sequence",
        NodeKind::Action,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("then").variadic(2, None),
        ],
    );
    node.json["variadic"] = json!({"then":{"order":[0,1],"next":2}});
    node
}

fn exec(n: u8, from: u8, from_port: &str, to: u8) -> Value {
    json!({
        "id": format!("30000000-0000-4000-8000-{n:012}"),
        "from": {"node": id(from), "port": from_port},
        "to": {"node": id(to), "port": "in"}
    })
}

fn data(n: u8, from: u8, from_port: &str, to: u8) -> Value {
    json!({
        "id": format!("30000000-0000-4000-8000-{n:012}"),
        "from": {"node": id(from), "port": from_port},
        "to": {"node": id(to), "port": "v"}
    })
}

fn codes(nodes: Vec<N>, edges: Vec<Value>) -> Vec<DiagnosticCode> {
    let doc: GraphDocument = from_json(
        &json!({
            "format":"orange.graph","format_version":1,
            "graph_id":"40000000-0000-4000-8000-000000000001","revision":0,
            "nodes": nodes.iter().map(|n| n.json.clone()).collect::<Vec<_>>(),
            "edges": edges,
            "signatures":{SIGNATURE:{"kind":"pure","ports":[]}}
        })
        .to_string(),
    )
    .unwrap();
    let resolved: BTreeMap<_, _> = nodes.into_iter().map(|n| (n.id, n.signature)).collect();
    let registry = registry();
    let connections = ConnectionValidation::default();
    let regions = validate_regions(&doc, &resolved, &connections, &registry);
    assert!(regions.is_valid(), "{:?}", regions.diagnostics);
    validate_availability(&doc, &resolved, &connections, &regions.index, &registry)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

const OK: [DiagnosticCode; 0] = [];
const UNAVAILABLE: [DiagnosticCode; 1] = [DiagnosticCode::UnavailableValue];
const CROSS: [DiagnosticCode; 1] = [DiagnosticCode::CrossActivation];

#[test]
fn straight_line_and_sequence_steps_dominate() {
    let result = codes(
        vec![entry(1), action(2), action(3)],
        vec![
            exec(1, 1, "then", 2),
            exec(2, 2, "then", 3),
            data(3, 2, "out", 3),
            data(4, 1, "payload", 2),
        ],
    );
    assert_eq!(result, OK);
    let result = codes(
        vec![entry(1), sequence(2), action(3), action(4)],
        vec![
            exec(1, 1, "then", 2),
            exec(2, 2, "then#0", 3),
            exec(3, 2, "then#1", 4),
            data(4, 3, "out", 4),
        ],
    );
    assert_eq!(result, OK);
    // Steps run in order, so the later step's value is not there for the earlier one.
    let result = codes(
        vec![entry(1), sequence(2), action(3), action(4)],
        vec![
            exec(1, 1, "then", 2),
            exec(2, 2, "then#0", 3),
            exec(3, 2, "then#1", 4),
            data(4, 4, "out", 3),
        ],
    );
    assert_eq!(result, UNAVAILABLE);
}

#[test]
fn consumer_before_the_producer_is_unavailable() {
    let result = codes(
        vec![entry(1), action(2), action(3)],
        vec![
            exec(1, 1, "then", 2),
            exec(2, 2, "then", 3),
            data(3, 3, "out", 2),
        ],
    );
    assert_eq!(result, UNAVAILABLE);
}

#[test]
fn branch_value_read_after_the_merge_is_unavailable() {
    let merged = vec![
        exec(1, 1, "then", 2),
        exec(2, 2, "true", 3),
        exec(3, 3, "then", 4),
        exec(4, 2, "false", 4),
        data(5, 3, "out", 4),
    ];
    assert_eq!(
        codes(vec![entry(1), branch(2), action(3), action(4)], merged),
        UNAVAILABLE
    );
    // Without the false-path merge the read is dominated.
    let inside = vec![
        exec(1, 1, "then", 2),
        exec(2, 2, "true", 3),
        exec(3, 3, "then", 4),
        data(5, 3, "out", 4),
    ];
    assert_eq!(
        codes(vec![entry(1), branch(2), action(3), action(4)], inside),
        OK
    );
}

#[test]
fn zero_iteration_loops_hide_body_and_iteration_values_after_completed() {
    let nodes = || vec![entry(1), looped(2), action(3), action(4), action(5)];
    let base = || {
        vec![
            exec(1, 1, "then", 2),
            exec(2, 2, "body", 3),
            exec(3, 2, "completed", 4),
            exec(4, 4, "then", 5),
        ]
    };
    let mut edges = base();
    edges.push(data(5, 2, "index", 3));
    assert_eq!(codes(nodes(), edges), OK);
    let mut edges = base();
    edges.push(data(5, 2, "index", 4));
    assert_eq!(codes(nodes(), edges), UNAVAILABLE);
    let mut edges = base();
    edges.push(data(5, 3, "out", 4));
    assert_eq!(codes(nodes(), edges), UNAVAILABLE);
    // A value produced before the loop dominates the body.
    let mut edges = base();
    edges.push(data(5, 1, "payload", 3));
    assert_eq!(codes(nodes(), edges), OK);
}

#[test]
fn loop_body_value_is_unavailable_to_an_earlier_body_node_next_iteration() {
    let result = codes(
        vec![entry(1), looped(2), action(3), action(4)],
        vec![
            exec(1, 1, "then", 2),
            exec(2, 2, "body", 3),
            exec(3, 3, "then", 4),
            data(4, 4, "out", 3),
        ],
    );
    assert_eq!(result, UNAVAILABLE);
}

#[test]
fn fork_values_need_join_all_and_never_cross_sibling_branches() {
    let nodes = |join: &str| vec![entry(1), fork(2, join), action(3), action(4), action(5)];
    let edges = |extra: Value| {
        vec![
            exec(1, 1, "then", 2),
            exec(2, 2, "branch#0", 3),
            exec(3, 2, "branch#1", 4),
            exec(4, 2, "completed", 5),
            extra,
        ]
    };
    assert_eq!(codes(nodes("all"), edges(data(5, 3, "out", 5))), OK);
    for join in ["any", "race", "detach", "unknown"] {
        assert_eq!(
            codes(nodes(join), edges(data(5, 3, "out", 5))),
            UNAVAILABLE,
            "{join}"
        );
    }
    assert_eq!(
        codes(nodes("all"), edges(data(5, 3, "out", 4))),
        UNAVAILABLE
    );
}

#[test]
fn async_failure_path_cannot_read_then_outputs_but_can_read_error() {
    let nodes = || vec![entry(1), asynchronous(2), action(3), action(4)];
    let wires = |extra: Value| {
        vec![
            exec(1, 1, "then", 2),
            exec(2, 2, "then", 3),
            exec(3, 2, "failed", 4),
            extra,
        ]
    };
    assert_eq!(codes(nodes(), wires(data(4, 2, "out", 3))), OK);
    assert_eq!(codes(nodes(), wires(data(4, 2, "out", 4))), UNAVAILABLE);
    assert_eq!(codes(nodes(), wires(data(4, 2, "err", 4))), OK);
    assert_eq!(codes(nodes(), wires(data(4, 2, "err", 3))), UNAVAILABLE);
}

#[test]
fn pure_cones_inherit_the_exec_consumers_requirement() {
    let nodes = || vec![entry(1), branch(2), action(3), action(4), pure(5)];
    let control = || {
        vec![
            exec(1, 1, "then", 2),
            exec(2, 2, "true", 3),
            exec(3, 3, "then", 4),
            exec(4, 2, "false", 4),
            data(5, 3, "out", 5),
        ]
    };
    let mut edges = control();
    edges.push(data(6, 5, "out", 4));
    assert_eq!(codes(nodes(), edges), UNAVAILABLE);
    // A pure node nobody executes reads nothing.
    assert_eq!(codes(nodes(), control()), OK);
}

#[test]
fn values_do_not_cross_activations() {
    let result = codes(
        vec![entry(1), entry(2), action(3), action(4)],
        vec![
            exec(1, 1, "then", 3),
            exec(2, 2, "then", 4),
            data(3, 3, "out", 4),
        ],
    );
    assert_eq!(result, CROSS);
    let result = codes(
        vec![entry(1), entry(2), action(3), action(4)],
        vec![
            exec(1, 1, "then", 3),
            exec(2, 2, "then", 4),
            data(3, 1, "payload", 4),
        ],
    );
    assert_eq!(result, CROSS);
}
