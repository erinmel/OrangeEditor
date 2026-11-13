//! A graph publishes on a declared MQTT connection and is triggered by the messages of
//! the topics it subscribes to, against a small broker run by the test.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orange_host::{FlowId, Host, HostOptions, NodeId, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::value::ListRepr;
use orange_model::{DocCommand, GraphDocument, from_json, to_json};
use serde_json::{Value as Json, json};

const STRING: &str = "core/string@1";
const BYTES: &str = "core/bytes@1";
const PAIR: &str = "core/list@1<core/string@1>";
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
fn set(n: u32, target: u32) -> Json {
    let mut node = core(n, "set-variable");
    node["config"] = json!({"variable": id(target)});
    node
}
fn edge(n: u32, from: u32, out: &str, to: u32, input: &str) -> Json {
    json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
        "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": input}})
}
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

// ---- a broker for one test ------------------------------------------------------

/// What the broker saw, for the test to check.
#[derive(Default)]
struct Seen {
    /// First byte of every PUBLISH received, in order.
    publishes: Mutex<Vec<u8>>,
    /// Acknowledgements received for messages the broker delivered.
    acknowledged: AtomicUsize,
    connections: AtomicUsize,
    /// Close the connection on the next acknowledged-delivery PUBLISH, without
    /// acknowledging or delivering it.
    drop_next: AtomicBool,
}
type Subscribers = Arc<Mutex<Vec<(Vec<String>, TcpStream)>>>;

fn string(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
}
fn frame(first: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![first];
    let mut length = body.len();
    loop {
        let mut byte = (length % 128) as u8;
        length /= 128;
        if length > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if length == 0 {
            break;
        }
    }
    out.extend_from_slice(body);
    out
}
fn read_packet(stream: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
    let mut byte = [0u8; 1];
    stream.read_exact(&mut byte).ok()?;
    let first = byte[0];
    let (mut length, mut shift) = (0usize, 0);
    loop {
        stream.read_exact(&mut byte).ok()?;
        length |= usize::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body).ok()?;
    Some((first, body))
}
fn take<'a>(body: &mut &'a [u8]) -> &'a [u8] {
    let length = usize::from(u16::from_be_bytes([body[0], body[1]]));
    let (value, rest) = body[2..].split_at(length);
    *body = rest;
    value
}
fn matches(filter: &str, topic: &str) -> bool {
    let (mut filters, mut levels) = (filter.split('/'), topic.split('/'));
    loop {
        match (filters.next(), levels.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

fn client(
    mut stream: TcpStream,
    subscribers: &Subscribers,
    seen: &Seen,
    logins: &Sender<(String, String, String)>,
) {
    while let Some((first, body)) = read_packet(&mut stream) {
        match first >> 4 {
            1 => {
                let flags = body[7];
                let mut rest = &body[10..];
                let text = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).unwrap();
                let client_id = text(take(&mut rest));
                let user = if flags & 0x80 != 0 {
                    text(take(&mut rest))
                } else {
                    String::new()
                };
                let password = if flags & 0x40 != 0 {
                    text(take(&mut rest))
                } else {
                    String::new()
                };
                assert_eq!(&body[..7], [0, 4, b'M', b'Q', b'T', b'T', 4]);
                let _ = seen.connections.fetch_add(1, Ordering::SeqCst);
                logins.send((client_id, user, password)).unwrap();
                stream.write_all(&[0x20, 2, 0, 0]).unwrap();
            }
            8 => {
                let mut rest = &body[2..];
                let mut filters = Vec::new();
                while !rest.is_empty() {
                    filters.push(String::from_utf8(take(&mut rest).to_vec()).unwrap());
                    rest = &rest[1..];
                }
                let mut reply = vec![body[0], body[1]];
                reply.extend(filters.iter().map(|_| 1u8));
                stream.write_all(&frame(0x90, &reply)).unwrap();
                subscribers
                    .lock()
                    .unwrap()
                    .push((filters, stream.try_clone().unwrap()));
            }
            3 => {
                seen.publishes.lock().unwrap().push(first);
                let qos = (first >> 1) & 3;
                let mut rest = &body[..];
                let topic = String::from_utf8(take(&mut rest).to_vec()).unwrap();
                if qos == 1 {
                    if seen.drop_next.swap(false, Ordering::SeqCst) {
                        return;
                    }
                    stream.write_all(&[0x40, 2, rest[0], rest[1]]).unwrap();
                    rest = &rest[2..];
                }
                // Deliver at the publisher's quality of service, with packet ID 77.
                let mut delivery = Vec::new();
                string(&mut delivery, topic.as_bytes());
                if qos == 1 {
                    delivery.extend_from_slice(&77u16.to_be_bytes());
                }
                delivery.extend_from_slice(rest);
                let delivery = frame(0x30 | (qos << 1), &delivery);
                for (filters, subscriber) in subscribers.lock().unwrap().iter_mut() {
                    if filters.iter().any(|filter| matches(filter, &topic)) {
                        let _ = subscriber.write_all(&delivery);
                    }
                }
            }
            4 => {
                assert_eq!(body, 77u16.to_be_bytes());
                let _ = seen.acknowledged.fetch_add(1, Ordering::SeqCst);
            }
            12 => stream.write_all(&[0xd0, 0]).unwrap(),
            14 => return,
            other => panic!("unexpected packet type {other}"),
        }
    }
}

fn broker() -> (String, Arc<Seen>, Receiver<(String, String, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Seen::default());
    let (logins, received) = channel();
    let subscribers: Subscribers = Arc::default();
    let shared = Arc::clone(&seen);
    let _ = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let (subscribers, seen, logins) = (
                Arc::clone(&subscribers),
                Arc::clone(&shared),
                logins.clone(),
            );
            let _ = std::thread::spawn(move || {
                let stream = stream.unwrap();
                let peer = stream.peer_addr().ok();
                client(stream, &subscribers, &seen, &logins);
                // A closed connection no longer receives.
                subscribers
                    .lock()
                    .unwrap()
                    .retain(|(_, subscriber)| subscriber.peer_addr().ok() != peer);
            });
        }
    });
    (format!("mqtt://{address}"), seen, received)
}

/// A credential store holding the broker password.
#[derive(Debug)]
struct Secrets;
impl orange_host::SecretStore for Secrets {
    fn get(&self, name: &str) -> Result<String, orange_host::SecretError> {
        if name == "mqtt/password" {
            Ok("generated-broker-value".to_owned())
        } else {
            Err(orange_host::SecretError::NotFound)
        }
    }
}

/// Text messages are stored in variable 100 and binary ones in 101. Manual entry 2
/// publishes `hello` to `orange/test`; manual entry 6 publishes two bytes to
/// `orange/raw`.
fn document(host: &Host, url: &str) -> String {
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let connection = json!({"id": CONNECTION, "name": "broker", "adapter": orange_net::MQTT_ADAPTER,
        "scope": "graph", "settings": {"url": url, "client_id": "orange-test",
            "username": "orange", "password": "credential://mqtt/password",
            "topics": ["orange/#"], "qos": 1}});
    let _ = document
        .apply([DocCommand::UpsertConnection(
            serde_json::from_value(connection).unwrap(),
        )])
        .unwrap();
    let no = json!({"type": "core/bool@1", "value": false});
    for (n, definition, inputs) in [
        (1, orange_net::MQTT_MESSAGE, json!({})),
        (5, orange_net::MQTT_BINARY, json!({})),
        (
            3,
            orange_net::MQTT_PUBLISH,
            json!({"topic": {"type": STRING, "value": "orange/test"},
                "text": {"type": STRING, "value": "hello"}, "retain": no}),
        ),
        (
            7,
            orange_net::MQTT_PUBLISH_BYTES,
            json!({"topic": {"type": STRING, "value": "orange/raw"},
                "data": {"type": BYTES, "value": "AP8="}, "retain": no}),
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
        if entry["definition_id"] == orange_net::MQTT_MESSAGE
            || entry["definition_id"] == orange_net::MQTT_BINARY
        {
            entry["activation"] = activation();
        }
    }
    saved["nodes"].as_array_mut().unwrap().extend([
        core(2, "on-manual"),
        core(6, "on-manual"),
        set(4, 100),
        set(8, 101),
    ]);
    saved["edges"] = json!([
        edge(1, 1, "then", 4, "in"),
        edge(2, 1, "payload", 4, "value"),
        edge(3, 5, "then", 8, "in"),
        edge(4, 5, "payload", 8, "value"),
        edge(5, 2, "then", 3, "in"),
        edge(6, 6, "then", 7, "in"),
    ]);
    saved["variables"] = json!([
        {"id": id(100), "name": "text", "type": PAIR, "scope": "run"},
        {"id": id(101), "name": "raw", "type": BYTES, "scope": "run"}
    ]);
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

fn read(host: &Host, flow: FlowId, n: u32) -> Option<Value> {
    let variable: VariableId = id(n).parse().unwrap();
    host.inspect(flow, move |run| run.variable(variable).cloned())
        .unwrap()
}
fn text_pair(host: &Host, flow: FlowId) -> Option<Vec<String>> {
    match read(host, flow, 100)? {
        Value::List(list) => match list.as_ref() {
            ListRepr::Str(items) => {
                Some(items.iter().map(|item| item.as_str().to_owned()).collect())
            }
            _ => None,
        },
        _ => None,
    }
}
fn host() -> Host {
    let options = HostOptions {
        secrets: Some(Arc::new(Secrets)),
        ..HostOptions::default()
    };
    Host::with_services(options, vec![orange_net::package()])
        .unwrap()
        .0
}

#[test]
fn a_graph_publishes_and_is_triggered_by_the_topics_it_subscribes_to() {
    let (url, seen, logins) = broker();
    let mut host = host();
    let text = document(&host, &url);
    assert!(host.check(&text).unwrap().is_empty());
    // The document holds the reference, not the password.
    assert!(!text.contains("generated-broker-value"));

    let flow = host.open(&text).unwrap();
    assert_eq!(
        logins.recv_timeout(WAIT).unwrap(),
        (
            "orange-test".to_owned(),
            "orange".to_owned(),
            "generated-broker-value".to_owned()
        )
    );
    host.trigger(flow, node(2), None).unwrap();
    until("the text message", || text_pair(&host, flow).is_some());
    assert_eq!(text_pair(&host, flow).unwrap(), ["orange/test", "hello"]);
    // The binary entry receives every message; this one happens to be text.
    until("the same message as bytes", || {
        read(&host, flow, 101) == Some(Value::Bytes(Arc::new(b"hello".to_vec())))
    });

    host.trigger(flow, node(6), None).unwrap();
    until("the binary message", || {
        read(&host, flow, 101) == Some(Value::Bytes(Arc::new(vec![0, 255])))
    });
    // Bytes that are not text do not reach the text entry.
    assert_eq!(text_pair(&host, flow).unwrap(), ["orange/test", "hello"]);
    // Both were published with acknowledgement, and both deliveries were acknowledged.
    assert_eq!(*seen.publishes.lock().unwrap(), [0x32, 0x32]);
    until("the acknowledgements", || {
        seen.acknowledged.load(Ordering::SeqCst) == 2
    });
}

#[test]
fn an_unacknowledged_message_is_sent_again_after_reconnecting() {
    let (url, seen, logins) = broker();
    let mut host = host();
    let text = document(&host, &url);
    let flow = host.open(&text).unwrap();
    let _ = logins.recv_timeout(WAIT).unwrap();

    // The broker drops the connection instead of acknowledging the message.
    seen.drop_next.store(true, Ordering::SeqCst);
    host.trigger(flow, node(2), None).unwrap();
    // The session reconnects, subscribes again and repeats it as a duplicate.
    let _ = logins.recv_timeout(WAIT).unwrap();
    until("the repeated message", || text_pair(&host, flow).is_some());
    assert_eq!(text_pair(&host, flow).unwrap(), ["orange/test", "hello"]);
    assert_eq!(*seen.publishes.lock().unwrap(), [0x32, 0x3a]);
    assert_eq!(seen.connections.load(Ordering::SeqCst), 2);
}

/// Runs against a real broker when one is named:
/// `ORANGE_MQTT_BROKER=mqtt://localhost:1883 cargo test -p orange-net --test mqtt -- --ignored`.
#[test]
#[ignore = "needs a real MQTT broker in ORANGE_MQTT_BROKER"]
fn a_real_broker_delivers_what_the_graph_publishes() {
    let url = std::env::var("ORANGE_MQTT_BROKER").expect("ORANGE_MQTT_BROKER is set");
    let mut host = host();
    let text = document(&host, &url).replace("orange-test", "orange-real-broker-test");
    let flow = host.open(&text).unwrap();
    // Publishing waits in the queue until the session is connected and subscribed.
    std::thread::sleep(Duration::from_secs(2));
    host.trigger(flow, node(2), None).unwrap();
    until("the text message", || text_pair(&host, flow).is_some());
    assert_eq!(text_pair(&host, flow).unwrap(), ["orange/test", "hello"]);
}
