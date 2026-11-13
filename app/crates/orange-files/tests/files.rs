//! A graph writes and reads a file through a declared folder and is triggered by a
//! change in it.
use std::time::{Duration, Instant};

use orange_host::{Host, HostOptions, NodeId, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{DocCommand, GraphDocument, from_json, to_json};
use serde_json::{Value as Json, json};

const STRING: &str = "core/string@1";
const FOLDER: &str = "40000000-0000-4000-8000-000000000001";
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
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_graph_writes_reads_and_watches_files_in_its_folder() {
    let root = std::env::temp_dir()
        .join(format!("orange-files-graph-{}", std::process::id()))
        .join("folder");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
    std::fs::create_dir_all(&root).unwrap();
    let (mut host, _events) =
        Host::with_services(HostOptions::default(), vec![orange_files::package()]).unwrap();
    // Every node of the package is registered with a valid definition.
    assert_eq!(
        host.nodes()
            .filter(|info| info.category() == "Files")
            .count(),
        9
    );

    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let folder = json!({"id": FOLDER, "name": "work", "adapter": orange_files::FOLDER_ADAPTER,
        "scope": "graph", "settings": {"path": root.to_str().unwrap(), "poll_ms": 50}});
    let _ = document
        .apply([DocCommand::UpsertConnection(
            serde_json::from_value(folder).unwrap(),
        )])
        .unwrap();
    let path = json!({"type": STRING, "value": "out/hello.txt"});
    for (n, definition, inputs) in [
        (
            2,
            orange_files::WRITE_TEXT,
            json!({"path": path, "text": {"type": STRING, "value": "héllo"}}),
        ),
        (3, orange_files::READ_TEXT, json!({"path": path})),
        (10, orange_files::ON_CHANGE, json!({})),
    ] {
        let mut new = NewNode::new(
            node(n),
            DefinitionRef {
                id: definition.parse().unwrap(),
                version: 1,
            },
        );
        new.config = serde_json::from_value(json!({"connection": FOLDER})).unwrap();
        new.inputs = serde_json::from_value(inputs).unwrap();
        let edit = new_node_edit(&document, host.registry(), new).unwrap();
        let _ = document.apply(edit.commands).unwrap();
    }
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    for entry in saved["nodes"].as_array_mut().unwrap() {
        if entry["definition_id"] == orange_files::ON_CHANGE {
            entry["activation"] = activation();
        }
    }
    saved["nodes"].as_array_mut().unwrap().extend([
        core(1, "on-manual"),
        set(4, 100),
        set(11, 101),
    ]);
    saved["edges"] = json!([
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 3, "text", 4, "value"),
        edge(5, 10, "then", 11, "in"),
        edge(6, 10, "payload", 11, "value"),
    ]);
    saved["variables"] = json!([
        {"id": id(100), "name": "read", "type": STRING, "scope": "run"},
        {"id": id(101), "name": "changed", "type": STRING, "scope": "run"}
    ]);
    let text = to_json(&from_json(&saved.to_string()).unwrap()).unwrap();
    assert!(host.check(&text).unwrap().is_empty());

    let flow = host.open(&text).unwrap();
    let read = |n: u32| {
        let variable: VariableId = id(n).parse().unwrap();
        match host
            .inspect(flow, move |run| run.variable(variable).cloned())
            .unwrap()
        {
            Some(Value::Str(text)) => Some(text.as_str().to_owned()),
            _ => None,
        }
    };
    host.trigger(flow, node(1), None).unwrap();
    until("the file to be read back", || read(100).is_some());
    assert_eq!(read(100).as_deref(), Some("héllo"));
    assert_eq!(
        std::fs::read_to_string(root.join("out").join("hello.txt")).unwrap(),
        "héllo"
    );
    // The write is itself a change in the folder, and so is a file made by another
    // program.
    until("the change entry", || {
        read(101).as_deref() == Some("out/hello.txt")
    });
    std::fs::write(root.join("other.txt"), "from outside").unwrap();
    until("the outside change", || {
        read(101).as_deref() == Some("other.txt")
    });
    host.close(flow).unwrap();
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}
