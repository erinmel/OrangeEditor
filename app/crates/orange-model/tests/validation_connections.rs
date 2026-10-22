use orange_model::validation::{DiagnosticCode, HostOrigin, validate_connections};
use orange_model::{GraphDocument, to_json};
use proptest::prelude::*;
use serde_json::{Value, json};

const SRC: &str = "10000000-0000-4000-8000-000000000001";
const SRC2: &str = "10000000-0000-4000-8000-000000000002";
const DST: &str = "10000000-0000-4000-8000-000000000003";
const DST2: &str = "10000000-0000-4000-8000-000000000004";
const SRC_SIG: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DST_SIG: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap()
}

fn node(id: &str, signature: &str, variadic: Value) -> Value {
    json!({
        "id": id,
        "definition_id": if signature == SRC_SIG { "example/source" } else { "example/sink" },
        "definition_version": 1,
        "config_version": 1,
        "config": {},
        "signature": signature,
        "variadic": variadic
    })
}

fn edge(id: u8, from_node: &str, from_port: &str, to_node: &str, to_port: &str) -> Value {
    json!({
        "id": format!("20000000-0000-4000-8000-{id:012}"),
        "from": {"node": from_node, "port": from_port},
        "to": {"node": to_node, "port": to_port}
    })
}

fn graph(edges: Vec<Value>, missing_node: bool) -> GraphDocument {
    let mut nodes = vec![node(SRC, SRC_SIG, json!({})), node(DST, DST_SIG, json!({}))];
    if !missing_node {
        nodes.push(node(SRC2, SRC_SIG, json!({})));
        nodes.push(node(DST2, DST_SIG, json!({})));
    }
    serde_json::from_value(json!({
        "format_version": 1,
        "graph_id": "30000000-0000-4000-8000-000000000001",
        "revision": 9,
        "nodes": nodes,
        "edges": edges,
        "signatures": {
            (SRC_SIG): {"kind":"action","ports":[
                {"port_id":"value","direction":"output","kind":"data","type":"core/i64@1","source":"computed"},
                {"port_id":"go","direction":"output","kind":"exec"},
                {"port_id":"tick","direction":"output","kind":"event","type":"core/i64@1","source":"payload"},
                {"port_id":"items","direction":"output","kind":"data","type":"core/stream@1<core/i64@1>","source":"execution"},
                {"port_id":"many","direction":"output","kind":"exec","variadic":{"min":0,"max":null}}
            ]},
            (DST_SIG): {"kind":"action","ports":[
                {"port_id":"value","direction":"input","kind":"data","type":"core/i64@1"},
                {"port_id":"in","direction":"input","kind":"exec"}
            ]}
        }
    })).unwrap()
}

fn codes(report: &orange_model::validation::ConnectionValidation) -> Vec<DiagnosticCode> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn accepts_data_exec_and_event_connections() {
    let doc = graph(
        vec![
            edge(1, SRC, "value", DST, "value"),
            edge(2, SRC, "go", DST, "in"),
            edge(3, SRC, "tick", DST, "in"),
        ],
        false,
    );
    let result = validate_connections(&doc, &origin());
    assert!(result.is_valid(), "{:?}", result.diagnostics);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn reports_missing_endpoints_and_preserves_document_state() {
    let doc = graph(
        vec![edge(
            1,
            SRC,
            "missing",
            "10000000-0000-4000-8000-000000000099",
            "value",
        )],
        true,
    );
    let before = to_json(&doc).unwrap();
    let revision = doc.revision();
    let state_id = doc.state_id();
    let result = validate_connections(&doc, &origin());
    assert_eq!(
        codes(&result),
        vec![DiagnosticCode::PortUnknown, DiagnosticCode::PortUnknown]
    );
    assert!(!result.is_valid());
    assert_eq!(to_json(&doc).unwrap(), before);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.state_id(), state_id);
}

#[test]
fn unavailable_metadata_is_incomplete_without_fabricating_an_unknown_port() {
    let mut value: Value = serde_json::from_str(
        &to_json(&graph(vec![edge(1, SRC, "value", DST, "value")], false)).unwrap(),
    )
    .unwrap();
    value["nodes"][0]
        .as_object_mut()
        .unwrap()
        .remove("signature");
    let doc: GraphDocument = serde_json::from_value(value).unwrap();
    let result = validate_connections(&doc, &origin());
    assert!(result.diagnostics.is_empty());
    assert!(result.incomplete_nodes.contains(&SRC.parse().unwrap()));
    assert!(!result.is_valid());
}

#[test]
fn cross_body_edge_gets_only_the_scope_diagnostic() {
    let mut value: Value = serde_json::from_str(
        &to_json(&graph(vec![edge(1, SRC, "value", DST, "value")], false)).unwrap(),
    )
    .unwrap();
    let nodes = value["nodes"].as_array_mut().unwrap();
    let dst_index = nodes.iter().position(|node| node["id"] == DST).unwrap();
    let dst_node = nodes.remove(dst_index);
    value["functions"] = json!([{
        "id":"40000000-0000-4000-8000-000000000001",
        "name":"body",
        "kind":"action",
        "inputs":[],
        "outputs":[],
        "nodes":[dst_node],
        "edges":[]
    }]);
    let doc: GraphDocument = serde_json::from_value(value).unwrap();
    let result = validate_connections(&doc, &origin());
    assert_eq!(codes(&result), vec![DiagnosticCode::ScopeEdge]);

    let mut misplaced_value: Value =
        serde_json::from_str(&to_json(&graph(vec![], false)).unwrap()).unwrap();
    misplaced_value["functions"] = json!([{
        "id":"40000000-0000-4000-8000-000000000001",
        "name":"body",
        "kind":"action",
        "inputs":[],
        "outputs":[],
        "nodes":[],
        "edges":[edge(2, SRC, "value", DST, "value")]
    }]);
    let misplaced: GraphDocument = serde_json::from_value(misplaced_value).unwrap();
    assert_eq!(
        codes(&validate_connections(&misplaced, &origin())),
        vec![DiagnosticCode::ScopeEdge]
    );
}

#[test]
fn checks_direction_kind_and_cardinality_including_streams() {
    let doc = graph(
        vec![
            edge(1, DST, "in", SRC, "value"),
            edge(2, SRC, "value", DST, "in"),
            edge(3, SRC, "go", DST, "value"),
            edge(4, SRC, "value", DST, "value"),
            edge(5, SRC2, "value", DST, "value"),
            edge(6, SRC, "go", DST, "in"),
            edge(7, SRC, "go", DST2, "in"),
            edge(8, SRC, "tick", DST, "in"),
            edge(9, SRC, "tick", DST2, "in"),
            edge(10, SRC, "items", DST, "value"),
            edge(11, SRC, "items", DST2, "value"),
        ],
        false,
    );
    let result = validate_connections(&doc, &origin());
    let actual: std::collections::BTreeSet<_> = codes(&result).into_iter().collect();
    for expected in [
        DiagnosticCode::PortDirection,
        DiagnosticCode::KindMismatch,
        DiagnosticCode::FaninData,
        DiagnosticCode::FanoutExec,
        DiagnosticCode::FanoutEvent,
        DiagnosticCode::StreamFanout,
    ] {
        assert!(
            actual.contains(&expected),
            "missing {expected:?} in {actual:?}"
        );
    }

    let mut mismatched_types: Value = serde_json::from_str(
        &to_json(&graph(vec![edge(1, SRC, "value", DST, "value")], false)).unwrap(),
    )
    .unwrap();
    mismatched_types["signatures"][DST_SIG]["ports"][0]["type"] = json!("core/u64@1");
    let mismatched_types: GraphDocument = serde_json::from_value(mismatched_types).unwrap();
    assert!(
        codes(&validate_connections(&mismatched_types, &origin()))
            .contains(&DiagnosticCode::TypeMismatch)
    );

    let malformed_peer = graph(
        vec![
            edge(1, SRC, "value", DST, "value"),
            edge(
                2,
                "10000000-0000-4000-8000-000000000099",
                "value",
                DST,
                "value",
            ),
        ],
        true,
    );
    let malformed_codes = codes(&validate_connections(&malformed_peer, &origin()));
    assert!(malformed_codes.contains(&DiagnosticCode::PortUnknown));
    assert!(malformed_codes.contains(&DiagnosticCode::FaninData));
}

#[test]
fn variadic_keys_resolve_by_persisted_instance_key() {
    let mut first = graph(vec![edge(1, SRC, "many#2", DST, "in")], false);
    let mut value: Value = serde_json::from_str(&to_json(&first).unwrap()).unwrap();
    value["nodes"][0]["variadic"] = json!({"many":{"order":[2,5],"next":6}});
    first = serde_json::from_value(value).unwrap();
    let valid = validate_connections(&first, &origin());
    assert!(valid.is_valid(), "{:?}", valid.diagnostics);

    let invalid = graph(vec![edge(1, SRC, "many#3", DST, "in")], false);
    assert_eq!(
        codes(&validate_connections(&invalid, &origin())),
        vec![DiagnosticCode::PortUnknown]
    );
}

proptest! {
    #[test]
    fn diagnostic_order_is_independent_of_node_and_edge_insertion_order(
        node_keys in prop::array::uniform4(any::<u32>()),
        edge_keys in prop::array::uniform4(any::<u32>())
    ) {
        let base = graph(vec![
            edge(1, DST, "in", SRC, "value"),
            edge(2, SRC, "value", DST, "in"),
            edge(3, SRC2, "value", DST, "value"),
            edge(4, SRC, "go", DST, "value"),
        ], false);
        let mut value: Value = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
        let nodes = value["nodes"].as_array().unwrap().clone();
        let mut node_order = [0usize, 1, 2, 3];
        node_order.sort_by_key(|index| node_keys[*index]);
        value["nodes"] = Value::Array(node_order.iter().map(|index| nodes[*index].clone()).collect());
        let edges = value["edges"].as_array().unwrap().clone();
        let mut edge_order = [0usize, 1, 2, 3];
        edge_order.sort_by_key(|index| edge_keys[*index]);
        value["edges"] = Value::Array(edge_order.iter().map(|index| edges[*index].clone()).collect());
        let shuffled: GraphDocument = serde_json::from_value(value).unwrap();
        prop_assert_eq!(
            validate_connections(&base, &origin()).diagnostics,
            validate_connections(&shuffled, &origin()).diagnostics
        );
    }
}
