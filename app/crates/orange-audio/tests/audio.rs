//! A graph plays a sound file on a declared output connection and listens to an input.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use orange_host::{FlowId, Host, HostOptions, NodeId, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{GraphDocument, from_json, to_json};
use serde_json::{Value as Json, json};

const I64: &str = "core/i64@1";
const F64: &str = "core/f64@1";
const STRING: &str = "core/string@1";
const OUTPUT: &str = "40000000-0000-4000-8000-000000000001";
const INPUT: &str = "40000000-0000-4000-8000-000000000002";
const WAIT: Duration = Duration::from_secs(20);

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn node(n: u32) -> NodeId {
    id(n).parse().unwrap()
}
fn variable(n: u32) -> VariableId {
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
fn set(n: u32, target: u32, value: i64) -> Json {
    let mut node = core(n, "set-variable");
    node["config"] = json!({"variable": id(target)});
    node["inputs"] = json!({"value": {"type": I64, "value": value.to_string()}});
    node
}
fn edge(n: u32, from: u32, out: &str, to: u32, input: &str) -> Json {
    json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
        "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": input}})
}
fn host() -> Host {
    Host::with_services(HostOptions::default(), vec![orange_audio::package()])
        .unwrap()
        .0
}
fn read(host: &Host, flow: FlowId, n: u32) -> Option<Value> {
    host.inspect(flow, move |run| run.variable(variable(n)).cloned())
        .unwrap()
}
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Writes a mono 8 kHz tone of `millis` and returns its path.
fn tone(name: &str, millis: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orange-audio-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let spec = hound_spec();
    let mut bytes = Vec::new();
    let samples = 8 * millis;
    // A minimal 16-bit PCM WAV file.
    let data = samples * 2;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&spec);
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for n in 0..samples {
        let value: i16 = if n % 16 < 8 { 8_000 } else { -8_000 };
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(&path, bytes).unwrap();
    path
}
/// The `fmt ` chunk of 16-bit mono PCM at 8 kHz.
fn hound_spec() -> Vec<u8> {
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&16u32.to_le_bytes());
    chunk.extend_from_slice(&1u16.to_le_bytes());
    chunk.extend_from_slice(&1u16.to_le_bytes());
    chunk.extend_from_slice(&8_000u32.to_le_bytes());
    chunk.extend_from_slice(&16_000u32.to_le_bytes());
    chunk.extend_from_slice(&2u16.to_le_bytes());
    chunk.extend_from_slice(&16u16.to_le_bytes());
    chunk
}

/// A manual entry plays `path` on the silent device, then stores 1; `failed` stores -1.
/// A second manual entry stops the device's sounds.
fn document(host: &Host, path: &Path, device: &str) -> String {
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let output = json!({"id": OUTPUT, "name": "speakers",
        "adapter": orange_audio::OUTPUT_ADAPTER, "scope": "graph",
        "settings": {"device": device}});
    let input = json!({"id": INPUT, "name": "microphone",
        "adapter": orange_audio::INPUT_ADAPTER, "scope": "graph",
        "settings": {"device": orange_audio::NULL_DEVICE, "threshold_db": -20}});
    for connection in [output, input] {
        let decl = serde_json::from_value(connection).unwrap();
        let _ = document
            .apply([orange_model::DocCommand::UpsertConnection(decl)])
            .unwrap();
    }
    let place =
        |document: &mut GraphDocument, n: u32, name: &str, connection: &str, inputs: Json| {
            let mut new = NewNode::new(
                node(n),
                DefinitionRef {
                    id: name.parse().unwrap(),
                    version: 1,
                },
            );
            new.config = serde_json::from_value(json!({"connection": connection})).unwrap();
            new.inputs = serde_json::from_value(inputs).unwrap();
            let edit = new_node_edit(document, host.registry(), new).unwrap();
            let _ = document.apply(edit.commands).unwrap();
        };
    place(
        &mut document,
        3,
        orange_audio::PLAY,
        OUTPUT,
        json!({"path": {"type": STRING, "value": path.to_str().unwrap()},
            "gain": {"type": F64, "bits": format!("{:016x}", 1.0f64.to_bits())}}),
    );
    place(&mut document, 8, orange_audio::STOP, OUTPUT, json!({}));
    place(
        &mut document,
        10,
        orange_audio::INPUT_LEVEL,
        INPUT,
        json!({}),
    );
    place(&mut document, 12, orange_audio::ON_LEVEL, INPUT, json!({}));
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    let mut level = core(11, "set-variable");
    level["config"] = json!({"variable": id(101)});
    let nodes = saved["nodes"].as_array_mut().unwrap();
    for node in nodes.iter_mut() {
        if node["definition_id"] == orange_audio::ON_LEVEL {
            node["activation"] = json!({"mode": "queue", "capacity": 4, "overflow": "fault",
                "on_fault": "continue", "order": 0});
        }
    }
    nodes.extend([
        core(1, "on-manual"),
        set(4, 100, 1),
        set(6, 100, -1),
        core(7, "on-manual"),
        core(9, "on-manual"),
        level,
    ]);
    saved["edges"] = json!([
        edge(1, 1, "then", 3, "in"),
        edge(2, 3, "then", 4, "in"),
        edge(3, 3, "failed", 6, "in"),
        edge(4, 7, "then", 8, "in"),
        edge(5, 9, "then", 10, "in"),
        edge(6, 10, "then", 11, "in"),
        edge(7, 10, "level", 11, "value"),
    ]);
    saved["variables"] = json!([
        {"id": id(100), "name": "played", "type": I64, "scope": "run"},
        {"id": id(101), "name": "level", "type": F64, "scope": "run"}
    ]);
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

#[test]
fn a_graph_plays_a_file_and_continues_when_it_has_ended() {
    let mut host = host();
    let path = tone("short.wav", 60);
    let text = document(&host, &path, orange_audio::NULL_DEVICE);
    assert!(host.check(&text).unwrap().is_empty());
    let flow = host.open(&text).unwrap();
    let started = Instant::now();
    host.trigger(flow, node(1), None).unwrap();
    until("the sound to end", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(1)));
    // The node waited for the sound instead of returning at once.
    assert!(started.elapsed() >= Duration::from_millis(40));
}

#[test]
fn a_stop_node_ends_a_long_sound_early() {
    let mut host = host();
    let path = tone("long.wav", 30_000);
    let text = document(&host, &path, orange_audio::NULL_DEVICE);
    let flow = host.open(&text).unwrap();
    let started = Instant::now();
    host.trigger(flow, node(1), None).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(read(&host, flow, 100), None);
    // Decoding takes a machine-dependent time and a stop before the sound has started
    // stops nothing, so stop until it has ended.
    let mut last = Instant::now() - Duration::from_secs(1);
    until("the sound to stop", || {
        if last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            host.trigger(flow, node(7), None).unwrap();
        }
        read(&host, flow, 100).is_some()
    });
    assert_eq!(read(&host, flow, 100), Some(Value::I64(1)));
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[test]
fn a_missing_file_or_device_takes_the_failed_route() {
    let mut host = host();
    let missing = std::env::temp_dir().join("orange-audio-no-such-file.wav");
    let text = document(&host, &missing, orange_audio::NULL_DEVICE);
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the failure", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(-1)));

    // The document opens although the device does not exist; playing is what fails.
    let path = tone("unused.wav", 20);
    let text = document(&host, &path, "no such device 7f3a");
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the failure", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(-1)));
}

#[test]
fn the_silent_input_reports_silence_and_never_fires() {
    let mut host = host();
    let path = tone("idle.wav", 20);
    let text = document(&host, &path, orange_audio::NULL_DEVICE);
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(9), None).unwrap();
    until("the level", || read(&host, flow, 101).is_some());
    assert_eq!(
        read(&host, flow, 101),
        Some(Value::F64(orange_audio::SILENCE_DB))
    );
}

#[test]
fn the_audio_nodes_are_listed_with_their_text() {
    let host = host();
    let play = host
        .nodes()
        .find(|info| info.definition.definition.id.as_str() == orange_audio::PLAY)
        .unwrap();
    assert_eq!(
        play.presentation.unwrap().label.as_ref().unwrap().fallback,
        "Play sound"
    );
    assert_eq!(
        play.presentation
            .unwrap()
            .category
            .as_ref()
            .unwrap()
            .fallback,
        "Audio"
    );
}

/// Plays a short tone on the real default device. Run by hand:
/// `cargo test -p orange-audio --test audio -- --ignored`.
#[test]
#[ignore = "needs an audio output device and makes a sound"]
fn a_tone_plays_on_the_default_device() {
    let mut host = host();
    let path = tone("beep.wav", 300);
    let mut saved: Json =
        serde_json::from_str(&document(&host, &path, orange_audio::NULL_DEVICE)).unwrap();
    let _ = saved["connections"][0]["settings"]
        .as_object_mut()
        .unwrap()
        .remove("device");
    let flow = host.open(&saved.to_string()).unwrap();
    println!("{:?}", orange_audio::devices());
    let started = Instant::now();
    host.trigger(flow, node(1), None).unwrap();
    until("the sound to end", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(1)));
    assert!(started.elapsed() >= Duration::from_millis(250));
}
