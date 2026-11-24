//! A graph sends on a declared WebSocket connection and is triggered by what arrives.
use orange_testkit::{WAIT, activation, core, edge, id, node, until};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use orange_host::{FlowId, Host, HostOptions, RunState, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{DocCommand, GraphDocument, from_json, to_json};
use serde_json::{Value as Json, json};
use tungstenite::Message;

const STRING: &str = "core/string@1";
const CONNECTION: &str = "40000000-0000-4000-8000-000000000001";

/// Greets each client with `hello`, then answers every text with `echo:<text>`.
fn server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let _ = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let _ = std::thread::spawn(move || {
                let mut socket = tungstenite::accept(stream.unwrap()).unwrap();
                socket.send(Message::text("hello")).unwrap();
                while let Ok(message) = socket.read() {
                    if let Message::Text(text) = message {
                        let reply = format!("echo:{}", text.as_str());
                        if socket.send(Message::text(reply)).is_err() {
                            break;
                        }
                    }
                }
            });
        }
    });
    format!("ws://{address}")
}

/// Each received message is stored in `reply`; a manual entry sends `ping`.
fn document(host: &Host, graph: u32, connection: Json) -> String {
    let mut document = GraphDocument::new(
        format!("20000000-0000-4000-8000-{graph:012x}")
            .parse()
            .unwrap(),
    );
    let decl = serde_json::from_value(connection).unwrap();
    let _ = document
        .apply([DocCommand::UpsertConnection(decl)])
        .unwrap();
    let uses = json!({"connection": CONNECTION});
    let package_node = |n: u32, definition: &str, inputs: Json| {
        let mut new = NewNode::new(
            node(n),
            DefinitionRef {
                id: definition.parse().unwrap(),
                version: 1,
            },
        );
        new.config = serde_json::from_value(uses.clone()).unwrap();
        new.inputs = serde_json::from_value(inputs).unwrap();
        new
    };
    let message = package_node(1, orange_net::WS_MESSAGE, json!({}));
    let edit = new_node_edit(&document, host.registry(), message).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    let send = package_node(
        3,
        orange_net::WS_SEND,
        json!({"text": {"type": STRING, "value": "ping"}}),
    );
    let edit = new_node_edit(&document, host.registry(), send).unwrap();
    let _ = document.apply(edit.commands).unwrap();

    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    for entry in saved["nodes"].as_array_mut().unwrap() {
        if entry["definition_id"] == orange_net::WS_MESSAGE {
            entry["activation"] = activation();
        }
    }
    let mut keep = core(4, "set-variable");
    keep["config"] = json!({"variable": id(100)});
    saved["nodes"]
        .as_array_mut()
        .unwrap()
        .extend([core(2, "on-manual"), keep]);
    saved["edges"] = json!([
        edge(1, 1, "then", 4, "in"),
        edge(2, 1, "payload", 4, "value"),
        edge(3, 2, "then", 3, "in"),
    ]);
    saved["variables"] = json!([{"id": id(100), "name": "reply", "type": STRING, "scope": "run"}]);
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

fn reply(host: &Host, flow: FlowId) -> Option<String> {
    let variable: VariableId = id(100).parse().unwrap();
    match host
        .inspect(flow, move |run| run.variable(variable).cloned())
        .unwrap()
    {
        Some(Value::Str(text)) => Some(text.as_str().to_owned()),
        _ => None,
    }
}
/// A credential store holding the test connection's password.
#[derive(Debug)]
struct Secrets;
impl orange_host::SecretStore for Secrets {
    fn get(&self, name: &str) -> Result<String, orange_host::SecretError> {
        match name {
            "feed/password" => Ok("generated-test-value".to_owned()),
            _ => Err(orange_host::SecretError::NotFound),
        }
    }
}

fn host() -> Host {
    Host::with_services(HostOptions::default(), vec![orange_net::package()])
        .unwrap()
        .0
}

#[test]
fn received_messages_trigger_the_graph_and_a_node_sends_on_the_same_socket() {
    let mut host = host();
    let connection = json!({"id": CONNECTION, "name": "peer", "adapter": orange_net::WS_ADAPTER,
        "scope": "graph", "settings": {"url": server()}});
    let text = document(&host, 1, connection);
    assert!(host.check(&text).unwrap().is_empty());
    let flow = host.open(&text).unwrap();
    // The package entry is fed by the connection; only the manual entry is offered.
    assert_eq!(host.info(flow).unwrap().manual_entries, [(node(2), false)]);

    until("the greeting", || {
        reply(&host, flow).as_deref() == Some("hello")
    });
    host.trigger(flow, node(2), None).unwrap();
    until("the echo", || {
        reply(&host, flow).as_deref() == Some("echo:ping")
    });
}

#[test]
fn two_diagrams_share_one_socket_and_a_paused_one_keeps_what_arrived() {
    let mut host = host();
    let url = server();
    host.share_connection(
        CONNECTION.parse().unwrap(),
        orange_net::WS_ADAPTER,
        json!({"url": url}).as_object().unwrap().clone(),
    )
    .unwrap();
    let shared = json!({"id": CONNECTION, "name": "peer", "adapter": orange_net::WS_ADAPTER,
        "scope": "shared"});
    let sender = host.open(&document(&host, 1, shared.clone())).unwrap();
    let listener = host.open(&document(&host, 2, shared)).unwrap();
    // One socket: one greeting, which both diagrams may or may not have been in time
    // for. Wait until the socket is up, then hold the listener.
    until("the socket", || reply(&host, sender).is_some());
    host.pause(listener).unwrap();
    until("the pause", || {
        host.state(listener) == Some(RunState::Paused)
    });

    host.trigger(sender, node(2), None).unwrap();
    until("the echo", || {
        reply(&host, sender).as_deref() == Some("echo:ping")
    });
    // Sent once by one diagram, heard by both; the held one has not acted on it.
    assert_ne!(reply(&host, listener).as_deref(), Some("echo:ping"));
    host.resume(listener).unwrap();
    until("the held message", || {
        reply(&host, listener).as_deref() == Some("echo:ping")
    });
}

#[test]
fn messages_sent_before_the_server_exists_are_delivered_once_it_does() {
    let mut host = host();
    // Reserve a port, then release it so the first connection attempts fail.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let connection = json!({"id": CONNECTION, "name": "peer", "adapter": orange_net::WS_ADAPTER,
        "scope": "graph", "settings": {"url": format!("ws://{address}")}});
    let flow = host.open(&document(&host, 1, connection)).unwrap();
    host.trigger(flow, node(2), None).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(reply(&host, flow), None);

    let listener = TcpListener::bind(address).unwrap();
    let _ = std::thread::spawn(move || {
        let mut socket = tungstenite::accept(listener.accept().unwrap().0).unwrap();
        while let Ok(message) = socket.read() {
            if let Message::Text(text) = message {
                let reply = format!("late:{}", text.as_str());
                socket.send(Message::text(reply)).unwrap();
            }
        }
    });
    until("the queued message", || {
        reply(&host, flow).as_deref() == Some("late:ping")
    });
}

#[test]
fn editing_a_running_graph_keeps_its_socket_and_its_trigger() {
    let mut host = host();
    let connection = json!({"id": CONNECTION, "name": "peer", "adapter": orange_net::WS_ADAPTER,
        "scope": "graph", "settings": {"url": server()}});
    let text = document(&host, 1, connection);
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(2), None).unwrap();
    until("the echo", || {
        reply(&host, flow).as_deref() == Some("echo:ping")
    });

    let mut edited: Json = serde_json::from_str(&text).unwrap();
    for entry in edited["nodes"].as_array_mut().unwrap() {
        if entry["definition_id"] == orange_net::WS_SEND {
            entry["inputs"]["text"]["value"] = json!("pong");
        }
    }
    let updated = host.update(flow, &edited.to_string()).unwrap();
    assert!(updated.changed);
    assert_eq!(updated.kept_variables, 1);
    // A new socket would be greeted with `hello`; the kept one is silent.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(reply(&host, flow).as_deref(), Some("echo:ping"));

    // The edited send node and the re-attached message entry use the same socket.
    host.trigger(flow, node(2), None).unwrap();
    until("the edited echo", || {
        reply(&host, flow).as_deref() == Some("echo:pong")
    });
}

const ON_CHAT: &str = "org.example.chat/on-chat";
const STAND_IN: &str = "40000000-0000-4000-8000-0000000000ff";

/// A package with no code whose one node is a trigger on a WebSocket connection: it
/// fires with every message that arrives on the connection the using node names.
fn chat_package(host: &Host) -> Vec<u8> {
    // The message node of the fragment is authored like any node, against a stand-in
    // connection.
    let mut scratch = GraphDocument::new("20000000-0000-4000-8000-0000000000aa".parse().unwrap());
    let stand_in = serde_json::from_value(json!({"id": STAND_IN, "name": "connection",
        "adapter": orange_net::WS_ADAPTER, "scope": "shared"}))
    .unwrap();
    let _ = scratch
        .apply([DocCommand::UpsertConnection(stand_in)])
        .unwrap();
    let mut message = NewNode::new(
        node(301),
        DefinitionRef {
            id: orange_net::WS_MESSAGE.parse().unwrap(),
            version: 1,
        },
    );
    message.config = serde_json::from_value(json!({"connection": STAND_IN})).unwrap();
    let edit = new_node_edit(&scratch, host.registry(), message).unwrap();
    let _ = scratch.apply(edit.commands).unwrap();
    let authored: Json = serde_json::from_str(&to_json(&scratch).unwrap()).unwrap();
    let mut inner = authored["nodes"][0].clone();
    inner["activation"] = activation();
    let mut emit = core(302, "emit");
    emit["config"] = json!({"event": id(310)});

    let graph = json!({
        "format": orange_host::GRAPH_NODE_FORMAT, "format_version": 1,
        "connection": STAND_IN,
        "trigger": {
            "event": id(310),
            "nodes": [inner, emit],
            "edges": [
                edge(301, 301, "then", 302, "in"),
                edge(302, 301, "payload", 302, "payload"),
            ]
        },
        "signatures": authored["signatures"],
        "package_lock": authored["package_lock"]
    });
    let definition = json!({"definition_id": ON_CHAT, "definition_version": 1,
        "config_version": 1, "kind": "entry", "determinism": "external", "connection": true,
        "label": {"key": "org.example.chat/on-chat-label", "fallback": "On chat message"},
        "ports": [{"port_id": "payload", "direction": "output", "kind": "data",
            "type": STRING, "source": "payload"}]});
    let manifest = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.chat", "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "dependencies": [],
        "contributions": {
            "nodes": [ON_CHAT],
            "providers": [{"slot": format!("node:{ON_CHAT}@1"), "id": orange_host::GRAPH_PROVIDER}]
        },
        "permissions": [], "license": "MIT"
    });
    orange_host::pack(
        manifest.to_string().as_bytes(),
        &[
            (
                "nodes/on-chat.json".into(),
                definition.to_string().into_bytes(),
            ),
            ("graphs/on-chat.json".into(), graph.to_string().into_bytes()),
        ],
    )
    .unwrap()
}

#[test]
fn a_trigger_made_without_code_fires_on_messages_from_the_connection_its_use_names() {
    let (mut host, events) =
        Host::with_services(HostOptions::default(), vec![orange_net::package()]).unwrap();
    let package = chat_package(&host);
    let _ = host
        .install(&package, orange_host::Grants::default())
        .unwrap();
    let listed = host
        .nodes()
        .find(|node| node.definition.definition.id.as_str() == ON_CHAT)
        .unwrap();
    assert_eq!(
        listed
            .presentation
            .unwrap()
            .label
            .as_ref()
            .unwrap()
            .fallback,
        "On chat message"
    );

    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let decl = serde_json::from_value(json!({"id": CONNECTION, "name": "peer",
        "adapter": orange_net::WS_ADAPTER, "scope": "graph", "settings": {"url": server()}}))
    .unwrap();
    let _ = document
        .apply([DocCommand::UpsertConnection(decl)])
        .unwrap();
    let mut chat = NewNode::new(
        node(1),
        DefinitionRef {
            id: ON_CHAT.parse().unwrap(),
            version: 1,
        },
    );
    chat.config = serde_json::from_value(json!({"connection": CONNECTION})).unwrap();
    let edit = new_node_edit(&document, host.registry(), chat).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    saved["nodes"][0]["activation"] = activation();
    let mut keep = core(4, "set-variable");
    keep["config"] = json!({"variable": id(100)});
    saved["nodes"].as_array_mut().unwrap().push(keep);
    saved["edges"] = json!([
        edge(1, 1, "then", 4, "in"),
        edge(2, 1, "payload", 4, "value"),
    ]);
    saved["variables"] = json!([{"id": id(100), "name": "reply", "type": STRING, "scope": "run"}]);
    let text = to_json(&from_json(&saved.to_string()).unwrap()).unwrap();
    // The saved diagram holds the package's trigger and nothing of what is behind it.
    assert!(!text.contains(orange_net::WS_MESSAGE));
    let diagnostics = host.check(&text).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let flow = host.open(&text).unwrap();
    host.observe(flow, true).unwrap();
    until("the server's greeting", || {
        reply(&host, flow).as_deref() == Some("hello")
    });
    // The value is also reported on the trigger's own output port.
    let start = Instant::now();
    let mut on_port = false;
    while !on_port {
        assert!(start.elapsed() < WAIT, "no port report");
        if let Ok(event) = events.recv_timeout(Duration::from_millis(50))
            && let orange_host::FlowEvent::Ports(ports) = event.kind
        {
            on_port = ports.iter().any(|(from, port, value)| {
                *from == node(1)
                    && port.to_string() == "payload"
                    && matches!(value, Value::Str(text) if text.as_str() == "hello")
            });
        }
    }
}

#[test]
fn binary_messages_cross_both_ways_and_the_handshake_is_authorized() {
    use tungstenite::handshake::server::{Request, Response};
    const BYTES: &str = "core/bytes@1";
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (seen, authorizations) = std::sync::mpsc::channel();
    let _ = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        #[allow(
            clippy::result_large_err,
            reason = "the callback type is the library's"
        )]
        let callback = |request: &Request, response: Response| {
            let header = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            seen.send(header).unwrap();
            Ok(response)
        };
        let mut socket = tungstenite::accept_hdr(stream, callback).unwrap();
        // Text is ignored by a binary entry; each binary message comes back reversed.
        socket
            .send(Message::text("not for the binary entry"))
            .unwrap();
        while let Ok(message) = socket.read() {
            if let Message::Binary(data) = message {
                let reversed: Vec<u8> = data.iter().rev().copied().collect();
                if socket.send(Message::binary(reversed)).is_err() {
                    break;
                }
            }
        }
    });

    // The password stays in the credential store; the document names it.
    let options = HostOptions {
        secrets: Some(std::sync::Arc::new(Secrets)),
        ..HostOptions::default()
    };
    let (mut host, _events) = Host::with_services(options, vec![orange_net::package()]).unwrap();
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000009".parse().unwrap());
    // user "orange", password "generated-test-value"
    let connection = json!({"id": CONNECTION, "name": "feed", "adapter": orange_net::WS_ADAPTER,
        "scope": "graph", "settings": {"url": format!("ws://{address}"),
            "basic": {"user": "orange", "password": "credential://feed/password"}}});
    let _ = document
        .apply([DocCommand::UpsertConnection(
            serde_json::from_value(connection).unwrap(),
        )])
        .unwrap();
    for (n, definition, inputs) in [
        (1, orange_net::WS_BINARY, json!({})),
        (
            3,
            orange_net::WS_SEND_BYTES,
            json!({"data": {"type": BYTES, "value": "AAEC/w=="}}),
        ),
    ] {
        let mut new = NewNode::new(
            node(n),
            DefinitionRef {
                id: definition.parse().unwrap(),
                version: 1,
            },
        );
        new.config = serde_json::from_value(json!({"connection": CONNECTION})).unwrap();
        new.inputs = serde_json::from_value(inputs).unwrap();
        let edit = new_node_edit(&document, host.registry(), new).unwrap();
        let _ = document.apply(edit.commands).unwrap();
    }
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    for entry in saved["nodes"].as_array_mut().unwrap() {
        if entry["definition_id"] == orange_net::WS_BINARY {
            entry["activation"] = activation();
        }
    }
    let mut keep = core(4, "set-variable");
    keep["config"] = json!({"variable": id(100)});
    saved["nodes"]
        .as_array_mut()
        .unwrap()
        .extend([core(2, "on-manual"), keep]);
    saved["edges"] = json!([
        edge(1, 1, "then", 4, "in"),
        edge(2, 1, "payload", 4, "value"),
        edge(3, 2, "then", 3, "in"),
    ]);
    saved["variables"] = json!([{"id": id(100), "name": "reply", "type": BYTES, "scope": "run"}]);
    let text = to_json(&from_json(&saved.to_string()).unwrap()).unwrap();
    assert!(host.check(&text).unwrap().is_empty());
    let draft = host.draft(&text).unwrap();
    host.approve_connection_credentials_for_session(&draft, CONNECTION)
        .unwrap();

    let flow = host.open(&text).unwrap();
    assert_eq!(
        authorizations.recv_timeout(WAIT).unwrap(),
        "Basic b3JhbmdlOmdlbmVyYXRlZC10ZXN0LXZhbHVl"
    );
    host.trigger(flow, node(2), None).unwrap();
    let variable: VariableId = id(100).parse().unwrap();
    let received = || {
        host.inspect(flow, move |run| run.variable(variable).cloned())
            .unwrap()
    };
    until("the binary reply", || received().is_some());
    assert_eq!(
        received(),
        Some(Value::Bytes(std::sync::Arc::new(vec![255, 2, 1, 0])))
    );
}

/// A document whose nodes `uses` are package nodes on the connection, each `(n,
/// definition)`; `rest` adds core nodes, `edges` and `variables` as saved.
fn built(
    host: &Host,
    connection: Json,
    uses: &[(u32, &str)],
    rest: Json,
    variables: Json,
) -> String {
    let mut document = GraphDocument::new("20000000-0000-4000-8000-0000000000c1".parse().unwrap());
    let decl = serde_json::from_value(connection).unwrap();
    let _ = document
        .apply([DocCommand::UpsertConnection(decl)])
        .unwrap();
    for (n, definition) in uses {
        let mut new = NewNode::new(
            node(*n),
            DefinitionRef {
                id: definition.parse().unwrap(),
                version: 1,
            },
        );
        new.config = serde_json::from_value(json!({"connection": CONNECTION})).unwrap();
        let edit = new_node_edit(&document, host.registry(), new).unwrap();
        let _ = document.apply(edit.commands).unwrap();
    }
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    for entry in saved["nodes"].as_array_mut().unwrap() {
        if entry["definition_id"]
            .as_str()
            .is_some_and(|id| id == orange_net::WS_CONNECTED || id == orange_net::WS_DISCONNECTED)
        {
            entry["activation"] = activation();
        }
    }
    saved["nodes"]
        .as_array_mut()
        .unwrap()
        .extend(rest["nodes"].as_array().unwrap().iter().cloned());
    saved["edges"] = rest["edges"].clone();
    saved["variables"] = variables;
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

fn keep(n: u32, variable: u32) -> Json {
    let mut keep = core(n, "set-variable");
    keep["config"] = json!({"variable": id(variable)});
    keep
}

fn variable(host: &Host, flow: FlowId, n: u32) -> Option<Value> {
    let variable: VariableId = id(n).parse().unwrap();
    host.inspect(flow, move |run| run.variable(variable).cloned())
        .unwrap()
}

#[test]
fn entries_fire_when_the_socket_opens_and_when_the_server_closes_it() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    // Closes every socket as soon as it is open.
    let _ = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut socket = tungstenite::accept(stream.unwrap()).unwrap();
            let _ = socket.close(None);
            while socket.read().is_ok() {}
        }
    });
    let mut host = host();
    let connection = json!({"id": CONNECTION, "name": "peer", "adapter": orange_net::WS_ADAPTER,
        "scope": "graph", "settings": {"url": url}});
    let rest = json!({
        "nodes": [keep(4, 100), keep(5, 101)],
        "edges": [
            edge(1, 1, "then", 4, "in"),
            edge(2, 1, "payload", 4, "value"),
            edge(3, 2, "then", 5, "in"),
            edge(4, 2, "payload", 5, "value"),
        ]
    });
    let variables = json!([
        {"id": id(100), "name": "up", "type": STRING, "scope": "run"},
        {"id": id(101), "name": "down", "type": STRING, "scope": "run"}
    ]);
    let text = built(
        &host,
        connection,
        &[
            (1, orange_net::WS_CONNECTED),
            (2, orange_net::WS_DISCONNECTED),
        ],
        rest,
        variables,
    );
    assert!(host.check(&text).unwrap().is_empty());
    let flow = host.open(&text).unwrap();
    until("both entries", || variable(&host, flow, 101).is_some());
    assert_eq!(
        variable(&host, flow, 100),
        Some(Value::Str(std::sync::Arc::new(url)))
    );
    assert_eq!(
        variable(&host, flow, 101),
        Some(Value::Str(std::sync::Arc::new(
            "closed by the server".into()
        )))
    );
}

#[test]
fn the_status_says_whether_the_socket_is_open_and_how_many_messages_wait() {
    let mut host = host();
    let connection = json!({"id": CONNECTION, "name": "peer", "adapter": orange_net::WS_ADAPTER,
        "scope": "graph", "settings": {"url": server()}});
    let rest = json!({
        "nodes": [core(2, "on-manual"), keep(4, 100), keep(5, 101), keep(6, 102)],
        "edges": [
            edge(1, 1, "then", 4, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 5, "in"),
            edge(4, 3, "connected", 5, "value"),
            edge(5, 5, "then", 6, "in"),
            edge(6, 3, "queued", 6, "value"),
            edge(7, 1, "payload", 4, "value"),
        ]
    });
    let variables = json!([
        {"id": id(100), "name": "up", "type": STRING, "scope": "run"},
        {"id": id(101), "name": "open", "type": "core/bool@1", "scope": "run"},
        {"id": id(102), "name": "queued", "type": "core/i64@1", "scope": "run"}
    ]);
    let text = built(
        &host,
        connection,
        &[(1, orange_net::WS_CONNECTED), (3, orange_net::WS_STATUS)],
        rest,
        variables,
    );
    assert!(host.check(&text).unwrap().is_empty());
    let flow = host.open(&text).unwrap();
    until("the socket", || variable(&host, flow, 100).is_some());
    host.trigger(flow, node(2), None).unwrap();
    until("the status", || variable(&host, flow, 102).is_some());
    assert_eq!(variable(&host, flow, 101), Some(Value::Bool(true)));
    assert_eq!(variable(&host, flow, 102), Some(Value::I64(0)));
}

#[test]
fn a_send_continues_on_a_step_shown_as_queued() {
    let host = host();
    let send = host
        .nodes()
        .find(|node| node.definition.definition.id.as_str() == orange_net::WS_SEND)
        .unwrap();
    let label = send.presentation.unwrap().then_label.as_ref().unwrap();
    assert_eq!(label.fallback, "Queued");
}

/// A package node's message goes through the host's own session: the same bounded
/// queue and socket as Send, with the address and credentials left with the host.
#[test]
fn a_session_queues_package_messages_on_its_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (seen, received) = std::sync::mpsc::channel();
    let _ = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        while let Ok(message) = socket.read() {
            if matches!(message, Message::Text(_) | Message::Binary(_))
                && seen.send(message).is_err()
            {
                break;
            }
        }
    });
    let package = orange_net::package();
    let (_, adapter) = package
        .adapters()
        .iter()
        .find(|(kind, _)| kind == orange_net::WS_ADAPTER)
        .unwrap();
    let settings = json!({"url": format!("ws://{address}")});
    let session = adapter.open(settings.as_object().unwrap()).unwrap();
    session.start();
    session
        .send_message(orange_sdk::Message::Text("from a package".into()))
        .unwrap();
    session
        .send_message(orange_sdk::Message::Binary(vec![1, 2, 3].into()))
        .unwrap();
    assert_eq!(
        received.recv_timeout(WAIT).unwrap(),
        Message::text("from a package")
    );
    assert_eq!(
        received.recv_timeout(WAIT).unwrap(),
        Message::binary(vec![1, 2, 3])
    );
}
