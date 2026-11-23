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
fn the_power_example_can_extract_a_two_node_function() {
    let (mut host, _) = desktop_host();
    let mut editor = Editor::new();
    editor.open_untitled(
        &mut host,
        include_str!("../../orange-cli/graphs/power-with-count.json").to_owned(),
    );
    let selected: Vec<_> = editor
        .graph()
        .unwrap()
        .nodes
        .iter()
        .filter(|node| matches!(node.name.as_str(), "Base" | "Multiply by base"))
        .map(|node| node.id)
        .collect();
    assert_eq!(selected.len(), 2);
    editor.canvas().select_many(&selected);
    let call = editor
        .extract_selection(&mut host, "Scale by base")
        .unwrap();
    let graph = editor.graph().unwrap();
    let card = graph.nodes.iter().find(|node| node.id == call).unwrap();
    assert_eq!(card.name, "Scale by base");
    assert_eq!(card.ports.iter().filter(|port| port.input).count(), 1);
    assert_eq!(card.ports.iter().filter(|port| !port.input).count(), 1);
    assert!(host.check(&editor.text().unwrap()).unwrap().is_empty());

    let source = editor.text().unwrap();
    let preview = host
        .function_package_preview(&source, "Scale by base")
        .unwrap();
    assert_eq!(preview.kind, "pure");
    assert_eq!(preview.inputs, [("b".into(), "core/i64@1".into())]);
    assert_eq!(preview.outputs, [("product".into(), "core/i64@1".into())]);
    assert!(
        preview
            .dependencies
            .iter()
            .any(|(id, _)| id == "org.orange.math")
    );
    let package = host
        .export_function_package(
            &source,
            "Scale by base",
            &orange_host::Publish {
                package: "org.example.scaling",
                version: "1.0.0",
                node: "scale-by-base",
                label: None,
            },
        )
        .unwrap();
    let (mut consumer, _) = desktop_host();
    let key = consumer
        .install(&package, orange_host::Grants::default())
        .unwrap();
    editor.open_untitled(&mut consumer, orange_host::new_document().unwrap());
    let call = editor
        .add_node(&mut consumer, "org.example.scaling/scale-by-base")
        .unwrap();
    assert!(editor.set_input(&mut consumer, (call, "b"), Some("7")));
    let text = editor.text().unwrap();
    let wanted = [(call, "product".parse().unwrap())];
    let value = consumer.preview(&text, &wanted).unwrap().remove(0).value;
    assert_eq!(value.unwrap(), orange_host::Value::I64(21));
    editor.open_untitled(&mut consumer, text.clone());
    assert!(consumer.check(&editor.text().unwrap()).unwrap().is_empty());
    consumer.uninstall(&key).unwrap();
    assert!(
        consumer
            .check(&text)
            .unwrap()
            .iter()
            .any(|problem| problem.severity == orange_host::Severity::Error)
    );
}

#[test]
fn a_published_file_function_uses_the_importing_graphs_folder_after_reopen() {
    let folder = std::env::temp_dir().join(format!("orange-file-package-{}", std::process::id()));
    let author_folder = folder.join("author");
    let consumer_folder = folder.join("consumer");
    std::fs::create_dir_all(&author_folder).unwrap();
    std::fs::create_dir_all(&consumer_folder).unwrap();
    let (mut author, _) = desktop_host();
    let mut editor = Editor::new();
    editor.open_untitled(&mut author, orange_host::new_document().unwrap());
    let connection = |path: &std::path::Path| Connection {
        id: None,
        name: "Work".into(),
        adapter: orange_files::FOLDER_ADAPTER.into(),
        scope: ConnectionScope::Graph,
        settings: json!({"path": path}),
    };
    let author_connection = editor
        .declare_connection(&mut author, &connection(&author_folder))
        .unwrap();
    let entry = editor.add_node(&mut author, "core/on-manual").unwrap();
    let write = editor
        .add_node(&mut author, orange_files::WRITE_TEXT)
        .unwrap();
    let selected = editor.file_picker_folder(write).flatten().unwrap();
    assert_eq!(selected.name, "Work");
    assert_eq!(selected.id.as_deref(), Some(author_connection.as_str()));
    let root = author_folder.canonicalize().unwrap();
    assert_eq!(
        super::relative_selection(&root, &author_folder.join("hello.txt"), true).as_deref(),
        Some("hello.txt")
    );
    assert!(super::relative_selection(&root, &consumer_folder.join("hello.txt"), true).is_none());
    assert!(editor.set_input(&mut author, (write, "path"), Some("hello.txt")));
    assert!(editor.set_input(&mut author, (write, "text"), Some("from package")));
    assert!(editor.connect(&mut author, (entry, "then"), (write, "in")));
    editor.canvas().select(Some(write));
    editor
        .extract_selection(&mut author, "Write hello")
        .unwrap();
    let source = editor.text().unwrap();
    let preview = author
        .function_package_preview(&source, "Write hello")
        .unwrap();
    assert_eq!(
        preview.connection_adapter.as_deref(),
        Some(orange_files::FOLDER_ADAPTER)
    );
    let package = author
        .export_function_package(
            &source,
            "Write hello",
            &orange_host::Publish {
                package: "org.example.writer",
                version: "1.0.0",
                node: "write-hello",
                label: None,
            },
        )
        .unwrap();

    let (mut consumer, _) = desktop_host();
    consumer
        .install(&package, orange_host::Grants::default())
        .unwrap();
    editor.open_untitled(&mut consumer, orange_host::new_document().unwrap());
    let consumer_connection = editor
        .declare_connection(&mut consumer, &connection(&consumer_folder))
        .unwrap();
    assert_ne!(author_connection, consumer_connection);
    let entry = editor.add_node(&mut consumer, "core/on-manual").unwrap();
    let write = editor
        .add_node(&mut consumer, "org.example.writer/write-hello")
        .unwrap();
    let write_view = editor
        .graph()
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.id == write)
        .unwrap();
    assert_eq!(write_view.settings["connection"], consumer_connection);
    assert!(editor.connect(&mut consumer, (entry, "then"), (write, "in")));
    let saved = folder.join("graph.json");
    editor.save_as(saved.to_string_lossy()).unwrap();
    let text = std::fs::read_to_string(saved).unwrap();
    editor.open_untitled(&mut consumer, text);
    editor.run(&mut consumer);
    editor.trigger(&consumer, entry);
    let deadline = Instant::now() + Duration::from_secs(3);
    let result = consumer_folder.join("hello.txt");
    while std::fs::read_to_string(&result).as_deref().ok() != Some("from package") {
        assert!(
            Instant::now() < deadline,
            "the installed function did not write its file"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!author_folder.join("hello.txt").exists());
    editor.stop(&mut consumer);
    std::fs::remove_file(result).unwrap();
}

#[test]
fn the_power_example_can_extract_and_run_the_whole_count_loop() {
    let (mut host, _) = desktop_host();
    let mut editor = Editor::new();
    editor.open_untitled(
        &mut host,
        include_str!("../../orange-cli/graphs/power-with-count.json").to_owned(),
    );
    let graph = editor.graph().unwrap();
    let trigger = graph.nodes.iter().find(|node| node.entry).unwrap().id;
    let selected: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.id != trigger)
        .map(|node| node.id)
        .collect();
    assert_eq!(selected.len(), 9);

    editor.canvas().select_many(&selected);
    let call = editor
        .extract_selection(&mut host, "Compute power")
        .unwrap();
    let graph = editor.graph().unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert!(
        graph
            .wires
            .iter()
            .any(|wire| wire.from.0 == trigger && wire.to.0 == call)
    );
    assert!(host.check(&editor.text().unwrap()).unwrap().is_empty());

    editor.run(&mut host);
    let flow = host.flows().next().unwrap();
    let result = "70000000-0000-4000-8000-000000000002".parse().unwrap();
    editor.trigger(&host, trigger);
    wait_for_power_result(&host, flow, result, 81);
    editor.stop(&mut host);
}

#[test]
fn the_power_loop_with_for_range_can_be_reused_with_two_arguments_and_a_result() {
    let (mut host, _) = desktop_host();
    let mut editor = Editor::new();
    let mut source: Json = serde_json::from_str(include_str!(
        "../../orange-cli/graphs/power-with-count.json"
    ))
    .unwrap();
    let range = source["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["definition_id"] == "core/count")
        .unwrap();
    range["definition_id"] = json!("core/for-range");
    range["config"] = json!({"max_iterations": 1000});
    let owner = range["id"].clone();
    let exponent = source["edges"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|edge| edge["to"]["node"] == owner && edge["to"]["port"] == "count")
        .unwrap();
    exponent["to"]["port"] = json!("end");
    editor.open_untitled(&mut host, source.to_string());
    let graph = editor.graph().unwrap();
    let trigger = graph.nodes.iter().find(|node| node.entry).unwrap().id;
    let save = graph
        .nodes
        .iter()
        .find(|node| node.name == "Save result")
        .unwrap()
        .id;
    let selected: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| {
            !node.entry && !matches!(node.name.as_str(), "Base" | "Exponent" | "Save result")
        })
        .map(|node| node.id)
        .collect();
    editor.canvas().select_many(&selected);
    let call = editor
        .extract_selection(&mut host, "Compute power")
        .unwrap();
    let graph = editor.graph().unwrap();
    let card = graph.nodes.iter().find(|node| node.id == call).unwrap();
    let mut inputs: Vec<_> = card
        .ports
        .iter()
        .filter(|port| port.input && !port.exec)
        .map(|port| port.id.as_str())
        .collect();
    inputs.sort_unstable();
    assert_eq!(inputs, ["a", "end"]);
    let outputs: Vec<_> = card
        .ports
        .iter()
        .filter(|port| !port.input && !port.exec)
        .map(|port| port.id.as_str())
        .collect();
    assert_eq!(outputs, ["value"]);
    assert!(
        graph
            .wires
            .iter()
            .any(|wire| wire.from == (call, "value".into()) && wire.to == (save, "value".into()))
    );

    assert!(editor.duplicate(&mut host));
    let second_call = editor.canvas().selection()[0];
    assert_ne!(second_call, call);
    assert!(editor.set_input(&mut host, (second_call, "a"), Some("2")));
    assert!(editor.set_input(&mut host, (second_call, "end"), Some("5")));
    editor.canvas().select(Some(save));
    assert!(editor.duplicate(&mut host));
    let second_save = editor.canvas().selection()[0];
    let second_trigger = editor.add_node(&mut host, "core/on-manual").unwrap();
    assert!(editor.connect(&mut host, (second_trigger, "then"), (second_call, "in")));
    assert!(editor.connect(&mut host, (second_call, "then"), (second_save, "in")));
    assert!(editor.connect(&mut host, (second_call, "value"), (second_save, "value")));
    let text = editor.text().unwrap();
    assert_eq!(host.draft(&text).unwrap().document().functions().len(), 1);
    assert!(host.check(&text).unwrap().is_empty());

    editor.run(&mut host);
    let flow = host.flows().next().unwrap();
    let result = "70000000-0000-4000-8000-000000000002".parse().unwrap();
    for (entry, expected) in [(trigger, 81), (second_trigger, 32)] {
        editor.trigger(&host, entry);
        wait_for_power_result(&host, flow, result, expected);
    }
    editor.stop(&mut host);
}

#[test]
fn the_power_example_repeats_multiplication_and_resets_on_every_fire() {
    let (mut host, _) = desktop_host();
    let text = include_str!("../../orange-cli/graphs/power-with-count.json");
    assert!(host.check(text).unwrap().is_empty());
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, text.to_owned());
    let node = |name| {
        editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap()
            .id
    };
    let (calculate, base, exponent) = (node("Calculate power"), node("Base"), node("Exponent"));
    let result = "70000000-0000-4000-8000-000000000002".parse().unwrap();
    editor.run(&mut host);
    let flow = host.flows().next().unwrap();
    for (inputs, expected) in [(None, 81), (Some(("3", "0")), 1), (Some(("-2", "3")), -8)] {
        if let Some((base_value, exponent_value)) = inputs {
            if base_value != "3" {
                assert!(editor.set_input(&mut host, (base, "value"), Some(base_value)));
            }
            assert!(editor.set_input(&mut host, (exponent, "value"), Some(exponent_value)));
        }
        editor.trigger(&host, calculate);
        wait_for_power_result(&host, flow, result, expected);
    }
    editor.stop(&mut host);
}

fn wait_for_power_result(
    host: &Host,
    flow: orange_host::FlowId,
    result: orange_host::VariableId,
    expected: i64,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let value = host
            .inspect(flow, move |run| run.variable(result).cloned())
            .unwrap();
        if value == Some(orange_host::Value::I64(expected)) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "expected {expected}, got {value:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
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
    let mut blocking_cards = Vec::new();
    let mut editor = Editor::new();
    let context = egui::Context::default();
    for found in &nodes {
        let mut draft = host.draft(&template).unwrap();
        match host.add_node(&mut draft, &found.definition, [50.0, 100.0]) {
            Ok(node) => {
                let saved = check_round_trip(&host, &draft, &found.definition);
                editor.open_untitled(&mut host, saved);
                if editor
                    .canvas()
                    .marks()
                    .nodes
                    .get(&node)
                    .is_some_and(|mark| mark.error)
                {
                    blocking_cards.push((found.definition.clone(), editor.problems().to_vec()));
                }
                let graph = editor.graph().unwrap().clone();
                assert_eq!(
                    graph.nodes[0].id, node,
                    "missing card for {}",
                    found.definition
                );
                let node_view = &graph.nodes[0];
                let size = orange_editor_ui::canvas::node_size(node_view);
                let mut positions = std::collections::HashSet::new();
                for port in &node_view.ports {
                    if found.definition == "core/gate" && port.input && port.id == "enter" {
                        assert!(
                            orange_editor_ui::canvas::port_offset(node_view, &port.id, true)
                                .is_none()
                        );
                        continue;
                    }
                    let offset =
                        orange_editor_ui::canvas::port_offset(node_view, &port.id, port.input)
                            .unwrap_or_else(|| {
                                panic!("{} lacks a canvas anchor for {}", found.definition, port.id)
                            });
                    assert_eq!(offset.x, if port.input { 0.0 } else { size.x });
                    assert!(
                        (32.0..size.y).contains(&offset.y),
                        "{} port {} is outside the card body at {offset:?}",
                        found.definition,
                        port.id,
                    );
                    assert!(
                        positions.insert((offset.x.to_bits(), offset.y.to_bits())),
                        "{} has overlapping ports at {offset:?}",
                        found.definition
                    );
                }
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
                            egui::Panel::right("catalogue-audit-properties")
                                .default_size(280.0)
                                .show(ui, |ui| editor.right_panel(ui, host));
                            editor.canvas_panel(ui, host);
                        })
                    },
                );
                let prefix: String = found.label.chars().take(16).collect();
                let visible = output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with(&prefix))
                });
                if matches!(
                    found.definition.as_str(),
                    orange_audio::NOISE_LEVEL
                        | orange_audio::MULTIBAND_SPECTRUM
                        | orange_net::HTTP_REQUEST
                ) {
                    let draft = orange_editor_ui::icons::EDIT.to_string();
                    assert!(output.shapes.iter().any(|shape| {
                        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == draft)
                    }), "{} needs the Draft icon", found.definition);
                }
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
    assert!(
        blocking_cards.is_empty(),
        "New cards classified as blockers: {blocking_cards:#?}"
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

#[test]
fn a_catalogue_gate_keeps_a_connected_entry_alias_visible() {
    let (host, _) = desktop_host();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let trigger = host
        .add_node(&mut draft, "core/on-manual", [20.0, 20.0])
        .unwrap();
    let gate = host
        .add_node(&mut draft, "core/gate", [340.0, 20.0])
        .unwrap();
    host.connect(&mut draft, (trigger, "then"), (gate, "enter"))
        .unwrap();
    let graph = host.draft_view(&draft).unwrap();
    let card = graph.nodes.iter().find(|node| node.id == gate).unwrap();
    let position = |port| orange_editor_ui::canvas::port_offset(card, port, true).unwrap();
    assert_ne!(position("enter"), position("in"));
    assert!(
        graph
            .wires
            .iter()
            .any(|wire| wire.to == (gate, "enter".to_owned()))
    );
}

/// Export palette snapshots in screen-sized batches for desktop screenshot review.
#[test]
#[ignore = "writes catalogue audit fixtures"]
fn export_desktop_catalogue_audit_graphs() {
    let out = PathBuf::from(std::env::var_os("ORANGE_CATALOGUE_AUDIT_DIR").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let (host, _) = desktop_host();
    let template = typed_template();
    let mut entries = host.search_nodes("", "en");
    entries.sort_by(|a, b| (&a.category, &a.label).cmp(&(&b.category, &b.label)));
    let mut all = host.draft(&template).unwrap();
    let save = |draft: &mut orange_host::Draft, path| {
        for mut connection in host.declarations(draft).connections {
            if matches!(
                connection.adapter.as_str(),
                orange_audio::INPUT_ADAPTER | orange_audio::OUTPUT_ADAPTER
            ) {
                connection.settings = json!({"device": orange_audio::NULL_DEVICE});
                host.declare_connection(draft, &connection).unwrap();
            }
        }
        std::fs::write(path, draft.text().unwrap()).unwrap();
    };
    for (page, batch) in entries.chunks(6).enumerate() {
        let mut draft = host.draft(&template).unwrap();
        let mut row_top = 20.0;
        for line in batch.chunks(3) {
            let mut left = 20.0;
            let mut tallest = 0.0_f32;
            for entry in line {
                let node = host
                    .add_node(&mut draft, &entry.definition, [left, row_top])
                    .unwrap();
                #[expect(clippy::cast_precision_loss, reason = "catalogue page grid is bounded")]
                let grid = [(page % 6) as f32 * 1500.0, (page / 6) as f32 * 1500.0];
                host.add_node(
                    &mut all,
                    &entry.definition,
                    [left + grid[0], row_top + grid[1]],
                )
                .unwrap();
                let view = host.draft_view(&draft).unwrap();
                let card = view.nodes.iter().find(|card| card.id == node).unwrap();
                let size = orange_editor_ui::canvas::node_size(card);
                left += size.x + 40.0;
                tallest = tallest.max(size.y);
            }
            row_top += tallest + 46.0;
        }
        save(&mut draft, out.join(format!("page-{:02}.json", page + 1)));
    }
    save(&mut all, out.join("all-nodes.json"));
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
fn desktop_http_request_sends_graph_built_json_and_reports_the_socket_response() {
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
        assert_eq!(
            line.trim_end(),
            "POST /automation?case=desktop%20%26%20caf%C3%A9 HTTP/1.1"
        );
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
        reader.get_mut().write_all(b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nX-Orange-Result: first\r\nX-Orange-Result: second\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{\"answer\":42}").unwrap();
    });

    let book = LogBook::default();
    let services = built_in_services(&Keyboard::default(), &book).unwrap();
    let (mut host, events) = Host::with_services(HostOptions::default(), services).unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let trigger = editor
        .add_node_at(&mut host, "core/on-manual", [0.0, 0.0])
        .unwrap();
    let request = editor
        .add_node_at(&mut host, orange_net::HTTP_REQUEST_JSON, [260.0, 0.0])
        .unwrap();
    assert!(editor.connect(&mut host, (trigger, "then"), (request, "in")));
    const TEXT_MAP: &str = "core/map@1<core/string@1,core/string@1>";
    for (port, ty, value) in [
        ("path", "core/string@1", json!("/automation")),
        ("query", TEXT_MAP, json!([["case", "desktop & café"]])),
        (
            "header_fields",
            TEXT_MAP,
            json!([["X-Orange-Case", "desktop"]]),
        ),
    ] {
        let literal = json!({"type": ty, "value": value}).to_string();
        assert!(editor.set_input(&mut host, (request, port), Some(&literal)));
    }
    let mut connection = editor.declarations().unwrap().connections[0].clone();
    connection.settings = json!({"base_url": format!("http://{address}"), "timeout_ms": 3000});
    editor.declare_connection(&mut host, &connection).unwrap();
    let saved = editor.text().unwrap();
    let mut draft = host.draft(&saved).unwrap();
    let number = host
        .add_node(&mut draft, "core/whole-number", [0.0, 140.0])
        .unwrap();
    let json_number = host
        .add_node(&mut draft, "org.orange.json/from-i64", [210.0, 140.0])
        .unwrap();
    let object = host
        .add_node(&mut draft, "org.orange.json/set-field", [510.0, 140.0])
        .unwrap();
    let match_response = host
        .add_node(&mut draft, "core/match-option", [530.0, 0.0])
        .unwrap();
    let lookup = host
        .add_node(&mut draft, "org.orange.json/get", [790.0, 150.0])
        .unwrap();
    let match_lookup = host
        .add_node(&mut draft, "core/match-option", [1050.0, 0.0])
        .unwrap();
    let log = host
        .add_node(&mut draft, "org.orange.logging/log-message", [1320.0, 0.0])
        .unwrap();
    for (node, port, ty, value) in [
        (number, "value", "core/i64@1", "42"),
        (object, "object", "core/json@1", "{}"),
        (object, "key", "core/string@1", "input"),
        (lookup, "key", "core/string@1", "absent"),
        (
            log,
            "message",
            "core/string@1",
            "HTTP response field is missing",
        ),
    ] {
        let literal = json!({"type":ty,"value":value}).to_string();
        host.set_input(&mut draft, (node, port), Some(&literal))
            .unwrap();
    }
    for (from, to) in [
        ((number, "result"), (json_number, "value")),
        ((json_number, "result"), (object, "value")),
        ((object, "result"), (request, "content")),
        ((request, "then"), (match_response, "in")),
        ((request, "json"), (match_response, "option")),
        ((match_response, "present"), (match_lookup, "in")),
        ((match_response, "value"), (lookup, "value")),
        ((lookup, "result"), (match_lookup, "option")),
        ((match_lookup, "missing"), (log, "in")),
    ] {
        host.connect(&mut draft, from, to).unwrap();
    }
    editor.open_untitled(&mut host, draft.text().unwrap());
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
        if book
            .take()
            .0
            .into_iter()
            .any(|(_, line)| line.contains("HTTP response field is missing"))
        {
            let body = editor
                .canvas()
                .port_value(&graph, request, "response", false)
                .unwrap();
            assert_eq!(body.full, r#"{"answer":42}"#);
            assert_eq!(editor.canvas().shown_value(request, "status"), Some("201"));
            assert_eq!(
                editor.canvas().shown_value(request, "success"),
                Some("true")
            );
            assert!(editor.canvas().shown_value(match_lookup, "value").is_none());
            let headers = editor
                .canvas()
                .port_value(&graph, request, "response_headers", false)
                .unwrap();
            assert!(
                headers.full.contains("x-orange-result")
                    && headers.full.contains("first")
                    && headers.full.contains("second"),
                "{}",
                headers.full
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "desktop did not finish the HTTP response graph: {:?}",
            host.flows()
                .next()
                .and_then(|flow| host.inspect(flow, |run| run.trace().clone()).ok())
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
        "core/duration@1" => json!("1000000"),
        "core/timestamp@1" => json!("1700000000123456789"),
        orange_host::RGBA_TYPE => json!({"red":32,"green":96,"blue":224,"alpha":160}),
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
        let literal = match (definition, port.id.as_str()) {
            ("org.orange.text/decode-base64", "text") => {
                Some(json!({"type": "core/string@1", "value": "AP8="}))
            }
            ("org.orange.text/decode-hex", "text") => {
                Some(json!({"type": "core/string@1", "value": "00ff"}))
            }
            ("org.orange.color/parse-hex", "text") => {
                Some(json!({"type": "core/string@1", "value": "#2060E0A0"}))
            }
            ("org.orange.time/parse-timestamp", "text") => Some(
                json!({"type": "core/string@1", "value": "2026-11-20T12:34:56.123456789-06:00"}),
            ),
            ("org.orange.math/map-range", "from_max" | "to_max") => match port.ty.as_deref() {
                Some("core/f64@1") => Some(json!({
                    "type": "core/f64@1", "bits": format!("{:016x}", 1.5_f64.to_bits())
                })),
                Some("core/f32@1") => Some(json!({
                    "type": "core/f32@1", "bits": format!("{:08x}", 1.5_f32.to_bits())
                })),
                _ => None,
            },
            _ => port.ty.as_deref().and_then(|ty| sample(ty, variant)),
        };
        if let Some(literal) = literal {
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
        if found.definition == "org.orange.text/inspect-error" {
            // Error has no literal form. The bundled control tests exercise this
            // node through a captured fault.
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

#[test]
fn list_builder_previews_wired_and_inline_items_in_card_order() {
    let (mut host, _) = desktop_host();
    let mut draft = host.draft(&typed_template()).unwrap();
    let builder = host
        .add_node(&mut draft, "org.orange.collections/build-list", [0.0, 0.0])
        .unwrap();
    let value = host
        .add_node(&mut draft, "core/whole-number", [300.0, 0.0])
        .unwrap();
    let first = host.add_port(&mut draft, builder, "item").unwrap();
    let removed = host.add_port(&mut draft, builder, "item").unwrap();
    let last = host.add_port(&mut draft, builder, "item").unwrap();
    host.remove_port(&mut draft, (builder, &removed)).unwrap();
    host.set_input(
        &mut draft,
        (value, "value"),
        Some(r#"{"type": "core/i64@1", "value": "7"}"#),
    )
    .unwrap();
    host.connect(&mut draft, (value, "result"), (builder, &first))
        .unwrap();
    host.set_input(
        &mut draft,
        (builder, &last),
        Some(r#"{"type": "core/i64@1", "value": "3"}"#),
    )
    .unwrap();
    let outputs = host
        .preview(
            &draft.text().unwrap(),
            &[(builder, "result".parse().unwrap())],
        )
        .unwrap();
    let orange_host::Value::List(items) = outputs[0].value.as_ref().unwrap() else {
        panic!("the builder must produce a list");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items.get(0), Some(orange_host::Value::I64(7)));
    assert_eq!(items.get(1), Some(orange_host::Value::I64(3)));
}

#[test]
fn placing_a_loop_creates_a_wired_pair_in_one_undoable_edit() {
    let (mut host, _) = desktop_host();
    for (start, finish, from, to) in [
        ("repeat", "repeat", "carried", "next"),
        ("map", "map", "element", "item"),
        ("filter", "filter", "element", "item"),
        ("fold", "fold", "accumulator", "next"),
    ] {
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, typed_template());
        let begin = editor
            .add_node_at(&mut host, &format!("core/{start}-begin"), [0.0, 0.0])
            .unwrap();
        let graph = editor.graph().unwrap();
        let end = graph
            .nodes
            .iter()
            .find(|node| node.definition == format!("core/{finish}-end"))
            .unwrap();
        let begin = graph.nodes.iter().find(|node| node.id == begin).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(begin.settings["zone"], end.settings["zone"]);
        assert!(begin.settings["zone"].is_string());
        assert!(
            graph
                .wires
                .iter()
                .any(|wire| wire.from == (begin.id, from.into()) && wire.to == (end.id, to.into()))
        );
        let original_zone = begin.settings["zone"].clone();
        assert!(editor.duplicate(&mut host));
        let graph = editor.graph().unwrap();
        assert_eq!(graph.nodes.len(), 4);
        let copied_zones: Vec<_> = graph
            .nodes
            .iter()
            .filter(|node| node.settings["zone"] != original_zone)
            .map(|node| &node.settings["zone"])
            .collect();
        assert_eq!(copied_zones.len(), 2);
        assert_eq!(copied_zones[0], copied_zones[1]);
        editor.undo(&mut host);
        assert_eq!(editor.graph().unwrap().nodes.len(), 2);
        editor.undo(&mut host);
        assert!(editor.graph().unwrap().nodes.is_empty());
    }
}
