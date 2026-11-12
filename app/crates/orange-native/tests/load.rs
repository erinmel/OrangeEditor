//! Builds the example plugin library, loads it and runs its node in a graph.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use orange_host::{Host, HostOptions, NodeId, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{GraphDocument, from_json, to_json};
use orange_native::{LoadError, load, load_directory};
use serde_json::{Value as Json, json};

const STRING: &str = "core/string@1";
const SHOUT: &str = "org.example.shout/shout";

/// The directory Cargo writes this build's libraries to: the parent of `deps`, where
/// this test binary is.
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().to_owned()
}

/// Builds the example plugin once, with the profile these tests were built with, and
/// returns the library's path.
fn plugin() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let mut build = Command::new(env!("CARGO"));
        let _ = build
            .args(["build", "--locked", "-p", "orange-plugin-example"])
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        if !cfg!(debug_assertions) {
            let _ = build.arg("--release");
        }
        let status = build.status().unwrap();
        assert!(status.success(), "the example plugin did not build");
        let name = format!(
            "{}orange_plugin_example{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        );
        let library = profile_dir().join(name);
        assert!(library.is_file(), "{} was not built", library.display());
        library
    })
}

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn node(n: u32) -> NodeId {
    id(n).parse().unwrap()
}
fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id": id(n), "definition_id": format!("core/{name}"),
        "definition_version": 1, "config_version": 1, "config": {}});
    if name.starts_with("on-") {
        node["activation"] = json!({"mode": "queue", "capacity": 4, "overflow": "fault",
            "on_fault": "continue", "order": 0});
    }
    node
}
fn edge(n: u32, from: u32, out: &str, to: u32, input: &str) -> Json {
    json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
        "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": input}})
}

#[test]
fn a_loaded_plugins_node_is_listed_and_runs_in_a_graph() {
    let package = load(plugin()).unwrap();
    assert_eq!(package.id(), "org.example.shout");
    assert_eq!(package.nodes().len(), 1);
    let (mut host, _events) = Host::with_services(HostOptions::default(), vec![package]).unwrap();

    let listed = host
        .nodes()
        .find(|info| info.definition.definition.id.as_str() == SHOUT)
        .unwrap();
    assert_eq!(listed.label(), "Shout");
    assert_eq!(listed.category(), "Example");
    assert!(
        host.packages()
            .any(|package| package.id == "org.example.shout")
    );

    // A manual entry shouts a constant and stores the result.
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let mut shout = NewNode::new(
        node(2),
        DefinitionRef {
            id: SHOUT.parse().unwrap(),
            version: 1,
        },
    );
    shout.inputs =
        serde_json::from_value(json!({"text": {"type": STRING, "value": "hello"}})).unwrap();
    let edit = new_node_edit(&document, host.registry(), shout).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    let mut keep = core(3, "set-variable");
    keep["config"] = json!({"variable": id(100)});
    saved["nodes"]
        .as_array_mut()
        .unwrap()
        .extend([core(1, "on-manual"), keep]);
    saved["edges"] = json!([
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 2, "loud", 3, "value"),
    ]);
    saved["variables"] = json!([{"id": id(100), "name": "said", "type": STRING, "scope": "run"}]);
    let text = to_json(&from_json(&saved.to_string()).unwrap()).unwrap();
    assert!(host.check(&text).unwrap().is_empty());

    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    let variable: VariableId = id(100).parse().unwrap();
    let started = Instant::now();
    let said = loop {
        let value = host
            .inspect(flow, move |run| run.variable(variable).cloned())
            .unwrap();
        if let Some(value) = value {
            break value;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "timed out");
        std::thread::sleep(Duration::from_millis(2));
    };
    let Value::Str(said) = said else {
        panic!("the result is text");
    };
    assert_eq!(said.as_str(), "HELLO!");
}

#[test]
fn files_that_are_not_plugins_are_refused_without_running_them() {
    let directory = std::env::temp_dir().join(format!("orange-native-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let extension = std::env::consts::DLL_EXTENSION;

    // Not a library at all.
    let garbage = directory.join(format!("a-garbage.{extension}"));
    std::fs::write(&garbage, b"not a library").unwrap();
    assert!(matches!(load(&garbage), Err(LoadError::Open(_))));
    assert!(matches!(
        load(&directory.join("missing.dll")),
        Err(LoadError::Open(_))
    ));

    // The real plugin beside it still loads, and other files are ignored.
    let copy = directory.join(format!("b-shout.{extension}"));
    let _ = std::fs::copy(plugin(), &copy).unwrap();
    std::fs::write(directory.join("readme.txt"), b"ignored").unwrap();
    let loaded = load_directory(&directory).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].0, garbage);
    assert!(matches!(loaded[0].1, Err(LoadError::Open(_))));
    assert_eq!(loaded[1].1.as_ref().unwrap().id(), "org.example.shout");

    assert!(matches!(
        load_directory(&directory.join("no-such-directory")),
        Err(LoadError::Directory(_))
    ));
    // The loaded library stays mapped, so its copy may not be removable on Windows.
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_library_without_the_plugin_symbols_is_not_a_plugin() {
    let system = if cfg!(windows) {
        "kernel32.dll"
    } else if cfg!(target_os = "macos") {
        "/usr/lib/libSystem.B.dylib"
    } else {
        "libc.so.6"
    };
    assert_eq!(load(Path::new(system)).err(), Some(LoadError::NotAPlugin));
}
