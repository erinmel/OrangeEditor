//! Live values and sources armed with arguments (11 §10.5): a package publishes a value
//! the graph reads like a variable, a Watch reacts to it, and a provider is started
//! again with new arguments when the values on its inputs change.
use orange_testkit::{activation, core, edge, id, node, set, until};

use orange_host::{Host, HostOptions, ServicePackage, Value, VariableId};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{GraphDocument, from_json, to_json};
use orange_sdk::{Arguments, Definition, Emitter, LiveSource, Publisher, ServiceSource, Session};
use serde_json::{Map, Value as Json, json};
use std::sync::{Arc, Mutex};

const PACKAGE: &str = "org.example.live";
const LEVEL: &str = "core/u64@1";
const STRING: &str = "core/string@1";

#[test]
#[ignore = "manual release performance measurement"]
fn composite_live_timing() {
    use orange_bundled::MathPackage;
    use orange_compiler::{CompileOptions, compile_control};
    use orange_model::value::ListRepr;
    use orange_runtime::{Run, RunOptions};

    struct Inert;
    impl LiveSource for Inert {
        fn start(
            &self,
            _: Option<Arc<dyn Session>>,
            _: &Map<String, Json>,
            _: &Arguments,
            _: Publisher,
        ) -> Result<Box<dyn Send>, String> {
            Ok(Box::new(()))
        }
    }

    let definition = Definition::read(PACKAGE, "composite", "Composite")
        .live("sample", "core/list@1<core/option@1<core/u64@1>>")
        .build();
    let package = ServicePackage::new(PACKAGE, "1.0.0")
        .live(&definition, Inert)
        .unwrap();
    let (host, _) = Host::with_services(HostOptions::default(), vec![package]).unwrap();
    let mut doc = GraphDocument::new("20000000-0000-4000-8000-0000000000a1".parse().unwrap());
    let new = NewNode::new(
        node(1),
        DefinitionRef {
            id: format!("{PACKAGE}/composite").parse().unwrap(),
            version: 1,
        },
    );
    let registry = host.registry();
    let edit = new_node_edit(&doc, registry, new).unwrap();
    let _ = doc.apply(edit.commands).unwrap();
    let analysis = orange_model::analyze(&doc, registry);
    let resolver = MathPackage::new(format!("sha256:{}", "0".repeat(64))).unwrap();
    let plan = compile_control(
        &doc,
        registry.clone(),
        &analysis,
        &resolver,
        CompileOptions::default(),
    )
    .unwrap();
    let mut run = Run::start(Arc::new(plan), &doc, registry, 0, RunOptions::default()).unwrap();
    let value = |last| {
        Value::List(Arc::new(ListRepr::Values(
            (0..1024)
                .map(|index| {
                    if index == 1023 {
                        Value::U64(last)
                    } else {
                        Value::None
                    }
                })
                .collect(),
        )))
    };
    let values = [value(1), value(2)];
    orange_testkit::measure_batches("512 composite live publications and pumps", || {
        for index in 0..512 {
            assert!(
                run.publish_live(node(1), values[index % 2].clone())
                    .unwrap()
            );
            run.pump(1).unwrap();
        }
    });
    assert_eq!(run.live_value(node(1)), Some(&values[1]));
}

#[test]
fn a_failed_staged_source_leaves_the_existing_run_and_can_be_retried() {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Source(Arc<AtomicBool>, Arc<Mutex<Option<Emitter>>>);
    impl ServiceSource for Source {
        fn start(
            &self,
            _: Option<Arc<dyn Session>>,
            emitter: Emitter,
        ) -> Result<Box<dyn Send>, String> {
            if self.0.load(Ordering::Relaxed) {
                return Err("source declined".into());
            }
            *self.1.lock().unwrap() = Some(emitter);
            Ok(Box::new(()))
        }
    }

    let refuse = Arc::new(AtomicBool::new(false));
    let emitter = Arc::new(Mutex::new(None));
    let definition = Definition::entry(PACKAGE, "on-ready", "On ready").build();
    let package = ServicePackage::new(PACKAGE, "1.0.0")
        .source(&definition, Source(refuse.clone(), emitter.clone()))
        .unwrap();
    let (mut host, _) = Host::with_services(HostOptions::default(), vec![package]).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    host.add_node(&mut draft, "org.example.live/on-ready", [0.0, 0.0])
        .unwrap();
    let flow = host.open(&draft.text().unwrap()).unwrap();
    let previous = emitter.lock().unwrap().clone().unwrap();
    host.add_node(&mut draft, "core/on-manual", [150.0, 0.0])
        .unwrap();
    let changed = draft.text().unwrap();
    refuse.store(true, Ordering::Relaxed);
    assert!(matches!(
        host.update(flow, &changed),
        Err(orange_host::HostError::Source(..))
    ));
    assert!(previous.emit(None).is_ok());
    refuse.store(false, Ordering::Relaxed);
    assert!(host.update(flow, &changed).unwrap().changed);
    assert_eq!(previous.emit(None), Err(orange_sdk::EmitError::Closed));
    host.close(flow).unwrap();
}

/// What the test's providers were started with, and how to make them publish or fire.
#[derive(Default)]
struct Seen {
    scales: Vec<u64>,
    publisher: Option<Publisher>,
    topics: Vec<String>,
    emitter: Option<Emitter>,
    stopped: usize,
}
type Shared = Arc<Mutex<Seen>>;

/// Counts how often a provider's guard is dropped.
struct Guard(Shared);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.lock().unwrap().stopped += 1;
    }
}

struct Level(Shared);
impl LiveSource for Level {
    fn start(
        &self,
        _session: Option<Arc<dyn Session>>,
        _settings: &Map<String, Json>,
        arguments: &Arguments,
        publisher: Publisher,
    ) -> Result<Box<dyn Send>, String> {
        let Some(Value::U64(scale)) = arguments.get("scale") else {
            return Err("no scale".into());
        };
        let mut seen = self.0.lock().unwrap();
        seen.scales.push(*scale);
        seen.publisher = Some(publisher);
        Ok(Box::new(Guard(Arc::clone(&self.0))))
    }
}

struct Ping(Shared);
impl ServiceSource for Ping {
    fn start_with_arguments(
        &self,
        _session: Option<Arc<dyn Session>>,
        _settings: &Map<String, Json>,
        arguments: &Arguments,
        emitter: Emitter,
    ) -> Result<Box<dyn Send>, String> {
        let Some(Value::Str(topic)) = arguments.get("topic") else {
            return Err("no topic".into());
        };
        let mut seen = self.0.lock().unwrap();
        seen.topics.push(topic.as_str().to_owned());
        seen.emitter = Some(emitter);
        Ok(Box::new(Guard(Arc::clone(&self.0))))
    }
}

fn package(seen: &Shared) -> ServicePackage {
    let level = Definition::read(PACKAGE, "level", "Level")
        .input("scale", LEVEL)
        .live("level", LEVEL)
        .build();
    let ping = Definition::entry(PACKAGE, "on-ping", "On ping")
        .input("topic", STRING)
        .payload(STRING)
        .build();
    ServicePackage::new(PACKAGE, "1.0.0")
        .live(&level, Level(Arc::clone(seen)))
        .and_then(|p| p.source(&ping, Ping(Arc::clone(seen))))
        .unwrap()
}

/// Variables: 100 `seen` (what the Watch saw), 101 `scale`, 102 `pinged`.
/// Nodes: 1 Level (scale from Get 4), 2 On watch of the level → Set 3 of `seen`;
/// 5 On ping (topic "news") → Set 6 of `pinged`; 7 On manual → Set 8 of `scale` to 3.
fn graph(host: &Host) -> String {
    let mut document = GraphDocument::new("20000000-0000-4000-8000-0000000000a1".parse().unwrap());
    for (n, name, inputs) in [
        (1, "level", json!({})),
        (
            5,
            "on-ping",
            json!({"topic": {"type": STRING, "value": "news"}}),
        ),
    ] {
        let mut new = NewNode::new(
            node(n),
            DefinitionRef {
                id: format!("{PACKAGE}/{name}").parse().unwrap(),
                version: 1,
            },
        );
        new.inputs = serde_json::from_value(inputs).unwrap();
        let edit = new_node_edit(&document, host.registry(), new).unwrap();
        let _ = document.apply(edit.commands).unwrap();
    }
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    for entry in saved["nodes"].as_array_mut().unwrap() {
        if entry["definition_id"] == format!("{PACKAGE}/on-ping") {
            entry["activation"] = activation();
        }
    }
    let mut watch = core(2, "on-watch");
    watch["config"] = json!({"mode": "changed"});
    let mut scale = core(4, "get-variable");
    scale["config"] = json!({"variable": id(101)});
    let mut three = set(8, 101);
    let level = |value: u64| json!({"type": LEVEL, "value": value.to_string()});
    three["inputs"] = json!({"value": level(3)});
    saved["nodes"].as_array_mut().unwrap().extend([
        watch,
        set(3, 100),
        scale,
        set(6, 102),
        core(7, "on-manual"),
        three,
    ]);
    saved["edges"] = json!([
        edge(1, 4, "value", 1, "scale"),
        edge(2, 1, "level", 2, "watch"),
        edge(3, 2, "then", 3, "in"),
        edge(4, 2, "value", 3, "value"),
        edge(5, 5, "then", 6, "in"),
        edge(6, 5, "payload", 6, "value"),
        edge(7, 7, "then", 8, "in"),
    ]);
    saved["variables"] = json!([
        {"id": id(100), "name": "seen", "type": LEVEL, "scope": "run", "init": level(0)},
        {"id": id(101), "name": "scale", "type": LEVEL, "scope": "run", "init": level(2)},
        {"id": id(102), "name": "pinged", "type": STRING, "scope": "run"}
    ]);
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

#[test]
fn live_values_reach_watches_and_new_arguments_restart_their_providers() {
    let seen = Shared::default();
    let (mut host, _events) =
        Host::with_services(HostOptions::default(), vec![package(&seen)]).unwrap();
    let text = graph(&host);
    assert_eq!(host.check(&text).unwrap(), []);
    let flow = host.open(&text).unwrap();
    let read = |host: &Host, n: u32| {
        let variable: VariableId = id(n).parse().unwrap();
        host.inspect(flow, move |run| run.variable(variable).cloned())
            .unwrap()
    };

    // Both providers are armed with the values on their inputs.
    until("the providers to start", || {
        let seen = seen.lock().unwrap();
        seen.scales == [2] && seen.topics == ["news"]
    });
    // A published value is what the Watch sees.
    let publish = |value| {
        let publisher = seen.lock().unwrap().publisher.clone().unwrap();
        publisher.publish(Value::U64(value)).unwrap();
    };
    publish(0);
    until("the initial sample", || {
        host.inspect(flow, |run| run.live_value(node(1)).cloned())
            .unwrap()
            == Some(Value::U64(0))
    });
    publish(10);
    until("the watch to see the level", || {
        read(&host, 100) == Some(Value::U64(10))
    });
    // The entry fires with its payload.
    let emitter = seen.lock().unwrap().emitter.clone().unwrap();
    emitter
        .emit(Some(Value::Str(Arc::new("hello".into()))))
        .unwrap();
    until("the ping", || {
        read(&host, 102) == Some(Value::Str(Arc::new("hello".into())))
    });
    // Changing the variable an argument reads restarts that provider alone, once.
    let previous_publisher = seen.lock().unwrap().publisher.clone().unwrap();
    host.trigger(flow, node(7), None).unwrap();
    until("the level to restart", || {
        seen.lock().unwrap().scales == [2, 3]
    });
    assert_eq!(seen.lock().unwrap().topics, ["news"]);
    assert_eq!(seen.lock().unwrap().stopped, 1);
    assert_eq!(
        previous_publisher.publish(Value::U64(999)),
        Err(orange_sdk::EmitError::Closed)
    );
    // The value the old provider published belonged to the old arguments: until the
    // new provider publishes, the level has none.
    assert_eq!(
        host.inspect(flow, |run| run.live_value(node(1)).cloned())
            .unwrap(),
        None
    );
    publish(15);
    until("the restarted level's value", || {
        read(&host, 100) == Some(Value::U64(15))
    });
    host.close(flow).unwrap();
    until("every provider to stop", || {
        seen.lock().unwrap().stopped == 3
    });
}

#[test]
fn an_entry_rearms_from_a_variable_even_in_a_graph_without_a_watch() {
    let seen = Shared::default();
    let (mut host, _events) =
        Host::with_services(HostOptions::default(), vec![package(&seen)]).unwrap();
    let mut graph: Json = serde_json::from_str(&graph(&host)).unwrap();
    for node in graph["nodes"].as_array_mut().unwrap() {
        if node["id"] == id(8) {
            node["inputs"]["value"] = json!({"type":STRING, "value":"weather"});
        }
    }
    graph["nodes"].as_array_mut().unwrap().retain(|node| {
        ![id(1), id(2), id(3)]
            .iter()
            .any(|removed| node["id"] == *removed)
    });
    graph["variables"] = json!([
        {"id": id(101), "name": "topic", "type": STRING, "scope": "run",
            "init": {"type":STRING, "value":"news"}},
        {"id": id(102), "name": "pinged", "type": STRING, "scope": "run"}
    ]);
    graph["edges"] = json!([
        edge(1, 4, "value", 5, "topic"),
        edge(2, 5, "then", 6, "in"),
        edge(3, 5, "payload", 6, "value"),
        edge(4, 7, "then", 8, "in")
    ]);
    let graph = graph.to_string();
    assert_eq!(host.check(&graph).unwrap(), []);
    let flow = host.open(&graph).unwrap();
    until("the initial entry arguments", || {
        seen.lock().unwrap().topics == ["news"]
    });
    let previous = seen.lock().unwrap().emitter.clone().unwrap();
    host.trigger(flow, node(7), None).unwrap();
    until("the new entry arguments", || {
        seen.lock().unwrap().topics == ["news", "weather"]
    });
    assert_eq!(previous.emit(None), Err(orange_sdk::EmitError::Closed));
    host.close(flow).unwrap();
    assert_eq!(seen.lock().unwrap().stopped, 2);
}

#[test]
fn the_run_coalesces_live_ingress_and_only_reevaluates_dependent_watches() {
    use orange_bundled::MathPackage;
    use orange_compiler::{CompileOptions, compile_control};
    use orange_runtime::{IngressError, Run, RunOptions, RunState};

    let (host, _events) =
        Host::with_services(HostOptions::default(), vec![package(&Shared::default())]).unwrap();
    let mut document = from_json(&graph(&host)).unwrap();
    let _ = document
        .apply([orange_model::DocCommand::RemoveNodes {
            ids: vec![node(5), node(6)],
        }])
        .unwrap();
    let registry = host.registry();
    // The new ID sorts before node 1 but is appended after it in this document.
    let mut unused = NewNode::new(
        node(0),
        DefinitionRef {
            id: format!("{PACKAGE}/level").parse().unwrap(),
            version: 1,
        },
    );
    unused.inputs =
        serde_json::from_value(json!({"scale": {"type": LEVEL, "value": "1"}})).unwrap();
    let edit = new_node_edit(&document, registry, unused).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    let analysis = orange_model::analyze(&document, registry);
    let plan = compile_control(
        &document,
        Arc::clone(registry),
        &analysis,
        &MathPackage::new(format!("sha256:{}", "0".repeat(64))).unwrap(),
        CompileOptions::default(),
    )
    .unwrap();
    let mut run = Run::start(
        Arc::new(plan),
        &document,
        registry,
        0,
        RunOptions::default(),
    )
    .unwrap();
    fn pump(run: &mut Run) {
        for _ in 0..1_000 {
            if run.state() == RunState::Stopped || !run.pump(1).unwrap().ready {
                return;
            }
        }
        panic!("the run did not become idle");
    }
    pump(&mut run);
    assert_eq!(
        run.watch_fault(node(2)).unwrap().fault.code,
        "core/NoValueYet"
    );
    assert_eq!(
        run.publish_live(node(1), Value::Bool(true)),
        Err(IngressError::InvalidPayload)
    );
    assert_eq!(
        run.publish_live(node(99), Value::U64(0)),
        Err(IngressError::UnknownLive)
    );
    run.publish_live(node(1), Value::U64(0)).unwrap();
    pump(&mut run);
    assert!(run.watch_fault(node(2)).is_none());
    let before = run.watch_evaluations();
    for latest in 1..=20 {
        run.publish_live(node(1), Value::U64(latest)).unwrap();
    }
    assert_eq!(
        run.publish_live(node(1), Value::Bool(true)),
        Err(IngressError::InvalidPayload)
    );
    assert_eq!(run.live_value(node(1)), Some(&Value::U64(0)));
    pump(&mut run);
    assert_eq!(run.watch_evaluations(), before + 1);
    assert_eq!(run.live_value(node(1)), Some(&Value::U64(20)));
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(20))
    );
    assert!(
        run.variable_values()
            .all(|(id, value)| value == run.variable(id))
    );
    assert!(!run.publish_live(node(1), Value::U64(20)).unwrap());
    assert!(run.publish_live(node(0), Value::U64(77)).unwrap());
    pump(&mut run);
    assert_eq!(run.watch_evaluations(), before + 1);
    assert_eq!(run.live_value(node(0)), Some(&Value::U64(77)));
    run.stop().unwrap();
    pump(&mut run);
    run.dispose().unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);
}
