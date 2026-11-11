//! A graph sends a real HTTP request through a declared connection and reads the reply.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use orange_host::{FlowId, Host, HostError, HostOptions, NodeId, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{GraphDocument, from_json, to_json};
use serde_json::{Value as Json, json};

const I64: &str = "core/i64@1";
const STRING: &str = "core/string@1";
const CONNECTION: &str = "40000000-0000-4000-8000-000000000001";
const WAIT: Duration = Duration::from_secs(20);

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn node(n: u32) -> NodeId {
    id(n).parse().unwrap()
}
fn variable(n: u32) -> VariableId {
    id(n).parse().unwrap()
}
fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id": id(n), "definition_id": format!("core/{name}"),
        "definition_version": 1, "config_version": 1, "config": {}});
    if name.starts_with("on-") {
        node["activation"] = json!({"mode": "queue", "capacity": 4, "overflow": "fault",
            "on_fault": "continue", "order": 0});
    }
    node
}
fn set(n: u32, target: u32) -> Json {
    let mut node = core(n, "set-variable");
    node["config"] = json!({"variable": id(target)});
    node
}
fn edge(n: u32, from: u32, out: &str, to: u32, input: &str) -> Json {
    json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
        "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": input}})
}

/// Answers each request with status 201 and a body echoing its request line and body.
fn server(requests: usize) -> String {
    server_with(requests, 201)
}

/// The same server answering with another status code.
fn server_with(requests: usize, status: u16) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let _ = std::thread::spawn(move || {
        for _ in 0..requests {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut request_line = String::new();
            let _ = reader.read_line(&mut request_line).unwrap();
            let mut length = 0;
            loop {
                let mut header = String::new();
                let _ = reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let reply = format!(
                "{}|{}",
                request_line.trim(),
                String::from_utf8(body).unwrap()
            );
            let mut stream = reader.into_inner();
            write!(
                stream,
                "HTTP/1.1 {status} Done\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
        }
    });
    format!("http://{address}")
}

/// A manual entry sends `POST /scene` with body `hi` and stores the status and reply.
/// `failed` stores -1 instead, so a transport failure is visible too.
fn document(host: &Host, connection: Json) -> String {
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let decl = serde_json::from_value(connection).unwrap();
    let _ = document
        .apply([orange_model::DocCommand::UpsertConnection(decl)])
        .unwrap();
    let mut request = NewNode::new(
        node(3),
        DefinitionRef {
            id: orange_net::HTTP_REQUEST.parse().unwrap(),
            version: 1,
        },
    );
    request.config = serde_json::from_value(json!({"connection": CONNECTION})).unwrap();
    request.inputs = serde_json::from_value(json!({
        "method": {"type": STRING, "value": "POST"},
        "path": {"type": STRING, "value": "/scene"},
        "content": {"type": STRING, "value": "hi"}
    }))
    .unwrap();
    let edit = new_node_edit(&document, host.registry(), request).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    let mut failed = set(6, 100);
    failed["inputs"] = json!({"value": {"type": I64, "value": "-1"}});
    saved["nodes"].as_array_mut().unwrap().extend([
        core(1, "on-manual"),
        set(4, 100),
        set(5, 101),
        failed,
    ]);
    saved["edges"] = json!([
        edge(1, 1, "then", 3, "in"),
        edge(2, 3, "then", 4, "in"),
        edge(3, 3, "status", 4, "value"),
        edge(4, 4, "then", 5, "in"),
        edge(5, 3, "response", 5, "value"),
        edge(6, 3, "failed", 6, "in"),
    ]);
    saved["variables"] = json!([
        {"id": id(100), "name": "status", "type": I64, "scope": "run"},
        {"id": id(101), "name": "reply", "type": STRING, "scope": "run"}
    ]);
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

fn read(host: &Host, flow: FlowId, n: u32) -> Option<Value> {
    host.inspect(flow, move |run| run.variable(variable(n)).cloned())
        .unwrap()
}
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn host() -> Host {
    Host::with_services(HostOptions::default(), vec![orange_net::package()])
        .unwrap()
        .0
}
fn graph_connection(base_url: &str) -> Json {
    json!({"id": CONNECTION, "name": "api", "adapter": orange_net::HTTP_ADAPTER,
        "scope": "graph", "settings": {"base_url": base_url, "timeout_ms": 5000}})
}

#[test]
fn a_node_sends_a_request_on_the_documents_connection() {
    let mut host = host();
    let text = document(&host, graph_connection(&server(1)));
    assert!(host.check(&text).unwrap().is_empty());
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the reply", || read(&host, flow, 101).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(201)));
    let Some(Value::Str(reply)) = read(&host, flow, 101) else {
        panic!("the reply is text");
    };
    assert_eq!(reply.as_str(), "POST /scene HTTP/1.1|hi");
}

#[test]
fn two_documents_use_one_shared_connection_whose_settings_stay_in_the_host() {
    let mut host = host();
    let shared = json!({"id": CONNECTION, "name": "api", "adapter": orange_net::HTTP_ADAPTER,
        "scope": "shared"});
    let text = document(&host, shared);
    assert!(!text.contains("base_url"));
    // Declared but not provided by the host yet.
    assert!(matches!(host.open(&text), Err(HostError::Connection(name, _)) if name == "api"));

    let settings = json!({"base_url": server(2)});
    host.share_connection(
        CONNECTION.parse().unwrap(),
        orange_net::HTTP_ADAPTER,
        settings.as_object().unwrap().clone(),
    )
    .unwrap();
    for _ in 0..2 {
        let flow = host.open(&text).unwrap();
        host.trigger(flow, node(1), None).unwrap();
        until("the reply", || read(&host, flow, 100).is_some());
        assert_eq!(read(&host, flow, 100), Some(Value::I64(201)));
    }
}

#[test]
fn an_unreachable_server_takes_the_nodes_failed_route() {
    let mut host = host();
    // Bind and drop to get a port nothing listens on.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let text = document(&host, graph_connection(&format!("http://127.0.0.1:{port}")));
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the failure", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(-1)));
}

#[test]
fn connections_that_cannot_work_are_refused_before_anything_runs() {
    let mut host = host();
    let scheme = document(&host, graph_connection("ftp://example.invalid"));
    assert!(
        matches!(host.open(&scheme), Err(HostError::Connection(_, reason)) if reason.contains("https"))
    );

    let unknown = json!({"id": CONNECTION, "name": "api", "adapter": "org.example/none",
        "scope": "graph"});
    let text = document(&host, unknown);
    assert!(matches!(host.open(&text), Err(HostError::Connection(..))));

    // A node that names a connection the document does not declare is a validation error.
    let mut saved: Json =
        serde_json::from_str(&document(&host, graph_connection("http://127.0.0.1:9"))).unwrap();
    let _ = saved.as_object_mut().unwrap().remove("connections");
    let diagnostics = host.check(&saved.to_string()).unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message_key == "core/graph-connection-unknown"),
        "{diagnostics:?}"
    );
    assert_eq!(host.flows().count(), 0);
}

#[test]
fn an_https_connection_opens_and_a_failed_handshake_takes_the_failed_route() {
    let mut host = host();
    // A plain-text server: the TLS handshake cannot succeed, and nothing is sent in clear.
    let plain = server(1).replacen("http://", "https://", 1);
    let text = document(&host, graph_connection(&plain));
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the failure", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(-1)));
}

/// Needs the public internet, so it is not part of the normal run:
/// `cargo test -p orange-net -- --ignored`.
#[test]
#[ignore = "contacts https://example.com"]
fn a_real_https_endpoint_is_reached_with_certificate_verification() {
    let mut host = host();
    let mut saved: Json =
        serde_json::from_str(&document(&host, graph_connection("https://example.com"))).unwrap();
    for node in saved["nodes"].as_array_mut().unwrap() {
        if node["definition_id"] == orange_net::HTTP_REQUEST {
            node["inputs"]["method"]["value"] = json!("GET");
            node["inputs"]["path"]["value"] = json!("/");
            node["inputs"]["content"]["value"] = json!("");
        }
    }
    let flow = host.open(&saved.to_string()).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the reply", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(200)));

    // A certificate for another name must be refused, not accepted.
    let wrong = document(&host, graph_connection("https://wrong.host.badssl.com"));
    let flow = host.open(&wrong).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the refusal", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(-1)));
}

const POST_SCENE: &str = "org.example.api/post-scene";
const STAND_IN: &str = "40000000-0000-4000-8000-0000000000ff";
const OTHER: &str = "40000000-0000-4000-8000-000000000002";

/// A package with no code: one node that posts its `content` to `/scene` on whichever
/// connection the node using it names, built from the base HTTP request node.
fn scene_package(host: &Host) -> Vec<u8> {
    // The request node of the body is authored like any node, against a stand-in
    // connection.
    let mut scratch = GraphDocument::new("20000000-0000-4000-8000-0000000000aa".parse().unwrap());
    let stand_in = serde_json::from_value(json!({"id": STAND_IN, "name": "connection",
        "adapter": orange_net::HTTP_ADAPTER, "scope": "shared"}))
    .unwrap();
    let _ = scratch
        .apply([orange_model::DocCommand::UpsertConnection(stand_in)])
        .unwrap();
    let mut request = NewNode::new(
        node(303),
        DefinitionRef {
            id: orange_net::HTTP_REQUEST.parse().unwrap(),
            version: 1,
        },
    );
    request.config = serde_json::from_value(json!({"connection": STAND_IN})).unwrap();
    request.inputs = serde_json::from_value(json!({
        "method": {"type": STRING, "value": "POST"},
        "path": {"type": STRING, "value": "/scene"}
    }))
    .unwrap();
    let edit = new_node_edit(&scratch, host.registry(), request).unwrap();
    let _ = scratch.apply(edit.commands).unwrap();
    let authored: Json = serde_json::from_str(&to_json(&scratch).unwrap()).unwrap();

    let port = |name: &str, direction: &str, ty: &str| json!({"port_id": name, "direction": direction, "kind": "data", "type": ty});
    let graph = json!({
        "format": orange_host::GRAPH_NODE_FORMAT, "format_version": 1,
        "connection": STAND_IN,
        "function": {"id": id(300), "name": "post-scene", "kind": "async",
            "inputs": [port("content", "input", STRING)],
            "outputs": [port("status", "output", I64)],
            "nodes": [core(301, "function-entry"), authored["nodes"][0], core(305, "return")],
            "edges": [
                edge(301, 301, "then", 303, "in"),
                edge(302, 301, "content", 303, "content"),
                edge(303, 303, "then", 305, "in"),
                edge(304, 303, "status", 305, "status"),
            ]},
        "signatures": authored["signatures"],
        "package_lock": authored["package_lock"]
    });
    let definition = json!({"definition_id": POST_SCENE, "definition_version": 1,
    "config_version": 1, "kind": "async", "suspends": "always", "determinism": "external",
    "connection": true,
    "ports": [
        port("content", "input", STRING),
        {"port_id": "status", "direction": "output", "kind": "data", "type": I64,
            "source": "execution", "available_on": ["then"]}
    ]});
    let manifest = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.api", "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "dependencies": [],
        "contributions": {
            "nodes": [POST_SCENE],
            "providers": [{"slot": format!("node:{POST_SCENE}@1"), "id": orange_host::GRAPH_PROVIDER}]
        },
        "permissions": [], "license": "MIT",
        "about": {"name": "Scene API", "keywords": ["http"]}
    });
    orange_host::pack(
        manifest.to_string().as_bytes(),
        &[
            (
                "nodes/post-scene.json".into(),
                definition.to_string().into_bytes(),
            ),
            (
                "graphs/post-scene.json".into(),
                graph.to_string().into_bytes(),
            ),
        ],
    )
    .unwrap()
}

#[test]
fn a_node_made_as_a_graph_uses_whichever_connection_each_use_names() {
    let mut host = host();
    let package = scene_package(&host);
    let _ = host
        .install(&package, orange_host::Grants::default())
        .unwrap();

    // One diagram, two servers, the same package node on each.
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000002".parse().unwrap());
    for (connection, url) in [
        (CONNECTION, server_with(1, 201)),
        (OTHER, server_with(1, 202)),
    ] {
        let decl = serde_json::from_value(json!({"id": connection, "name": "api",
            "adapter": orange_net::HTTP_ADAPTER, "scope": "graph",
            "settings": {"base_url": url}}))
        .unwrap();
        let _ = document
            .apply([orange_model::DocCommand::UpsertConnection(decl)])
            .unwrap();
    }
    for (n, connection) in [(3, CONNECTION), (5, OTHER)] {
        let mut call = NewNode::new(
            node(n),
            DefinitionRef {
                id: POST_SCENE.parse().unwrap(),
                version: 1,
            },
        );
        call.config = serde_json::from_value(json!({"connection": connection})).unwrap();
        call.inputs =
            serde_json::from_value(json!({"content": {"type": STRING, "value": "hi"}})).unwrap();
        let edit = new_node_edit(&document, host.registry(), call).unwrap();
        assert_eq!(edit.provider.id, orange_host::GRAPH_PROVIDER);
        let _ = document.apply(edit.commands).unwrap();
    }
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    saved["nodes"]
        .as_array_mut()
        .unwrap()
        .extend([core(1, "on-manual"), set(4, 100), set(6, 101)]);
    saved["edges"] = json!([
        edge(1, 1, "then", 3, "in"),
        edge(2, 3, "then", 4, "in"),
        edge(3, 3, "status", 4, "value"),
        edge(4, 4, "then", 5, "in"),
        edge(5, 5, "then", 6, "in"),
        edge(6, 5, "status", 6, "value"),
    ]);
    saved["variables"] = json!([
        {"id": id(100), "name": "first", "type": I64, "scope": "run"},
        {"id": id(101), "name": "second", "type": I64, "scope": "run"}
    ]);
    let text = to_json(&from_json(&saved.to_string()).unwrap()).unwrap();
    // The saved diagram holds two package nodes and nothing of the HTTP node behind them.
    assert!(!text.contains(orange_net::HTTP_REQUEST));
    let diagnostics = host.check(&text).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("both requests", || read(&host, flow, 101).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(201)));
    assert_eq!(read(&host, flow, 101), Some(Value::I64(202)));
}
