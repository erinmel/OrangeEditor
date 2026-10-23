use std::collections::BTreeMap;
use std::sync::Arc;

use orange_model::GraphDocument;
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::SignatureSnapshot;
use orange_model::id::{FunctionId, NodeId};
use orange_model::validation::{DiagnosticCode, FunctionEvidence, HostOrigin, analyze_functions};
use serde_json::{Value, json};

const CALLER: &str = "10000000-0000-4000-8000-000000000001";
const CALLEE: &str = "10000000-0000-4000-8000-000000000002";
const RUN_VAR: &str = "40000000-0000-4000-8000-000000000001";
const LOCAL_VAR: &str = "40000000-0000-4000-8000-000000000002";
const WORKSPACE_VAR: &str = "40000000-0000-4000-8000-000000000003";
const GET_NODE: &str = "20000000-0000-4000-8000-000000000001";
const SET_NODE: &str = "20000000-0000-4000-8000-000000000002";
const UPDATE_NODE: &str = "20000000-0000-4000-8000-000000000003";
const CALL_NODE: &str = "20000000-0000-4000-8000-000000000004";

fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "b".repeat(64))).unwrap()
}

fn registry() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(origin())
}

fn document(variables: Value, functions: Value) -> GraphDocument {
    serde_json::from_value(json!({
        "format_version":1,
        "graph_id":"30000000-0000-4000-8000-000000000001",
        "revision":1,
        "nodes":[],
        "edges":[],
        "variables":variables,
        "functions":functions,
        "signatures":{}
    }))
    .unwrap()
}

fn variable(id: &str, scope: &str) -> Value {
    json!({"id":id,"name":"v","type":"core/i64@1","scope":scope})
}

fn function(id: &str, kind: &str, nodes: Value) -> Value {
    json!({"id":id,"name":"f","kind":kind,"nodes":nodes,"edges":[]})
}

fn node(id: &str, definition: &str, config: Value) -> Value {
    json!({
        "id":id,
        "definition_id":definition,
        "definition_version":1,
        "config_version":1,
        "config":config
    })
}

fn analyze(doc: &GraphDocument) -> orange_model::validation::FunctionAnalysis {
    analyze_functions(
        doc,
        &FunctionEvidence::new(
            doc,
            BTreeMap::<NodeId, SignatureSnapshot>::new(),
            BTreeMap::new(),
        ),
        &registry(),
    )
}

fn diagnostic_subjects(
    report: &orange_model::validation::FunctionAnalysis,
    code: DiagnosticCode,
) -> Vec<NodeId> {
    report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .filter_map(|diagnostic| diagnostic.subject.node)
        .collect()
}

#[test]
fn builtin_variable_reads_and_writes_propagate_only_run_effects_to_callers() {
    let doc = document(
        json!([variable(RUN_VAR, "run"), variable(LOCAL_VAR, "local")]),
        json!([
            function(
                CALLER,
                "action",
                json!([node(CALL_NODE, "core/call", json!({"function":CALLEE}))])
            ),
            function(
                CALLEE,
                "action",
                json!([
                    node(GET_NODE, "core/get-variable", json!({"variable":RUN_VAR})),
                    node(SET_NODE, "core/set-variable", json!({"variable":RUN_VAR})),
                    node(
                        UPDATE_NODE,
                        "core/update-variable",
                        json!({"variable":RUN_VAR,"operation":"add"})
                    ),
                    node(
                        "20000000-0000-4000-8000-000000000006",
                        "core/update-variable",
                        json!({"variable":RUN_VAR,"operation":"min"})
                    ),
                    node(
                        "20000000-0000-4000-8000-000000000007",
                        "core/update-variable",
                        json!({"variable":RUN_VAR,"operation":"max"})
                    ),
                    node(
                        "20000000-0000-4000-8000-000000000008",
                        "core/update-variable",
                        json!({"variable":RUN_VAR,"operation":"compare-and-set"})
                    ),
                    node(
                        "20000000-0000-4000-8000-000000000009",
                        "core/update-variable",
                        json!({"variable":RUN_VAR,"operation":"and"})
                    ),
                    node(
                        "20000000-0000-4000-8000-000000000010",
                        "core/update-variable",
                        json!({"variable":RUN_VAR,"operation":"or"})
                    ),
                    node(
                        "20000000-0000-4000-8000-000000000011",
                        "core/update-variable",
                        json!({"variable":RUN_VAR,"operation":"xor"})
                    ),
                    node(
                        "20000000-0000-4000-8000-000000000005",
                        "core/update-variable",
                        json!({"variable":LOCAL_VAR,"operation":"add"})
                    )
                ])
            )
        ]),
    );

    let report = analyze(&doc);
    let caller: FunctionId = CALLER.parse().unwrap();
    let callee: FunctionId = CALLEE.parse().unwrap();
    let run: orange_model::id::VariableId = RUN_VAR.parse().unwrap();

    assert!(!report.incomplete_functions.contains(&callee));
    assert!(!report.incomplete_functions.contains(&caller));
    assert_eq!(report.summaries[&callee].run_writes.len(), 1);
    assert_eq!(report.summaries[&caller].run_writes.len(), 1);
    assert_eq!(
        report.summaries[&caller].run_writes[&run],
        [
            orange_model::validation::WriteOperation::Set,
            orange_model::validation::WriteOperation::Add,
            orange_model::validation::WriteOperation::Min,
            orange_model::validation::WriteOperation::Max,
            orange_model::validation::WriteOperation::CompareAndSet,
            orange_model::validation::WriteOperation::And,
            orange_model::validation::WriteOperation::Or,
            orange_model::validation::WriteOperation::Xor
        ]
        .into()
    );
}

#[test]
fn workspace_sync_write_is_incomplete_instead_of_becoming_a_run_effect() {
    let doc = document(
        json!([variable(WORKSPACE_VAR, "workspace")]),
        json!([function(
            CALLER,
            "action",
            json!([node(
                SET_NODE,
                "core/set-variable",
                json!({"variable":WORKSPACE_VAR})
            )])
        )]),
    );
    let report = analyze(&doc);
    let function: FunctionId = CALLER.parse().unwrap();

    assert!(report.incomplete_functions.contains(&function));
    assert!(!report.summaries.contains_key(&function));
    assert_eq!(
        diagnostic_subjects(&report, DiagnosticCode::FunctionMetadataUnavailable),
        vec![SET_NODE.parse().unwrap()]
    );
}

#[test]
fn malformed_variable_reference_and_update_operation_have_node_subjects() {
    let doc = document(
        json!([variable(RUN_VAR, "run")]),
        json!([function(
            CALLER,
            "action",
            json!([
                node(
                    GET_NODE,
                    "core/get-variable",
                    json!({"variable":"not-an-id"})
                ),
                node(
                    UPDATE_NODE,
                    "core/update-variable",
                    json!({"variable":RUN_VAR,"operation":"invented"})
                )
            ])
        )]),
    );
    let report = analyze(&doc);

    assert_eq!(
        diagnostic_subjects(&report, DiagnosticCode::VariableUnknown),
        vec![GET_NODE.parse().unwrap()]
    );
    assert_eq!(
        diagnostic_subjects(&report, DiagnosticCode::UpdateOperation),
        vec![UPDATE_NODE.parse().unwrap()]
    );
}

#[test]
fn provider_bound_core_wait_cannot_fall_back_to_host_intrinsic() {
    let mut wait = node(
        "20000000-0000-4000-8000-000000000006",
        "core/wait",
        json!({}),
    );
    wait["provider"] = json!({"package":"example.pkg","version":"1.0.0","id":"wait"});
    let doc = document(json!([]), json!([function(CALLER, "async", json!([wait]))]));
    let report = analyze(&doc);
    let function: FunctionId = CALLER.parse().unwrap();
    let wait_id: NodeId = "20000000-0000-4000-8000-000000000006".parse().unwrap();

    assert!(report.incomplete_functions.contains(&function));
    assert!(!report.summaries.contains_key(&function));
    assert_eq!(
        diagnostic_subjects(&report, DiagnosticCode::FunctionMetadataUnavailable),
        vec![wait_id]
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::SuspendsMissing)
    );
}

#[test]
fn unsupported_reserved_pure_zone_metadata_stays_incomplete() {
    let zone = node(
        "20000000-0000-4000-8000-000000000007",
        "core/repeat-begin",
        json!({}),
    );
    let doc = document(json!([]), json!([function(CALLER, "pure", json!([zone]))]));
    let report = analyze(&doc);
    let function: FunctionId = CALLER.parse().unwrap();
    let zone_id: NodeId = "20000000-0000-4000-8000-000000000007".parse().unwrap();

    assert!(report.incomplete_functions.contains(&function));
    assert!(!report.summaries.contains_key(&function));
    assert_eq!(
        diagnostic_subjects(&report, DiagnosticCode::FunctionMetadataUnavailable),
        vec![zone_id]
    );
}

#[test]
fn invalid_contextual_update_keeps_function_summary_incomplete() {
    for (operation, ty, code) in [
        ("append", "core/i64@1", DiagnosticCode::UpdateUnsupported),
        ("add", "core/bool@1", DiagnosticCode::ConstraintUnsatisfied),
        ("min", "core/f64@1", DiagnosticCode::ConstraintUnsatisfied),
        ("and", "core/f64@1", DiagnosticCode::ConstraintUnsatisfied),
        (
            "or",
            "core/duration@1",
            DiagnosticCode::ConstraintUnsatisfied,
        ),
        (
            "xor",
            "core/string@1",
            DiagnosticCode::ConstraintUnsatisfied,
        ),
        (
            "compare-and-set",
            "core/f64@1",
            DiagnosticCode::ConstraintUnsatisfied,
        ),
    ] {
        let mut declaration = variable(RUN_VAR, "run");
        declaration["type"] = json!(ty);
        let doc = document(
            json!([declaration]),
            json!([function(
                CALLER,
                "action",
                json!([node(
                    UPDATE_NODE,
                    "core/update-variable",
                    json!({"variable":RUN_VAR,"operation":operation})
                )])
            )]),
        );
        // A caller-supplied projection cannot substitute for the current intrinsic contract.
        let evidence = FunctionEvidence::new(
            &doc,
            BTreeMap::from([(
                UPDATE_NODE.parse().unwrap(),
                serde_json::from_value(json!({"kind":"action","ports":[]})).unwrap(),
            )]),
            BTreeMap::new(),
        );
        let report = analyze_functions(&doc, &evidence, &registry());
        let function: FunctionId = CALLER.parse().unwrap();
        assert!(report.incomplete_functions.contains(&function));
        assert!(!report.summaries.contains_key(&function));
        assert_eq!(
            diagnostic_subjects(&report, code),
            vec![UPDATE_NODE.parse().unwrap()]
        );
    }
}
