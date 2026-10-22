use std::collections::BTreeMap;

use orange_model::GraphDocument;
use orange_model::document::{SignatureSnapshot, Suspends};
use orange_model::id::{FunctionId, NodeId};
use orange_model::validation::{
    DiagnosticCode, HostOrigin, LeafEffects, VariableWrite, WriteOperation, analyze_functions,
};
use serde_json::{Value, json};

const F1: &str = "10000000-0000-4000-8000-000000000001";
const F2: &str = "10000000-0000-4000-8000-000000000002";
const F3: &str = "10000000-0000-4000-8000-000000000003";
const N1: &str = "20000000-0000-4000-8000-000000000001";
const N2: &str = "20000000-0000-4000-8000-000000000002";
const N3: &str = "20000000-0000-4000-8000-000000000003";
const SIG: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "b".repeat(64))).unwrap()
}

fn document(functions: Value) -> GraphDocument {
    serde_json::from_value(json!({
        "format_version":1,"graph_id":"30000000-0000-4000-8000-000000000001",
        "revision":7,"nodes":[],"edges":[],"functions":functions,"signatures":{}
    }))
    .unwrap()
}

fn function(id: &str, kind: &str, nodes: Value) -> Value {
    json!({"id":id,"name":"f","kind":kind,"nodes":nodes,"edges":[]})
}

fn custom_node(id: &str, definition: &str) -> Value {
    json!({"id":id,"definition_id":definition,"definition_version":1,"config_version":1,
        "config":{},"signature":SIG})
}

fn snapshot(kind: &str, suspends: Option<&str>) -> SignatureSnapshot {
    serde_json::from_value(json!({"kind":kind,"suspends":suspends,"ports":[]})).unwrap()
}

fn analyze(
    doc: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    effects: &BTreeMap<NodeId, LeafEffects>,
) -> orange_model::validation::FunctionAnalysis {
    analyze_functions(doc, resolved, effects, &origin())
}

#[test]
fn infers_strongest_kind_and_ignores_declared_kind_as_evidence() {
    let doc = document(json!([function(
        F1,
        "pure",
        json!([
            custom_node(N1, "example/volatile"),
            custom_node(N2, "example/action")
        ])
    )]));
    let resolved = BTreeMap::from([
        (N1.parse().unwrap(), snapshot("volatile", None)),
        (N2.parse().unwrap(), snapshot("action", None)),
    ]);
    let effects = BTreeMap::from([(N2.parse().unwrap(), LeafEffects::Known(vec![]))]);
    let report = analyze(&doc, &resolved, &effects);
    let id: FunctionId = F1.parse().unwrap();
    assert_eq!(
        report.summaries[&id].kind,
        orange_model::document::NodeKind::Action
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::FunctionKind)
    );
}

#[test]
fn nested_calls_compose_kind_suspension_and_run_writes_conservatively() {
    let call_id = "20000000-0000-4000-8000-000000000003";
    let doc = document(json!([
        function(
            F1,
            "action",
            json!([{
                "id":call_id,"definition_id":"core/call","definition_version":1,
                "config_version":1,"config":{"function":F2}
            }])
        ),
        function(F2, "async", json!([custom_node(N1, "example/task")]))
    ]));
    let resolved = BTreeMap::from([(N1.parse().unwrap(), snapshot("async", Some("maybe")))]);
    let effects = BTreeMap::from([(N1.parse().unwrap(), LeafEffects::Known(vec![]))]);
    let report = analyze(&doc, &resolved, &effects);
    let outer: FunctionId = F1.parse().unwrap();
    assert_eq!(
        report.summaries[&outer].kind,
        orange_model::document::NodeKind::Async
    );
    assert_eq!(report.summaries[&outer].suspends, Suspends::Maybe);
}

#[test]
fn missing_leaf_effects_and_unavailable_graph_bodies_are_incomplete() {
    let doc = document(json!([function(
        F1,
        "pure",
        json!([custom_node(N1, "example/task")])
    )]));
    let resolved = BTreeMap::from([(N1.parse().unwrap(), snapshot("action", None))]);
    let report = analyze(&doc, &resolved, &BTreeMap::new());
    let id: FunctionId = F1.parse().unwrap();
    assert!(report.incomplete_functions.contains(&id));
    assert!(!report.summaries.contains_key(&id));
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::FunctionKind)
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.subject.node == Some(N1.parse().unwrap()))
    );
    let effects = BTreeMap::from([(N1.parse().unwrap(), LeafEffects::GraphBodyUnavailable)]);
    let report = analyze(&doc, &resolved, &effects);
    assert!(!report.summaries.contains_key(&id));
}

#[test]
fn asynchronous_kind_cannot_declare_never_suspends() {
    let doc = document(json!([function(
        F1,
        "async",
        json!([custom_node(N1, "example/task")])
    )]));
    let resolved = BTreeMap::from([(N1.parse().unwrap(), snapshot("async", Some("never")))]);
    let effects = BTreeMap::from([(N1.parse().unwrap(), LeafEffects::Known(vec![]))]);
    let report = analyze(&doc, &resolved, &effects);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::SuspendsInvalid
                && d.subject.node == Some(N1.parse().unwrap()))
    );
    assert!(!report.summaries.contains_key(&F1.parse().unwrap()));
}

#[test]
fn core_intrinsic_version_mismatch_does_not_use_host_fallback() {
    let doc: GraphDocument = serde_json::from_value(json!({
        "format_version":1,"graph_id":"30000000-0000-4000-8000-000000000001","revision":1,
        "nodes":[],"edges":[],"functions":[function(F1,"action",json!([{
            "id":N1,"definition_id":"core/wait","definition_version":2,
            "config_version":1,"config":{}
        }]))],"signatures":{}
    }))
    .unwrap();
    let report = analyze(&doc, &BTreeMap::new(), &BTreeMap::new());
    assert!(report.incomplete_functions.contains(&F1.parse().unwrap()));
    assert!(!report.summaries.contains_key(&F1.parse().unwrap()));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::FunctionMetadataUnavailable
                && d.subject.node == Some(N1.parse().unwrap()))
    );
}

#[test]
fn provider_backed_core_namespace_definition_uses_resolved_host_contract() {
    let mut node = custom_node(N1, "core/add");
    node["provider"] = json!({"package":"core.math","version":"1.0.0","id":"add"});
    let doc = document(json!([function(F1, "pure", json!([node]))]));
    let resolved = BTreeMap::from([(N1.parse().unwrap(), snapshot("pure", None))]);
    let effects = BTreeMap::from([(N1.parse().unwrap(), LeafEffects::Known(vec![]))]);
    let report = analyze(&doc, &resolved, &effects);
    assert_eq!(
        report.summaries[&F1.parse::<FunctionId>().unwrap()].kind,
        orange_model::document::NodeKind::Pure
    );
    assert!(!report.incomplete_functions.contains(&F1.parse().unwrap()));
}

#[test]
fn wait_and_yield_are_always_but_function_summary_is_only_maybe() {
    let doc = document(json!([function(
        F1,
        "async",
        json!([{
            "id":N1,"definition_id":"core/wait","definition_version":1,
            "config_version":1,"config":{}
        }])
    )]));
    let report = analyze(&doc, &BTreeMap::new(), &BTreeMap::new());
    assert_eq!(
        report.summaries[&F1.parse::<FunctionId>().unwrap()].suspends,
        Suspends::Maybe
    );
}

#[test]
fn direct_and_indirect_recursion_report_stable_closed_paths() {
    let call = |node: &str, function: &str| {
        json!({"id":node,"definition_id":"core/call",
        "definition_version":1,"config_version":1,"config":{"function":function}})
    };
    let doc = document(json!([
        function(F1, "pure", json!([call(N1, F1)])),
        function(F2, "pure", json!([call(N2, F3)])),
        function(F3, "pure", json!([call(N3, F2)]))
    ]));
    let report = analyze(&doc, &BTreeMap::new(), &BTreeMap::new());
    assert_eq!(report.recursion_paths.len(), 2);
    assert!(
        report
            .recursion_paths
            .contains(&vec![F1.parse().unwrap(), F1.parse().unwrap()])
    );
    assert!(report.recursion_paths.contains(&vec![
        F2.parse().unwrap(),
        F3.parse().unwrap(),
        F2.parse().unwrap()
    ]));
    assert!(report.incomplete_functions.contains(&F1.parse().unwrap()));
    assert!(report.incomplete_functions.contains(&F2.parse().unwrap()));
    assert!(report.incomplete_functions.contains(&F3.parse().unwrap()));
}

#[test]
fn run_write_summaries_filter_local_and_workspace_and_preserve_operations() {
    let run = "40000000-0000-4000-8000-000000000001";
    let local = "40000000-0000-4000-8000-000000000002";
    let workspace = "40000000-0000-4000-8000-000000000003";
    let doc: GraphDocument = serde_json::from_value(json!({
        "format_version":1,"graph_id":"30000000-0000-4000-8000-000000000001",
        "revision":2,"nodes":[],"edges":[],
        "variables":[
            {"id":run,"name":"r","type":"core/i64@1","scope":"run"},
            {"id":local,"name":"l","type":"core/i64@1","scope":"local"},
            {"id":workspace,"name":"w","type":"core/i64@1","scope":"workspace"}
        ],
        "functions":[function(F1,"action",json!([
            custom_node(N1,"example/action"),custom_node(N2,"example/action"),custom_node(N3,"example/action")
        ]))],"signatures":{}
    })).unwrap();
    let resolved = [N1, N2, N3]
        .into_iter()
        .map(|id| (id.parse().unwrap(), snapshot("action", None)))
        .collect();
    let effects = BTreeMap::from([
        (
            N1.parse().unwrap(),
            LeafEffects::Known(vec![VariableWrite {
                variable: run.parse().unwrap(),
                operation: WriteOperation::Add,
            }]),
        ),
        (
            N2.parse().unwrap(),
            LeafEffects::Known(vec![VariableWrite {
                variable: local.parse().unwrap(),
                operation: WriteOperation::Set,
            }]),
        ),
        (
            N3.parse().unwrap(),
            LeafEffects::Known(vec![VariableWrite {
                variable: workspace.parse().unwrap(),
                operation: WriteOperation::Append,
            }]),
        ),
    ]);
    let report = analyze(&doc, &resolved, &effects);
    let summary = &report.summaries[&F1.parse().unwrap()];
    assert_eq!(summary.run_writes.len(), 1);
    assert_eq!(
        summary.run_writes[&run.parse().unwrap()],
        [WriteOperation::Add].into()
    );
}

#[test]
fn nested_callee_run_writes_are_included_in_caller_summary() {
    let run = "40000000-0000-4000-8000-000000000011";
    let call = "20000000-0000-4000-8000-000000000011";
    let doc: GraphDocument = serde_json::from_value(json!({
        "format_version":1,"graph_id":"30000000-0000-4000-8000-000000000001",
        "revision":2,"nodes":[],"edges":[],
        "variables":[{"id":run,"name":"r","type":"core/i64@1","scope":"run"}],
        "functions":[
            function(F1,"action",json!([{"id":call,"definition_id":"core/call",
                "definition_version":1,"config_version":1,"config":{"function":F2}}])),
            function(F2,"action",json!([custom_node(N1,"example/action")]))
        ],"signatures":{}
    }))
    .unwrap();
    let resolved = BTreeMap::from([(N1.parse().unwrap(), snapshot("action", None))]);
    let effects = BTreeMap::from([(
        N1.parse().unwrap(),
        LeafEffects::Known(vec![VariableWrite {
            variable: run.parse().unwrap(),
            operation: WriteOperation::Add,
        }]),
    )]);
    let report = analyze(&doc, &resolved, &effects);
    assert_eq!(
        report.summaries[&F1.parse().unwrap()].run_writes[&run.parse().unwrap()],
        [WriteOperation::Add].into()
    );
}

#[test]
fn analysis_is_order_independent_and_read_only() {
    let first = document(json!([function(
        F1,
        "action",
        json!([
            custom_node(N1, "example/action"),
            custom_node(N2, "example/volatile")
        ])
    )]));
    let second = document(json!([function(
        F1,
        "action",
        json!([
            custom_node(N2, "example/volatile"),
            custom_node(N1, "example/action")
        ])
    )]));
    let resolved = BTreeMap::from([
        (N1.parse().unwrap(), snapshot("action", None)),
        (N2.parse().unwrap(), snapshot("volatile", None)),
    ]);
    let effects = BTreeMap::from([(N1.parse().unwrap(), LeafEffects::Known(vec![]))]);
    let first_state = first.state_id();
    let second_state = second.state_id();
    let a = analyze(&first, &resolved, &effects);
    let b = analyze(&second, &resolved, &effects);
    assert_eq!(a.summaries, b.summaries);
    assert_eq!(a.diagnostics, b.diagnostics);
    assert_eq!(first.state_id(), first_state);
    assert_eq!(second.state_id(), second_state);
    assert_eq!(first.revision(), 7);
    assert_eq!(second.revision(), 7);
}

#[test]
fn large_diamond_call_graph_does_not_enumerate_paths_or_rescan_all_pending_nodes() {
    let count = 400usize;
    let mut functions = Vec::with_capacity(count);
    for index in 0..count {
        let current = format!("10000000-0000-4000-8000-{index:012x}");
        let targets = [index + 1, index + 2]
            .into_iter()
            .filter(|target| *target < count);
        let nodes: Vec<_> = targets
            .enumerate()
            .map(|(offset, target)| {
                json!({
                    "id":format!("20000000-0000-4000-8000-{:012x}",index * 2 + offset),
                    "definition_id":"core/call","definition_version":1,"config_version":1,
                    "config":{"function":format!("10000000-0000-4000-8000-{target:012x}")}
                })
            })
            .collect();
        functions.push(function(&current, "pure", json!(nodes)));
    }
    let doc = document(json!(functions));
    let report = analyze(&doc, &BTreeMap::new(), &BTreeMap::new());
    assert_eq!(
        report.summaries.len(),
        count,
        "incomplete={:?} diagnostics={:?}",
        report.incomplete_functions,
        report.diagnostics.iter().take(4).collect::<Vec<_>>()
    );
    assert!(report.recursion_paths.is_empty());
}
