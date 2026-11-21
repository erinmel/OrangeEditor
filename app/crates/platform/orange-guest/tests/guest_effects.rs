//! A package's action and async nodes run through the `effects` export of its component:
//! an action inline in the run, an async node through the host's effect requests.
use std::sync::Arc;

use orange_bundled::MathPackage;
use orange_compiler::{
    BindingRequest, CompileOptions, EffectBinding, EffectResolver, ResolveError,
    compile_control_with_effects,
};
use orange_guest::{Grants, GuestEngine, GuestLimits, GuestPackage, WithGuests, guest_effect};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::catalog::{NodeDefinition, RegistryBuilder};
use orange_model::document::DefinitionRef;
use orange_model::validation::HostOrigin;
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use orange_packages::{HOST_API_VERSION, MemoryJournal, PackageStore};
use orange_runtime::{Run, RunOptions, RunState};
use serde_json::{Value as Json, json};

const I64: &str = "core/i64@1";
const ACCUMULATE: &str = "org.example.probe/accumulate";
const REFUSE: &str = "org.example.probe/refuse";
const TOTAL: u32 = 100;

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}
fn dref(definition: &str) -> DefinitionRef {
    DefinitionRef {
        id: definition.parse().unwrap(),
        version: 1,
    }
}

/// The host has no effect adapters of its own in these graphs.
struct NoHostEffects;
impl EffectResolver for NoHostEffects {
    fn resolve_effect(&self, _: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
        Err(ResolveError::Unavailable)
    }
}

fn manifest() -> Json {
    json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.probe", "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "execution": {"kind": "wasm-component", "entry": "component.wasm",
            "world": "orange:expansion/plugin@1.0.0"},
        "dependencies": [],
        "contributions": {
            "nodes": [ACCUMULATE, REFUSE],
            "providers": [
                {"slot": format!("node:{ACCUMULATE}@1"), "id": "guest"},
                {"slot": format!("node:{REFUSE}@1"), "id": "guest"}
            ]
        },
        "permissions": [],
        "integrity": {"algorithm": "sha256", "content_digest": "0".repeat(64)},
        "license": "MIT"
    })
}

fn definition(definition: &str, kind: &str, inputs: &[&str]) -> NodeDefinition {
    let mut ports: Vec<Json> = inputs
        .iter()
        .map(|name| json!({"port_id": name, "direction": "input", "kind": "data", "type": I64}))
        .collect();
    ports.push(
        json!({"port_id": "result", "direction": "output", "kind": "data", "type": I64,
        "source": "execution", "available_on": ["then"]}),
    );
    NodeDefinition::from_json(
        &json!({"definition_id": definition, "definition_version": 1, "config_version": 1,
            "kind": kind, "suspends": if kind == "action" {"never"} else {"always"},
            "determinism": "external", "ports": ports})
        .to_string(),
    )
    .unwrap()
}

fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id": id(n), "definition_id": format!("core/{name}"),
        "definition_version": 1, "config_version": 1, "config": {}});
    if name == "on-start" {
        node["activation"] = json!({"mode": "queue", "capacity": 4, "overflow": "fault",
            "on_fault": "continue", "order": 0});
    }
    node
}
fn edge(n: u32, from: u32, out: &str, to: u32, input: &str) -> Json {
    json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
        "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": input}})
}

/// On Start runs the probe's `effect` node three times, each adding five, and stores the
/// node's result. `kind` selects whether the node is an action or an async node.
fn looping_run(kind: &str, effect: &str) -> Run {
    let native = MathPackage::new(digest('b')).unwrap();
    let base = native
        .register(RegistryBuilder::core(
            HostOrigin::new("0.0.0", digest('c')).unwrap(),
        ))
        .unwrap()
        .build()
        .unwrap();
    let engine = GuestEngine::new(GuestLimits::default()).unwrap();
    // The probe component imports `log`, so its instances need that grant.
    let (verified, files) = crate::archive_nodes::archive(manifest());
    let guest = GuestPackage::from_archive(
        &engine,
        &verified,
        &files,
        "guest",
        Grants {
            log: true,
            files: false,
            http: false,
        },
    )
    .unwrap();
    let mut store = PackageStore::new(base, HOST_API_VERSION, MemoryJournal::new());
    let _ = store
        .install(verified.into_contents(vec![
            definition(ACCUMULATE, kind, &["amount"]),
            definition(REFUSE, kind, &[]),
        ]))
        .unwrap();
    let _ = store
        .select_default(dref(ACCUMULATE), guest.provider().clone())
        .unwrap();
    let registry = store
        .select_default(dref(REFUSE), guest.provider().clone())
        .unwrap();

    // The package node is authored through the binding API, so it carries its exact
    // provider and signature; the control nodes around it are added to the saved form.
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let mut node = NewNode::new(id(3).parse().unwrap(), dref(effect));
    if effect == ACCUMULATE {
        node.inputs =
            serde_json::from_value(json!({"amount": {"type": I64, "value": "5"}})).unwrap();
    }
    let edit = new_node_edit(&document, &registry, node).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    let mut count = core(2, "count");
    count["inputs"] = json!({"count": {"type": I64, "value": "3"}});
    let mut set = core(4, "set-variable");
    set["config"] = json!({"variable": id(TOTAL)});
    let nodes = saved["nodes"].as_array_mut().unwrap();
    nodes.extend([core(1, "on-start"), count, set]);
    saved["edges"] = json!([
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "body", 3, "in"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 3, "result", 4, "value"),
    ]);
    saved["variables"] = json!([{"id": id(TOTAL), "name": "total", "type": I64, "scope": "run"}]);
    let document = from_json(&saved.to_string()).unwrap();

    let analysis = analyze(&document, &registry);
    assert!(
        analysis.is_ready_for_lowering(&document, &registry),
        "{:?}",
        analysis.report().diagnostics
    );
    let mut effects = WithGuests::new(NoHostEffects);
    effects.add(guest);
    let plan = compile_control_with_effects(
        &document,
        Arc::clone(&registry),
        &analysis,
        &native,
        &effects,
        CompileOptions::default(),
    )
    .unwrap();
    Run::start(
        Arc::new(plan),
        &document,
        &registry,
        0,
        RunOptions::default(),
    )
    .unwrap()
}

fn total(run: &Run) -> Option<&Value> {
    run.variable(id(TOTAL).parse().unwrap())
}
fn idle(run: &mut Run) {
    for _ in 0..1000 {
        if run.state() == RunState::Stopped || !run.pump(8).unwrap().ready {
            return;
        }
    }
    panic!("run did not idle");
}

#[test]
fn a_guest_action_runs_inline_and_keeps_state_between_calls() {
    let mut run = looping_run("action", ACCUMULATE);
    idle(&mut run);
    assert!(run.take_effect().is_none());
    assert_eq!(total(&run), Some(&Value::I64(15)));
}

#[test]
fn a_guest_async_node_is_run_by_the_host_from_its_effect_request() {
    let mut run = looping_run("async", ACCUMULATE);
    for expected in [5, 10, 15] {
        idle(&mut run);
        let request = run.take_effect().expect("the node asks the host to run it");
        let effect = guest_effect(&request.context).expect("a package node");
        let result = effect.run(&request.inputs).unwrap();
        assert_eq!(result, [Value::I64(expected)]);
        run.complete_effect(request.id, Ok(&result)).unwrap();
    }
    idle(&mut run);
    assert!(run.take_effect().is_none());
    assert_eq!(total(&run), Some(&Value::I64(15)));
}

#[test]
fn a_guest_effect_error_reaches_the_host_as_the_nodes_own_fault() {
    let mut run = looping_run("async", REFUSE);
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let fault = guest_effect(&request.context)
        .unwrap()
        .run(&request.inputs)
        .unwrap_err();
    assert_eq!(fault.code, "org.example.probe/Refused");
    assert_eq!(fault.message_key, "org.example.probe/failed");
    // The instance is not poisoned by an error the guest reported itself.
    assert_eq!(
        guest_effect(&request.context)
            .unwrap()
            .run(&request.inputs)
            .unwrap_err()
            .code,
        "org.example.probe/Refused"
    );
}
