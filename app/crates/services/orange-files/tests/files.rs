//! A graph writes and reads a file through a declared folder and is triggered by a
//! change in it.
use orange_testkit::{activation, core, edge, id, node, set, until};

use orange_host::{EventTiming, FlowEvent, Host, HostOptions, RunState, Value, VariableId};
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
        18
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
        {"id": id(101), "name": "changed", "type": orange_files::CHANGE_TYPE, "scope": "run"}
    ]);
    let text = to_json(&from_json(&saved.to_string()).unwrap()).unwrap();
    let mut draft = host.draft(&text).unwrap();
    assert!(
        host.set_entry_timing(
            &mut draft,
            node(10),
            Some(EventTiming::Debounce {
                interval_ms: 150.try_into().unwrap()
            }),
        )
        .unwrap()
    );
    let text = draft.text().unwrap();
    assert!(host.check(&text).unwrap().is_empty());

    let flow = host.open(&text).unwrap();
    let read = |n: u32| {
        let variable: VariableId = id(n).parse().unwrap();
        match host
            .inspect(flow, move |run| run.variable(variable).cloned())
            .unwrap()
        {
            Some(Value::Str(text)) => Some(text.as_str().to_owned()),
            Some(Value::Record(change)) => match &change.fields[1] {
                Some(Value::Str(path)) => Some(path.as_str().to_owned()),
                _ => None,
            },
            _ => None,
        }
    };
    // The first scan establishes a baseline. Probe with a changing file to know the
    // watcher is active before checking the graph's own write.
    let stats = || host.inspect(flow, |run| run.event_stats()).unwrap();
    let mut probe = 0;
    until("the watcher to establish its baseline", || {
        if stats().accepted > 0 {
            return true;
        }
        probe += 1;
        std::fs::write(root.join("baseline.txt"), probe.to_string()).unwrap();
        false
    });
    until("the baseline change to settle", || {
        read(101).as_deref() == Some("baseline.txt")
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
    host.pause(flow).unwrap();
    until("the flow to pause", || {
        host.state(flow) == Some(RunState::Paused)
    });
    let before = stats();
    std::fs::write(root.join("first.txt"), "earlier").unwrap();
    until("the first file change to arrive", || {
        stats().accepted > before.accepted
    });
    let first = stats().accepted;
    std::fs::write(root.join("last.txt"), "newest").unwrap();
    until("the last file change to arrive", || {
        stats().accepted > first
    });
    assert_eq!(stats().pending, 1);
    assert_eq!(stats().delivered, before.delivered);
    host.resume(flow).unwrap();
    until("the newest file change to fire", || {
        read(101).as_deref() == Some("last.txt")
    });
    assert_eq!(stats().delivered, before.delivered + 1);
    host.close(flow).unwrap();
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn a_wired_folder_scopes_a_watcher_and_reports_an_unavailable_path() {
    watched_folder_appearance(false);
}

#[test]
fn a_watcher_can_wait_for_its_folder_without_reporting_a_fault() {
    watched_folder_appearance(true);
}

fn watched_folder_appearance(wait_for_folder: bool) {
    let root = std::env::temp_dir().join(format!(
        "orange-files-wired-{}-{wait_for_folder}",
        std::process::id()
    ));
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
            id: orange_files::ON_CHANGE.parse().unwrap(),
            version: 1,
        },
    );
    new.config = serde_json::from_value(json!({"connection": FOLDER})).unwrap();
    let edit = new_node_edit(&document, host.registry(), new).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    saved["nodes"][0]["activation"] = activation();
    saved["nodes"][0]["inputs"]["wait_for_folder"] =
        json!({"type": "core/bool@1", "value": wait_for_folder});
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
        {"id": id(101), "name": "change", "type": orange_files::CHANGE_TYPE, "scope": "run"},
        {"id": id(102), "name": "folder", "type": STRING, "scope": "run",
            "init": {"type": STRING, "value": "chosen"}},
    ]);
    let text = to_json(&from_json(&saved.to_string()).unwrap()).unwrap();
    let diagnostics = host.check(&text).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let flow = host.open(&text).unwrap();
    let missing_folder = || {
        events.try_iter().any(|event| {
            matches!(event.kind,
            FlowEvent::SourceFault { node: entry, fault } if entry == node(10)
                && fault.code.as_str() == "org.orange.files/NotFound")
        })
    };
    if wait_for_folder {
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(!missing_folder());
    } else {
        until("the missing watched folder to be reported", missing_folder);
    }
    let read = |host: &Host| {
        let variable: VariableId = id(101).parse().unwrap();
        match host
            .inspect(flow, move |run| run.variable(variable).cloned())
            .unwrap()
        {
            Some(Value::Record(change)) => match &change.fields[1] {
                Some(Value::Str(path)) => Some(path.as_str().to_owned()),
                _ => None,
            },
            _ => None,
        }
    };
    std::fs::create_dir(root.join("chosen")).unwrap();
    std::fs::write(root.join("outside.txt"), "outside").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert_eq!(read(&host), None);
    // The watcher starts on the new folder at its next poll and takes what is there
    // then as its baseline, which never fires. Writing again until the change is seen
    // keeps the test from depending on how soon that poll comes.
    let mut checks = 0u32;
    until("the file change in the wired folder", || {
        // About every 100 ms: `until` checks every 2 ms.
        if checks.is_multiple_of(50) {
            std::fs::write(root.join("chosen/inside.txt"), format!("inside {checks}")).unwrap();
        }
        checks += 1;
        read(&host).as_deref() == Some("chosen/inside.txt")
    });
    host.close(flow).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

/// One manual entry runs every file action in a chain, each on what the one before
/// it made: bytes are written, grown and read back in part, text is appended, files
/// and folders are copied, moved and deleted, and the results are kept in variables.
#[test]
fn every_file_action_runs_in_one_chain() {
    use orange_host::{VarScope, Variable};
    let root = std::env::temp_dir()
        .join(format!("orange-files-chain-{}", std::process::id()))
        .join("folder");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
    std::fs::create_dir_all(&root).unwrap();
    let (mut host, _events) =
        Host::with_services(HostOptions::default(), vec![orange_files::package()]).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let manual = host
        .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
        .unwrap();
    let mut previous = manual;
    // Each step: the action, then its inputs as a person types them.
    let steps: [(&str, &[(&str, &str)]); 16] = [
        (
            orange_files::WRITE_BYTES,
            &[
                ("path", "data/a.bin"),
                ("data", r#"{"type":"core/bytes@1","value":"AAEC"}"#),
            ],
        ),
        (
            "org.orange.files/append-bytes",
            &[
                ("path", "data/a.bin"),
                ("data", r#"{"type":"core/bytes@1","value":"AwQ="}"#),
            ],
        ),
        (
            "org.orange.files/read-range",
            &[
                ("path", "data/a.bin"),
                ("offset", r#"{"type":"core/u64@1","value":"1"}"#),
                ("length", r#"{"type":"core/u64@1","value":"3"}"#),
            ],
        ),
        (orange_files::READ_BYTES, &[("path", "data/a.bin")]),
        (
            orange_files::WRITE_TEXT,
            &[("path", "notes.txt"), ("text", "one")],
        ),
        (
            orange_files::APPEND_TEXT,
            &[("path", "notes.txt"), ("text", "-two")],
        ),
        (orange_files::READ_TEXT, &[("path", "notes.txt")]),
        (
            orange_files::COPY,
            &[("path", "notes.txt"), ("to", "copy.txt")],
        ),
        (
            orange_files::MOVE,
            &[("path", "copy.txt"), ("to", "moved.txt")],
        ),
        (orange_files::MAKE_DIRECTORY, &[("path", "empty/inner")]),
        (
            orange_files::COPY_DIRECTORY,
            &[("path", "data"), ("to", "data-copy")],
        ),
        (
            orange_files::MOVE_DIRECTORY,
            &[("path", "data-copy"), ("to", "archive")],
        ),
        (orange_files::DELETE_DIRECTORY, &[("path", "empty")]),
        (orange_files::DELETE, &[("path", "moved.txt")]),
        (orange_files::EXISTS, &[("path", "archive/a.bin")]),
        (orange_files::FILE_INFO, &[("path", "notes.txt")]),
    ];
    let mut nodes = Vec::new();
    for (definition, inputs) in steps {
        let action = host.add_node(&mut draft, definition, [0.0, 0.0]).unwrap();
        for (port, value) in inputs {
            assert!(
                host.set_input(&mut draft, (action, port), Some(value))
                    .unwrap(),
                "{definition} {port}"
            );
        }
        host.connect(&mut draft, (previous, "then"), (action, "in"))
            .unwrap();
        previous = action;
        nodes.push(action);
    }
    // Keep what the reading steps found.
    let mut kept = Vec::new();
    for (step, output, ty) in [
        (2, "data", "core/bytes@1"),
        (3, "data", "core/bytes@1"),
        (6, "text", "core/string@1"),
        (14, "found", "core/bool@1"),
        (15, "bytes", "core/u64@1"),
    ] {
        let variable = host
            .declare_variable(
                &mut draft,
                &Variable {
                    id: None,
                    name: format!("{output} {step}"),
                    ty: ty.into(),
                    scope: VarScope::Run,
                    persistent: false,
                    initial: None,
                },
            )
            .unwrap();
        let store = host
            .add_node(&mut draft, "core/set-variable", [0.0, 0.0])
            .unwrap();
        host.set_config(&mut draft, store, &json!({"variable": variable}))
            .unwrap();
        host.connect(&mut draft, (nodes[step], output), (store, "value"))
            .unwrap();
        host.connect(&mut draft, (previous, "then"), (store, "in"))
            .unwrap();
        previous = store;
        kept.push(variable.parse::<VariableId>().unwrap());
    }
    let mut folder = host.declarations(&draft).connections.remove(0);
    folder.settings = json!({"path": root.to_str().unwrap()});
    host.declare_connection(&mut draft, &folder).unwrap();
    let text = draft.text().unwrap();
    let diagnostics = host.check(&text).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let flow = host.open(&text).unwrap();
    host.trigger(flow, manual, None).unwrap();
    let read = |index: usize| {
        let variable = kept[index];
        host.inspect(flow, move |run| run.variable(variable).cloned())
            .unwrap()
    };
    until("the chain to finish", || read(4).is_some());
    let bytes = |value: Option<Value>| match value {
        Some(Value::Bytes(data)) => data.as_ref().clone(),
        other => panic!("not bytes: {other:?}"),
    };
    assert_eq!(bytes(read(0)), [1, 2, 3]);
    assert_eq!(bytes(read(1)), [0, 1, 2, 3, 4]);
    assert_eq!(
        read(2),
        Some(Value::Str(std::sync::Arc::new("one-two".into())))
    );
    assert_eq!(read(3), Some(Value::Bool(true)));
    assert_eq!(read(4), Some(Value::U64(7)));
    assert!(root.join("archive/a.bin").is_file());
    assert!(!root.join("data-copy").exists());
    assert!(!root.join("empty").exists());
    assert!(!root.join("moved.txt").exists());
    assert!(!root.join("copy.txt").exists());
    host.close(flow).unwrap();
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}
