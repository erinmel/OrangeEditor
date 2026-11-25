use orange_host::{Call, Host, HostOptions, KernelFault, ServiceNode, ServicePackage, Value};
use orange_model::conversion::{ConnectionError, ConnectionRequest, plan_connection};
use orange_model::document::PortRef;
use serde_json::{Value as Json, json};

const PACKAGE: &str = "example.outcomes";
const REQUEST: &str = "example.outcomes/request";

struct Request;

impl ServiceNode for Request {
    fn run(&self, _: &Call<'_>) -> Result<Vec<Value>, KernelFault> {
        Ok(vec![Value::I64(0)])
    }
}

fn host_with_request() -> Host {
    let definition = orange_sdk::Definition::node(PACKAGE, "request", "Request")
        .input("amount", "core/i64@1")
        .variadic_input("parameter", "core/i64@1", 4)
        .build();
    let mut definition: Json = serde_json::from_str(&definition).unwrap();
    definition["ports"].as_array_mut().unwrap().extend([
        json!({"port_id":"received", "direction":"output", "kind":"exec"}),
        json!({"port_id":"result", "direction":"output", "kind":"data",
            "type":"core/i64@1", "source":"execution", "available_on":["received"]}),
    ]);
    let package = ServicePackage::new(PACKAGE, "1.0.0")
        .node(&definition.to_string(), Request)
        .unwrap();
    Host::with_services(HostOptions::default(), vec![package])
        .unwrap()
        .0
}

fn draft_with_request(
    host: &Host,
) -> (orange_host::Draft, orange_host::NodeId, orange_host::NodeId) {
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let entry = host
        .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
        .unwrap();
    let request = host.add_node(&mut draft, REQUEST, [300.0, 0.0]).unwrap();
    let wait = host
        .add_node(&mut draft, "core/wait", [600.0, 0.0])
        .unwrap();
    host.set_input(&mut draft, (wait, "duration"), Some("1000000"))
        .unwrap();
    host.set_input(&mut draft, (request, "amount"), Some("12"))
        .unwrap();
    host.connect(&mut draft, (entry, "then"), (request, "in"))
        .unwrap();
    host.connect(&mut draft, (request, "received"), (wait, "in"))
        .unwrap();
    (draft, request, wait)
}

#[test]
fn an_unavailable_exact_digest_uses_saved_outcomes_and_rejects_port_edits() {
    let host = host_with_request();
    let (draft, request, wait) = draft_with_request(&host);
    let saved = draft.text().unwrap();
    let mut document: Json = serde_json::from_str(&saved).unwrap();
    let lock = document["package_lock"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["id"] == PACKAGE)
        .unwrap();
    lock["content_digest"] = json!(format!("sha256:{}", "f".repeat(64)));
    let mut draft = host.draft(&document.to_string()).unwrap();
    let number = host
        .add_node(&mut draft, "core/whole-number", [0.0, 200.0])
        .unwrap();
    let view = host.draft_authoring_view(&draft).unwrap();
    let card = view
        .graph
        .nodes
        .iter()
        .find(|card| card.id == request)
        .unwrap();
    let port = |key| card.ports.iter().find(|port| port.id == key);
    assert!(port("received").unwrap().connected);
    assert!(port("then").is_none());
    for key in ["failed", "error", "cancelled"] {
        assert!(view.failure_routes[&request].contains(key));
        assert!(port(key).is_some());
    }
    assert_eq!(port("error").unwrap().ty.as_deref(), Some("core/error@1"));
    assert_eq!(view.graph.wires.len(), 2);
    let before = draft.text().unwrap();
    let request_wire = ConnectionRequest {
        from: PortRef {
            node: number,
            port: "result".parse().unwrap(),
        },
        to: PortRef {
            node: request,
            port: "amount".parse().unwrap(),
        },
        choice: None,
    };
    assert!(matches!(
        plan_connection(draft.document(), host.registry(), request_wire, || {
            panic!("an unavailable provider should be rejected before allocating IDs")
        }),
        Err(ConnectionError::NotADataPort { node, .. }) if node == request
    ));
    assert!(
        host.connect(&mut draft, (request, "failed"), (wait, "in"))
            .is_err()
    );
    assert!(
        host.connect_through(&mut draft, (number, "result"), (request, "amount"), None)
            .is_err()
    );
    assert!(
        host.set_input(&mut draft, (request, "amount"), Some("13"))
            .is_err()
    );
    assert!(host.givers(&draft, (request, "amount")).is_empty());
    assert!(host.takers(&draft, (request, "result")).is_empty());
    assert_eq!(draft.text().unwrap(), before);
    let view_before = host.draft_authoring_view(&draft).unwrap();
    assert!(draft.place(&[(request, [350.0, 30.0])]).unwrap());
    assert!(draft.undo());
    assert_eq!(host.draft_authoring_view(&draft).unwrap(), view_before);
    assert!(
        host.disconnect_wire(&mut draft, (request, "received"), (wait, "in"))
            .unwrap()
    );
    assert!(draft.undo());
    assert_eq!(host.draft_authoring_view(&draft).unwrap(), view_before);

    let mut restored = host.draft(&saved).unwrap();
    assert!(
        host.set_input(&mut restored, (request, "amount"), Some("13"))
            .unwrap()
    );
}

#[test]
fn conversion_planning_refuses_new_orphan_variadic_endpoints() {
    let host = host_with_request();
    let (mut draft, request, _) = draft_with_request(&host);
    let number = host
        .add_node(&mut draft, "core/whole-number", [0.0, 200.0])
        .unwrap();
    assert_eq!(
        host.add_port(&mut draft, request, "parameter").unwrap(),
        "parameter#0"
    );
    assert_eq!(
        host.add_port(&mut draft, request, "parameter").unwrap(),
        "parameter#1"
    );
    assert!(
        host.remove_port(&mut draft, (request, "parameter#1"))
            .unwrap()
    );
    let from = PortRef {
        node: number,
        port: "result".parse().unwrap(),
    };
    for key in ["parameter", "parameter#1", "amount#0"] {
        let to = PortRef {
            node: request,
            port: key.parse().unwrap(),
        };
        assert!(matches!(
            plan_connection(
                draft.document(),
                host.registry(),
                ConnectionRequest {
                    from: from.clone(),
                    to,
                    choice: None,
                },
                || panic!("an invalid endpoint should not allocate IDs")
            ),
            Err(ConnectionError::NotADataPort { .. })
        ));
        assert!(
            host.connect_through(&mut draft, (number, "result"), (request, key), None)
                .is_err()
        );
    }
    assert!(
        host.connect_through(
            &mut draft,
            (number, "result"),
            (request, "parameter#0"),
            None
        )
        .is_ok()
    );
}

#[test]
fn orphans_stay_in_the_document_and_can_be_detached_without_a_provider() {
    let host = host_with_request();
    let (draft, request, wait) = draft_with_request(&host);
    let mut document: Json = serde_json::from_str(&draft.text().unwrap()).unwrap();
    let edge = document["edges"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|edge| edge["from"]["port"] == "received")
        .unwrap();
    edge["from"]["port"] = json!("removed");
    let orphan = edge["id"].as_str().unwrap().parse().unwrap();
    let (host, _) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&document.to_string()).unwrap();
    let view = host.draft_authoring_view(&draft).unwrap();
    let card = view
        .graph
        .nodes
        .iter()
        .find(|card| card.id == request)
        .unwrap();
    assert!(card.ports.iter().all(|port| port.id != "removed"));
    assert!(
        view.graph
            .wires
            .iter()
            .any(|wire| wire.from.0 == request && wire.from.1 == "removed")
    );
    assert!(draft.document().edge(orphan).is_some());
    assert!(
        host.disconnect_wire(&mut draft, (request, "removed"), (wait, "in"))
            .unwrap()
    );
    assert!(draft.document().edge(orphan).is_none());
    assert!(draft.undo());
    assert!(draft.document().edge(orphan).is_some());
}
