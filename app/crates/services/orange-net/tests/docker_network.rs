//! The graph a user can open in the editor, exercised against the Docker lab.
use orange_host::{Host, HostOptions, SecretError, SecretStore, Value};
use orange_model::value::ListRepr;
use orange_testkit::until;
use serde_json::{Value as Json, json};
use std::sync::Arc;

const GRAPH: &str = include_str!("../../../apps/orange-cli/graphs/network-lab.json");
const BYTES: [u8; 4] = [0, 255, 128, 10];
const HTTP_BODY: &str = r#"{"source":"orange-network-lab","ok":true}"#;

// Deliberately synthetic values for the local fixtures only.
#[derive(Debug)]
struct LabSecrets;

fn open(host: &mut Host, graph: &str) -> orange_host::FlowId {
    let draft = host.draft(graph).unwrap();
    for connection in host.declarations(&draft).connections {
        if let Some(id) = connection.id {
            host.approve_connection_credentials_for_session(&draft, &id)
                .unwrap();
        }
    }
    host.open(graph).unwrap()
}

impl SecretStore for LabSecrets {
    fn get(&self, name: &str) -> Result<String, SecretError> {
        Ok(match name {
            "lab/http" => "lab-http-only",
            "lab/bytes" => "lab-bytes-only",
            "lab/stream" => "lab-stream-only",
            "lab/websocket" => "lab-websocket-only",
            "lab/mqtt" => "lab-mqtt-only",
            "lab/amqp" => "lab-amqp-only",
            "lab/wrong" => "invalid-lab-value",
            _ => return Err(SecretError::NotFound),
        }
        .to_owned())
    }
}

fn host() -> Host {
    Host::with_services(
        HostOptions {
            secrets: Some(Arc::new(LabSecrets)),
            ..HostOptions::default()
        },
        vec![orange_net::package()],
    )
    .unwrap()
    .0
}

fn protected_graph(reject_http: bool) -> String {
    let mut document: Json = serde_json::from_str(GRAPH).unwrap();
    let profiles = document["connections"].as_array_mut().unwrap();
    let http = profiles
        .iter()
        .find(|profile| profile["name"] == "HTTP")
        .unwrap()
        .clone();
    for (offset, name) in [(2, "HTTP bytes"), (3, "HTTP stream")] {
        let mut profile = http.clone();
        profile["id"] = id(1000 + offset).into();
        profile["name"] = name.into();
        profiles.push(profile);
    }
    for profile in profiles {
        let protected = match profile["name"].as_str().unwrap() {
            "HTTP" => json!({
                "base_url": "http://127.0.0.1:18080/basic", "timeout_ms": 2000,
                "basic": {"user": "lab", "password": if reject_http { "credential://lab/wrong" } else { "credential://lab/http" }}
            }),
            "HTTP bytes" => json!({
                "base_url": "http://127.0.0.1:18080/key", "timeout_ms": 2000,
                "api_key": {"header": "X-API-Key", "token": "credential://lab/bytes"}
            }),
            "HTTP stream" => json!({
                "base_url": "http://127.0.0.1:18080/bearer", "timeout_ms": 2000,
                "stream_timeout_ms": 5000, "bearer": "credential://lab/stream"
            }),
            "WebSocket" => json!({
                "url": "ws://127.0.0.1:18081/auth/echo", "bearer": "credential://lab/websocket"
            }),
            "MQTT" => json!({
                "url": "mqtt://127.0.0.1:11884", "qos": 1, "username": "lab", "password": "credential://lab/mqtt"
            }),
            "AMQP" => json!({
                "url": "amqp://127.0.0.1:15673/orange-auth",
                "username": "lab", "password": "credential://lab/amqp"
            }),
            name => panic!("unrecognized lab connection: {name}"),
        };
        profile["settings"] = protected;
    }
    for node in document["nodes"].as_array_mut().unwrap() {
        let profile = match node["definition_id"].as_str() {
            Some(orange_net::HTTP_REQUEST_BYTES) => 1002,
            Some(orange_net::HTTP_STREAM | orange_net::HTTP_STREAM_LINE) => 1003,
            _ => continue,
        };
        node["config"]["connection"] = id(profile).into();
    }
    document.to_string()
}

fn id(n: u32) -> String {
    format!("4e657477-6f72-4000-8000-{n:012x}")
}

fn text(text: &str) -> Value {
    Value::Str(Arc::new(text.to_owned()))
}

#[test]
fn the_saved_lab_and_its_authenticated_profiles_validate_without_docker() {
    let host = host();
    for (graph, count) in [
        (GRAPH.to_owned(), 4),
        (protected_graph(false), 6),
        (protected_graph(true), 6),
    ] {
        assert_eq!(host.check(&graph).unwrap(), []);
        assert_eq!(
            host.declarations(&host.draft(&graph).unwrap())
                .connections
                .len(),
            count
        );
    }
}

#[test]
#[ignore = "requires the Docker network lab and access to the published loopback ports"]
fn the_saved_lab_exchanges_messages_with_and_without_explicit_authentication() {
    let mut host = host();
    let bytes = Value::Bytes(Arc::new(BYTES.to_vec()));
    let expected = [
        (1, Value::I64(200)),
        (2, text(HTTP_BODY)),
        (3, bytes.clone()),
        (4, Value::I64(2)),
        (5, text(r#"{"sample":2}"#)),
        (6, text("hello WebSocket")),
        (7, bytes.clone()),
        (9, bytes.clone()),
        (12, Value::Bool(true)),
        (13, Value::Bool(true)),
    ];

    for (label, graph) in [
        ("baseline", GRAPH.to_owned()),
        ("authenticated", protected_graph(false)),
    ] {
        let flow = open(&mut host, &graph);
        let read = |n: u32| {
            let variable = id(300 + n).parse().unwrap();
            host.inspect(flow, move |run| run.variable(variable).cloned())
                .unwrap()
        };
        for n in [1, 11, 21, 31, 41, 51, 61, 71, 81] {
            host.trigger(flow, id(100 + n).parse().unwrap(), None)
                .unwrap();
        }
        for (n, value) in &expected {
            until(
                &format!("{label} variable {n} to contain the expected value"),
                || read(*n).as_ref() == Some(value),
            );
        }

        until(&format!("{label} MQTT text"), || read(8).is_some());
        let Some(Value::List(pair)) = read(8) else {
            panic!("{label}: expected MQTT topic and text");
        };
        let ListRepr::Str(pair) = pair.as_ref() else {
            panic!("{label}: expected MQTT strings");
        };
        assert_eq!(
            pair.iter().map(|entry| entry.as_str()).collect::<Vec<_>>(),
            ["orange/lab/text", "hello MQTT"]
        );

        for (n, routing_key, payload) in [
            (10, "orange.lab.text", text("hello AMQP")),
            (11, "orange.lab.binary", bytes.clone()),
        ] {
            until(&format!("{label} AMQP delivery {routing_key}"), || {
                read(n).is_some()
            });
            let Some(Value::Record(delivery)) = read(n) else {
                panic!("{label}: expected AMQP delivery record");
            };
            assert_eq!(delivery.fields[0], Some(text("")));
            assert_eq!(delivery.fields[1], Some(text(routing_key)));
            assert_eq!(delivery.fields[3], Some(payload));
        }
        host.close(flow).unwrap();
        eprintln!(
            "{label}: HTTP JSON, bytes, 2 stream lines, WebSocket and MQTT text/bytes, AMQP confirms and deliveries"
        );
    }

    let flow = open(&mut host, &protected_graph(true));
    host.trigger(flow, id(101).parse().unwrap(), None).unwrap();
    let status = id(301).parse().unwrap();
    let response = id(302).parse().unwrap();
    until(
        "a rejected HTTP credential to produce 401 and an explanatory response",
        || {
            host.inspect(flow, move |run| {
                run.variable(status) == Some(&Value::I64(401))
                    && run.variable(response) == Some(&text("credentials rejected"))
            })
            .unwrap()
        },
    );
    host.close(flow).unwrap();
    let rejected = tungstenite::connect("ws://127.0.0.1:18081/auth/echo");
    assert!(
        matches!(rejected, Err(tungstenite::Error::Http(response)) if response.status().as_u16() == 401)
    );
    eprintln!("invalid HTTP credentials and missing WebSocket credentials both returned 401");
}
