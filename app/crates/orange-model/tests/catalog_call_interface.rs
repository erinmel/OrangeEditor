use std::sync::Arc;

use orange_model::GraphDocument;
use orange_model::catalog::{CallInterfaceError, RegistrySnapshot, call_interface};
use orange_model::document::PortDirection;
use orange_model::validation::HostOrigin;
use serde_json::{Value, json};

const F1: &str = "10000000-0000-4000-8000-000000000001";
const F2: &str = "10000000-0000-4000-8000-000000000002";
const N1: &str = "20000000-0000-4000-8000-000000000001";
const N2: &str = "20000000-0000-4000-8000-000000000002";
const GRAPH: &str = "30000000-0000-4000-8000-000000000001";

fn registry() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(HostOrigin::new("1.0.0", format!("sha256:{}", "b".repeat(64))).unwrap())
}

fn function(id: &str, kind: &str, inputs: Value, outputs: Value) -> Value {
    json!({
        "id": id,
        "name": "callable",
        "kind": kind,
        "type_params": [],
        "inputs": inputs,
        "outputs": outputs,
        "nodes": [],
        "edges": []
    })
}

fn call_node(id: &str, function: Option<&str>) -> Value {
    let mut node = json!({
        "id": id,
        "definition_id": "core/call",
        "definition_version": 1,
        "config_version": 1,
        "config": {}
    });
    if let Some(function) = function {
        node["config"]["function"] = json!(function);
    }
    node
}

fn document(functions: Value, nodes: Value) -> GraphDocument {
    serde_json::from_value(json!({
        "format_version": 1,
        "graph_id": GRAPH,
        "revision": 0,
        "nodes": nodes,
        "edges": [],
        "functions": functions,
        "signatures": {}
    }))
    .unwrap()
}

fn call(
    doc: &GraphDocument,
    id: &str,
) -> Result<Option<orange_model::catalog::CallInterface>, CallInterfaceError> {
    call_interface(doc, doc.node(id.parse().unwrap()).unwrap(), &registry())
}

#[test]
fn each_call_projects_its_current_targets_interface() {
    let doc = document(
        json!([
            function(
                F1,
                "pure",
                json!([{"port_id":"left","direction":"input","kind":"data","type":"core/i64@1"}]),
                json!([])
            ),
            function(
                F2,
                "async",
                json!([{"port_id":"right","direction":"input","kind":"data","type":"core/string@1"}]),
                json!([])
            )
        ]),
        json!([call_node(N1, Some(F1)), call_node(N2, Some(F2))]),
    );

    let first = call(&doc, N1).unwrap().unwrap();
    let second = call(&doc, N2).unwrap().unwrap();
    assert_eq!(first.function, F1.parse().unwrap());
    assert_eq!(first.ports[0].port_id.as_str(), "left");
    assert_eq!(first.ports[0].ty.as_ref().unwrap().as_str(), "core/i64@1");
    assert_eq!(second.function, F2.parse().unwrap());
    assert_eq!(second.ports[0].port_id.as_str(), "right");
    assert_eq!(
        second.ports[0].ty.as_ref().unwrap().as_str(),
        "core/string@1"
    );
}

#[test]
fn generic_parameters_and_input_authoring_metadata_are_preserved_without_provenance() {
    let doc = document(
        json!([{
            "id": F1,
            "name": "generic",
            "kind": "pure",
            "type_params": [{"name":"T","constraints":["eq"]}],
            "inputs": [{
                "port_id":"items", "direction":"input", "kind":"data",
                "type":"core/option@1<core/list@1<$T>>", "optional":true,
                "variadic":{"min":1,"max":4}
            }],
            "outputs": [{
                "port_id":"result", "direction":"output", "kind":"data",
                "type":"$T", "source":"computed", "available_on":["then"],
                "optional":false, "lazy":false
            }],
            "nodes":[], "edges":[]
        }]),
        json!([call_node(N1, Some(F1))]),
    );
    let before = serde_json::to_value(&doc).unwrap();
    let interface = call(&doc, N1).unwrap().unwrap();

    assert_eq!(interface.type_params[0].name, "T");
    assert_eq!(interface.type_params[0].constraints, ["eq"]);
    assert_eq!(interface.ports[0].direction, PortDirection::Input);
    assert_eq!(
        interface.ports[0].ty.as_ref().unwrap().as_str(),
        "core/option@1<core/list@1<$T>>"
    );
    assert_eq!(interface.ports[0].optional, Some(true));
    assert_eq!(interface.ports[0].variadic.unwrap().min, 1);
    assert_eq!(interface.ports[0].source, None);
    assert_eq!(interface.ports[0].available_on, None);
    assert_eq!(interface.ports[1].source, None);
    assert_eq!(interface.ports[1].available_on, None);
    assert_eq!(interface.ports[1].optional, None);
    assert_eq!(interface.ports[1].lazy, None);
    assert_eq!(serde_json::to_value(&doc).unwrap(), before);
}

#[test]
fn kind_does_not_change_the_type_projection() {
    let inputs = json!([{"port_id":"x","direction":"input","kind":"data","type":"core/bool@1"}]);
    let pure = document(
        json!([function(F1, "pure", inputs.clone(), json!([]))]),
        json!([call_node(N1, Some(F1))]),
    );
    let action = document(
        json!([function(F1, "action", inputs, json!([]))]),
        json!([call_node(N1, Some(F1))]),
    );
    assert_eq!(call(&pure, N1).unwrap(), call(&action, N1).unwrap());
}

#[test]
fn rejects_wrong_call_identity_target_and_domains() {
    let wrong_identity = document(
        json!([function(F1, "pure", json!([]), json!([]))]),
        json!([{
            "id":N1,"definition_id":"core/call","definition_version":2,
            "config_version":1,"config":{"function":F1}
        }]),
    );
    assert_eq!(call(&wrong_identity, N1), Err(CallInterfaceError::Identity));

    let missing = document(
        json!([function(F1, "pure", json!([]), json!([]))]),
        json!([call_node(N1, None)]),
    );
    assert_eq!(call(&missing, N1), Err(CallInterfaceError::TargetMissing));

    let malformed = document(
        json!([function(F1, "pure", json!([]), json!([]))]),
        json!([call_node(N1, Some("not-a-function-id"))]),
    );
    assert_eq!(call(&malformed, N1), Err(CallInterfaceError::TargetMissing));

    let unknown = document(
        json!([function(F1, "pure", json!([]), json!([]))]),
        json!([call_node(N1, Some(F2))]),
    );
    assert_eq!(call(&unknown, N1), Err(CallInterfaceError::TargetUnknown));

    let bad_caller_domain: GraphDocument = serde_json::from_value(json!({
        "format_version":1,"graph_id":GRAPH,"revision":0,"domain":"other/domain",
        "nodes":[call_node(N1, Some(F1))],"edges":[],
        "functions":[function(F1,"pure",json!([]),json!([]))],"signatures":{}
    }))
    .unwrap();
    assert_eq!(
        call(&bad_caller_domain, N1),
        Err(CallInterfaceError::DomainUnsupported)
    );

    let bad_target_domain: GraphDocument = serde_json::from_value(json!({
        "format_version":1,"graph_id":GRAPH,"revision":0,
        "nodes":[call_node(N1, Some(F1))],"edges":[],
        "functions":[{
            "id":F1,"name":"f","kind":"pure","domain":null,
            "inputs":[],"outputs":[],"nodes":[],"edges":[]
        }],"signatures":{}
    }))
    .unwrap();
    assert_eq!(
        call(&bad_target_domain, N1),
        Err(CallInterfaceError::DomainUnsupported)
    );
}

#[test]
fn rejects_malformed_or_non_data_interfaces() {
    let cases = [
        // Wrong direction in the inputs list.
        json!({"inputs":[{"port_id":"x","direction":"output","kind":"data","type":"core/i64@1"}],"outputs":[]}),
        // Duplicate ID across input and output.
        json!({"inputs":[{"port_id":"x","direction":"input","kind":"data","type":"core/i64@1"}],"outputs":[{"port_id":"x","direction":"output","kind":"data","type":"core/i64@1"}]}),
        // Reserved implicit ID.
        json!({"inputs":[{"port_id":"then","direction":"input","kind":"data","type":"core/i64@1"}],"outputs":[]}),
        // Exec port.
        json!({"inputs":[{"port_id":"x","direction":"input","kind":"exec"}],"outputs":[]}),
        // Missing type.
        json!({"inputs":[{"port_id":"x","direction":"input","kind":"data"}],"outputs":[]}),
        // Undeclared parameter.
        json!({"inputs":[{"port_id":"x","direction":"input","kind":"data","type":"$U"}],"outputs":[]}),
        // Duplicate parameter names.
        json!({"type_params":[{"name":"T"},{"name":"T"}],"inputs":[],"outputs":[]}),
        // Output default is not input authoring metadata.
        json!({"inputs":[],"outputs":[{"port_id":"x","direction":"output","kind":"data","type":"core/i64@1","default":{"type":"core/i64@1","value":"1"}}]}),
        // Output optional/lazy flags are invalid when enabled.
        json!({"inputs":[],"outputs":[{"port_id":"x","direction":"output","kind":"data","type":"core/i64@1","optional":true}]}),
        // Input descriptors cannot claim output provenance or availability.
        json!({"inputs":[{"port_id":"x","direction":"input","kind":"data","type":"core/i64@1","source":"state"}],"outputs":[]}),
        json!({"inputs":[{"port_id":"x","direction":"input","kind":"data","type":"core/i64@1","available_on":["then"]}],"outputs":[]}),
    ];
    for interface in cases {
        let mut target = function(F1, "pure", json!([]), json!([]));
        if let Some(params) = interface.get("type_params") {
            target["type_params"] = params.clone();
        }
        target["inputs"] = interface["inputs"].clone();
        target["outputs"] = interface["outputs"].clone();
        let doc = document(json!([target]), json!([call_node(N1, Some(F1))]));
        assert_eq!(call(&doc, N1), Err(CallInterfaceError::InvalidInterface));
    }
}

#[test]
fn non_call_node_returns_none() {
    let doc = document(
        json!([function(F1, "pure", json!([]), json!([]))]),
        json!([{
            "id":N1,"definition_id":"example/node","definition_version":1,
            "config_version":1,"config":{}
        }]),
    );
    assert_eq!(
        call_interface(&doc, doc.node(N1.parse().unwrap()).unwrap(), &registry()),
        Ok(None)
    );
}
