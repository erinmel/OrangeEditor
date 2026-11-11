//! A graph sends on a declared WebSocket connection and is triggered by what arrives.
use std::net::TcpListener;
use std::time::{Duration, Instant};

use orange_host::{FlowId, Host, HostOptions, NodeId, RunState, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{DocCommand, GraphDocument, from_json, to_json};
use serde_json::{Value as Json, json};
use tungstenite::Message;

const STRING: &str = "core/string@1";
const CONNECTION: &str = "40000000-0000-4000-8000-000000000001";
const WAIT: Duration = Duration::from_secs(20);

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn node(n: u32) -> NodeId {
    id(n).parse().unwrap()
}
fn activation() -> Json {
    json!({"mode": "queue", "capacity": 8, "overflow": "fault", "on_fault": "continue", "order": 0})
}
fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id": id(n), "definition_id": format!("core/{name}"),
        "definition_version": 1, "config_version": 1, "config": {}});
    if name.starts_with("on-") {
        node["activation"] = activation();
    }
    node
}
fn edge(n: u32, from: u32, out: &str, to: u32, input: &str) -> Json {
    json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
        "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": input}})
}

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
