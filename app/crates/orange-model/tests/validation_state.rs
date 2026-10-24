use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{
    GraphDocument, NodeKind, PortDescriptor, SignatureSnapshot, Suspends,
};
use orange_model::from_json;
use orange_model::id::{FunctionId, NodeId};
use orange_model::validation::{
    ConnectionValidation, DiagnosticCode, FunctionSummary, HostOrigin, WriteOperation,
    validate_regions, validate_state,
};
use serde_json::{Value, json};

const SIGNATURE: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const VAR: &str = "50000000-0000-4000-8000-000000000001";
const FUNCTION: &str = "60000000-0000-4000-8000-000000000001";

fn id(n: u8) -> String {
    format!("10000000-0000-4000-8000-{n:012}")
}

fn registry() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap())
}

struct N {
    json: Value,
    id: NodeId,
    signature: SignatureSnapshot,
}

fn node(n: u8, definition: &str, config: Value, ports: Vec<PortDescriptor>) -> N {
    let mut json = json!({
        "id": id(n), "definition_id": definition, "definition_version": 1,
        "config_version": 1, "config": config
    });
    let kind = if definition == "example/entry" {
        NodeKind::Entry
    } else {
        NodeKind::Action
    };
    if !definition.starts_with("core/") {
        json["signature"] = json!(SIGNATURE);
    }
    N {
        json,
        id: id(n).parse().unwrap(),
        signature: SignatureSnapshot {
            kind,
            suspends: Some(Suspends::Never),
            ports,
            extensions: Default::default(),
        },
    }
}

fn entry(n: u8) -> N {
    node(
        n,
        "example/entry",
        json!({}),
        vec![PortDescriptor::exec_out("then")],
    )
}

fn exec(n: u8, definition: &str, config: Value) -> N {
    node(
        n,
        definition,
        config,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("then"),
        ],
    )
}

fn set(n: u8) -> N {
    exec(n, "core/set-variable", json!({"variable": VAR}))
}

fn update(n: u8, operation: &str) -> N {
    exec(
        n,
        "core/update-variable",
        json!({"variable": VAR, "operation": operation}),
    )
}

fn call(n: u8) -> N {
    exec(n, "core/call", json!({"function": FUNCTION}))
}

fn multi(n: u8, definition: &str, port: &str) -> N {
    let mut node = node(
        n,
        definition,
        json!({}),
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out(port).variadic(1, None),
        ],
    );
    node.json["variadic"] = json!({port: {"order": [0, 1], "next": 2}});
    node
}

fn fork(n: u8) -> N {
    multi(n, "core/fork", "branch")
}

fn wire(n: u8, from: u8, from_port: &str, to: u8) -> Value {
    json!({
        "id": format!("30000000-0000-4000-8000-{n:012}"),
        "from": {"node": id(from), "port": from_port},
        "to": {"node": id(to), "port": "in"}
    })
}

fn conflicts(
    ty: &str,
    policy: &str,
    nodes: Vec<N>,
    edges: Vec<Value>,
    calls: &[WriteOperation],
) -> Vec<DiagnosticCode> {
    let doc: GraphDocument = from_json(
        &json!({
            "format":"orange.graph","format_version":1,
            "graph_id":"40000000-0000-4000-8000-000000000001","revision":0,
            "variables":[{"id":VAR,"name":"v","type":ty,"scope":"run","conflict":policy}],
            "nodes": nodes.iter().map(|n| n.json.clone()).collect::<Vec<_>>(),
            "edges": edges,
            "signatures":{SIGNATURE:{"kind":"pure","ports":[]}}
        })
        .to_string(),
    )
    .unwrap();
    let resolved: BTreeMap<_, _> = nodes.into_iter().map(|n| (n.id, n.signature)).collect();
    let registry = registry();
    let regions = validate_regions(&doc, &resolved, &ConnectionValidation::default(), &registry);
    let mut summaries = BTreeMap::new();
    if !calls.is_empty() {
        summaries.insert(
            FUNCTION.parse::<FunctionId>().unwrap(),
            FunctionSummary {
                kind: NodeKind::Action,
                suspends: Suspends::Never,
                run_writes: BTreeMap::from([(
                    VAR.parse().unwrap(),
                    calls.iter().copied().collect::<BTreeSet<_>>(),
                )]),
            },
        );
    }
    validate_state(
        &doc,
        &registry,
        &regions.index,
        &summaries,
        &BTreeMap::new(),
    )
    .iter()
    .map(|d| d.code)
    .collect()
}

fn two_branches(a: N, b: N) -> (Vec<N>, Vec<Value>) {
    let (a_id, b_id) = (a.id, b.id);
    let index = |n: NodeId| {
        (1..=9)
            .find(|candidate| id(*candidate).parse::<NodeId>().unwrap() == n)
            .unwrap()
    };
    let (a_n, b_n) = (index(a_id), index(b_id));
    (
        vec![entry(1), fork(2), a, b],
        vec![
            wire(1, 1, "then", 2),
            wire(2, 2, "branch#0", a_n),
            wire(3, 2, "branch#1", b_n),
        ],
    )
}

const I64: &str = "core/i64@1";

#[test]
fn two_branches_setting_one_variable_conflict() {
    let (nodes, edges) = two_branches(set(3), set(4));
    assert_eq!(
        conflicts(I64, "reject", nodes, edges, &[]),
        [DiagnosticCode::WriteConflict]
    );
}

#[test]
fn last_writer_wins_and_same_commutative_update_are_exempt() {
    let (nodes, edges) = two_branches(set(3), set(4));
    assert!(conflicts(I64, "last-writer-wins", nodes, edges, &[]).is_empty());
    for operation in ["add", "and", "or", "xor", "min", "max"] {
        let (nodes, edges) = two_branches(update(3, operation), update(4, operation));
        let ty = if operation == "add" {
            I64
        } else {
            "core/u32@1"
        };
        assert!(
            conflicts(ty, "reject", nodes, edges, &[]).is_empty(),
            "{operation}"
        );
    }
}

#[test]
fn mixed_or_non_commutative_updates_conflict() {
    for (a, b) in [("add", "sub"), ("sub", "sub"), ("append", "append")] {
        let (nodes, edges) = two_branches(update(3, a), update(4, b));
        assert_eq!(
            conflicts(I64, "reject", nodes, edges, &[]),
            [DiagnosticCode::WriteConflict],
            "{a}/{b}"
        );
    }
    let (nodes, edges) = two_branches(update(3, "add"), set(4));
    assert_eq!(
        conflicts(I64, "reject", nodes, edges, &[]),
        [DiagnosticCode::WriteConflict]
    );
}

#[test]
fn package_min_max_conflicts_unless_last_writer_wins() {
    let ty = "org.example/ordered@1";
    let (nodes, edges) = two_branches(update(3, "min"), update(4, "min"));
    assert_eq!(
        conflicts(ty, "reject", nodes, edges, &[]),
        [DiagnosticCode::WriteConflict]
    );
    let (nodes, edges) = two_branches(update(3, "min"), update(4, "min"));
    assert!(conflicts(ty, "last-writer-wins", nodes, edges, &[]).is_empty());
}

#[test]
fn one_branch_and_nested_regions_are_attributed_to_their_branch() {
    // Two writes in one branch (through a nested sequence) never conflict with themselves.
    let nodes = vec![
        entry(1),
        fork(2),
        multi(3, "core/sequence", "then"),
        set(4),
        set(5),
    ];
    let edges = vec![
        wire(1, 1, "then", 2),
        wire(2, 2, "branch#0", 3),
        wire(3, 3, "then#0", 4),
        wire(4, 3, "then#1", 5),
    ];
    assert!(conflicts(I64, "reject", nodes, edges, &[]).is_empty());

    // The same nested write conflicts with a write in the sibling branch.
    let nodes = vec![
        entry(1),
        fork(2),
        multi(3, "core/sequence", "then"),
        set(4),
        set(5),
    ];
    let edges = vec![
        wire(1, 1, "then", 2),
        wire(2, 2, "branch#0", 3),
        wire(3, 3, "then#0", 4),
        wire(4, 2, "branch#1", 5),
    ];
    assert_eq!(
        conflicts(I64, "reject", nodes, edges, &[]),
        [DiagnosticCode::WriteConflict]
    );
}

#[test]
fn function_call_writes_come_from_the_callee_summary() {
    let (nodes, edges) = two_branches(call(3), set(4));
    assert_eq!(
        conflicts(I64, "reject", nodes, edges, &[WriteOperation::Add]),
        [DiagnosticCode::WriteConflict]
    );
    let (nodes, edges) = two_branches(call(3), update(4, "add"));
    assert!(conflicts(I64, "reject", nodes, edges, &[WriteOperation::Add]).is_empty());
    let (nodes, edges) = two_branches(call(3), update(4, "add"));
    assert_eq!(
        conflicts(
            I64,
            "reject",
            nodes,
            edges,
            &[WriteOperation::Add, WriteOperation::Set]
        ),
        [DiagnosticCode::WriteConflict]
    );
}
