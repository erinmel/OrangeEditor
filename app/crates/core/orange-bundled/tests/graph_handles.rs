//! C08b public-boundary resource acquisition, borrowing and ownership regressions.
use crate::data_graph::{edge, id};
use orange_bundled::MathPackage;
use orange_compiler::{
    BindingRequest, CompileOptions, EffectBinding, EffectResolver, ResolveError,
    compile_control_with_effects,
};
use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder,
};
use orange_model::document::{PortDirection, PortKind};
use orange_model::types::TypeRegistryBuilder;
use orange_model::validation::{HostOrigin, LeafEffects, analyze_with_effects};
use orange_model::{Value, from_json};
use orange_runtime::{
    CloseReason, EffectError, ResourceError, Run, RunOptions, RunResource, RunState,
};
use serde_json::{Value as Json, json};
use std::sync::{Arc, Mutex};

const KIND: &str = "acme/session@1";
const OTHER: &str = "acme/file@1";
const HANDLE: &str = "core/handle@1<acme/session@1>";
type Log = Arc<Mutex<Vec<(u8, CloseReason, u32)>>>;
struct Probe {
    log: Log,
    tag: u8,
    uses: u32,
}
impl RunResource for Probe {
    fn close(self: Box<Self>, reason: CloseReason) {
        self.log.lock().unwrap().push((self.tag, reason, self.uses));
    }
}
fn probe(log: &Log, tag: u8) -> Box<dyn RunResource> {
    Box::new(Probe {
        log: log.clone(),
        tag,
        uses: 0,
    })
}
struct Resolver;
impl EffectResolver for Resolver {
    fn resolve_effect(&self, r: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
        Ok(EffectBinding {
            route: if r.definition().id.as_str().ends_with("/open") {
                1
            } else {
                2
            },
            context: Arc::new(r.provider().cloned()),
            inputs: r
                .signature()
                .ports
                .iter()
                .filter(|p| p.kind == PortKind::Data && p.direction == PortDirection::Input)
                .map(|p| p.port_id.as_str().parse().unwrap())
                .collect(),
            outputs: r
                .signature()
                .ports
                .iter()
                .filter(|p| {
                    p.kind == PortKind::Data
                        && p.direction == PortDirection::Output
                        && p.port_id.as_str() != "error"
                })
                .map(|p| p.port_id.as_str().parse().unwrap())
                .collect(),
            action: None,
        })
    }
}
fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}});
    if name == "on-start" {
        node["activation"] =
            json!({"mode":"queue","capacity":4,"overflow":"fault","on_fault":"continue","order":0});
    }
    node
}
fn effect(n: u32, name: &str) -> Json {
    json!({"id":id(n),"definition_id":format!("acme.resources/{name}"),"definition_version":1,"config_version":1,"config":{},"signature":name,"provider":{"package":"acme.resources","version":"1.0.0","id":"inert"}})
}
fn wait(n: u32, time: u64) -> Json {
    let mut node = core(n, "wait");
    node["inputs"] = json!({"duration":{"type":"core/duration@1","value":time.to_string()}});
    node
}
fn pick(wire: &Json, n: u32) -> Json {
    wire["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n2| n2["id"] == id(n))
        .unwrap()
        .clone()
}
fn setup(options: RunOptions, edit: impl FnOnce(&mut Json)) -> Run {
    try_setup(options, edit).unwrap()
}
fn try_setup(
    options: RunOptions,
    edit: impl FnOnce(&mut Json),
) -> Result<Run, orange_compiler::CompileError> {
    let mut types = TypeRegistryBuilder::core();
    for kind in [KIND, OTHER] {
        types
            .register_handle_kind("acme", &kind.parse().unwrap())
            .unwrap();
    }
    let package = MathPackage::new(format!("sha256:{}", "a".repeat(64))).unwrap();
    let mut builder = package
        .register(
            RegistryBuilder::core(
                HostOrigin::new("0.0.0", format!("sha256:{}", "a".repeat(64))).unwrap(),
            )
            .types(Arc::new(types.build().unwrap())),
        )
        .unwrap();
    let key = ProviderKey {
        package: "acme.resources".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "b".repeat(64)),
        id: "inert".into(),
    };
    let mut provider = ProviderDescriptor {
        key: key.clone(),
        implements: Default::default(),
        specializations: Default::default(),
    };
    let mut signatures = serde_json::Map::new();
    for name in ["open", "use"] {
        let mut ports = vec![];
        if name == "use" {
            ports.push(json!({"port_id":"handle","direction":"input","kind":"data","type":HANDLE}));
        }
        ports.push(json!({"port_id":if name=="open"{"handle"}else{"result"},"direction":"output","kind":"data","type":if name=="open"{HANDLE}else{"core/u64@1"},"source":"execution","available_on":["then"]}));
        let definition=NodeDefinition::from_json(&json!({"definition_id":format!("acme.resources/{name}"),"definition_version":1,"config_version":1,"kind":"async","suspends":"always","determinism":"external","ports":ports}).to_string()).unwrap();
        let Ports::Declared(ports) = &definition.ports else {
            unreachable!()
        };
        let signature = orange_model::signature::project_with_suspension(
            definition.kind,
            definition.suspends,
            ports.clone(),
            &Default::default(),
        );
        signatures.insert(name.into(), serde_json::to_value(signature).unwrap());
        provider
            .implements
            .insert((definition.definition.clone(), 1));
        builder = builder.definition(definition);
    }
    let snapshot = builder.provider(provider).build().unwrap();
    let mut set = core(7, "set-variable");
    set["config"] = json!({"variable":id(100)});
    let mut wire = json!({"format":"orange.graph","format_version":1,"graph_id":id(900),"revision":0,
        "package_lock":[{"id":"acme.resources","version":"1.0.0","content_digest":key.content_digest}],"signatures":signatures,
        "variables":[{"id":id(100),"name":"seen","type":"core/u64@1","scope":"run","init":{"type":"core/u64@1","value":"0"}}],
        "nodes":[core(1,"on-start"),effect(2,"open"),wait(3,10),effect(4,"use"),core(5,"close-handle"),wait(6,10),set],
        "edges":[edge(1,1,"then",2,"in"),edge(2,2,"then",3,"in"),edge(3,3,"then",4,"in"),edge(4,2,"handle",4,"handle"),edge(5,4,"then",7,"in"),
            edge(6,4,"result",7,"value"),edge(7,7,"then",5,"in"),edge(8,2,"handle",5,"handle"),edge(9,5,"then",6,"in")]});
    edit(&mut wire);
    let doc = from_json(&wire.to_string()).unwrap();
    let leaves = doc
        .functions()
        .values()
        .flat_map(|f| f.body.nodes.values())
        .filter(|n| !n.definition.id.is_core())
        .map(|n| (n.id, LeafEffects::Known(vec![])))
        .collect();
    let analysis = analyze_with_effects(&doc, &snapshot, leaves);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let plan = compile_control_with_effects(
        &doc,
        snapshot.clone(),
        &analysis,
        &package,
        &Resolver,
        CompileOptions::default(),
    )?;
    Ok(Run::start(Arc::new(plan), &doc, &snapshot, 1, options).unwrap())
}
fn idle(run: &mut Run) {
    for _ in 0..1000 {
        if run.state() == RunState::Stopped || !run.pump(1).unwrap().ready {
            return;
        }
    }
    panic!("did not idle");
}
fn open(run: &mut Run, log: &Log) {
    idle(run);
    let request = run.take_effect().unwrap();
    assert_eq!(request.route, 1);
    run.complete_effect_resource(request.id, &KIND.parse().unwrap(), probe(log, 1))
        .unwrap();
    idle(run);
}

#[test]
fn compiled_handle_crosses_wait_and_closes_explicitly_once() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |_| {});
    open(&mut run, &log);
    assert!(log.lock().unwrap().is_empty());
    run.advance_to(10).unwrap();
    idle(&mut run);
    let request = run.take_effect().unwrap();
    assert_eq!(request.route, 2);
    let handle = run
        .resource_handle(&request.inputs[0], &KIND.parse().unwrap())
        .unwrap();
    run.resource_mut::<Probe>(handle, &KIND.parse().unwrap())
        .unwrap()
        .uses += 1;
    assert_eq!(
        run.resource_handle(&request.inputs[0], &OTHER.parse().unwrap()),
        Err(ResourceError::WrongKind)
    );
    run.complete_effect(request.id, Ok(&[Value::U64(7)]))
        .unwrap();
    idle(&mut run);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(7)));
    assert_eq!(&*log.lock().unwrap(), &[(1, CloseReason::Closed, 1)]);
    assert_eq!(run.ownership().resources, 0);
    run.stop().unwrap();
    idle(&mut run);
    run.dispose().unwrap();
    drop(request);
    assert_eq!(run.retention().reserved_bytes, 0);
}
#[test]
fn forged_and_foreign_values_refuse_before_publication() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |_| {});
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let host = run
        .acquire_resource(&KIND.parse().unwrap(), probe(&log, 1))
        .unwrap();
    let value = run.resource_value(host).unwrap();
    let Value::Handle(reference) = &value else {
        panic!()
    };
    let forged = Value::Handle(Arc::new(orange_model::value::HandleValue::new(
        KIND.parse().unwrap(),
        reference.id(),
        Arc::new(()),
    )));
    assert_eq!(
        run.resource_handle(&forged, &KIND.parse().unwrap()),
        Err(ResourceError::Forged)
    );
    assert_eq!(
        run.complete_effect(request.id, Ok(&[forged])),
        Err(EffectError::Contract)
    );
    let mut other = setup(RunOptions::default(), |_| {});
    idle(&mut other);
    let other_request = other.take_effect().unwrap();
    assert_eq!(
        other.complete_effect(other_request.id, Ok(std::slice::from_ref(&value))),
        Err(EffectError::Contract)
    );
    assert_eq!(
        other.resource_handle(&value, &KIND.parse().unwrap()),
        Err(ResourceError::WrongRun)
    );
    run.complete_effect(request.id, Ok(&[value])).unwrap();
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(log.lock().unwrap().len(), 1);
}
#[test]
fn resource_completion_refusals_return_the_object_unclosed_for_retry() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |_| {});
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let refused = run
        .complete_effect_resource(request.id, &OTHER.parse().unwrap(), probe(&log, 1))
        .unwrap_err();
    assert_eq!(refused.error, EffectError::Contract);
    assert!(log.lock().unwrap().is_empty());
    run.complete_effect_resource(request.id, &KIND.parse().unwrap(), refused.resource)
        .unwrap();
    idle(&mut run);
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(log.lock().unwrap().len(), 1);
    assert_eq!(run.ownership().resources, 0);
}
#[test]
fn revocation_invalidates_an_inflight_borrow_and_its_late_completion() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |_| {});
    open(&mut run, &log);
    run.advance_to(10).unwrap();
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let handle = run
        .resource_handle(&request.inputs[0], &KIND.parse().unwrap())
        .unwrap();
    run.close_resource(handle).unwrap();
    assert!(request.lease.is_cancelled());
    assert_eq!(
        run.complete_effect(request.id, Ok(&[Value::U64(9)])),
        Err(EffectError::Stale)
    );
    idle(&mut run);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(0)));
    assert_eq!(run.ownership().effects, 0);
    assert_eq!(log.lock().unwrap().len(), 1);
}
#[test]
fn natural_region_retirement_closes_unreturned_resource() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |wire| {
        wire["nodes"] = json!([pick(wire, 1), pick(wire, 2), wait(3, 1)]);
        wire["edges"] = json!([edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")]);
    });
    open(&mut run, &log);
    run.advance_to(1).unwrap();
    idle(&mut run);
    assert_eq!(&*log.lock().unwrap(), &[(1, CloseReason::ScopeEnded, 0)]);
    assert_eq!(run.ownership().resources, 0);
}

fn call(n: u32, function: u32) -> Json {
    let mut node = core(n, "call");
    node["config"] = json!({"function":id(function)});
    node
}
fn port(name: &str, direction: &str) -> Json {
    json!({"port_id":name,"direction":direction,"kind":"data","type":HANDLE})
}

#[test]
fn return_transfers_new_resource_through_nested_calls_to_the_caller() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |wire| {
        wire["functions"] = json!([
            {"id":id(200),"name":"open","kind":"async","inputs":[],"outputs":[port("handle","output")],
             "nodes":[core(201,"function-entry"),effect(202,"open"),core(203,"return")],
             "edges":[edge(201,201,"then",202,"in"),edge(202,202,"then",203,"in"),edge(203,202,"handle",203,"handle")]},
            {"id":id(300),"name":"nested","kind":"async","inputs":[],"outputs":[port("handle","output")],
             "nodes":[core(301,"function-entry"),call(302,200),core(303,"return")],
             "edges":[edge(301,301,"then",302,"in"),edge(302,302,"then",303,"in"),edge(303,302,"handle",303,"handle")]}
        ]);
        wire["nodes"][1] = call(2, 300);
    });
    open(&mut run, &log);
    assert!(log.lock().unwrap().is_empty());
    assert_eq!(run.ownership().resources, 1);
    run.advance_to(10).unwrap();
    idle(&mut run);
    let request = run.take_effect().expect("returned handle reaches caller");
    assert!(
        run.resource_handle(&request.inputs[0], &KIND.parse().unwrap())
            .is_ok()
    );
    run.complete_effect(request.id, Ok(&[Value::U64(42)]))
        .unwrap();
    idle(&mut run);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(42))
    );
    assert_eq!(&*log.lock().unwrap(), &[(1, CloseReason::Closed, 0)]);
}

#[test]
fn borrowed_argument_keeps_caller_owner_and_unreturned_acquisition_closes() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |wire| {
        wire["functions"] = json!([
            {"id":id(200),"name":"borrow","kind":"async","inputs":[port("borrowed","input")],"outputs":[port("handle","output")],
             "nodes":[core(201,"function-entry"),effect(202,"open"),core(203,"return")],
             "edges":[edge(201,201,"then",202,"in"),edge(202,202,"then",203,"in"),edge(203,201,"borrowed",203,"handle")]}
        ]);
        wire["nodes"].as_array_mut().unwrap().push(call(8, 200));
        wire["edges"][1] = edge(2, 2, "then", 8, "in");
        wire["edges"].as_array_mut().unwrap().extend([
            edge(10, 2, "handle", 8, "borrowed"),
            edge(11, 8, "then", 3, "in"),
        ]);
        wire["edges"][3] = edge(4, 8, "handle", 4, "handle");
    });
    open(&mut run, &log);
    let request = run.take_effect().expect("callee acquisition");
    run.complete_effect_resource(request.id, &KIND.parse().unwrap(), probe(&log, 2))
        .unwrap();
    idle(&mut run);
    assert_eq!(&*log.lock().unwrap(), &[(2, CloseReason::ScopeEnded, 0)]);
    assert_eq!(run.ownership().resources, 1);
    run.advance_to(10).unwrap();
    idle(&mut run);
    let request = run
        .take_effect()
        .expect("borrow remains live after callee returns");
    assert!(
        run.resource_handle(&request.inputs[0], &KIND.parse().unwrap())
            .is_ok()
    );
    run.complete_effect(request.id, Ok(&[Value::U64(3)]))
        .unwrap();
    idle(&mut run);
    assert_eq!(
        &*log.lock().unwrap(),
        &[(2, CloseReason::ScopeEnded, 0), (1, CloseReason::Closed, 0)]
    );
}

#[test]
fn return_rechecks_liveness_after_detached_cleanup_closes_its_result() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |wire| {
        let mut fork = core(204, "fork");
        fork["config"] = json!({"join":"detach"});
        fork["variadic"] = json!({"branch":{"next":1,"order":[0]}});
        wire["functions"] = json!([
            {"id":id(200),"name":"closing-return","kind":"async","inputs":[],"outputs":[port("handle","output")],
             "nodes":[core(201,"function-entry"),effect(202,"open"),core(203,"return"),fork,wait(205,100),core(206,"close-handle")],
             "edges":[edge(201,201,"then",202,"in"),edge(202,202,"then",204,"in"),edge(203,204,"completed",203,"in"),
                edge(204,202,"handle",203,"handle"),edge(205,204,"branch#0",205,"in"),edge(206,205,"cancelled",206,"in"),edge(207,202,"handle",206,"handle")]}
        ]);
        wire["nodes"][1] = call(2, 200);
    });
    open(&mut run, &log);
    assert_eq!(&*log.lock().unwrap(), &[(1, CloseReason::Closed, 0)]);
    run.advance_to(10).unwrap();
    idle(&mut run);
    assert!(run.take_effect().is_none());
    assert!(run.trace().iter().any(|e| {
        e.node.to_string() == id(2)
            && e.fault
                .as_ref()
                .is_some_and(|f| f.fault.code == "core/StaleHandle")
    }));
}

#[test]
fn race_and_stop_cleanup_can_close_the_acquired_handle_idempotently() {
    for race in [false, true] {
        let log = Log::default();
        let mut run = setup(RunOptions::default(), |wire| {
            let mut fork = core(8, "fork");
            fork["config"] = json!({"join":"race"});
            fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
            wire["nodes"] = json!([
                pick(wire, 1),
                effect(2, "open"),
                wait(3, 100),
                core(5, "close-handle"),
                core(6, "close-handle"),
                fork,
                wait(9, 1)
            ]);
            let mut edges = vec![
                edge(2, 2, "then", 3, "in"),
                edge(3, 3, "cancelled", 5, "in"),
                edge(4, 2, "handle", 5, "handle"),
                edge(5, 5, "then", 6, "in"),
                edge(6, 2, "handle", 6, "handle"),
            ];
            if race {
                edges.extend([
                    edge(1, 1, "then", 8, "in"),
                    edge(7, 8, "branch#0", 2, "in"),
                    edge(8, 8, "branch#1", 9, "in"),
                ]);
            } else {
                edges.push(edge(1, 1, "then", 2, "in"));
                // An unreachable Fork is legal but has no bearing on stop cleanup.
            }
            wire["edges"] = json!(edges);
        });
        open(&mut run, &log);
        assert!(log.lock().unwrap().is_empty());
        if race {
            run.advance_to(1).unwrap();
        } else {
            run.stop().unwrap();
        }
        idle(&mut run);
        assert_eq!(&*log.lock().unwrap(), &[(1, CloseReason::Closed, 0)]);
        assert_eq!(run.ownership().resources, 0);
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(6)
                    && e.outcome == orange_runtime::ControlOutcome::Then)
                .count(),
            1
        );
    }
}

#[test]
fn resource_limit_refusal_rolls_back_without_closing_and_reuses_generation_safely() {
    let log = Log::default();
    let mut options = RunOptions::default();
    options.limits.max_resources = 1;
    let mut run = setup(options, |_| {});
    idle(&mut run);
    let pending = run.take_effect().unwrap();
    let kind = KIND.parse().unwrap();
    let occupied = run.acquire_resource(&kind, probe(&log, 1)).unwrap();
    let stale = run.resource_value(occupied).unwrap();
    let refused = run
        .complete_effect_resource(pending.id, &kind, probe(&log, 2))
        .unwrap_err();
    assert_eq!(refused.error, EffectError::Limit);
    assert!(log.lock().unwrap().is_empty());
    run.close_resource(occupied).unwrap();
    run.complete_effect_resource(pending.id, &kind, refused.resource)
        .unwrap();
    assert_eq!(
        run.resource_handle(&stale, &kind),
        Err(ResourceError::StaleHandle)
    );
    idle(&mut run);
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(
        &*log.lock().unwrap(),
        &[(1, CloseReason::Closed, 0), (2, CloseReason::ScopeEnded, 0)]
    );
}

#[test]
fn revocation_retracts_an_accepted_completion_before_publication() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |_| {});
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let kind = KIND.parse().unwrap();
    let handle = run.acquire_resource(&kind, probe(&log, 1)).unwrap();
    let value = run.resource_value(handle).unwrap();
    let schemas = orange_model::value::literal_codec::SchemaSet::default();
    assert!(
        orange_model::value::literal_codec::encode(&HANDLE.parse().unwrap(), &value, &schemas)
            .is_err()
    );
    assert!(orange_model::value::shape::ValueShape::of(&HANDLE.parse().unwrap()).is_none());
    run.complete_effect(request.id, Ok(&[value])).unwrap();
    run.close_resource(handle).unwrap();
    idle(&mut run);
    assert!(request.lease.is_cancelled());
    assert_eq!(run.ownership().effects, 0);
    assert!(run.trace().iter().any(|e| {
        e.fault
            .as_ref()
            .is_some_and(|f| f.fault.code == "core/StaleHandle")
    }));
    assert_eq!(log.lock().unwrap().len(), 1);
}

#[test]
fn sibling_scope_cannot_publish_another_branchs_handle() {
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |wire| {
        let mut fork = core(9, "fork");
        fork["config"] = json!({"join":"all"});
        fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
        wire["nodes"] = json!([
            pick(wire, 1),
            effect(2, "open"),
            effect(4, "use"),
            effect(8, "open"),
            fork
        ]);
        wire["edges"] = json!([
            edge(1, 1, "then", 9, "in"),
            edge(2, 9, "branch#0", 2, "in"),
            edge(3, 9, "branch#1", 8, "in"),
            edge(4, 2, "then", 4, "in"),
            edge(5, 2, "handle", 4, "handle")
        ]);
    });
    idle(&mut run);
    let a = run.take_effect().unwrap();
    let b = run.take_effect().unwrap();
    assert_eq!(a.node.to_string(), id(2));
    assert_eq!(b.node.to_string(), id(8));
    run.complete_effect_resource(a.id, &KIND.parse().unwrap(), probe(&log, 1))
        .unwrap();
    idle(&mut run);
    let use_request = run.take_effect().unwrap();
    assert_eq!(
        run.complete_effect(b.id, Ok(&use_request.inputs)),
        Err(EffectError::Contract)
    );
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(log.lock().unwrap().len(), 1);
}

#[test]
fn output_quota_refusal_returns_object_and_refunds_graph_token_metadata() {
    let log = Log::default();
    let mut options = RunOptions::default();
    options.limits.max_effect_bytes = 0;
    let mut run = setup(options, |_| {});
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let before = run.retention().reserved_bytes;
    let refused = run
        .complete_effect_resource(request.id, &KIND.parse().unwrap(), probe(&log, 1))
        .unwrap_err();
    assert_eq!(refused.error, EffectError::Limit);
    assert_eq!(run.retention().reserved_bytes, before);
    assert_eq!(run.ownership().resources, 0);
    assert_eq!(run.ownership().effects, 1);
    assert!(log.lock().unwrap().is_empty());
    refused.resource.close(CloseReason::Closed);
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(log.lock().unwrap().len(), 1);
}

#[test]
fn accepted_unpublished_resource_closes_on_stop_or_drop_and_aliases_keep_only_metadata() {
    for stop in [false, true] {
        let log = Log::default();
        let mut run = setup(RunOptions::default(), |_| {});
        idle(&mut run);
        let request = run.take_effect().unwrap();
        run.complete_effect_resource(request.id, &KIND.parse().unwrap(), probe(&log, 1))
            .unwrap();
        if stop {
            run.stop().unwrap();
            idle(&mut run);
            assert_eq!(run.ownership().resources, 0);
        }
        drop(run);
        assert!(request.lease.is_cancelled());
        assert_eq!(log.lock().unwrap().len(), 1);
        assert_eq!(
            log.lock().unwrap()[0].1,
            if stop {
                CloseReason::ScopeEnded
            } else {
                CloseReason::RunDropped
            }
        );
    }
    let log = Log::default();
    let mut run = setup(RunOptions::default(), |_| {});
    let kind = KIND.parse().unwrap();
    let handle = run.acquire_resource(&kind, probe(&log, 1)).unwrap();
    let value = run.resource_value(handle).unwrap();
    run.close_resource(handle).unwrap();
    run.stop().unwrap();
    idle(&mut run);
    run.dispose().unwrap();
    assert!(run.retention().reserved_bytes > 0);
    drop(value);
    assert_eq!(run.retention().reserved_bytes, 0);
    assert_eq!(log.lock().unwrap().len(), 1);
    assert_eq!(std::mem::size_of::<Value>(), 16);
    let shape = orange_model::value::shape::ValueShape::Handle(kind);
    assert!(!shape.host_eq());
    assert!(!shape.map_key());
}

#[test]
fn direct_handle_profile_refuses_manual_payloads_and_register_storage() {
    for manual in [false, true] {
        let Err(error) = try_setup(RunOptions::default(), |wire| {
            if manual {
                let mut entry = pick(wire, 1);
                entry["definition_id"] = json!("core/on-manual");
                entry["config"] = json!({"payload":HANDLE});
                wire["nodes"] = json!([entry, core(5, "close-handle")]);
                wire["edges"] = json!([
                    edge(1, 1, "then", 5, "in"),
                    edge(2, 1, "payload", 5, "handle")
                ]);
            } else {
                let mut register = core(3, "register");
                register["config"] = json!({"scope":"activation"});
                wire["nodes"] = json!([pick(wire, 1), effect(2, "open"), register]);
                wire["edges"] = json!([
                    edge(1, 1, "then", 2, "in"),
                    edge(2, 2, "then", 3, "in"),
                    edge(3, 2, "handle", 3, "next")
                ]);
            }
        }) else {
            panic!("unsupported handle holder must fail compilation");
        };
        assert!(matches!(
            error,
            orange_compiler::CompileError::UnsupportedPort { .. }
        ));
    }
}

#[test]
fn a_resource_acquired_while_paused_is_paused_as_it_is_taken() {
    struct Held(Arc<Mutex<Vec<&'static str>>>);
    impl RunResource for Held {
        fn close(self: Box<Self>, _: CloseReason) {}
        fn pause(&self) {
            self.0.lock().unwrap().push("pause");
        }
        fn resume(&self) {
            self.0.lock().unwrap().push("resume");
        }
    }
    let holds = Arc::new(Mutex::new(Vec::new()));
    let mut run = setup(RunOptions::default(), |_| {});
    idle(&mut run);
    let request = run.take_effect().unwrap();
    run.pause().unwrap();
    run.complete_effect_resource(
        request.id,
        &KIND.parse().unwrap(),
        Box::new(Held(holds.clone())),
    )
    .unwrap();
    assert_eq!(*holds.lock().unwrap(), ["pause"]);
    run.resume().unwrap();
    assert_eq!(*holds.lock().unwrap(), ["pause", "resume"]);
}
