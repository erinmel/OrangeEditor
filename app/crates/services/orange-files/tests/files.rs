//! A graph writes and reads a file through a declared folder and is triggered by a
//! change in it.
use orange_testkit::{activation, core, edge, id, node, set, until};

use orange_host::{FlowEvent, Host, HostOptions, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{DocCommand, GraphDocument, from_json, to_json};
use serde_json::{Value as Json, json};

const STRING: &str = "core/string@1";
const FOLDER: &str = "40000000-0000-4000-8000-000000000001";

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
        22
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
    // The first scan establishes a baseline. Probe with a changing file to know the
    // watcher is active before checking the graph's own write.
    let mut probe = 0;
    until("the watcher to establish its baseline", || {
        if read(101).as_deref() == Some("baseline.txt") {
            return true;
        }
        probe += 1;
        std::fs::write(root.join("baseline.txt"), probe.to_string()).unwrap();
        false
    });
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

#[test]
fn a_wired_folder_scopes_a_watcher_and_reports_an_unavailable_path() {
    let root = std::env::temp_dir().join(format!("orange-files-wired-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let (mut host, events) =
        Host::with_services(HostOptions::default(), vec![orange_files::package()]).unwrap();
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000002".parse().unwrap());
    let connection = json!({"id": FOLDER, "name": "work", "adapter": orange_files::FOLDER_ADAPTER,
        "scope": "graph", "settings": {"path": root.to_str().unwrap(), "poll_ms": 50}});
    let _ = document
        .apply([DocCommand::UpsertConnection(
            serde_json::from_value(connection).unwrap(),
        )])
        .unwrap();
    let mut new = NewNode::new(
        node(10),
        DefinitionRef {
            id: orange_files::ON_PATH_CHANGE.parse().unwrap(),
            version: 1,
        },
    );
    new.config = serde_json::from_value(json!({"connection": FOLDER})).unwrap();
    let edit = new_node_edit(&document, host.registry(), new).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    saved["nodes"][0]["activation"] = activation();
    let mut get = core(12, "get-variable");
    get["config"] = json!({"variable": id(102)});
    saved["nodes"]
        .as_array_mut()
        .unwrap()
        .extend([set(11, 101), get]);
    saved["edges"] = json!([
        edge(1, 10, "then", 11, "in"),
        edge(2, 10, "payload", 11, "value"),
        edge(3, 12, "value", 10, "folder"),
    ]);
    saved["variables"] = json!([
        {"id": id(101), "name": "change", "type": "core/json@1", "scope": "run"},
        {"id": id(102), "name": "folder", "type": STRING, "scope": "run",
            "init": {"type": STRING, "value": "chosen"}},
    ]);
    let text = to_json(&from_json(&saved.to_string()).unwrap()).unwrap();
    let diagnostics = host.check(&text).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let flow = host.open(&text).unwrap();
    until("the missing watched folder to be reported", || {
        events.try_iter().any(|event| {
            matches!(event.kind,
            FlowEvent::SourceFault { node: entry, fault } if entry == node(10)
                && fault.code.as_str() == "org.orange.files/NotFound")
        })
    });
    let read = |host: &Host| {
        let variable: VariableId = id(101).parse().unwrap();
        match host
            .inspect(flow, move |run| run.variable(variable).cloned())
            .unwrap()
        {
            Some(Value::Json(text)) => Some(serde_json::from_str::<Json>(text.as_str()).unwrap()),
            _ => None,
        }
    };
    std::fs::create_dir(root.join("chosen")).unwrap();
    std::fs::write(root.join("outside.txt"), "outside").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert_eq!(read(&host), None);
    std::fs::write(root.join("chosen/inside.txt"), "inside").unwrap();
    until("the file change in the wired folder", || {
        read(&host).is_some_and(|value| value["path"] == "chosen/inside.txt")
    });
    host.close(flow).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
