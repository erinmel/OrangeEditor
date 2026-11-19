//! A graph plays a sound file on a declared output connection and listens to an input.
use orange_testkit::{activation, core, edge, id, node, until, variable};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use orange_host::{FlowId, Host, HostOptions, Value};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{GraphDocument, from_json, to_json};
use serde_json::{Value as Json, json};

const I64: &str = "core/i64@1";
const F64: &str = "core/f64@1";
const STRING: &str = "core/string@1";
const OUTPUT: &str = "40000000-0000-4000-8000-000000000001";
const INPUT: &str = "40000000-0000-4000-8000-000000000002";

fn set(n: u32, target: u32, value: i64) -> Json {
    let mut node = core(n, "set-variable");
    node["config"] = json!({"variable": id(target)});
    node["inputs"] = json!({"value": {"type": I64, "value": value.to_string()}});
    node
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

/// Writes a mono 8 kHz tone of `millis` and returns its path.
fn tone(name: &str, millis: u32) -> PathBuf {
    wav_signal(name, millis, |n| if n % 16 < 8 { 8_000 } else { -8_000 })
}

fn wav_signal(name: &str, millis: u32, sample: impl Fn(u32) -> i16) -> PathBuf {
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
        let value = sample(n);
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
/// A second manual entry stops the device's sounds. `play` is the playing node. Manual
/// entries 20, 22, 24 and 26 pause, resume, seek to 0.95 s and set the volume, which
/// then stores 5.
fn document(host: &Host, path: &Path, device: &str, play: &str) -> String {
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
        play,
        OUTPUT,
        json!({"path": {"type": STRING, "value": path.to_str().unwrap()},
            "gain": {"type": F64, "bits": format!("{:016x}", 1.0f64.to_bits())}}),
    );
    place(&mut document, 8, orange_audio::STOP, OUTPUT, json!({}));
    let number = |name: &str, value: f64| json!({name: {"type": F64, "bits": format!("{:016x}", value.to_bits())}});
    place(&mut document, 21, orange_audio::PAUSE, OUTPUT, json!({}));
    place(&mut document, 23, orange_audio::RESUME, OUTPUT, json!({}));
    place(
        &mut document,
        25,
        orange_audio::SEEK,
        OUTPUT,
        number("seconds", 0.95),
    );
    place(
        &mut document,
        27,
        orange_audio::SET_VOLUME,
        OUTPUT,
        number("gain", 0.5),
    );
    place(
        &mut document,
        10,
        orange_audio::INPUT_LEVEL,
        INPUT,
        json!({}),
    );
    place(
        &mut document,
        12,
        orange_audio::ON_BAND_LEVEL,
        INPUT,
        json!({}),
    );
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    let mut level = core(11, "set-variable");
    level["config"] = json!({"variable": id(101)});
    let nodes = saved["nodes"].as_array_mut().unwrap();
    for node in nodes.iter_mut() {
        if node["definition_id"] == orange_audio::ON_BAND_LEVEL {
            node["activation"] = json!({"mode": "queue", "capacity": 4, "overflow": "fault",
                "on_fault": "continue", "order": 0});
            // The threshold is the entry's own setting, not the microphone's.
            node["config"]["threshold_db"] = json!(-20);
        }
    }
    nodes.extend([
        core(1, "on-manual"),
        set(4, 100, 1),
        set(6, 100, -1),
        core(7, "on-manual"),
        core(9, "on-manual"),
        level,
        core(20, "on-manual"),
        core(22, "on-manual"),
        core(24, "on-manual"),
        core(26, "on-manual"),
        set(28, 102, 5),
    ]);
    saved["edges"] = json!([
        edge(1, 1, "then", 3, "in"),
        edge(2, 3, "then", 4, "in"),
        edge(3, 3, "failed", 6, "in"),
        edge(4, 7, "then", 8, "in"),
        edge(5, 9, "then", 10, "in"),
        edge(6, 10, "then", 11, "in"),
        edge(7, 10, "level", 11, "value"),
        edge(8, 20, "then", 21, "in"),
        edge(9, 22, "then", 23, "in"),
        edge(10, 24, "then", 25, "in"),
        edge(11, 26, "then", 27, "in"),
        edge(12, 27, "then", 28, "in"),
    ]);
    saved["variables"] = json!([
        {"id": id(100), "name": "played", "type": I64, "scope": "run"},
        {"id": id(101), "name": "level", "type": F64, "scope": "run"},
        {"id": id(102), "name": "volume set", "type": I64, "scope": "run"}
    ]);
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

#[test]
fn a_graph_plays_a_file_and_continues_when_it_has_ended() {
    let mut host = host();
    let path = tone("short.wav", 60);
    let text = document(&host, &path, orange_audio::NULL_DEVICE, orange_audio::PLAY);
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
    let text = document(&host, &path, orange_audio::NULL_DEVICE, orange_audio::PLAY);
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
fn a_looped_sound_plays_until_it_is_stopped() {
    let mut host = host();
    let path = tone("loop.wav", 50);
    let text = document(&host, &path, orange_audio::NULL_DEVICE, orange_audio::LOOP);
    assert!(host.check(&text).unwrap().is_empty());
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    // Many times the sound's length later the node is still playing.
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(read(&host, flow, 100), None);
    let mut last = Instant::now() - Duration::from_secs(1);
    until("the loop to stop", || {
        if last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            host.trigger(flow, node(7), None).unwrap();
        }
        read(&host, flow, 100).is_some()
    });
    assert_eq!(read(&host, flow, 100), Some(Value::I64(1)));
}

#[test]
fn sounds_pause_resume_and_seek_and_the_volume_is_set() {
    let mut host = host();
    let path = tone("second.wav", 1_000);
    let text = document(&host, &path, orange_audio::NULL_DEVICE, orange_audio::PLAY);
    assert!(host.check(&text).unwrap().is_empty());
    let flow = host.open(&text).unwrap();
    // Setting the volume before anything plays opens the device.
    host.trigger(flow, node(26), None).unwrap();
    until("the volume", || {
        read(&host, flow, 102) == Some(Value::I64(5))
    });

    host.trigger(flow, node(1), None).unwrap();
    // A pause before the sound has started holds nothing, so pause for a while.
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(50));
        host.trigger(flow, node(20), None).unwrap();
    }
    // Paused, the one-second sound is still there long after it would have ended.
    std::thread::sleep(Duration::from_millis(1_500));
    assert_eq!(read(&host, flow, 100), None);

    // Resumed and moved near its end, it ends at once and the graph continues.
    host.trigger(flow, node(22), None).unwrap();
    host.trigger(flow, node(24), None).unwrap();
    let moved = Instant::now();
    until("the sound to end", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(1)));
    assert!(moved.elapsed() < Duration::from_millis(500));
}

#[test]
fn a_missing_file_or_device_takes_the_failed_route() {
    let mut host = host();
    let missing = std::env::temp_dir().join("orange-audio-no-such-file.wav");
    let text = document(
        &host,
        &missing,
        orange_audio::NULL_DEVICE,
        orange_audio::PLAY,
    );
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the failure", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(-1)));

    // The document opens although the device does not exist; playing is what fails.
    let path = tone("unused.wav", 20);
    let text = document(&host, &path, "no such device 7f3a", orange_audio::PLAY);
    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the failure", || read(&host, flow, 100).is_some());
    assert_eq!(read(&host, flow, 100), Some(Value::I64(-1)));
}

#[test]
fn the_silent_input_reports_silence_and_never_fires() {
    let mut host = host();
    let path = tone("idle.wav", 20);
    let text = document(&host, &path, orange_audio::NULL_DEVICE, orange_audio::PLAY);
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
    // The stream nodes are named by what they do, and Seek by where it goes.
    let labels: Vec<String> = host
        .nodes()
        .filter(|info| {
            info.definition
                .definition
                .id
                .as_str()
                .starts_with("org.orange.audio/")
        })
        .map(|info| info.label().into_owned())
        .collect();
    for wanted in [
        "Audio in",
        "Audio out",
        "Level",
        "On band level",
        "On transient",
        "Band level",
        "Record to file",
        "Jump to time",
    ] {
        assert!(labels.iter().any(|label| label == wanted), "{labels:?}");
    }
    assert!(!labels.iter().any(|label| label.contains("Seek")));

    let palette = host.search_nodes("", "es");
    assert!(palette.iter().any(|item| {
        item.definition == orange_audio::AUDIO_SOURCE && item.label == "Fuente de audio"
    }));
    for former in [
        orange_audio::ON_BAND_LEVEL,
        orange_audio::ON_TRANSIENT,
        orange_audio::BAND_LEVEL,
        orange_audio::SEEK,
    ] {
        assert!(!palette.iter().any(|item| item.definition == former));
    }
}

#[test]
fn the_band_trigger_template_places_and_undoes_the_pair_as_one_edit() {
    let mut host = host();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let source = host
        .add_node(&mut draft, orange_audio::AUDIO_SOURCE, [-300.0, 30.0])
        .unwrap();
    let when = host.add_band_trigger(&mut draft, [20.0, 30.0]).unwrap();
    let view = host.view(&draft.text().unwrap()).unwrap();
    assert_eq!(view.nodes.len(), 3);
    let band = view
        .nodes
        .iter()
        .find(|node| node.definition == orange_audio::LIVE_BAND_LEVEL)
        .unwrap();
    let watch = view.nodes.iter().find(|node| node.id == when).unwrap();
    assert_eq!(band.position, Some([20.0, 30.0]));
    assert_eq!(
        watch.settings,
        json!({"mode": "above", "threshold": -20, "release": -26, "hold": 50})
    );
    assert_ne!(band.position, watch.position);
    assert_eq!(
        view.wires,
        [orange_host::WireView {
            from: (band.id, "level".into()),
            to: (when, "watch".into()),
            exec: false,
        }]
    );
    assert!(draft.undo());
    let undone = host.view(&draft.text().unwrap()).unwrap();
    assert_eq!(
        undone.nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
        [source]
    );
    assert!(undone.wires.is_empty());
    assert!(draft.redo());

    assert!(
        host.connect(&mut draft, (source, "stream"), (band.id, "stream"))
            .unwrap()
    );
    let mut input = host.declarations(&draft).connections.remove(0);
    input.settings = json!({"device": orange_audio::NULL_DEVICE});
    let _ = host.declare_connection(&mut draft, &input).unwrap();
    let text = draft.text().unwrap();
    let diagnostics = host.check(&text).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let flow = host.open(&text).unwrap();
    host.close(flow).unwrap();
}

/// One connected file supplies two live meters and the stream-wired entries. Manual
/// playback waits for the live baseline, making the test independent of decode time.
fn live_file_document(host: &Host, path: &Path) -> String {
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000007".parse().unwrap());
    let connection = json!({"id": INPUT, "name": "track",
        "adapter": orange_audio::INPUT_ADAPTER, "scope": "graph",
        "settings": {"file": path.to_str().unwrap()}});
    let _ = document
        .apply([orange_model::DocCommand::UpsertConnection(
            serde_json::from_value(connection).unwrap(),
        )])
        .unwrap();
    let literal = |value: f64| json!({"type": F64, "bits": format!("{:016x}", value.to_bits())});
    for (n, definition, config, inputs) in [
        (
            1,
            orange_audio::AUDIO_SOURCE,
            json!({"connection": INPUT}),
            json!({}),
        ),
        (
            2,
            orange_audio::LIVE_BAND_LEVEL,
            json!({}),
            json!({"low_hz": literal(45.0), "high_hz": literal(120.0)}),
        ),
        (3, orange_audio::LIVE_LEVEL, json!({}), json!({})),
        (4, orange_audio::TRANSIENT, json!({}), json!({})),
        (5, orange_audio::SOURCE_FINISHED, json!({}), json!({})),
        (6, orange_audio::SOURCE_CONTROL, json!({}), json!({})),
        (7, orange_audio::AUDIO_FORMAT, json!({}), json!({})),
        (13, orange_audio::AUDIO_SPECTRUM, json!({}), json!({})),
        (
            14,
            orange_audio::BAND_TRIGGER,
            json!({"threshold_db": -20.0, "release_db": -28.0, "hold_ms": 20.0, "window_ms": 20.0}),
            json!({"low_hz": literal(45.0), "high_hz": literal(120.0)}),
        ),
    ] {
        let mut new = NewNode::new(
            node(n),
            DefinitionRef {
                id: definition.parse().unwrap(),
                version: 1,
            },
        );
        new.config = serde_json::from_value(config).unwrap();
        new.inputs = serde_json::from_value(inputs).unwrap();
        let edit = new_node_edit(&document, host.registry(), new).unwrap();
        let _ = document.apply(edit.commands).unwrap();
    }
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    for entry in saved["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|node| {
            matches!(
                node["definition_id"].as_str(),
                Some(
                    orange_audio::TRANSIENT
                        | orange_audio::SOURCE_FINISHED
                        | orange_audio::BAND_TRIGGER
                )
            )
        })
    {
        entry["activation"] = activation();
    }
    let mut when = core(9, "on-watch");
    when["config_version"] = json!(2);
    when["config"] = json!({"mode": "above", "threshold": -20.0, "release": -28.0, "hold": 40});
    let count = |n, target| {
        let mut node = core(n, "update-variable");
        node["config"] = json!({"variable": id(target), "operation": "add"});
        node["inputs"] = json!({"value": {"type": "core/u64@1", "value": "1"}});
        node
    };
    saved["nodes"].as_array_mut().unwrap().extend([
        core(8, "on-manual"),
        when,
        count(10, 100),
        count(11, 101),
        count(12, 102),
        count(15, 103),
    ]);
    saved["edges"] = json!([
        edge(1, 1, "stream", 2, "stream"),
        edge(2, 1, "stream", 3, "stream"),
        edge(3, 1, "stream", 4, "stream"),
        edge(4, 1, "stream", 5, "stream"),
        edge(5, 1, "stream", 6, "stream"),
        edge(6, 1, "stream", 7, "stream"),
        edge(7, 8, "then", 6, "in"),
        edge(8, 2, "level", 9, "watch"),
        edge(9, 9, "then", 10, "in"),
        edge(10, 4, "then", 11, "in"),
        edge(11, 5, "then", 12, "in"),
        edge(12, 1, "stream", 13, "stream"),
        edge(13, 1, "stream", 14, "stream"),
        edge(14, 14, "then", 15, "in"),
    ]);
    saved["variables"] = json!(
        [
            ("crossings", 100),
            ("hits", 101),
            ("finished", 102),
            ("band triggers", 103)
        ]
        .map(
            |(name, n)| json!({"id": id(n), "name": name, "type": "core/u64@1", "scope": "run",
            "init": {"type": "core/u64@1", "value": "0"}})
        )
    );
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

#[test]
fn a_noise_gate_filters_the_pcm_stream_before_a_downstream_meter() {
    let mut host = host();
    let path = wav_signal("gate-quiet-loud-quiet.wav", 2_200, |frame| {
        let amplitude = if (700..1_200).contains(&(frame / 8)) {
            8_000
        } else {
            64
        };
        if frame % 16 < 8 {
            amplitude
        } else {
            -amplitude
        }
    });
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let add =
        |draft: &mut orange_host::Draft, kind| host.add_node(draft, kind, [0.0, 0.0]).unwrap();
    let source = add(&mut draft, orange_audio::AUDIO_SOURCE);
    let gate = add(&mut draft, orange_audio::NOISE_GATE);
    let before = add(&mut draft, orange_audio::LIVE_LEVEL);
    let after = add(&mut draft, orange_audio::LIVE_LEVEL);
    for (from, to) in [
        ((source, "stream"), (gate, "stream")),
        ((source, "stream"), (before, "stream")),
        ((gate, "gated_stream"), (after, "stream")),
    ] {
        host.connect(&mut draft, from, to).unwrap();
    }
    let mut input = host.declarations(&draft).connections.remove(0);
    input.settings = json!({"file": path, "autoplay": true});
    host.declare_connection(&mut draft, &input).unwrap();
    let flow = host.open(&to_json(draft.document()).unwrap()).unwrap();

    let levels = || {
        host.inspect(flow, move |run| {
            let port = "level".parse().unwrap();
            let (Some(Value::F64(before)), Some(Value::F64(after))) = (
                run.live_port_value(before, &port),
                run.live_port_value(after, &port),
            ) else {
                return None;
            };
            Some((*before, *after))
        })
        .unwrap()
    };
    let closed = || {
        levels().is_some_and(|(before, after)| {
            (-60.0..-50.0).contains(&before) && after == orange_audio::SILENCE_DB
        })
    };
    until("gate mutes quiet PCM", closed);
    until("gate passes loud PCM", || {
        levels().is_some_and(|(before, after)| before > -15.0 && (before - after).abs() < 1.0)
    });
    until("gate closes after loud PCM", closed);
    host.close(flow).unwrap();
}

#[test]
fn one_spectrum_routes_two_bands_and_noise_level_observes_the_full_stream() {
    use orange_host::{VarScope, Variable};
    let mut host = host();
    let path = wav_signal("multi-band.wav", 2_100, |frame| {
        let hz = match frame / 8 {
            400..425 | 650..1_000 => 80.0,
            1_300..1_650 => 1_800.0,
            _ => return 0,
        };
        let phase = std::f64::consts::TAU * hz * f64::from(frame) / 8_000.0;
        #[expect(clippy::cast_possible_truncation, reason = "bounded PCM sample")]
        let sample = (phase.sin() * 16_000.0) as i16;
        sample
    });
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let add =
        |draft: &mut orange_host::Draft, kind| host.add_node(draft, kind, [0.0, 0.0]).unwrap();
    let spectrum = node(201);
    let mut document = draft.document().clone();
    let original = NewNode::new(
        spectrum,
        DefinitionRef {
            id: orange_audio::MULTIBAND_SPECTRUM.parse().unwrap(),
            version: 1,
        },
    );
    let binding = new_node_edit(&document, host.registry(), original).unwrap();
    let _ = document.apply(binding.commands).unwrap();
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    saved["nodes"][0]["activation"] = activation();
    draft = host.draft(&saved.to_string()).unwrap();
    let source = add(&mut draft, orange_audio::AUDIO_SOURCE);
    let noise = add(&mut draft, orange_audio::NOISE_LEVEL);
    let playback = add(&mut draft, orange_audio::SOURCE_CONTROL);
    let manual = add(&mut draft, "core/on-manual");
    let finished = add(&mut draft, orange_audio::SOURCE_FINISHED);
    for target in [spectrum, noise, playback, finished] {
        host.connect(&mut draft, (source, "stream"), (target, "stream"))
            .unwrap();
    }
    host.connect(&mut draft, (manual, "then"), (playback, "in"))
        .unwrap();
    let mut input = host.declarations(&draft).connections.remove(0);
    input.settings = json!({"file": path});
    host.declare_connection(&mut draft, &input).unwrap();
    let count = draft.document().root().nodes.len();
    host.add_audio_band(&mut draft, spectrum, [60, 115])
        .unwrap();
    host.add_audio_band(&mut draft, spectrum, [1_400, 2_300])
        .unwrap();
    assert_eq!(draft.document().root().nodes.len(), count);
    host.change_audio_band(
        &mut draft,
        spectrum,
        0,
        Some(json!({
            "name": "Bass", "low_hz": 60, "high_hz": 115, "threshold_db": -28, "mode": "both",
        })),
    )
    .unwrap();
    host.change_audio_band(
        &mut draft,
        spectrum,
        1,
        Some(json!({
            "name": "Treble", "low_hz": 1400, "high_hz": 2300, "threshold_db": -28,
        })),
    )
    .unwrap();
    let mut markers = Vec::new();
    let mut bass_action = None;
    for (provider, output, name) in [
        (spectrum, "band_above#0", "Bass"),
        (spectrum, "band_above#1", "Treble"),
        (spectrum, "band_below#0", "Bass quiet"),
        (noise, "then", "Noise"),
        (finished, "then", "Finished"),
    ] {
        let variable = host
            .declare_variable(
                &mut draft,
                &Variable {
                    id: None,
                    name: name.into(),
                    ty: "core/u64@1".into(),
                    scope: VarScope::Run,
                    persistent: false,
                    initial: Some("0".into()),
                },
            )
            .unwrap();
        let action = add(&mut draft, "core/update-variable");
        host.set_config(
            &mut draft,
            action,
            &json!({"variable": variable, "operation": "add"}),
        )
        .unwrap();
        host.set_input(&mut draft, (action, "value"), Some("1"))
            .unwrap();
        host.connect(&mut draft, (provider, output), (action, "in"))
            .unwrap();
        if output == "band_above#0" {
            bass_action = Some(action);
        }
        markers.push(variable);
    }
    let measured = host
        .declare_variable(
            &mut draft,
            &Variable {
                id: None,
                name: "Bass level".into(),
                ty: F64.into(),
                scope: VarScope::Run,
                persistent: false,
                initial: None,
            },
        )
        .unwrap();
    let read_band = add(&mut draft, "core/set-variable");
    host.set_config(&mut draft, read_band, &json!({"variable": measured}))
        .unwrap();
    host.connect(&mut draft, (spectrum, "band_level#0"), (read_band, "value"))
        .unwrap();
    host.connect(
        &mut draft,
        (bass_action.unwrap(), "then"),
        (read_band, "in"),
    )
    .unwrap();
    let wires = draft.document().root().edges.len();
    assert!(host.move_audio_band(&mut draft, spectrum, 0, 1).unwrap());
    assert_eq!(draft.document().root().edges.len(), wires);
    let view = host.draft_view(&draft).unwrap();
    let shown = view.nodes.iter().find(|node| node.id == spectrum).unwrap();
    assert_eq!(
        shown
            .ports
            .iter()
            .filter(|port| port.id.starts_with("band_level#"))
            .map(|port| port.id.as_str())
            .collect::<Vec<_>>(),
        ["band_level#1", "band_level#0"]
    );
    assert!(
        host.change_audio_band(&mut draft, spectrum, 0, None)
            .unwrap()
    );
    assert_eq!(draft.document().root().edges.len(), wires - 3);
    assert!(draft.undo());
    assert_eq!(draft.document().root().edges.len(), wires);

    let legacy = draft.document().root();
    let legacy_node_count = legacy.nodes.len();
    let legacy_edges = legacy.edges.clone();
    assert_eq!(legacy.nodes[&spectrum].definition.version, 1);
    host.add_filtered_audio_band(&mut draft, spectrum, [500, 700])
        .unwrap();
    let updated = draft.document().root();
    assert_eq!(updated.nodes.len(), legacy_node_count);
    assert_eq!(updated.nodes[&spectrum].definition.version, 3);
    assert_eq!(updated.edges, legacy_edges);
    assert_eq!(
        updated.nodes[&spectrum].variadic[&orange_model::port_id!("band_stream")]
            .order
            .len(),
        3,
    );
    assert!(draft.undo());
    assert_eq!(
        draft.document().root().nodes[&spectrum].definition.version,
        1
    );
    assert_eq!(draft.document().root().edges, legacy_edges);
    assert!(draft.redo());

    let graph = draft.text().unwrap();
    let diagnostics = host.check(&graph).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let flow = host.open(&graph).unwrap();
    let read_live = |node, port: &str| {
        let port = port.parse().unwrap();
        host.inspect(flow, move |run| run.live_port_value(node, &port).cloned())
            .unwrap()
    };
    let read_count = |index: usize| {
        let variable = markers[index].parse().unwrap();
        host.inspect(flow, move |run| run.variable(variable).cloned())
            .unwrap()
    };
    until("two bands and the noise meter to report silence", || {
        read_live(spectrum, "band_level#0") == Some(Value::F64(orange_audio::SILENCE_DB))
            && read_live(spectrum, "band_level#1") == Some(Value::F64(orange_audio::SILENCE_DB))
            && read_live(noise, "level") == Some(Value::F64(orange_audio::SILENCE_DB))
            && matches!(read_live(spectrum, "spectrum"), Some(Value::List(bins)) if bins.len() == 96)
    });
    assert_eq!(read_count(2), Some(Value::U64(0)));
    host.trigger(flow, manual, None).unwrap();
    until("the bass route without the treble route", || {
        read_count(0) == Some(Value::U64(1))
    });
    assert_eq!(read_count(1), Some(Value::U64(0)));
    assert!(matches!(read_live(spectrum, "band_level#0"), Some(Value::F64(db)) if db > -28.0));
    assert_eq!(read_live(noise, "condition_met"), Some(Value::Bool(true)));
    until("the audio file to finish", || {
        read_count(4) == Some(Value::U64(1))
    });
    let counts: Vec<_> = (0..4).map(read_count).collect();
    assert_eq!(counts, [1, 1, 1, 2].map(|count| Some(Value::U64(count))));
    let routes = host
        .inspect(flow, move |run| {
            run.trace()
                .iter()
                .filter(|event| event.node == spectrum)
                .filter_map(|event| event.route.as_ref().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(routes, ["band_above#0", "band_below#0", "band_above#1"]);
    let measured = measured.parse().unwrap();
    assert!(matches!(
        host.inspect(flow, move |run| run.variable(measured).cloned())
            .unwrap(),
        Some(Value::F64(_))
    ));
    host.close(flow).unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn audio_out_is_a_passive_sink_with_a_wireable_enable_control() {
    use orange_host::{VarScope, Variable};
    let mut host = host();
    let path = wav_signal("disabled-output.wav", 300, |_| 0);
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let source = host
        .add_node(&mut draft, orange_audio::AUDIO_SOURCE, [0.0, 0.0])
        .unwrap();
    let output = host
        .add_node(&mut draft, orange_audio::AUDIO_OUT, [220.0, 0.0])
        .unwrap();
    host.connect(&mut draft, (source, "stream"), (output, "stream"))
        .unwrap();
    let view = host.draft_view(&draft).unwrap();
    let card = view.nodes.iter().find(|node| node.id == output).unwrap();
    assert!(card.ports.iter().all(|port| !port.exec));
    let enabled = card.ports.iter().find(|port| port.id == "enabled").unwrap();
    assert!(enabled.input);
    assert_eq!(enabled.default.as_deref(), Some("true"));
    let enabled = host
        .declare_variable(
            &mut draft,
            &Variable {
                id: None,
                name: "Listen".into(),
                ty: "core/bool@1".into(),
                scope: VarScope::Run,
                persistent: false,
                initial: Some("false".into()),
            },
        )
        .unwrap();
    let get = host
        .add_node(&mut draft, "core/get-variable", [0.0, 0.0])
        .unwrap();
    host.set_config(&mut draft, get, &json!({"variable": enabled}))
        .unwrap();
    host.connect(&mut draft, (get, "value"), (output, "enabled"))
        .unwrap();
    let controls: Vec<_> = ["true", "false"]
        .into_iter()
        .map(|value| {
            let manual = host
                .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
                .unwrap();
            let set = host
                .add_node(&mut draft, "core/set-variable", [0.0, 0.0])
                .unwrap();
            host.set_config(&mut draft, set, &json!({"variable": enabled}))
                .unwrap();
            host.set_input(&mut draft, (set, "value"), Some(value))
                .unwrap();
            host.connect(&mut draft, (manual, "then"), (set, "in"))
                .unwrap();
            manual
        })
        .collect();
    for mut connection in host.declarations(&draft).connections {
        if connection.adapter == orange_audio::INPUT_ADAPTER {
            connection.settings = json!({"file": path});
        } else if connection.adapter == orange_audio::OUTPUT_ADAPTER {
            connection.settings = json!({"device": orange_audio::NULL_DEVICE});
        }
        host.declare_connection(&mut draft, &connection).unwrap();
    }
    let graph = draft.text().unwrap();
    assert!(host.check(&graph).unwrap().is_empty());
    let flow = host.open(&graph).unwrap();
    let is_playing = |state| {
        host.inspect(flow, move |run| {
            run.live_port_value(output, &"playing".parse().unwrap())
                .cloned()
        })
        .unwrap()
            == Some(Value::Bool(state))
    };
    until("disabled Audio Out to publish its status", || {
        is_playing(false)
    });
    host.trigger(flow, controls[0], None).unwrap();
    until("enabled Audio Out to start its voice", || is_playing(true));
    host.trigger(flow, controls[1], None).unwrap();
    until("disabled Audio Out to stop", || is_playing(false));
    host.close(flow).unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn spectrum_bands_send_separately_filtered_pcm_to_live_meters() {
    let mut host = host();
    let path = wav_signal("filtered-stream-bands.wav", 2_700, |frame| {
        let frequency = match frame / 8 {
            400..1_100 => 500.0,
            1_600..2_300 => 1_050.0,
            _ => return 0,
        };
        let phase = std::f64::consts::TAU * frequency * f64::from(frame) / 8_000.0;
        #[expect(clippy::cast_possible_truncation, reason = "bounded PCM sample")]
        let sample = (phase.sin() * 16_000.0) as i16;
        sample
    });
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let add =
        |draft: &mut orange_host::Draft, kind| host.add_node(draft, kind, [0.0, 0.0]).unwrap();
    let source = add(&mut draft, orange_audio::AUDIO_SOURCE);
    let spectrum = add(&mut draft, orange_audio::MULTIBAND_SPECTRUM);
    let first = add(&mut draft, orange_audio::LIVE_LEVEL);
    let second = add(&mut draft, orange_audio::LIVE_LEVEL);
    let playback = add(&mut draft, orange_audio::SOURCE_CONTROL);
    let manual = add(&mut draft, "core/on-manual");
    host.add_audio_band(&mut draft, spectrum, [400, 600])
        .unwrap();
    host.add_audio_band(&mut draft, spectrum, [1000, 1100])
        .unwrap();
    let view = host.draft_view(&draft).unwrap();
    let spectrum_view = view.nodes.iter().find(|node| node.id == spectrum).unwrap();
    assert_eq!(
        spectrum_view
            .ports
            .iter()
            .filter(|port| !port.input)
            .map(|port| port.id.as_str())
            .collect::<Vec<_>>(),
        ["band_stream#0", "band_stream#1"]
    );
    for (from, output, to, input) in [
        (source, "stream", spectrum, "stream"),
        (source, "stream", playback, "stream"),
        (manual, "then", playback, "in"),
        (spectrum, "band_stream#0", first, "stream"),
        (spectrum, "band_stream#1", second, "stream"),
    ] {
        host.connect(&mut draft, (from, output), (to, input))
            .unwrap();
    }
    host.move_audio_band(&mut draft, spectrum, 1, 0).unwrap();
    host.change_audio_band(
        &mut draft,
        spectrum,
        0,
        Some(json!({"low_hz": 400, "high_hz": 600, "name": "Low"})),
    )
    .unwrap();
    let wires = draft.document().root().edges.len();
    host.change_audio_band(&mut draft, spectrum, 0, None)
        .unwrap();
    assert_eq!(draft.document().root().edges.len(), wires - 1);
    assert!(draft.undo());
    let mut input = host.declarations(&draft).connections.remove(0);
    input.settings = json!({"file": path});
    host.declare_connection(&mut draft, &input).unwrap();
    let graph = draft.text().unwrap();
    assert!(host.check(&graph).unwrap().is_empty());
    let flow = host.open(&graph).unwrap();
    let levels = || {
        host.inspect(flow, move |run| {
            (
                run.live_value(first).cloned(),
                run.live_value(second).cloned(),
            )
        })
        .unwrap()
    };
    until("both filtered streams to reach their live meters", || {
        levels()
            == (
                Some(Value::F64(orange_audio::SILENCE_DB)),
                Some(Value::F64(orange_audio::SILENCE_DB)),
            )
    });
    host.trigger(flow, manual, None).unwrap();
    let separates = |wanted: Option<Value>, rejected: Option<Value>, limit: f64| matches!((wanted, rejected), (Some(Value::F64(a)), Some(Value::F64(b))) if a > limit && a - b > 16.0);
    until("500 Hz on the first stream", || {
        let (a, b) = levels();
        separates(a, b, -21.0)
    });
    until("1050 Hz on the second stream", || {
        let (a, b) = levels();
        separates(b, a, -22.0)
    });
    host.close(flow).unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_wired_file_source_drives_when_transient_and_finished_from_one_stream() {
    let mut host = host();
    // A strong 80 Hz signal for 140 ms, repeated twice a second (120 BPM).
    let path = wav_signal("source-kicks.wav", 1_600, |frame| {
        if (160..300).contains(&(frame / 8 % 500)) {
            if frame % 100 < 50 { 16_000 } else { -16_000 }
        } else {
            0
        }
    });
    let graph = live_file_document(&host, &path);
    let diagnostics = host.check(&graph).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let flow = host.open(&graph).unwrap();
    let live = |host: &Host, flow, n| {
        host.inspect(flow, move |run| run.live_value(node(n)).cloned())
            .unwrap()
    };
    until("audio readers to establish their baselines", || {
        live(&host, flow, 2) == Some(Value::F64(orange_audio::SILENCE_DB))
            && live(&host, flow, 3) == Some(Value::F64(orange_audio::SILENCE_DB))
            && live(&host, flow, 7).is_some()
            && matches!(live(&host, flow, 13), Some(Value::List(bins)) if bins.len() == 96)
    });
    let first_source = live(&host, flow, 1).unwrap();
    let Some(Value::Record(format)) = live(&host, flow, 7) else {
        panic!("no stream format")
    };
    assert_eq!(
        &*format.fields,
        &[Some(Value::U64(48_000)), Some(Value::U64(2))]
    );

    host.trigger(flow, node(8), None).unwrap();
    until(
        "the source to finish with exactly three analyzed hits",
        || read(&host, flow, 102) == Some(Value::U64(1)),
    );
    assert_eq!(read(&host, flow, 100), Some(Value::U64(3)));
    assert_eq!(read(&host, flow, 101), Some(Value::U64(3)));
    assert_eq!(read(&host, flow, 103), Some(Value::U64(3)));
    until(
        "both readers to return to silence at the end of the file",
        || {
            live(&host, flow, 2) == Some(Value::F64(orange_audio::SILENCE_DB))
                && live(&host, flow, 3) == Some(Value::F64(orange_audio::SILENCE_DB))
        },
    );
    host.close(flow).unwrap();
    let reopened = host.open(&graph).unwrap();
    until("the new stream value", || {
        live(&host, reopened, 1).is_some()
    });
    assert_ne!(live(&host, reopened, 1), Some(first_source));
    host.close(reopened).unwrap();
    std::fs::remove_file(path).unwrap();
}

/// Plays a short tone on the real default device. Run by hand:
/// `cargo test -p orange-audio --test audio -- --ignored`.
#[test]
#[ignore = "needs an audio output device and makes a sound"]
fn a_tone_plays_on_the_default_device() {
    let mut host = host();
    let path = tone("beep.wav", 300);
    let mut saved: Json = serde_json::from_str(&document(
        &host,
        &path,
        orange_audio::NULL_DEVICE,
        orange_audio::PLAY,
    ))
    .unwrap();
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

/// What a person does in the editor: drops nodes, picks the device and types a folder.
#[test]
fn a_dropped_play_node_gets_a_connection_and_plays_a_random_file_of_a_folder() {
    let mut host = host();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let play = host
        .add_node(&mut draft, orange_audio::PLAY_FROM, [200.0, 0.0])
        .unwrap();
    // Dropping it declared an output on the system default device, in the same edit.
    let declared = host.declarations(&draft).connections;
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].adapter, orange_audio::OUTPUT_ADAPTER);
    assert_eq!(declared[0].settings, json!({}));
    assert!(draft.undo());
    assert!(host.declarations(&draft).connections.is_empty());
    assert!(draft.redo());
    // Another output node uses the same connection; an input node gets its own kind.
    let other = host
        .add_node(&mut draft, orange_audio::STOP, [0.0, 200.0])
        .unwrap();
    let level = host
        .add_node(&mut draft, orange_audio::INPUT_LEVEL, [0.0, 400.0])
        .unwrap();
    let kinds: Vec<String> = host
        .declarations(&draft)
        .connections
        .into_iter()
        .map(|connection| connection.adapter)
        .collect();
    assert_eq!(
        kinds.len(),
        2,
        "one output and one input connection: {kinds:?}"
    );
    assert!(host.remove_nodes(&mut draft, &[other, level]).unwrap());

    // The device list offers the silent device beside whatever this machine has.
    let choices = host.adapter_choices(orange_audio::OUTPUT_ADAPTER);
    assert_eq!(choices[0].setting, "device");
    assert!(
        choices[0]
            .values
            .iter()
            .any(|device| device == orange_audio::NULL_DEVICE)
    );
    let mut output = host
        .declarations(&draft)
        .connections
        .into_iter()
        .find(|connection| connection.adapter == orange_audio::OUTPUT_ADAPTER)
        .unwrap();
    output.settings = json!({"device": orange_audio::NULL_DEVICE});
    let _ = host.declare_connection(&mut draft, &output).unwrap();

    let folder = std::env::temp_dir().join(format!("orange-audio-random-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    for name in ["a.wav", "b.wav"] {
        std::fs::copy(tone(name, 20), folder.join(name)).unwrap();
    }
    assert!(
        host.set_input(&mut draft, (play, "folder"), folder.to_str())
            .unwrap()
    );
    let random = host
        .add_node(&mut draft, "org.orange.math/random-u64", [0.0, 100.0])
        .unwrap();
    assert!(
        host.connect(&mut draft, (random, "value"), (play, "pick"))
            .unwrap()
    );
    let manual = host
        .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
        .unwrap();
    assert!(
        host.connect(&mut draft, (manual, "then"), (play, "in"))
            .unwrap()
    );
    let played = host
        .declare_variable(
            &mut draft,
            &orange_host::Variable {
                id: None,
                name: "played".into(),
                ty: STRING.into(),
                scope: orange_host::VarScope::Run,
                persistent: false,
                initial: None,
            },
        )
        .unwrap();
    let store = host
        .add_node(&mut draft, "core/set-variable", [400.0, 0.0])
        .unwrap();
    assert!(
        host.set_config(&mut draft, store, &json!({"variable": played}))
            .unwrap()
    );
    assert!(
        host.connect(&mut draft, (play, "then"), (store, "in"))
            .unwrap()
    );
    assert!(
        host.connect(&mut draft, (play, "file"), (store, "value"))
            .unwrap()
    );
    let problems = host.problems(&mut draft).unwrap();
    assert!(problems.is_empty(), "{problems:?}");

    let flow = host.open(&draft.text().unwrap()).unwrap();
    let played: orange_host::VariableId = played.parse().unwrap();
    let file = || {
        host.inspect(flow, move |run| run.variable(played).cloned())
            .unwrap()
    };
    host.trigger(flow, manual, None).unwrap();
    until("a file to play", || file().is_some());
    let Some(Value::Str(name)) = file() else {
        panic!("not a file name: {:?}", file());
    };
    assert!(["a.wav", "b.wav"].contains(&name.as_str()), "{name}");
    std::fs::remove_dir_all(&folder).unwrap();
}

/// Audio in on the silent input, wired into Level, Record to file and Audio out on the
/// silent output. Manual entry 30 measures and records `seconds` to `path`, storing
/// the level in variable 104 and the seconds recorded in 103; entry 40 plays the
/// input on the output until entry 7 stops it, then stores 1 in 105.
fn stream_document(host: &Host, path: &Path, seconds: f64) -> String {
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000002".parse().unwrap());
    for (id, adapter) in [
        (OUTPUT, orange_audio::OUTPUT_ADAPTER),
        (INPUT, orange_audio::INPUT_ADAPTER),
    ] {
        let declared = json!({"id": id, "name": id, "adapter": adapter, "scope": "graph",
            "settings": {"device": orange_audio::NULL_DEVICE}});
        let _ = document
            .apply([orange_model::DocCommand::UpsertConnection(
                serde_json::from_value(declared).unwrap(),
            )])
            .unwrap();
    }
    let place = |document: &mut GraphDocument,
                 n: u32,
                 name: &str,
                 connection: Option<&str>,
                 inputs: Json| {
        let mut new = NewNode::new(
            node(n),
            DefinitionRef {
                id: name.parse().unwrap(),
                version: 1,
            },
        );
        if let Some(connection) = connection {
            new.config = serde_json::from_value(json!({"connection": connection})).unwrap();
        }
        new.inputs = serde_json::from_value(inputs).unwrap();
        let edit = new_node_edit(document, host.registry(), new).unwrap();
        let _ = document.apply(edit.commands).unwrap();
    };
    let number = |value: f64| json!({"type": F64, "bits": format!("{:016x}", value.to_bits())});
    place(
        &mut document,
        31,
        orange_audio::AUDIO_IN,
        Some(INPUT),
        json!({}),
    );
    place(&mut document, 34, orange_audio::LEVEL, None, json!({}));
    place(
        &mut document,
        32,
        orange_audio::RECORD,
        None,
        json!({"path": {"type": STRING, "value": path.to_str().unwrap()}, "seconds": number(seconds)}),
    );
    place(
        &mut document,
        41,
        orange_audio::AUDIO_IN,
        Some(INPUT),
        json!({}),
    );
    place(
        &mut document,
        42,
        orange_audio::AUDIO_OUT,
        Some(OUTPUT),
        json!({}),
    );
    place(
        &mut document,
        8,
        orange_audio::STOP,
        Some(OUTPUT),
        json!({}),
    );
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    let store = |n: u32, target: u32| {
        let mut node = core(n, "set-variable");
        node["config"] = json!({"variable": id(target)});
        node
    };
    let nodes = saved["nodes"].as_array_mut().unwrap();
    nodes.extend([
        core(30, "on-manual"),
        store(35, 104),
        store(33, 103),
        core(40, "on-manual"),
        set(43, 105, 1),
        core(7, "on-manual"),
    ]);
    saved["edges"] = json!([
        edge(1, 30, "then", 31, "in"),
        edge(2, 31, "then", 34, "in"),
        edge(3, 31, "stream", 34, "stream"),
        edge(4, 34, "then", 35, "in"),
        edge(5, 34, "level", 35, "value"),
        edge(6, 35, "then", 32, "in"),
        edge(7, 31, "stream", 32, "stream"),
        edge(8, 32, "then", 33, "in"),
        edge(9, 32, "recorded", 33, "value"),
        edge(10, 40, "then", 41, "in"),
        edge(11, 41, "then", 42, "in"),
        edge(12, 41, "stream", 42, "stream"),
        edge(13, 42, "then", 43, "in"),
        edge(14, 7, "then", 8, "in"),
    ]);
    saved["variables"] = json!([
        {"id": id(103), "name": "recorded", "type": F64, "scope": "run"},
        {"id": id(104), "name": "level", "type": F64, "scope": "run"},
        {"id": id(105), "name": "played", "type": I64, "scope": "run"}
    ]);
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

#[test]
fn an_input_stream_is_measured_recorded_and_played_through_to_an_output() {
    let mut host = host();
    let dir = std::env::temp_dir().join(format!("orange-audio-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("recorded.wav");
    let text = stream_document(&host, &path, 0.2);
    assert_eq!(host.check(&text).unwrap(), []);
    let flow = host.open(&text).unwrap();

    host.trigger(flow, node(30), None).unwrap();
    until("the recording", || read(&host, flow, 103).is_some());
    assert_eq!(
        read(&host, flow, 104),
        Some(Value::F64(orange_audio::SILENCE_DB))
    );
    let Some(Value::F64(recorded)) = read(&host, flow, 103) else {
        panic!("seconds recorded");
    };
    assert!((recorded - 0.2).abs() < 1e-9);
    let reader = hound::WavReader::open(&path).unwrap();
    assert_eq!(reader.spec().sample_rate, 48_000);
    assert_eq!(reader.len(), 48_000 / 5 * 2);

    // The input plays on the output until the output's sounds are stopped.
    host.trigger(flow, node(40), None).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(read(&host, flow, 105), None);
    let mut last = Instant::now() - Duration::from_secs(1);
    until("the pass-through to stop", || {
        if last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            host.trigger(flow, node(7), None).unwrap();
        }
        read(&host, flow, 105).is_some()
    });
    std::fs::remove_file(&path).unwrap();
}

/// A document saved with the first On level opens with On band level in its place,
/// keeping its connection, place and activation (12 §6.4).
#[test]
fn a_saved_on_level_opens_as_on_band_level() {
    let host = host();
    let mut saved: Json = serde_json::from_str(
        &to_json(&GraphDocument::new(
            "20000000-0000-4000-8000-000000000002".parse().unwrap(),
        ))
        .unwrap(),
    )
    .unwrap();
    saved["connections"] = json!([{"id": INPUT, "name": "microphone",
        "adapter": orange_audio::INPUT_ADAPTER, "scope": "graph",
        "settings": {"device": orange_audio::NULL_DEVICE}}]);
    saved["nodes"] = json!([{"id": id(12), "definition_id": orange_audio::ON_LEVEL,
        "definition_version": 1, "config_version": 1, "config": {"connection": INPUT},
        "activation": {"mode": "queue", "capacity": 4, "overflow": "fault",
            "on_fault": "continue", "order": 0}}]);
    let draft = host.draft(&saved.to_string()).unwrap();
    let replaced = &draft.opened().replaced;
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].to.id.as_str(), orange_audio::ON_BAND_LEVEL);
    let node = draft.document().node(node(12)).unwrap();
    assert_eq!(node.config["connection"], json!(INPUT));
    assert!(node.activation.is_some() && node.provider.is_some());
    assert_eq!(host.check(&saved.to_string()).unwrap(), []);
}

/// The package's own words find its nodes, and its Spanish catalogue names them in
/// Spanish (13 §3): no host change is needed for a package's language.
#[test]
fn a_package_brings_its_own_search_words_and_spanish_names() {
    let host = host();
    let first = |search: &str, locale: &str| {
        host.search_nodes(search, locale)
            .into_iter()
            .next()
            .map(|found| (found.definition, found.label))
    };
    assert_eq!(
        first("kick", "en"),
        Some((orange_audio::TRANSIENT.to_owned(), "Transient".to_owned()))
    );
    assert_eq!(
        first("bombo", "es"),
        Some((orange_audio::TRANSIENT.to_owned(), "Transitorio".to_owned()))
    );
    // A Spanish search still finds a node by its English name.
    assert_eq!(
        first("transient", "es").map(|(definition, _)| definition),
        Some(orange_audio::TRANSIENT.to_owned())
    );
    // A language the package has no text for falls back to English.
    assert_eq!(
        first("kick", "fr").map(|(_, label)| label),
        Some("Transient".to_owned())
    );
}

/// Editors label a node's connection by what its adapter calls itself.
#[test]
fn a_nodes_connection_is_named_by_its_adapter() {
    let host = host();
    for listener in [
        orange_audio::ON_TRANSIENT,
        orange_audio::ON_BAND_LEVEL,
        orange_audio::AUDIO_IN,
    ] {
        assert_eq!(
            host.connection_label(listener).as_deref(),
            Some("Audio source")
        );
    }
    assert_eq!(
        host.connection_label(orange_audio::AUDIO_OUT).as_deref(),
        Some("Audio output")
    );
    assert_eq!(host.connection_label(orange_audio::LEVEL), None);
}
