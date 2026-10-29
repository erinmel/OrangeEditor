use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{
    GraphDocument, NodeKind, OutputSource, PortDescriptor, SignatureSnapshot, Suspends,
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

/// An Add update with an explicit integer overflow policy.
fn add_with(n: u8, overflow: &str) -> N {
    exec(
        n,
        "core/update-variable",
        json!({"variable": VAR, "operation": "add", "int_overflow": overflow}),
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
        &resolved,
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
    for operation in ["and", "or", "xor", "min", "max"] {
        let (nodes, edges) = two_branches(update(3, operation), update(4, operation));
        assert!(
            conflicts("core/u32@1", "reject", nodes, edges, &[]).is_empty(),
            "{operation}"
        );
    }
    for ty in [I64, "core/u32@1"] {
        let (nodes, edges) = two_branches(add_with(3, "wrapping"), add_with(4, "wrapping"));
        assert!(
            conflicts(ty, "reject", nodes, edges, &[]).is_empty(),
            "{ty}"
        );
    }
}

/// Add commutes exactly only as fixed-width wrapping integer addition. Saturation near a
/// bound, checked faults and float rounding depend on branch order (11 §11.2).
#[test]
fn add_is_exempt_only_with_wrapping_integer_evidence() {
    let conflict = |ty: &str, a: N, b: N| {
        let (nodes, edges) = two_branches(a, b);
        conflicts(ty, "reject", nodes, edges, &[]) == [DiagnosticCode::WriteConflict]
    };
    assert!(conflict(
        I64,
        add_with(3, "saturating"),
        add_with(4, "saturating")
    ));
    assert!(conflict(
        I64,
        add_with(3, "checked"),
        add_with(4, "checked")
    ));
    assert!(
        conflict(I64, update(3, "add"), update(4, "add")),
        "checked by default"
    );
    assert!(conflict(
        I64,
        add_with(3, "wrapping"),
        add_with(4, "saturating")
    ));
    assert!(conflict(
        "core/f64@1",
        add_with(3, "wrapping"),
        add_with(4, "wrapping")
    ));
    assert!(conflict(
        "core/duration@1",
        add_with(3, "wrapping"),
        add_with(4, "wrapping")
    ));
    // Explicit last-writer-wins accepts the order dependence.
    let (nodes, edges) = two_branches(add_with(3, "saturating"), add_with(4, "saturating"));
    assert!(conflicts(I64, "last-writer-wins", nodes, edges, &[]).is_empty());
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
    // A summary records the operation but not its overflow policy, so it is no evidence.
    let (nodes, edges) = two_branches(call(3), add_with(4, "wrapping"));
    assert_eq!(
        conflicts(I64, "reject", nodes, edges, &[WriteOperation::Add]),
        [DiagnosticCode::WriteConflict]
    );
    let (nodes, edges) = two_branches(call(3), update(4, "and"));
    assert!(conflicts("core/u32@1", "reject", nodes, edges, &[WriteOperation::And]).is_empty());
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

fn stale(captured: bool, update_instead: bool, suspending: bool) -> Vec<DiagnosticCode> {
    let ty = || "core/i64@1".parse().unwrap();
    let mut entry = entry(1);
    entry.json["activation"] = json!({"mode":"parallel","max_parallel":2,"capacity":4,
        "overflow":"drop-oldest","on_fault":"continue","order":0});
    let mut capture = node(
        2,
        "example/action",
        json!({}),
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("then"),
            PortDescriptor::data_in("x", ty()),
            PortDescriptor::data_out("out", ty(), OutputSource::Execution),
        ],
    );
    capture.signature.suspends = Some(if suspending {
        Suspends::Maybe
    } else {
        Suspends::Never
    });
    let mut get = node(
        3,
        "core/get-variable",
        json!({"variable": VAR}),
        vec![PortDescriptor::data_out("value", ty(), OutputSource::State)],
    );
    get.signature.kind = NodeKind::Volatile;
    let mut write = if update_instead {
        update(4, "add")
    } else {
        set(4)
    };
    write
        .signature
        .ports
        .push(PortDescriptor::data_in("value", ty()));
    let data = |n: u8, from: u8, from_port: &str, to: u8, to_port: &str| {
        json!({"id": format!("30000000-0000-4000-8000-{n:012}"),
            "from": {"node": id(from), "port": from_port},
            "to": {"node": id(to), "port": to_port}})
    };
    let mut edges = vec![wire(1, 1, "then", 2), wire(2, 2, "then", 4)];
    if captured {
        edges.push(data(3, 3, "value", 2, "x"));
        edges.push(data(4, 2, "out", 4, "value"));
    } else {
        edges.push(data(3, 3, "value", 4, "value"));
    }
    stale_run(vec![entry, capture, get, write], edges)
}

fn stale_run(nodes: Vec<N>, edges: Vec<Value>) -> Vec<DiagnosticCode> {
    let doc: GraphDocument = from_json(
        &json!({
            "format":"orange.graph","format_version":1,
            "graph_id":"40000000-0000-4000-8000-000000000001","revision":0,
            "variables":[{"id":VAR,"name":"v","type":"core/i64@1","scope":"run"}],
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
    validate_state(
        &doc,
        &registry,
        &resolved,
        &regions.index,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .iter()
    .map(|d| d.code)
    .collect()
}

#[test]
fn captured_read_written_back_warns_only_when_parallel_and_suspending() {
    assert_eq!(stale(true, false, true), [DiagnosticCode::StaleCapture]);
    assert!(
        stale(false, false, true).is_empty(),
        "direct get-set is atomic"
    );
    assert!(stale(true, false, false).is_empty(), "nothing suspends");
    assert!(
        stale(true, true, true).is_empty(),
        "update-variable is atomic"
    );
}

#[test]
fn local_persistence_and_initializer_agreement_are_declaration_errors() {
    let decl = |scope: &str, extra: Value| {
        let mut d = json!({"id": VAR, "name": "v", "type": "core/i64@1", "scope": scope});
        d.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let doc: GraphDocument = from_json(
            &json!({
                "format":"orange.graph","format_version":1,
                "graph_id":"40000000-0000-4000-8000-000000000001","revision":0,
                "variables":[d]
            })
            .to_string(),
        )
        .unwrap();
        validate_state(
            &doc,
            &registry(),
            &BTreeMap::new(),
            &Default::default(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .iter()
        .map(|d| d.code)
        .collect::<Vec<_>>()
    };
    assert_eq!(
        decl("local", json!({"persistent": true})),
        [DiagnosticCode::VariablePersistent]
    );
    assert!(decl("run", json!({"persistent": true})).is_empty());
    let good = json!({"init": {"type": "core/i64@1", "value": "3"}});
    assert!(decl("run", good).is_empty());
    for init in [
        json!({"type": "core/string@1", "value": "x"}),
        json!({"type": "core/i64@1", "value": "nope"}),
    ] {
        assert_eq!(
            decl("run", json!({"init": init})),
            [DiagnosticCode::LiteralType]
        );
    }
}

fn register_codes(ty: &str, config: Value) -> Vec<DiagnosticCode> {
    let mut register = node(
        2,
        "core/register",
        config,
        vec![PortDescriptor::exec_in("store")],
    );
    register.json["type_pins"] = json!({"T": ty});
    let doc: GraphDocument = from_json(
        &json!({
            "format":"orange.graph","format_version":1,
            "graph_id":"40000000-0000-4000-8000-000000000001","revision":0,
            "nodes":[register.json]
        })
        .to_string(),
    )
    .unwrap();
    let registry = registry();
    let resolved = orange_model::validation::validate_nodes(&doc, &registry).resolved;
    assert_eq!(resolved.len(), 1);
    validate_state(
        &doc,
        &registry,
        &resolved,
        &Default::default(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .iter()
    .map(|d| d.code)
    .collect()
}

#[test]
fn register_stream_scope_and_initial_rules() {
    let i64_ty = "core/i64@1";
    assert!(register_codes(i64_ty, json!({})).is_empty());
    assert!(
        register_codes(
            i64_ty,
            json!({"scope":"activation","initial":{"type":i64_ty,"value":"1"}})
        )
        .is_empty()
    );
    assert_eq!(
        register_codes(i64_ty, json!({"scope":"workspace"})),
        [DiagnosticCode::RegisterConfig]
    );
    for initial in [
        json!({"type":"core/string@1","value":"x"}),
        json!({"type":i64_ty,"value":"nope"}),
        json!(7),
    ] {
        assert_eq!(
            register_codes(i64_ty, json!({"initial": initial})),
            [DiagnosticCode::RegisterConfig]
        );
    }
    assert_eq!(
        register_codes("core/stream@1<core/i64@1>", json!({})),
        [DiagnosticCode::StreamStore]
    );
}
