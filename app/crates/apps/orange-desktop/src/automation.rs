//! Automation acceptance tests through the desktop's built-in host and editor API.
use super::{built_in_services, keyboard::Keyboard, logbook::LogBook};
use eframe::egui;
use orange_editor_ui::Editor;
use orange_host::{Connection, ConnectionScope, Host, HostOptions};
use serde_json::{Value as Json, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn desktop_host() -> (Host, orange_host::Events) {
    let services = built_in_services(&Keyboard::default(), &LogBook::default()).unwrap();
    Host::with_services(HostOptions::default(), services).unwrap()
}

#[test]
fn the_desktop_round_trips_all_conversions_and_paints_every_catalogue_node() {
    let (mut host, _) = desktop_host();
    let template = typed_template();
    let nodes = host.search_nodes("", "en");
    for locale in ["en", "es"] {
        let mut labels = std::collections::BTreeMap::new();
        for found in host.search_nodes("", locale) {
            labels
                .entry((found.category.clone(), found.label.to_lowercase()))
                .or_insert_with(Vec::new)
                .push(found.definition);
        }
        let ambiguous: Vec<_> = labels
            .into_iter()
            .filter(|(_, definitions)| definitions.len() > 1)
            .collect();
        assert!(
            ambiguous.is_empty(),
            "ambiguous catalogue labels in {locale}: {ambiguous:?}"
        );
    }
    let mut rejected = Vec::new();
    let mut editor = Editor::new();
    let context = egui::Context::default();
    for found in &nodes {
        let mut draft = host.draft(&template).unwrap();
        match host.add_node(&mut draft, &found.definition, [50.0, 100.0]) {
            Ok(node) => {
                let saved = check_round_trip(&host, &draft, &found.definition);
                editor.open_untitled(&mut host, saved);
                let graph = editor.graph().unwrap().clone();
                assert_eq!(
                    graph.nodes[0].id, node,
                    "missing card for {}",
                    found.definition
                );
                editor.canvas().focus(&graph, node);
                let output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        editor.ui_with_layout(ui, &mut host, |editor, ui, host| {
                            editor.canvas_panel(ui, host);
                        })
                    },
                );
                let prefix: String = found.label.chars().take(16).collect();
                let visible = output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with(&prefix))
                });
                output.drop_without_applying_deltas();
                assert!(visible, "the canvas did not paint {}", found.definition);
            }
            Err(error) => rejected.push(format!("{}: {error}", found.definition)),
        }
    }
    assert!(
        rejected.is_empty(),
        "palette nodes that could not be added:\n{}",
        rejected.join("\n")
    );

    let conversions = host.conversions();
    for conversion in &conversions {
        let mut draft = host.draft(&template).unwrap();
        host.add_conversion(&mut draft, conversion, [50.0, 100.0])
            .unwrap();
        let _ = check_round_trip(&host, &draft, "conversion");
    }
    eprintln!(
        "Saved and reopened {} searchable node kinds and {} conversion entries",
        nodes.len(),
        conversions.len()
    );
}

/// The template's enum, whose `message` choice carries text.
const MODE_TYPE: &str = "6c0e4a52-8b1f-4a5e-9c3d-1e2f3a4b5c6d";
/// The template's record, with one text field `name`.
const ITEM_TYPE: &str = "6c0e4a52-8b1f-4a5e-9c3d-1e2f3a4b5c6e";

/// A new document that declares an enum with a payload choice and a record, so the
/// nodes that need a declared type can be added.
fn typed_template() -> String {
    let mut template: Json = serde_json::from_str(&orange_host::new_document().unwrap()).unwrap();
    template["types"] = json!([
        {"id": MODE_TYPE, "name": "Mode", "kind": "enum",
            "variants": [
                {"id": "ready", "name": "Ready"},
                {"id": "message", "name": "Message", "payload": "core/string@1"}
            ]},
        {"id": ITEM_TYPE, "name": "Item", "kind": "record",
            "fields": [{"id": "name", "name": "Name", "type": "core/string@1"}]}
    ]);
    template.to_string()
}

fn check_round_trip(host: &Host, draft: &orange_host::Draft, label: &str) -> String {
    let saved = draft.text().unwrap();
    let before = host.draft_view(draft).unwrap();
    assert_eq!(
        before.nodes.len(),
        1,
        "{label} should place exactly one card"
    );
    let after = host
        .view(&saved)
        .unwrap_or_else(|error| panic!("{label} cannot be reopened: {error}"));
    assert_eq!(
        before, after,
        "card, ports or wiring changed when reopening {label}"
    );
    saved
}

#[test]
fn the_saved_json_example_extracts_fields_and_uses_its_fallbacks() {
    let (mut host, _) = desktop_host();
    let example = include_str!("../../orange-cli/graphs/json-extraction.json");
    let mut editor = Editor::new();
    for (path, expected) in [
        ("items[0].name", "Studio mic"),
        (r#"headers["Content-Type"]"#, "application/json"),
        ("items[0].missing", "(field missing)"),
        ("items[0].level", "(field is not text)"),
    ] {
        editor.open_untitled(&mut host, example.to_owned());
        let graph = editor.graph().unwrap();
        let read = graph
            .nodes
            .iter()
            .find(|node| node.definition == "core/on-manual")
            .unwrap()
            .id;
        let extract = graph
            .nodes
            .iter()
            .find(|node| node.definition == "org.orange.json/get-path")
            .unwrap()
            .id;
        let variable: orange_host::VariableId = editor.declarations().unwrap().variables[0]
            .id
            .as_deref()
            .unwrap()
            .parse()
            .unwrap();
        let selector = json!({"type": "core/string@1", "value": path}).to_string();
        let _ = editor.set_input(&mut host, (extract, "path"), Some(&selector));
        editor.run(&mut host);
        let flow = host.flows().next().unwrap();
        editor.trigger(&host, read);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let value = host
                .inspect(flow, move |run| run.variable(variable).cloned())
                .unwrap();
            if let Some(orange_host::Value::Str(value)) = value {
                assert_eq!(value.as_str(), expected, "{path}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{path} never set the run variable"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        editor.stop(&mut host);
    }
}

/// Owns only the named fixture file and its newly created directory.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder =
            std::env::temp_dir().join(format!("orange-automation-{}-{time}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        Self(folder)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("sample.json"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[test]
fn a_reopened_editor_graph_executes_a_connection_and_reports_success_and_failure() {
    let fixture = Fixture::new();
    let expected = "{\n  \"state\": \"ready\"\n}\n";
    std::fs::write(fixture.0.join("sample.json"), expected).unwrap();
    let (mut host, events) = desktop_host();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());

    let mut routes = Vec::new();
    for (row, path) in [(0.0, "sample.json"), (230.0, "missing.json")] {
        let trigger = editor
            .add_node_at(&mut host, "core/on-manual", [0.0, row])
            .unwrap();
        let read = editor
            .add_node_at(&mut host, orange_files::READ_TEXT, [260.0, row])
            .unwrap();
        assert!(editor.set_input(&mut host, (read, "path"), Some(path)));
        assert!(editor.connect(&mut host, (trigger, "then"), (read, "in")));
        routes.push((trigger, read));
    }
    let mut connection: Connection = editor.declarations().unwrap().connections[0].clone();
    connection.scope = ConnectionScope::Graph;
    connection.settings = json!({"path": fixture.0, "read_only": true});
    editor.declare_connection(&mut host, &connection).unwrap();

    let saved = editor.text().unwrap();
    assert!(host.check(&saved).unwrap().is_empty());
    editor.open_untitled(&mut host, saved);
    editor.run(&mut host);
    assert_eq!(host.flows().count(), 1);
    editor.trigger(&host, routes[0].0);
    editor.trigger(&host, routes[1].0);
    let graph = editor.graph().unwrap().clone();
    let start = Instant::now();
    loop {
        editor.take_events(&events);
        let success = editor
            .canvas()
            .port_value(&graph, routes[0].1, "text", false)
            .is_some_and(|value| value.full == expected);
        let failure = editor.canvas().failure(routes[1].1).is_some();
        if success && failure {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "missing editor result: success={success}, failure={failure}; log={:?}",
            editor.log().collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        editor
            .canvas()
            .port_value(&graph, routes[1].1, "text", false)
            .is_none()
    );
    editor.stop(&mut host);
    assert_eq!(host.flows().count(), 0);
}

#[test]
#[ignore = "requires permission to connect to IPv4 loopback"]
fn desktop_http_request_reports_the_actual_socket_response() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let reply = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(8);
        let socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("desktop did not connect to HTTP fixture: {error}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(socket);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim_end(), "POST /automation?case=desktop HTTP/1.1");
        let mut length = 0;
        let mut saw_header = false;
        loop {
            line.clear();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            saw_header |= lower.trim_end() == "x-orange-case: desktop";
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        assert_eq!(body, br#"{"input":42}"#);
        assert!(saw_header);
        reader.get_mut().write_all(b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{\"answer\":42}").unwrap();
    });

    let (mut host, events) = desktop_host();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let trigger = editor
        .add_node_at(&mut host, "core/on-manual", [0.0, 0.0])
        .unwrap();
    let request = editor
        .add_node_at(&mut host, orange_net::HTTP_REQUEST, [260.0, 0.0])
        .unwrap();
    assert!(editor.connect(&mut host, (trigger, "then"), (request, "in")));
    for (port, value) in [
        ("method", "POST"),
        ("path", "/automation?case=desktop"),
        ("content_type", "application/json"),
        ("headers", "X-Orange-Case: desktop"),
        ("content", r#"{"input":42}"#),
    ] {
        let literal = json!({"type": "core/string@1", "value": value}).to_string();
        assert!(editor.set_input(&mut host, (request, port), Some(&literal)));
    }
    let mut connection = editor.declarations().unwrap().connections[0].clone();
    connection.settings = json!({"base_url": format!("http://{address}"), "timeout_ms": 3000});
    editor.declare_connection(&mut host, &connection).unwrap();
    let saved = editor.text().unwrap();
    editor.open_untitled(&mut host, saved);
    editor.run(&mut host);
    assert_eq!(host.flows().count(), 1);
    editor.trigger(&host, trigger);
    let graph = editor.graph().unwrap().clone();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        editor.take_events(&events);
        assert!(
            editor.canvas().failure(request).is_none(),
            "{:?}",
            editor.canvas().failure(request)
        );
        if let Some(body) = editor
            .canvas()
            .port_value(&graph, request, "response", false)
        {
            assert_eq!(body.full, r#"{"answer":42}"#);
            assert_eq!(editor.canvas().shown_value(request, "status"), Some("201"));
            assert_eq!(
                editor.canvas().shown_value(request, "success"),
                Some("true")
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "desktop did not report the HTTP response"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    reply.join().unwrap();
    editor.stop(&mut host);
}

/// The type arguments of `ty`, such as `["core/string@1", "core/i64@1"]` for a map.
fn type_arguments(ty: &str) -> Vec<&str> {
    let Some(open) = ty.find('<') else {
        return Vec::new();
    };
    let inner = &ty[open + 1..ty.len() - 1];
    let (mut depth, mut start, mut parts) = (0, 0, Vec::new());
    for (index, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(inner[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(inner[start..].trim());
    parts
}

/// The stored value of a sample of type `ty` (12 §6.1), the `n`th of its kind so map
/// keys differ; `None` for a type this test does not make values of. Floats keep their
/// own `bits` form and are made only at the top level.
fn sample_value(ty: &str, n: u32) -> Option<Json> {
    let base = ty.split('<').next().unwrap_or(ty);
    Some(match base {
        // 64-bit integers are stored as text, smaller ones as numbers.
        "core/i64@1" | "core/u64@1" => json!((n + 1).to_string()),
        "core/i32@1" | "core/u32@1" => json!(n + 1),
        // Valid JSON text, so Parse JSON and JSON paths have something to read.
        "core/string@1" => json!(if n == 0 { "[1]" } else { "b" }),
        "core/bool@1" => json!(true),
        "core/json@1" => json!(if n == 0 {
            r#"[{"a":1},2]"#
        } else {
            r#"{"a":[1,2],"b":"text"}"#
        }),
        "core/bytes@1" => json!("AP8="),
        // A declared type is named `local/t<its ID without hyphens>@1`.
        _ if ty.contains(&ITEM_TYPE.replace('-', "")) => json!({"name": "x"}),
        _ if ty.contains(&MODE_TYPE.replace('-', "")) => {
            json!({"variant": "message", "payload": "x"})
        }
        "core/list@1" => {
            let [item] = type_arguments(ty)[..] else {
                return None;
            };
            json!([sample_value(item, 0)?, sample_value(item, 1)?])
        }
        "core/option@1" => {
            let [item] = type_arguments(ty)[..] else {
                return None;
            };
            json!({"some": sample_value(item, n)?})
        }
        "core/map@1" => {
            let [key, value] = type_arguments(ty)[..] else {
                return None;
            };
            json!([
                [sample_value(key, 0)?, sample_value(value, 0)?],
                [sample_value(key, 1)?, sample_value(value, 1)?]
            ])
        }
        _ => return None,
    })
}

/// A sample literal for an input of type `ty`; `variant` 1 gives a JSON object
/// instead of an array.
fn sample(ty: &str, variant: u32) -> Option<Json> {
    let literal = match ty {
        "core/f64@1" => json!({"bits": format!("{:016x}", 0.5f64.to_bits())}),
        "core/f32@1" => json!({"bits": format!("{:08x}", 0.5f32.to_bits())}),
        "core/json@1" => json!({"value": sample_value(ty, variant)?}),
        _ => json!({"value": sample_value(ty, 0)?}),
    };
    let mut literal = literal;
    literal["type"] = json!(ty);
    Some(literal)
}

/// What one node gave when its outputs were computed from sample inputs.
enum Computed {
    Values,
    Fault(String),
    Unfilled(String),
}

/// Adds `definition` to a fresh draft, gives every open type parameter `ty`, fills its
/// inputs with samples and computes its outputs without running anything.
fn compute(host: &mut Host, template: &str, definition: &str, ty: &str, variant: u32) -> Computed {
    let mut draft = host.draft(template).unwrap();
    let node = host.add_node(&mut draft, definition, [0.0, 0.0]).unwrap();
    let card = host.draft_view(&draft).unwrap().nodes[0].clone();
    for choice in &card.types {
        if choice.chosen.is_none() && choice.found.is_none() {
            let _ = host.choose_type(&mut draft, node, &choice.name, Some(ty));
        }
    }
    let card = host.draft_view(&draft).unwrap().nodes[0].clone();
    for port in card.ports.iter().filter(|port| port.input && !port.exec) {
        if port.value.is_some() || port.default.is_some() {
            continue;
        }
        if let Some(literal) = port.ty.as_deref().and_then(|ty| sample(ty, variant)) {
            let _ = host.set_input(
                &mut draft,
                (node, port.id.as_str()),
                Some(&literal.to_string()),
            );
        }
    }
    let outputs: Vec<_> = card
        .ports
        .iter()
        .filter(|port| !port.input && !port.exec)
        .map(|port| (node, port.id.parse().unwrap()))
        .collect();
    match host.preview(&draft.text().unwrap(), &outputs) {
        Ok(values) => match values.into_iter().find_map(|value| value.value.err()) {
            None => Computed::Values,
            Some((_, fault)) => Computed::Fault(format!("{}: {}", fault.code, fault.message)),
        },
        Err(error) => Computed::Unfilled(format!("{error:?}").chars().take(200).collect()),
    }
}

/// Every node without control flow computes its outputs from sample inputs, for the
/// first candidate type that fits. Live and volatile nodes, which only a running flow
/// computes, and loop zones, which work only in pairs, are counted apart: the packages'
/// own tests run those.
#[test]
fn every_pure_catalogue_node_computes_its_outputs_from_sample_inputs() {
    let (mut host, _) = desktop_host();
    let template = typed_template();
    let candidates = [
        "core/i64@1",
        "core/f64@1",
        "core/string@1",
        "core/bool@1",
        "core/json@1",
        "core/list@1<core/i64@1>",
    ];
    let (mut computed, mut faults, mut unfilled, mut flows) = (0, Vec::new(), Vec::new(), 0);
    let (mut live, mut zones) = (Vec::new(), Vec::new());
    for found in host.search_nodes("", "en") {
        let mut draft = host.draft(&template).unwrap();
        host.add_node(&mut draft, &found.definition, [0.0, 0.0])
            .unwrap();
        let card = host.draft_view(&draft).unwrap().nodes[0].clone();
        let has_outputs = card.ports.iter().any(|port| !port.input && !port.exec);
        if card.entry || card.ports.iter().any(|port| port.exec) || !has_outputs {
            flows += 1;
            continue;
        }
        let mut last = None;
        'tries: for ty in candidates {
            for variant in [0, 1] {
                match compute(&mut host, &template, &found.definition, ty, variant) {
                    Computed::Values => {
                        last = None;
                        computed += 1;
                        break 'tries;
                    }
                    other => last = Some(other),
                }
            }
        }
        match last {
            Some(Computed::Fault(fault)) => faults.push(format!("{}: {fault}", found.definition)),
            // Streams and live values exist only in a running flow (so does a stream
            // input's value), and Now and Random
            // change on every read: preview never computes them by design.
            Some(Computed::Unfilled(why))
                if why.contains("ExportType")
                    || why.contains("UnsupportedNode")
                    || why.contains(r#"PortId("stream")"#) =>
            {
                live.push(found.definition.clone());
            }
            // Loop zones work only as a begin and end pair.
            Some(Computed::Unfilled(why)) if why.contains("ZoneStructure") => {
                zones.push(found.definition.clone());
            }
            Some(Computed::Unfilled(why)) => unfilled.push(format!("{}: {why}", found.definition)),
            _ => {}
        }
    }
    eprintln!(
        "computed {computed}, faulted {}, not filled {}, live or volatile {}, zones {}, with control flow {flows}",
        faults.len(),
        unfilled.len(),
        live.len(),
        zones.len()
    );
    assert!(
        unfilled.is_empty(),
        "nodes this test could not give inputs:
{}",
        unfilled.join(
            "
"
        )
    );
    assert!(
        faults.is_empty(),
        "nodes that fault on sample inputs:\n{}",
        faults.join("\n")
    );
}
