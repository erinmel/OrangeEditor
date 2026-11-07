//! C09b public graph stream behavior through compiled graphs and controlled producers.
use crate::data_graph::{catalog, edge, graph, id};
use orange_compiler::{
    BindingRequest, CompileOptions, EffectBinding, EffectResolver, KernelFault, ResolveError,
    compile_control_with_effects,
};
use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder,
};
use orange_model::{Value, from_json, to_json};
use orange_runtime::{
    CloseReason, EffectError, PullStream, Run, RunOptions, RunState, StreamDemand, StreamError,
    StreamStep,
};
use serde_json::{Value as Json, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

const ITEM: &str = "core/u64@1";
struct Adapter;
impl EffectResolver for Adapter {
    fn resolve_effect(&self, r: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
        Ok(EffectBinding {
            route: 1,
            context: Arc::new(r.provider().cloned()),
            inputs: Box::new([]),
            outputs: vec!["stream".parse().unwrap()].into(),
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
fn source(n: u32) -> Json {
    json!({"id":id(n),"definition_id":"acme.streams/open","definition_version":1,"config_version":1,"config":{},"signature":"open","provider":{"package":"acme.streams","version":"1.0.0","id":"inert"}})
}
fn wait(n: u32, ns: u64) -> Json {
    let mut n = core(n, "wait");
    n["inputs"] = json!({"duration":{"type":"core/duration@1","value":ns.to_string()}});
    n
}
fn pick(wire: &Json, n: u32) -> Json {
    wire["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == id(n))
        .unwrap()
        .clone()
}
fn setup(options: RunOptions, edit: impl FnOnce(&mut Json)) -> Run {
    setup_item(ITEM, options, edit)
}
fn setup_item(item: &str, options: RunOptions, edit: impl FnOnce(&mut Json)) -> Run {
    let (package, base) = catalog();
    let definition=NodeDefinition::from_json(&json!({"definition_id":"acme.streams/open","definition_version":1,"config_version":1,"kind":"async","suspends":"always","determinism":"external",
        "ports":[{"port_id":"stream","direction":"output","kind":"data","type":format!("core/stream@1<{item}>"),"source":"execution","available_on":["then"]}]}).to_string()).unwrap();
    let Ports::Declared(ports) = &definition.ports else {
        unreachable!()
    };
    let signature = orange_model::signature::project_with_suspension(
        definition.kind,
        definition.suspends,
        ports.clone(),
        &Default::default(),
    );
    let key = ProviderKey {
        package: "acme.streams".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "b".repeat(64)),
        id: "inert".into(),
    };
    let provider = ProviderDescriptor {
        key: key.clone(),
        implements: [(definition.definition.clone(), 1)].into(),
        specializations: Default::default(),
    };
    let snapshot = RegistryBuilder::from_snapshot(&base)
        .definition(definition)
        .provider(provider)
        .build()
        .unwrap();
    let mut update = core(5, "update-variable");
    update["config"] = json!({"variable":id(100),"operation":"add"});
    let doc = graph(
        &package,
        vec![
            core(1, "on-start"),
            source(2),
            core(3, "for-each-item"),
            wait(4, 5),
            update,
            wait(6, 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "stream", 3, "stream"),
            edge(4, 3, "body", 4, "in"),
            edge(5, 4, "then", 5, "in"),
            edge(6, 3, "item", 5, "value"),
            edge(7, 3, "completed", 6, "in"),
        ],
        vec![],
    );
    let mut wire: Json = serde_json::from_str(&to_json(&doc).unwrap()).unwrap();
    wire["package_lock"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":key.package,"version":key.version,"content_digest":key.content_digest}));
    wire["signatures"]["open"] = serde_json::to_value(signature).unwrap();
    wire["variables"] = json!([{"id":id(100),"name":"seen","scope":"run","type":ITEM,"init":{"type":ITEM,"value":"0"}}]);
    edit(&mut wire);
    let doc = from_json(&wire.to_string()).unwrap();
    let leaves = doc
        .functions()
        .values()
        .flat_map(|f| f.body.nodes.values())
        .filter(|n| !n.definition.id.is_core())
        .map(|n| (n.id, orange_model::validation::LeafEffects::Known(vec![])))
        .collect();
    let analysis = orange_model::validation::analyze_with_effects(&doc, &snapshot, leaves);
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
        &Adapter,
        CompileOptions::default(),
    )
    .unwrap();
    Run::start(Arc::new(plan), &doc, &snapshot, 1, options).unwrap()
}
fn idle(run: &mut Run) {
    for _ in 0..3000 {
        if run.state() == RunState::Stopped || !run.pump(1).unwrap().ready {
            return;
        }
    }
    panic!("did not idle");
}
#[derive(Default, Clone)]
struct Probe {
    reads: Arc<AtomicUsize>,
    closes: Arc<Mutex<Vec<CloseReason>>>,
    demands: Arc<Mutex<Vec<StreamDemand>>>,
}
struct Producer {
    probe: Probe,
    length: usize,
    asynchronous: bool,
}
impl PullStream for Producer {
    fn next(&mut self, demand: StreamDemand) -> Result<StreamStep, KernelFault> {
        let n = self.probe.reads.fetch_add(1, Ordering::Relaxed);
        self.probe.demands.lock().unwrap().push(demand);
        Ok(if self.asynchronous {
            StreamStep::Pending
        } else if n == self.length {
            StreamStep::End
        } else {
            StreamStep::Item(Value::U64(n as u64 + 1))
        })
    }
    fn close(self: Box<Self>, reason: CloseReason) {
        self.probe.closes.lock().unwrap().push(reason);
    }
}
fn producer(length: usize, asynchronous: bool) -> (Box<dyn PullStream>, Probe) {
    let probe = Probe::default();
    (
        Box::new(Producer {
            probe: probe.clone(),
            length,
            asynchronous,
        }),
        probe,
    )
}
fn open(run: &mut Run, length: usize, asynchronous: bool) -> Probe {
    idle(run);
    let request = run.take_effect().unwrap();
    let (producer, probe) = producer(length, asynchronous);
    run.complete_effect_stream(request.id, &ITEM.parse().unwrap(), producer)
        .unwrap();
    assert_eq!(probe.reads.load(Ordering::Relaxed), 0);
    probe
}
fn variable(run: &Run) -> Option<&Value> {
    run.variable(id(100).parse().unwrap())
}
fn next_wire(wire: &mut Json) {
    let mut set = core(5, "set-variable");
    set["config"] = json!({"variable":id(100)});
    wire["variables"][0]["type"] = json!("core/option@1<core/u64@1>");
    wire["variables"][0]["init"] =
        json!({"type":"core/option@1<core/u64@1>","value":{"none":true}});
    wire["nodes"] = json!([
        pick(wire, 1),
        source(2),
        core(3, "next-item"),
        set,
        wait(6, 100)
    ]);
    wire["edges"] = json!([
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 2, "stream", 3, "stream"),
        edge(4, 3, "then", 5, "in"),
        edge(5, 3, "item", 5, "value"),
        edge(6, 5, "then", 6, "in")
    ]);
}
fn collect_wire(wire: &mut Json, max: usize) {
    let mut collect = core(3, "collect");
    collect["config"] = json!({"max_items":max});
    let mut set = core(5, "set-variable");
    set["config"] = json!({"variable":id(100)});
    wire["variables"][0]["type"] = json!("core/list@1<core/u64@1>");
    wire["variables"][0]["init"] = json!({"type":"core/list@1<core/u64@1>","value":[]});
    wire["nodes"] = json!([pick(wire, 1), source(2), collect, set, wait(6, 100)]);
    wire["edges"] = json!([
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 2, "stream", 3, "stream"),
        edge(4, 3, "then", 5, "in"),
        edge(5, 3, "items", 5, "value"),
        edge(6, 5, "then", 6, "in")
    ]);
}

#[test]
fn iterator_pulls_only_after_the_waiting_body_retires_and_closes_at_end() {
    let mut run = setup(RunOptions::default(), |_| {});
    let probe = open(&mut run, 3, false);
    idle(&mut run);
    for at in [5, 10, 15] {
        assert_eq!(probe.reads.load(Ordering::Relaxed), at as usize / 5);
        assert!(probe.closes.lock().unwrap().is_empty());
        run.advance_to(at).unwrap();
        idle(&mut run);
    }
    assert_eq!(variable(&run), Some(&Value::U64(6)));
    assert_eq!(probe.reads.load(Ordering::Relaxed), 4);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    assert_eq!(run.ownership().resources, 0);
}
#[test]
fn collect_is_budgeted_and_publishes_one_complete_typed_list() {
    let mut run = setup(RunOptions::default(), |wire| collect_wire(wire, 3));
    let probe = open(&mut run, 3, false);
    for _ in 0..100 {
        let before = probe.reads.load(Ordering::Relaxed);
        let _ = run.pump(0).unwrap();
        assert_eq!(probe.reads.load(Ordering::Relaxed), before);
        let ready = run.pump(1).unwrap().ready;
        assert!(probe.reads.load(Ordering::Relaxed) <= before + 1);
        if !ready {
            break;
        }
    }
    let Some(Value::List(items)) = variable(&run) else {
        panic!()
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items.get(2), Some(Value::U64(3)));
    assert_eq!(probe.reads.load(Ordering::Relaxed), 4);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
}
#[test]
fn pending_next_is_not_polled_and_matching_completion_only_wakes_later_pump() {
    let mut run = setup(RunOptions::default(), next_wire);
    let probe = open(&mut run, 0, true);
    idle(&mut run);
    let demand = probe.demands.lock().unwrap()[0].clone();
    for _ in 0..20 {
        assert!(!run.pump(1).unwrap().ready);
    }
    assert_eq!(probe.reads.load(Ordering::Relaxed), 1);
    assert_eq!(run.ownership().stream_reads, 1);
    assert_eq!(
        run.complete_stream(demand.id, Ok(Some(&Value::Bool(true)))),
        Err(StreamError::Contract)
    );
    run.complete_stream(demand.id, Ok(Some(&Value::U64(7))))
        .unwrap();
    assert_eq!(variable(&run), Some(&Value::None));
    assert!(run.pump(0).unwrap().ready);
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::U64(7)));
    assert_eq!(
        run.complete_stream(demand.id, Ok(None)),
        Err(StreamError::Stale)
    );
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    assert!(demand.is_closed());
}

#[test]
fn next_reentry_preserves_one_consumer_and_end_remains_none() {
    let mut run = setup(RunOptions::default(), |wire| {
        next_wire(wire);
        let mut count = core(8, "count");
        count["inputs"] = json!({"count":{"type":"core/i64@1","value":"5"}});
        wire["nodes"].as_array_mut().unwrap().push(count);
        wire["edges"][1] = edge(2, 2, "then", 8, "in");
        wire["edges"][5] = edge(6, 8, "completed", 6, "in");
        wire["edges"]
            .as_array_mut()
            .unwrap()
            .push(edge(7, 8, "body", 3, "in"));
    });
    let probe = open(&mut run, 2, false);
    idle(&mut run);
    assert_eq!(probe.reads.load(Ordering::Relaxed), 3);
    assert_eq!(variable(&run), Some(&Value::None));
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.node.to_string() == id(3)
                && e.outcome == orange_runtime::ControlOutcome::Then)
            .count(),
        5
    );
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
}

#[test]
fn iterator_break_closes_without_an_extra_pull_and_continue_advances_after_body() {
    for terminate in ["break", "continue"] {
        let mut run = setup(RunOptions::default(), |wire| {
            wire["nodes"]
                .as_array_mut()
                .unwrap()
                .push(core(8, terminate));
            wire["edges"]
                .as_array_mut()
                .unwrap()
                .push(edge(8, 5, "then", 8, "in"));
        });
        let probe = open(&mut run, 3, false);
        idle(&mut run);
        run.advance_to(5).unwrap();
        idle(&mut run);
        if terminate == "break" {
            assert_eq!(variable(&run), Some(&Value::U64(1)));
            assert_eq!(probe.reads.load(Ordering::Relaxed), 1);
        } else {
            assert_eq!(probe.reads.load(Ordering::Relaxed), 2);
            run.advance_to(10).unwrap();
            idle(&mut run);
            run.advance_to(15).unwrap();
            idle(&mut run);
            assert_eq!(variable(&run), Some(&Value::U64(6)));
        }
        assert_eq!(probe.closes.lock().unwrap().len(), 1);
        assert_eq!(run.ownership().resources, 0);
    }
}

#[test]
fn race_cancels_a_waiting_iterator_and_keeps_its_item_available_to_cleanup() {
    let mut run = setup(RunOptions::default(), |wire| {
        let mut fork = core(8, "fork");
        fork["config"] = json!({"join":"race"});
        fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
        wire["nodes"]
            .as_array_mut()
            .unwrap()
            .extend([fork, wait(9, 1)]);
        wire["edges"][1] = edge(2, 2, "then", 8, "in");
        wire["edges"][4] = edge(5, 4, "cancelled", 5, "in");
        wire["edges"][6] = edge(7, 8, "completed", 6, "in");
        wire["edges"].as_array_mut().unwrap().extend([
            edge(8, 8, "branch#0", 3, "in"),
            edge(9, 8, "branch#1", 9, "in"),
        ]);
    });
    let probe = open(&mut run, 100, false);
    idle(&mut run);
    run.advance_to(1).unwrap();
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::U64(1)));
    assert_eq!(probe.reads.load(Ordering::Relaxed), 1);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    assert_eq!(run.ownership().resources, 0);
}

#[test]
fn stop_cancels_accepted_unpublished_pull_and_partial_collection_without_results() {
    for collect in [false, true] {
        let mut run = setup(RunOptions::default(), |wire| {
            if collect {
                collect_wire(wire, 4)
            } else {
                next_wire(wire)
            }
        });
        let probe = open(&mut run, 0, true);
        idle(&mut run);
        let first = probe.demands.lock().unwrap()[0].clone();
        run.complete_stream(first.id, Ok(Some(&Value::U64(7))))
            .unwrap();
        if collect {
            idle(&mut run);
            let second = probe.demands.lock().unwrap()[1].clone();
            run.complete_stream(second.id, Ok(Some(&Value::U64(8))))
                .unwrap();
        }
        run.stop().unwrap();
        idle(&mut run);
        assert_eq!(probe.closes.lock().unwrap().len(), 1);
        assert_eq!(run.ownership().stream_reads, 0);
        assert!(run.complete_stream(first.id, Ok(None)).is_err());
        if collect {
            assert!(matches!(variable(&run),Some(Value::List(items)) if items.is_empty()));
        } else {
            assert_eq!(variable(&run), Some(&Value::None));
        }
        run.dispose().unwrap();
        assert_eq!(run.retention().reserved_bytes, 0);
    }
}

#[test]
fn collection_bounds_fail_atomically_and_empty_zero_bound_succeeds() {
    for (length, max, success) in [(0, 0, true), (1, 0, false), (3, 2, false)] {
        let mut run = setup(RunOptions::default(), |wire| collect_wire(wire, max));
        let probe = open(&mut run, length, false);
        idle(&mut run);
        assert!(matches!(variable(&run),Some(Value::List(items)) if items.is_empty()));
        assert_eq!(probe.closes.lock().unwrap().len(), 1);
        assert_eq!(
            run.trace().iter().any(|e| e
                .fault
                .as_ref()
                .is_some_and(|f| f.fault.code == "core/LimitExceeded")),
            !success
        );
        assert_eq!(run.ownership().resources, 0);
    }
}

#[test]
fn refused_stream_results_preserve_producer_for_retry_and_forged_aliases_refuse() {
    let mut run = setup(RunOptions::default(), next_wire);
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let (producer, probe) = producer(1, false);
    let refused = run
        .complete_effect_stream(request.id, &"core/bool@1".parse().unwrap(), producer)
        .unwrap_err();
    assert_eq!(refused.error, EffectError::Contract);
    assert!(probe.closes.lock().unwrap().is_empty());
    let forged = Value::Stream(Arc::new(orange_model::value::HandleValue::new(
        ITEM.parse().unwrap(),
        orange_model::value::HandleId {
            index: 0,
            generation: 1,
            kind: 0,
        },
        Arc::new(()),
    )));
    assert_eq!(
        run.complete_effect(request.id, Ok(&[forged])),
        Err(EffectError::Contract)
    );
    run.complete_effect_stream(request.id, &ITEM.parse().unwrap(), refused.producer)
        .unwrap();
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::U64(1)));
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
}

#[test]
fn stream_close_is_idempotent_and_never_pulls() {
    let mut run = setup(RunOptions::default(), |wire| {
        let mut count = core(8, "count");
        count["inputs"] = json!({"count":{"type":"core/i64@1","value":"2"}});
        wire["nodes"] = json!([
            pick(wire, 1),
            source(2),
            core(3, "close-stream"),
            count,
            wait(6, 100)
        ]);
        wire["edges"] = json!([
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 8, "in"),
            edge(3, 8, "body", 3, "in"),
            edge(4, 2, "stream", 3, "stream"),
            edge(5, 8, "completed", 6, "in")
        ]);
    });
    let probe = open(&mut run, 2, false);
    idle(&mut run);
    assert_eq!(probe.reads.load(Ordering::Relaxed), 0);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.node.to_string() == id(3)
                && e.outcome == orange_runtime::ControlOutcome::Then)
            .count(),
        2
    );
}

fn call(n: u32, function: u32) -> Json {
    let mut node = core(n, "call");
    node["config"] = json!({"function":id(function)});
    node
}
fn port(name: &str, direction: &str, ty: &str) -> Json {
    json!({"port_id":name,"direction":direction,"kind":"data","type":ty})
}
#[test]
fn new_stream_return_transfers_ownership_and_unreturned_argument_closes_in_callee() {
    let mut returned = setup(RunOptions::default(), |wire| {
        wire["functions"] = json!([{"id":id(200),"name":"stream","kind":"async","inputs":[],"outputs":[port("stream","output","core/stream@1<core/u64@1>")],
            "nodes":[core(201,"function-entry"),source(202),core(203,"return")],
            "edges":[edge(201,201,"then",202,"in"),edge(202,202,"then",203,"in"),edge(203,202,"stream",203,"stream")]}]);
        wire["nodes"][1] = call(2, 200);
    });
    let probe = open(&mut returned, 1, false);
    idle(&mut returned);
    assert!(probe.closes.lock().unwrap().is_empty());
    returned.advance_to(5).unwrap();
    idle(&mut returned);
    assert_eq!(variable(&returned), Some(&Value::U64(1)));
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    let mut argument = setup(RunOptions::default(), |wire| {
        wire["functions"] = json!([{"id":id(200),"name":"consume","kind":"async","inputs":[port("incoming","input","core/stream@1<core/u64@1>")],"outputs":[],
            "nodes":[core(201,"function-entry"),wait(204,1),core(203,"return")],
            "edges":[edge(201,201,"then",204,"in"),edge(202,204,"then",203,"in")]}]);
        wire["nodes"] = json!([pick(wire, 1), source(2), call(3, 200), wait(6, 100)]);
        wire["edges"] = json!([
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "stream", 3, "incoming"),
            edge(4, 3, "then", 6, "in")
        ]);
    });
    let probe = open(&mut argument, 100, false);
    idle(&mut argument);
    argument.advance_to(1).unwrap();
    idle(&mut argument);
    assert_eq!(probe.reads.load(Ordering::Relaxed), 0);
    assert_eq!(*probe.closes.lock().unwrap(), [CloseReason::ScopeEnded]);
    assert_eq!(argument.ownership().resources, 0);
}

#[test]
fn return_from_iterator_body_closes_the_unfinished_producer_before_caller_resumes() {
    let mut run = setup(RunOptions::default(), |wire| {
        let mut fallthrough = core(206, "return");
        fallthrough["inputs"] = json!({"result":{"type":ITEM,"value":"999"}});
        wire["functions"] = json!([{"id":id(200),"name":"first","kind":"async","inputs":[],"outputs":[port("result","output",ITEM)],
            "nodes":[core(201,"function-entry"),source(202),core(204,"for-each-item"),wait(205,5),core(203,"return"),fallthrough],
            "edges":[edge(201,201,"then",202,"in"),edge(202,202,"then",204,"in"),edge(203,202,"stream",204,"stream"),edge(204,204,"body",205,"in"),
                edge(205,205,"then",203,"in"),edge(206,204,"item",203,"result"),edge(207,204,"completed",206,"in")]}]);
        let mut set = core(5, "set-variable");
        set["config"] = json!({"variable":id(100)});
        wire["nodes"] = json!([pick(wire, 1), call(2, 200), set, wait(6, 100)]);
        wire["edges"] = json!([
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 5, "in"),
            edge(3, 2, "result", 5, "value"),
            edge(4, 5, "then", 6, "in")
        ]);
    });
    let probe = open(&mut run, 100, false);
    idle(&mut run);
    run.advance_to(5).unwrap();
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::U64(1)));
    assert_eq!(probe.reads.load(Ordering::Relaxed), 1);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    assert_eq!(run.ownership().stream_reads, 0);
}

#[test]
fn host_revocation_discards_an_accepted_item_before_graph_publication() {
    let mut run = setup(RunOptions::default(), next_wire);
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let (producer, probe) = producer(0, true);
    let handle = run.open_stream(&ITEM.parse().unwrap(), producer).unwrap();
    let value = run.stream_value(handle).unwrap();
    let resource = run.resource_handle(&value, &ITEM.parse().unwrap()).unwrap();
    let Value::Stream(reference) = &value else {
        panic!()
    };
    let relabeled = Value::Handle(reference.clone());
    assert!(
        run.resource_handle(&relabeled, &ITEM.parse().unwrap())
            .is_err()
    );
    run.complete_effect(request.id, Ok(std::slice::from_ref(&value)))
        .unwrap();
    idle(&mut run);
    let demand = probe.demands.lock().unwrap()[0].clone();
    run.complete_stream(demand.id, Ok(Some(&Value::U64(7))))
        .unwrap();
    run.close_resource(resource).unwrap();
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::None));
    assert_eq!(run.ownership().stream_reads, 0);
    assert!(run.trace().iter().any(|e| {
        e.fault
            .as_ref()
            .is_some_and(|f| f.fault.code == "core/StaleHandle")
    }));
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    assert!(demand.is_closed());
}

#[test]
fn resource_result_quota_refusals_return_the_original_unclosed_producer() {
    for resource_limit in [false, true] {
        let mut options = RunOptions::default();
        if resource_limit {
            options.limits.max_resources = 0;
        } else {
            options.limits.max_effect_bytes = 0;
        }
        let mut run = setup(options, next_wire);
        idle(&mut run);
        let request = run.take_effect().unwrap();
        let (producer, probe) = producer(1, false);
        let before = run.retention().reserved_bytes;
        let refused = run
            .complete_effect_stream(request.id, &ITEM.parse().unwrap(), producer)
            .unwrap_err();
        assert_eq!(refused.error, EffectError::Limit);
        assert_eq!(run.retention().reserved_bytes, before);
        assert_eq!(run.ownership().resources, 0);
        assert_eq!(run.ownership().effects, 1);
        assert!(probe.closes.lock().unwrap().is_empty());
        refused.producer.close(CloseReason::Closed);
        run.stop().unwrap();
        idle(&mut run);
        assert_eq!(probe.closes.lock().unwrap().len(), 1);
    }
}

#[test]
fn exact_collection_byte_bound_succeeds_and_one_under_discards_partial_values() {
    for bytes in [64, 63] {
        let mut options = RunOptions::default();
        options.limits.max_stream_collect_bytes = bytes;
        let mut run = setup(options, |wire| collect_wire(wire, 3));
        let probe = open(&mut run, 3, false);
        idle(&mut run);
        assert!(
            matches!(variable(&run),Some(Value::List(items)) if items.len()==if bytes==64{3}else{0})
        );
        assert_eq!(probe.closes.lock().unwrap().len(), 1);
        assert_eq!(run.ownership().resources, 0);
    }
}

struct Scripted {
    steps: std::collections::VecDeque<Result<StreamStep, KernelFault>>,
    probe: Probe,
}
impl PullStream for Scripted {
    fn next(&mut self, demand: StreamDemand) -> Result<StreamStep, KernelFault> {
        self.probe.reads.fetch_add(1, Ordering::Relaxed);
        self.probe.demands.lock().unwrap().push(demand);
        self.steps.pop_front().unwrap_or(Ok(StreamStep::End))
    }
    fn close(self: Box<Self>, reason: CloseReason) {
        self.probe.closes.lock().unwrap().push(reason);
    }
}
#[test]
fn next_preserves_some_none_and_producer_faults_or_bad_items_never_publish_success() {
    let item = "core/option@1<core/u64@1>";
    let mut run = setup_item(item, RunOptions::default(), |wire| {
        next_wire(wire);
        wire["variables"][0]["type"] = json!("core/option@1<core/option@1<core/u64@1>>");
        wire["variables"][0]["init"] =
            json!({"type":"core/option@1<core/option@1<core/u64@1>>","value":{"none":true}});
    });
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let probe = Probe::default();
    run.complete_effect_stream(
        request.id,
        &item.parse().unwrap(),
        Box::new(Scripted {
            steps: [Ok(StreamStep::Item(Value::None))].into(),
            probe: probe.clone(),
        }),
    )
    .unwrap();
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::Some(Arc::new(Value::None))));
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    const FAULT: KernelFault = KernelFault {
        code: "acme/ReadFailed",
        message_key: "acme/read-failed",
        message: "The read failed.",
    };
    for bad_item in [false, true] {
        let mut run = setup(RunOptions::default(), next_wire);
        idle(&mut run);
        let request = run.take_effect().unwrap();
        let probe = Probe::default();
        let step = if bad_item {
            Ok(StreamStep::Item(Value::Bool(true)))
        } else {
            Err(FAULT)
        };
        run.complete_effect_stream(
            request.id,
            &ITEM.parse().unwrap(),
            Box::new(Scripted {
                steps: [step].into(),
                probe: probe.clone(),
            }),
        )
        .unwrap();
        idle(&mut run);
        assert_eq!(variable(&run), Some(&Value::None));
        assert!(
            run.trace()
                .iter()
                .any(|e| e.fault.as_ref().is_some_and(|f| f.fault.code
                    == if bad_item {
                        "core/InternalFault"
                    } else {
                        FAULT.code
                    }))
        );
        assert_eq!(probe.closes.lock().unwrap().len(), 1);
    }
}

#[test]
fn producer_panic_closes_once_and_the_interrupted_run_cannot_resume_the_pull() {
    struct Panics(Probe);
    impl PullStream for Panics {
        fn next(&mut self, _: StreamDemand) -> Result<StreamStep, KernelFault> {
            self.0.reads.fetch_add(1, Ordering::Relaxed);
            panic!("producer failed")
        }
        fn close(self: Box<Self>, reason: CloseReason) {
            self.0.closes.lock().unwrap().push(reason);
        }
    }
    let mut run = setup(RunOptions::default(), next_wire);
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let probe = Probe::default();
    run.complete_effect_stream(
        request.id,
        &ITEM.parse().unwrap(),
        Box::new(Panics(probe.clone())),
    )
    .unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| idle(&mut run))).is_err());
    assert!(matches!(
        run.pump(1),
        Err(orange_runtime::RunError::Interrupted)
    ));
    run.stop().unwrap();
    idle(&mut run);
    run.dispose().unwrap();
    assert_eq!(probe.reads.load(Ordering::Relaxed), 1);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    assert_eq!(run.retention().reserved_bytes, 0);
}

#[test]
fn iterator_waits_for_detached_body_descendants_before_demanding_another_item() {
    let mut run = setup(RunOptions::default(), |wire| {
        let mut fork = core(8, "fork");
        fork["config"] = json!({"join":"detach"});
        fork["variadic"] = json!({"branch":{"next":1,"order":[0]}});
        wire["nodes"].as_array_mut().unwrap().push(fork);
        wire["edges"][3] = edge(4, 3, "body", 8, "in");
        wire["edges"][4] = edge(5, 8, "completed", 5, "in");
        wire["edges"]
            .as_array_mut()
            .unwrap()
            .push(edge(8, 8, "branch#0", 4, "in"));
    });
    let probe = open(&mut run, 2, false);
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::U64(1)));
    assert_eq!(probe.reads.load(Ordering::Relaxed), 1);
    run.advance_to(5).unwrap();
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::U64(3)));
    assert_eq!(probe.reads.load(Ordering::Relaxed), 2);
    assert!(probe.closes.lock().unwrap().is_empty());
    run.advance_to(10).unwrap();
    idle(&mut run);
    assert_eq!(probe.reads.load(Ordering::Relaxed), 3);
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
}

#[test]
fn compiled_tee_keeps_fast_and_slow_iterators_bounded_and_one_can_break_early() {
    for break_left in [false, true] {
        let mut run = setup(RunOptions::default(), |wire| {
            let mut tee = core(7, "tee");
            tee["config"] = json!({"capacity":1,"max_bytes":16,"overflow":"backpressure"});
            let mut fork = core(8, "fork");
            fork["config"] = json!({"join":"all"});
            fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
            let mut left = pick(wire, 5);
            left["config"]["int_overflow"] = json!("wrapping");
            let mut right = left.clone();
            right["id"] = json!(id(10));
            wire["nodes"] = json!([
                pick(wire, 1),
                source(2),
                tee,
                fork,
                core(3, "for-each-item"),
                wait(4, 1),
                left,
                core(9, "for-each-item"),
                wait(11, 10),
                right,
                wait(6, 100)
            ]);
            let mut edges = vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "then", 7, "in"),
                edge(3, 2, "stream", 7, "stream"),
                edge(4, 7, "then", 8, "in"),
                edge(5, 7, "left", 3, "stream"),
                edge(6, 7, "right", 9, "stream"),
                edge(7, 8, "branch#0", 3, "in"),
                edge(8, 8, "branch#1", 9, "in"),
                edge(9, 3, "body", 4, "in"),
                edge(10, 4, "then", 5, "in"),
                edge(11, 3, "item", 5, "value"),
                edge(12, 9, "body", 11, "in"),
                edge(13, 11, "then", 10, "in"),
                edge(14, 9, "item", 10, "value"),
                edge(15, 8, "completed", 6, "in"),
            ];
            if break_left {
                wire["nodes"]
                    .as_array_mut()
                    .unwrap()
                    .push(core(12, "break"));
                edges.push(edge(16, 5, "then", 12, "in"));
            }
            wire["edges"] = json!(edges);
        });
        let probe = open(&mut run, 3, false);
        idle(&mut run);
        for _ in 0..100 {
            if run.now() >= 30 {
                break;
            }
            let next = run.pump(0).unwrap().next_wakeup.expect("waiting body");
            run.advance_to(next.min(30)).unwrap();
            idle(&mut run);
            for stats in run.stream_bridges() {
                for branch in stats.branches.into_iter().flatten() {
                    assert!(branch.queued_items <= 1 && branch.queued_bytes <= 16);
                }
            }
        }
        assert_eq!(
            variable(&run),
            Some(&Value::U64(if break_left { 7 } else { 12 }))
        );
        assert_eq!(probe.reads.load(Ordering::Relaxed), 4);
        assert_eq!(probe.closes.lock().unwrap().len(), 1);
        assert_eq!(run.stream_bridges().count(), 0);
        assert_eq!(run.ownership().resources, 0);
        run.stop().unwrap();
        idle(&mut run);
        run.dispose().unwrap();
        assert_eq!(run.retention().reserved_bytes, 0);
    }
}
#[test]
fn a_returned_tee_branch_keeps_upstream_alive_after_the_other_branchs_scope_ends() {
    let mut run = setup(RunOptions::default(), |wire| {
        let mut tee = core(204, "tee");
        tee["config"] = json!({"capacity":1,"max_bytes":16,"overflow":"backpressure"});
        wire["functions"] = json!([{"id":id(200),"name":"one-branch","kind":"async","inputs":[],"outputs":[port("stream","output","core/stream@1<core/u64@1>")],
            "nodes":[core(201,"function-entry"),source(202),tee,core(203,"return")],
            "edges":[edge(201,201,"then",202,"in"),edge(202,202,"then",204,"in"),edge(203,202,"stream",204,"stream"),edge(204,204,"then",203,"in"),edge(205,204,"left",203,"stream")]}]);
        wire["nodes"][1] = call(2, 200);
    });
    let probe = open(&mut run, 3, false);
    idle(&mut run);
    let stats = run.stream_bridges().next().unwrap();
    assert!(stats.branches[1].unwrap().closed);
    assert!(!stats.branches[0].unwrap().closed);
    assert!(probe.closes.lock().unwrap().is_empty());
    for at in [5, 10, 15] {
        run.advance_to(at).unwrap();
        idle(&mut run);
    }
    assert_eq!(variable(&run), Some(&Value::U64(6)));
    assert_eq!(probe.closes.lock().unwrap().len(), 1);
    assert_eq!(run.ownership().resources, 0);
}
#[test]
fn pushed_items_wake_a_graph_iterator_without_running_graph_work_inline() {
    let mut run = setup(RunOptions::default(), |_| {});
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let push = run
        .open_push_stream(
            &ITEM.parse().unwrap(),
            orange_runtime::StreamBufferOptions {
                capacity: 1,
                max_bytes: 16,
                overflow: orange_runtime::StreamOverflow::Backpressure,
            },
        )
        .unwrap();
    let value = run.stream_value(push.stream).unwrap();
    run.complete_effect(request.id, Ok(&[value])).unwrap();
    idle(&mut run);
    assert!(!run.pump(0).unwrap().ready);
    run.push_stream(&push.source, 1, &Value::U64(7)).unwrap();
    assert_eq!(variable(&run), Some(&Value::U64(0)));
    assert!(run.pump(0).unwrap().ready);
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::U64(0)));
    run.advance_to(5).unwrap();
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::U64(7)));
    run.request_stop(orange_runtime::StopMode::Graceful)
        .unwrap();
    assert_eq!(
        run.push_stream(&push.source, 2, &Value::U64(8)),
        Err(StreamError::NotRunning)
    );
    run.finish_push_stream(&push.source).unwrap();
    idle(&mut run);
    assert!(push.source.is_closed());
    assert_eq!(run.stream_bridges().count(), 0);
    run.stop().unwrap();
    idle(&mut run);
    run.dispose().unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);
}
#[test]
fn push_overflow_fault_wakes_a_pending_graph_read_even_at_the_item_byte_limit() {
    let mut options = RunOptions::default();
    options.limits.max_stream_item_bytes = 16;
    let mut run = setup(options, next_wire);
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let push = run
        .open_push_stream(
            &ITEM.parse().unwrap(),
            orange_runtime::StreamBufferOptions {
                capacity: 1,
                max_bytes: 16,
                overflow: orange_runtime::StreamOverflow::Fault,
            },
        )
        .unwrap();
    let value = run.stream_value(push.stream).unwrap();
    run.complete_effect(request.id, Ok(&[value])).unwrap();
    idle(&mut run);
    run.push_stream(&push.source, 1, &Value::U64(1)).unwrap();
    assert_eq!(
        run.push_stream(&push.source, 2, &Value::U64(2)),
        Err(StreamError::Overflow)
    );
    idle(&mut run);
    assert_eq!(variable(&run), Some(&Value::None));
    assert!(push.source.is_closed());
    assert!(run.trace().iter().any(|e| {
        e.fault
            .as_ref()
            .is_some_and(|f| f.fault.code == "core/QueueOverflow")
    }));
    assert_eq!(run.ownership().resources, 0);
    assert_eq!(run.stream_bridges().count(), 0);
}
