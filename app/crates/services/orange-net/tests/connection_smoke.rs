//! The same graph a user can open in the desktop editor.
use orange_host::{Host, HostOptions, Value};
use orange_testkit::{node, until, variable};

const GRAPH: &str = include_str!("../../../apps/orange-cli/graphs/connection-smoke.json");

#[test]
fn saved_connection_graph_runs_json_feedback_and_failure_routes_without_a_server() {
    let (mut host, _) =
        Host::with_services(HostOptions::default(), vec![orange_net::package()]).unwrap();
    assert_eq!(host.check(GRAPH).unwrap(), []);
    let flow = host.open(GRAPH).unwrap();
    let read = |n| {
        host.inspect(flow, move |run| run.variable(variable(n)).cloned())
            .unwrap()
    };

    for _ in 0..3 {
        host.trigger(flow, node(1), None).unwrap();
    }
    until("three JSON runs and their feedback", || {
        read(102) == Some(Value::U64(3))
            && read(103) == Some(Value::U64(1))
            && read(104) == Some(Value::U64(2))
    });
    let Some(Value::Json(member)) = read(100) else {
        panic!("expected a present JSON member");
    };
    assert_eq!(member.as_str(), "null");
    assert!(matches!(read(101), Some(Value::Err(_))));

    host.trigger(flow, node(40), None).unwrap();
    until("the invalid request failure branch", || {
        read(114) == Some(Value::I64(-1))
    });
    host.close(flow).unwrap();
}

#[test]
fn the_same_graph_parses_http_and_websocket_responses() {
    use serde_json::Value as Json;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::Duration;
    use tungstenite::Message;

    let http = TcpListener::bind("127.0.0.1:0").unwrap();
    let ws = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut graph: Json = serde_json::from_str(GRAPH).unwrap();
    for profile in graph["connections"].as_array_mut().unwrap() {
        match profile["name"].as_str().unwrap() {
            "Local HTTP" => {
                profile["settings"]["base_url"] =
                    format!("http://{}", http.local_addr().unwrap()).into();
            }
            "Local WebSocket" => {
                profile["settings"]["url"] =
                    format!("ws://{}/echo", ws.local_addr().unwrap()).into();
            }
            _ => {}
        }
    }

    let answer = std::thread::spawn(move || {
        let (mut socket, _) = http.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = BufReader::new(&socket);
        let mut line = String::new();
        request.read_line(&mut line).unwrap();
        assert_eq!(line.trim_end(), "GET /demo HTTP/1.1");
        loop {
            line.clear();
            let read = request.read_line(&mut line).unwrap();
            assert_ne!(read, 0);
            if line == "\r\n" {
                break;
            }
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
    });
    let echo = std::thread::spawn(move || {
        let (socket, _) = ws.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut socket = tungstenite::accept(socket).unwrap();
        let message = socket.read().unwrap();
        assert_eq!(message, Message::text(r#"{"echo": true}"#));
        socket.send(message).unwrap();
    });

    let (mut host, _) =
        Host::with_services(HostOptions::default(), vec![orange_net::package()]).unwrap();
    let flow = host.open(&graph.to_string()).unwrap();
    host.trigger(flow, node(20), None).unwrap();
    host.trigger(flow, node(50), None).unwrap();
    let read = |n| {
        host.inspect(flow, move |run| run.variable(variable(n)).cloned())
            .unwrap()
    };
    let object = Some(Value::Str(std::sync::Arc::new("object".to_owned())));
    until("HTTP status and JSON type, and WebSocket JSON echo", || {
        read(110) == Some(Value::I64(200)) && read(111) == object && read(120) == object
    });
    host.close(flow).unwrap();
    answer.join().unwrap();
    echo.join().unwrap();
}
