//! AMQP graph integration with a loopback broker and an opt-in RabbitMQ check.

use orange_host::{FlowId, Host, HostOptions, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{DocCommand, GraphDocument, from_json, to_json};
use orange_testkit::{WAIT, activation, core, edge, id, node, set, until};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CONNECTION: &str = "40000000-0000-4000-8000-000000000001";
const STRING: &str = "core/string@1";
const BYTES: &str = "core/bytes@1";
const TEXT_DELIVERY: &str = "org.orange.net/amqp-text-delivery@1";
const BYTES_DELIVERY: &str = "org.orange.net/amqp-binary-delivery@1";

struct Broker {
    queues: Mutex<BTreeMap<String, Sender<Vec<u8>>>>,
    subscribed: Sender<String>,
    issued: Sender<Vec<u8>>,
    confirmed: Mutex<Receiver<()>>,
    acknowledged: Sender<(String, u64)>,
}

fn frame(kind: u8, channel: u16, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![kind];
    bytes.extend_from_slice(&channel.to_be_bytes());
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend_from_slice(body);
    bytes.push(0xce);
    bytes
}

fn method(stream: &mut TcpStream, ch: u16, class: u16, id: u16, fields: &[u8]) {
    let mut body = class.to_be_bytes().to_vec();
    body.extend_from_slice(&id.to_be_bytes());
    body.extend_from_slice(fields);
    stream.write_all(&frame(1, ch, &body)).unwrap();
}

fn read_frame(stream: &mut TcpStream) -> Option<(u8, u16, Vec<u8>)> {
    let mut header = [0; 7];
    stream.read_exact(&mut header).ok()?;
    let size = u32::from_be_bytes(header[3..].try_into().unwrap()) as usize;
    assert!(size <= 4096 - 8);
    let mut data = vec![0; size + 1];
    stream.read_exact(&mut data).unwrap();
    assert_eq!(data.pop(), Some(0xce));
    Some((
        header[0],
        u16::from_be_bytes(header[1..3].try_into().unwrap()),
        data,
    ))
}

fn read_method(stream: &mut TcpStream, ch: u16, class: u16, id: u16) -> Vec<u8> {
    let (kind, channel, data) = read_frame(stream).expect("a frame");
    assert_eq!(
        (kind, channel, &data[..4]),
        (
            1,
            ch,
            &[(class >> 8) as u8, class as u8, (id >> 8) as u8, id as u8,][..]
        )
    );
    data[4..].to_vec()
}

fn short(out: &mut Vec<u8>, value: &str) {
    out.push(value.len() as u8);
    out.extend_from_slice(value.as_bytes());
}

fn take_short<'a>(bytes: &mut &'a [u8]) -> &'a str {
    let size = usize::from(bytes[0]);
    let (value, rest) = bytes[1..].split_at(size);
    *bytes = rest;
    std::str::from_utf8(value).unwrap()
}

fn handshake(stream: &mut TcpStream) {
    let mut protocol = [0; 8];
    stream.read_exact(&mut protocol).unwrap();
    assert_eq!(&protocol, b"AMQP\0\0\x09\x01");
    let mut start = vec![0, 9, 0, 0, 0, 0]; // version and empty server properties
    start.extend_from_slice(b"\0\0\0\x05PLAIN\0\0\0\x05en_US");
    method(stream, 0, 10, 10, &start);
    let login = read_method(stream, 0, 10, 11);
    assert!(
        login
            .windows(b"\0guest\0guest".len())
            .any(|part| part == b"\0guest\0guest")
    );
    method(stream, 0, 10, 30, &[0, 8, 0, 0, 16, 0, 0, 30]);
    assert_eq!(read_method(stream, 0, 10, 31), [0, 1, 0, 0, 16, 0, 0, 30]);
    assert_eq!(read_method(stream, 0, 10, 40), [1, b'/', 0, 0]);
    method(stream, 0, 10, 41, &[0]);
    assert_eq!(read_method(stream, 1, 20, 10), [0]);
    method(stream, 1, 20, 11, &[0; 4]);
}

fn send_delivery(stream: &mut TcpStream, queue: &str, body: &[u8]) {
    let mut args = Vec::new();
    short(&mut args, "mock-consumer");
    args.extend_from_slice(&17u64.to_be_bytes());
    args.push(1); // the record exposes the broker's redelivery bit
    short(&mut args, "");
    short(&mut args, queue);
    method(stream, 1, 60, 60, &args);
    let mut header = vec![0, 60, 0, 0];
    header.extend_from_slice(&(body.len() as u64).to_be_bytes());
    header.extend_from_slice(&[0, 0]);
    stream.write_all(&frame(2, 1, &header)).unwrap();
    for part in body.chunks(4096 - 8) {
        stream.write_all(&frame(3, 1, part)).unwrap();
    }
}

fn broker_consume(stream: &mut TcpStream, broker: &Broker, qos: &[u8]) {
    assert_eq!(qos, [0, 0, 0, 0, 0, 16, 0]);
    method(stream, 1, 60, 11, &[]);
    let request = read_method(stream, 1, 60, 20);
    let queue = take_short(&mut &request[2..]).to_owned();
    let (send, receive) = mpsc::channel();
    let _ = broker.queues.lock().unwrap().insert(queue.clone(), send);
    method(stream, 1, 60, 21, b"\x0dmock-consumer");
    broker.subscribed.send(queue.clone()).unwrap();
    if let Ok(body) = receive.recv_timeout(WAIT) {
        send_delivery(stream, &queue, &body);
        let ack = read_method(stream, 1, 60, 80);
        assert_eq!(ack[8], 0);
        let tag = u64::from_be_bytes(ack[..8].try_into().unwrap());
        broker.acknowledged.send((queue, tag)).unwrap();
    }
}

fn broker_publish(stream: &mut TcpStream, broker: &Broker, ch: u16, confirm: &[u8]) {
    assert_eq!((ch, confirm), (1, &[0][..]));
    method(stream, 1, 85, 11, &[]);
    for seq in 1u64.. {
        let Some((kind, ch, data)) = read_frame(stream) else {
            return;
        };
        assert_eq!((kind, ch, &data[..4]), (1, 1, &[0, 60, 0, 40][..]));
        let mut args = &data[6..];
        assert_eq!(take_short(&mut args), "");
        let route = take_short(&mut args);
        assert_eq!(args, [1], "publish is mandatory");
        let (kind, channel, header) = read_frame(stream).unwrap();
        assert_eq!((kind, channel, &header[..4]), (2, 1, &[0, 60, 0, 0][..]));
        let size = u64::from_be_bytes(header[4..12].try_into().unwrap()) as usize;
        let mut body = Vec::new();
        while body.len() < size {
            let (kind, channel, part) = read_frame(stream).unwrap();
            assert_eq!((kind, channel), (3, 1));
            body.extend_from_slice(&part);
        }
        assert_eq!(body.len(), size);
        broker.issued.send(body.clone()).unwrap();
        if route == "missing" {
            let mut returned = vec![1, 56]; // 312 NO_ROUTE
            short(&mut returned, "NO_ROUTE");
            short(&mut returned, "");
            short(&mut returned, route);
            method(stream, 1, 60, 50, &returned);
            return;
        }
        broker.confirmed.lock().unwrap().recv_timeout(WAIT).unwrap();
        let mut ack = seq.to_be_bytes().to_vec();
        ack.push(0);
        method(stream, 1, 60, 80, &ack);
        broker.queues.lock().unwrap()[route].send(body).unwrap();
    }
}

struct Fixture {
    url: String,
    subscribed: Receiver<String>,
    issued: Receiver<Vec<u8>>,
    confirm: Sender<()>,
    acknowledged: Receiver<(String, u64)>,
}

fn broker() -> Fixture {
    let listening = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("amqp://{}", listening.local_addr().unwrap());
    let (subscribed, subscriptions) = mpsc::channel();
    let (issued, publishes) = mpsc::channel();
    let (confirm, confirmed) = mpsc::channel();
    let (acknowledged, acknowledgements) = mpsc::channel();
    let broker = Arc::new(Broker {
        queues: Mutex::default(),
        subscribed,
        issued,
        confirmed: Mutex::new(confirmed),
        acknowledged,
    });
    let _ = std::thread::spawn(move || {
        for stream in listening.incoming() {
            let broker = Arc::clone(&broker);
            let _ = std::thread::spawn(move || {
                let mut stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                handshake(&mut stream);
                let Some((kind, ch, data)) = read_frame(&mut stream) else {
                    return;
                };
                assert_eq!((kind, ch), (1, 1));
                match &data[..4] {
                    [0, 60, 0, 10] => broker_consume(&mut stream, &broker, &data[4..]),
                    [0, 85, 0, 10] => broker_publish(&mut stream, &broker, ch, &data[4..]),
                    method => panic!("unexpected first method {method:?}"),
                }
            });
        }
    });
    Fixture {
        url,
        subscribed: subscriptions,
        issued: publishes,
        confirm,
        acknowledged: acknowledgements,
    }
}

fn graph(host: &Host, settings: Json, queue: &str, route: &str, binary: bool) -> String {
    let mut doc = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let connection = json!({"id": CONNECTION, "name": "broker", "adapter": orange_net::AMQP_ADAPTER,
        "scope": "graph", "settings": settings});
    let _ = doc
        .apply([DocCommand::UpsertConnection(
            serde_json::from_value(connection).unwrap(),
        )])
        .unwrap();
    let (entry, publish, ty, body, data) = if binary {
        (
            orange_net::AMQP_BINARY,
            orange_net::AMQP_PUBLISH_BYTES,
            BYTES_DELIVERY,
            "data",
            json!({"type": BYTES, "value": "AP9/"}),
        )
    } else {
        (
            orange_net::AMQP_MESSAGE,
            orange_net::AMQP_PUBLISH,
            TEXT_DELIVERY,
            "text",
            json!({"type": STRING, "value": "a".repeat(9000)}),
        )
    };
    for (n, definition, inputs) in [
        (1, entry, json!({"queue": {"type": STRING, "value": queue}})),
        (
            3,
            publish,
            json!({"routing_key": {"type": STRING, "value": route}, (body): data}),
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
        let edit = new_node_edit(&doc, host.registry(), new).unwrap();
        let _ = doc.apply(edit.commands).unwrap();
    }
    let mut saved: Json = serde_json::from_str(&to_json(&doc).unwrap()).unwrap();
    for n in saved["nodes"].as_array_mut().unwrap() {
        if n["definition_id"] == entry {
            n["activation"] = activation();
        }
    }
    let mark = |n, variable| {
        let mut node = set(n, variable);
        node["inputs"] = json!({"value": {"type": "core/bool@1", "value": true}});
        node
    };
    saved["nodes"].as_array_mut().unwrap().extend([
        core(2, "on-manual"),
        set(4, 100),
        mark(5, 101),
        mark(6, 102),
    ]);
    saved["edges"] = json!([
        edge(1, 2, "then", 3, "in"),
        edge(2, 1, "then", 4, "in"),
        edge(3, 1, "payload", 4, "value"),
        edge(4, 3, "then", 5, "in"),
        edge(5, 3, "failed", 6, "in"),
    ]);
    saved["variables"] = json!([
        {"id": id(100), "name": "delivery", "type": ty, "scope": "run"},
        {"id": id(101), "name": "confirmed", "type": "core/bool@1", "scope": "run"},
        {"id": id(102), "name": "failed", "type": "core/bool@1", "scope": "run"},
    ]);
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

fn read(host: &Host, flow: FlowId, n: u32) -> Option<Value> {
    let variable: VariableId = id(n).parse().unwrap();
    host.inspect(flow, move |run| run.variable(variable).cloned())
        .unwrap()
}

fn host() -> Host {
    Host::with_services(HostOptions::default(), vec![orange_net::package()])
        .unwrap()
        .0
}

#[test]
fn a_graph_waits_for_confirms_and_acknowledges_text_and_binary_deliveries() {
    let test = broker();
    for (binary, queue, body) in [
        (false, "alpha", vec![b'a'; 9000]),
        (true, "beta", vec![0, 255, 127]),
    ] {
        let mut host = host();
        let settings = json!({"url": test.url, "queue": "unused-default"});
        let graph = graph(&host, settings, queue, queue, binary);
        let diagnostics = host.check(&graph).unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let flow = host.open(&graph).unwrap();
        assert_eq!(test.subscribed.recv_timeout(WAIT).unwrap(), queue);
        host.trigger(flow, node(2), None).unwrap();
        assert_eq!(test.issued.recv_timeout(WAIT).unwrap(), body);
        assert_ne!(read(&host, flow, 101), Some(Value::Bool(true)));
        test.confirm.send(()).unwrap();
        until("publish confirmation", || {
            read(&host, flow, 101) == Some(Value::Bool(true))
        });
        assert_eq!(
            test.acknowledged.recv_timeout(WAIT).unwrap(),
            (queue.to_owned(), 17)
        );
        until("AMQP delivery", || read(&host, flow, 100).is_some());
        let Some(Value::Record(record)) = read(&host, flow, 100) else {
            panic!("delivery record")
        };
        let payload = if binary {
            Value::Bytes(Arc::new(body))
        } else {
            Value::Str(Arc::new(String::from_utf8(body).unwrap()))
        };
        assert_eq!(
            &record.fields[..],
            [
                Some(Value::Str(Arc::new(String::new()))),
                Some(Value::Str(Arc::new(queue.to_owned()))),
                Some(Value::Bool(true)),
                Some(payload),
            ]
        );
    }
}

#[test]
fn an_unroutable_mandatory_publish_uses_failed() {
    let test = broker();
    let mut host = host();
    let graph = graph(&host, json!({"url": test.url}), "alpha", "missing", false);
    let flow = host.open(&graph).unwrap();
    let _ = test.subscribed.recv_timeout(WAIT).unwrap();
    host.trigger(flow, node(2), None).unwrap();
    let _ = test.issued.recv_timeout(WAIT).unwrap();
    until("AMQP publish failure", || {
        read(&host, flow, 102) == Some(Value::Bool(true))
    });
    assert_ne!(read(&host, flow, 101), Some(Value::Bool(true)));
}

#[test]
#[cfg(feature = "network-service-tests")]
fn real_rabbitmq_delivers_confirmed_publish() {
    let url = std::env::var("ORANGE_AMQP_BROKER").expect("AMQP broker endpoint");
    let queue = std::env::var("ORANGE_AMQP_QUEUE").expect("dedicated pre-created queue");
    let mut settings = json!({"url": url, "queue": queue, "routing_key": queue});
    for (key, var) in [
        ("username", "ORANGE_AMQP_USER"),
        ("password", "ORANGE_AMQP_PASSWORD"),
        ("ca_file", "ORANGE_AMQP_CA"),
    ] {
        if let Ok(value) = std::env::var(var) {
            settings[key] = json!(value);
        }
    }
    let mut host = host();
    let graph = graph(&host, settings, "", "", false);
    let flow = host.open(&graph).unwrap();
    host.trigger(flow, node(2), None).unwrap();
    until("confirmed real broker publish", || {
        read(&host, flow, 101) == Some(Value::Bool(true))
    });
    until("real broker message", || read(&host, flow, 100).is_some());
}
