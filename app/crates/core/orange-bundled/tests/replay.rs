//! Reference-only recording and effect-safe replay through real compiled graphs.
use crate::data_graph::{catalog, edge, graph, id};
use orange_compiler::{
    ActionKernel, BindingRequest, CompileOptions, ControlPlan, EffectBinding, EffectResolver,
    KernelFault, ResolveError, compile_control_with_effects,
};
use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder, RegistrySnapshot,
};
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use orange_runtime::{
    EffectError, IngressError, RecordedRun, ReplayArchive, ReplayArchiveError, ReplayError,
    ReplayOptions, ReplayPayloadRef, RunError, RunOptions, RunState, StopMode,
};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
const EVENT: &str = "40000000-0000-4000-8000-000000000001";
const FAULT: KernelFault = KernelFault {
    code: orange_compiler::FaultText::Static("acme.effects/Refused"),
    message_key: orange_compiler::FaultText::Static("acme.effects/refused"),
    message: "The operation was refused.",
};
#[derive(Default)]
struct Archive {
    values: Mutex<BTreeMap<u64, Arc<[Value]>>>,
    missing: AtomicBool,
    wrong: AtomicBool,
    exclude: AtomicBool,
    panic_resolve: AtomicBool,
}
impl ReplayArchive for Archive {
    fn capture(&self, values: &[Value]) -> Result<ReplayPayloadRef, ReplayArchiveError> {
        if self.exclude.load(Ordering::Relaxed) {
            return Err(ReplayArchiveError::Excluded);
        }
        let mut stored = self.values.lock().unwrap();
        let key = stored.len() as u64 + 1;
        stored.insert(key, values.into());
        Ok(ReplayPayloadRef { namespace: 7, key })
    }
    fn resolve(&self, reference: ReplayPayloadRef) -> Result<Arc<[Value]>, ReplayArchiveError> {
        assert!(!self.panic_resolve.load(Ordering::Relaxed), "archive panic");
        if reference.namespace != 7 || self.missing.load(Ordering::Relaxed) {
            return Err(ReplayArchiveError::Missing);
        }
        if self.wrong.load(Ordering::Relaxed) {
            return Ok(Arc::from([Value::Bool(true)]));
        }
        self.values
            .lock()
            .unwrap()
            .get(&reference.key)
            .cloned()
            .ok_or(ReplayArchiveError::Missing)
    }
}
struct Adapter {
    count: Arc<AtomicUsize>,
    fault: bool,
    panic: bool,
    action: bool,
}
#[derive(Debug)]
struct Increment {
    count: Arc<AtomicUsize>,
    fault: bool,
    panic: bool,
}
impl ActionKernel for Increment {
    fn execute(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        self.count.fetch_add(1, Ordering::Relaxed);
        assert!(!self.panic, "provider panic");
        if self.fault {
            return Err(FAULT);
        }
        let Value::U64(n) = inputs[0] else { panic!() };
        outputs[0] = Value::U64(n + 1);
        Ok(())
    }
}
impl EffectResolver for Adapter {
    fn resolve_effect(&self, _: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
        Ok(EffectBinding {
            route: 77,
            context: Arc::new(()),
            inputs: vec!["value".parse().unwrap()].into(),
            outputs: vec!["result".parse().unwrap()].into(),
            action: self.action.then(|| {
                Arc::new(Increment {
                    count: self.count.clone(),
                    fault: self.fault,
                    panic: self.panic,
                }) as Arc<dyn ActionKernel>
            }),
        })
    }
}
fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}});
    if name.starts_with("on-") {
        node["activation"] =
            json!({"mode":"queue","capacity":4,"overflow":"fault","on_fault":"continue","order":0});
    }
    node
}
fn fixture(
    adapter: &Adapter,
    edit: impl FnOnce(&mut Json),
) -> (GraphDocument, Arc<RegistrySnapshot>, Arc<ControlPlan>) {
    let (package, base) = catalog();
    let definition=NodeDefinition::from_json(&json!({"definition_id":"acme.effects/increment","definition_version":1,"config_version":1,
        "kind":if adapter.action {"action"}else{"async"},"suspends":if adapter.action {"never"}else{"always"},"determinism":"external",
        "ports":[{"port_id":"value","direction":"input","kind":"data","type":"core/u64@1"},{"port_id":"result","direction":"output","kind":"data","type":"core/u64@1","source":"execution","available_on":["then"]}]}).to_string()).unwrap();
    let key = ProviderKey {
        package: "acme.effects".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "b".repeat(64)),
        id: "inert".into(),
    };
    let provider = ProviderDescriptor {
        key: key.clone(),
        implements: [(definition.definition.clone(), 1)].into(),
        specializations: Default::default(),
    };
    let Ports::Declared(ports) = &definition.ports else {
        unreachable!()
    };
    let mut signature =
        orange_model::signature::project(definition.kind, ports.clone(), &Default::default());
    signature.suspends = definition.suspends;
    let snapshot = RegistryBuilder::from_snapshot(&base)
        .definition(definition)
        .provider(provider)
        .build()
        .unwrap();
    let mut set = core(3, "set-variable");
    set["config"] = json!({"variable":id(100)});
    let effect = json!({"id":id(2),"definition_id":"acme.effects/increment","definition_version":1,"config_version":1,"config":{},"signature":"effect","provider":{"package":key.package,"version":key.version,"id":key.id},"inputs":{"value":{"type":"core/u64@1","value":"41"}}});
    let doc = graph(
        &package,
        vec![core(1, "on-start"), effect, set],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "result", 3, "value"),
        ],
        vec![],
    );
    let mut wire: Json = serde_json::from_str(&to_json(&doc).unwrap()).unwrap();
    wire["package_lock"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":key.package,"version":key.version,"content_digest":key.content_digest}));
    wire["signatures"]["effect"] = serde_json::to_value(signature).unwrap();
    wire["variables"] = json!([{"id":id(100),"name":"result","type":"core/u64@1","scope":"run"}]);
    edit(&mut wire);
    let doc = from_json(&wire.to_string()).unwrap();
    let analysis = analyze(&doc, &snapshot);
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
        adapter,
        CompileOptions::default(),
    )
    .unwrap();
    (doc, snapshot, Arc::new(plan))
}
fn adapter(action: bool) -> Adapter {
    Adapter {
        action,
        count: Arc::new(AtomicUsize::new(0)),
        fault: false,
        panic: false,
    }
}
fn start(
    doc: &GraphDocument,
    snapshot: &RegistrySnapshot,
    plan: Arc<ControlPlan>,
    archive: Arc<Archive>,
    limits: ReplayOptions,
) -> RecordedRun {
    RecordedRun::start(
        plan,
        doc,
        snapshot,
        123,
        RunOptions::default(),
        limits,
        archive,
    )
    .unwrap()
}
fn idle(run: &mut RecordedRun, budget: usize) {
    for _ in 0..3000 {
        if run.inspect().state() == RunState::Stopped || !run.pump(budget).unwrap().ready {
            return;
        }
    }
    panic!("did not idle")
}
fn result(run: &orange_runtime::Run) -> Option<&Value> {
    run.variable(id(100).parse().unwrap())
}

#[test]
fn actions_replay_known_results_and_faults_without_resending_callbacks() {
    for fault in [false, true] {
        for budget in [1, 7, 1000] {
            let mut adapter = adapter(true);
            adapter.fault = fault;
            let (doc, snapshot, plan) = fixture(&adapter, |_| {});
            let archive = Arc::new(Archive::default());
            let mut run = start(
                &doc,
                &snapshot,
                plan,
                archive.clone(),
                ReplayOptions::default(),
            );
            idle(&mut run, budget);
            assert_eq!(adapter.count.load(Ordering::Relaxed), 1);
            assert_eq!(
                result(run.inspect()),
                if fault { None } else { Some(&Value::U64(42)) }
            );
            run.request_stop(StopMode::Immediate).unwrap();
            idle(&mut run, budget);
            let (original, log) = run.finish().unwrap();
            for _ in 0..2 {
                let replay = log.replay(&doc, &snapshot, archive.clone()).unwrap();
                assert_eq!(result(replay.inspect()), result(&original));
                assert_eq!(replay.inspect().trace(), original.trace());
                assert_eq!(replay.inspect().transitions(), original.transitions());
            }
            assert_eq!(adapter.count.load(Ordering::Relaxed), 1);
        }
    }
}
#[test]
fn async_completion_refusal_and_complete_once_order_replay_without_a_host_executor() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |_| {});
    let archive = Arc::new(Archive::default());
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions::default(),
    );
    idle(&mut run, 1);
    let request = run.take_effect().unwrap().unwrap();
    assert!(run.take_effect().unwrap().is_none());
    assert_eq!(
        run.complete_effect(request.id, Ok(&[Value::Bool(true)])),
        Err(ReplayError::Effect(EffectError::Contract))
    );
    run.complete_effect(request.id, Ok(&[Value::U64(42)]))
        .unwrap();
    assert_eq!(result(run.inspect()), None);
    assert_eq!(
        run.complete_effect(request.id, Ok(&[Value::U64(99)])),
        Err(ReplayError::Effect(EffectError::Stale))
    );
    idle(&mut run, 1);
    run.request_stop(StopMode::Immediate).unwrap();
    idle(&mut run, 1);
    let (original, log) = run.finish().unwrap();
    let replay = log.replay(&doc, &snapshot, archive).unwrap();
    assert_eq!(result(replay.inspect()), Some(&Value::U64(42)));
    assert_eq!(replay.inspect().trace(), original.trace());
    assert!(request.lease.is_finished());
    assert_eq!(adapter.count.load(Ordering::Relaxed), 0);
}

#[test]
fn shared_fault_text_is_budgeted_replayed_and_released_with_its_last_owner() {
    for tight in [false, true] {
        let adapter = adapter(false);
        let (doc, snapshot, plan) = fixture(&adapter, |_| {});
        let archive = Arc::new(Archive::default());
        let mut limits = ReplayOptions {
            max_entries: 128,
            max_bytes: 1024 * 1024,
        };
        let probe = start(&doc, &snapshot, plan.clone(), archive.clone(), limits);
        let fixed = probe.stats().reserved_bytes;
        drop(probe);
        if tight {
            limits.max_bytes = fixed;
        }
        let mut run = start(&doc, &snapshot, plan, archive.clone(), limits);
        idle(&mut run, 1);
        let request = run.take_effect().unwrap().unwrap();
        let code: Arc<str> = Arc::from("acme.effects/OwnedFault");
        let weak = Arc::downgrade(&code);
        let fault = KernelFault {
            code: orange_compiler::FaultText::Shared(code),
            message_key: String::from("acme.effects/owned-fault").into(),
            message: "The operation failed.",
        };
        let text_bytes = fault.code.retained_bytes() + fault.message_key.retained_bytes();
        let result = run.complete_effect(request.id, Err(fault));
        if tight {
            assert_eq!(result, Err(ReplayError::Full));
            assert!(run.stats().incomplete);
            assert_eq!(run.stats().reserved_bytes, fixed);
            assert_eq!(run.finish().unwrap_err(), ReplayError::Incomplete);
        } else {
            result.unwrap();
            assert_eq!(run.stats().reserved_bytes, fixed + text_bytes);
            idle(&mut run, 1);
            run.request_stop(StopMode::Immediate).unwrap();
            idle(&mut run, 1);
            let (original, log) = run.finish().unwrap();
            let playback = log.replay(&doc, &snapshot, archive).unwrap();
            assert_eq!(playback.inspect().trace(), original.trace());
            assert!(original.trace().iter().any(|entry| {
                entry
                    .fault
                    .as_ref()
                    .is_some_and(|fault| fault.fault.code == "acme.effects/OwnedFault")
            }));
            drop(original);
            drop(playback);
            assert!(weak.upgrade().is_some(), "the log owns its saved fault");
            drop(log);
        }
        assert!(weak.upgrade().is_none());
    }
}
#[test]
fn ic1_manual_timer_emit_await_watch_and_effect_replay_have_known_answers() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |wire| {
        wire["nodes"][0]["definition_id"] = json!("core/on-manual");
        wire["events"] = json!([{"id":EVENT,"name":"pulse","payload":"core/u64@1","scope":"run"}]);
        wire["variables"].as_array_mut().unwrap().extend([
            json!({"id":id(101),"name":"watched","type":"core/u64@1","scope":"run","init":{"type":"core/u64@1","value":"0"}}),
            json!({"id":id(102),"name":"at","type":"core/duration@1","scope":"run"})]);
        let mut fork = core(4, "fork");
        fork["config"] = json!({"join":"all"});
        fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
        let mut awaiter = core(5, "await-event");
        awaiter["config"] = json!({"event":EVENT});
        let mut set = core(6, "set-variable");
        set["config"] = json!({"variable":id(101)});
        let mut emit = core(7, "emit");
        emit["config"] = json!({"event":EVENT});
        emit["inputs"] = json!({"payload":{"type":"core/u64@1","value":"41"}});
        let mut watch = core(8, "on-watch");
        watch["config"] = json!({"mode":"changed"});
        watch["type_pins"] = json!({"T":"core/u64@1"});
        let mut get = core(9, "get-variable");
        get["config"] = json!({"variable":id(101)});
        let mut timer = core(10, "on-timer");
        timer["config"] = json!({"interval":"10"});
        let mut at = core(11, "set-variable");
        at["config"] = json!({"variable":id(102)});
        wire["nodes"]
            .as_array_mut()
            .unwrap()
            .extend([fork, awaiter, set, emit, watch, get, timer, at]);
        wire["edges"] = json!([
            edge(1, 1, "then", 4, "in"),
            edge(2, 4, "branch#0", 5, "in"),
            edge(3, 5, "then", 6, "in"),
            edge(4, 5, "payload", 6, "value"),
            edge(5, 4, "branch#1", 7, "in"),
            edge(6, 9, "value", 8, "watch"),
            edge(7, 8, "then", 2, "in"),
            edge(8, 8, "value", 2, "value"),
            edge(9, 2, "then", 3, "in"),
            edge(10, 2, "result", 3, "value"),
            edge(11, 10, "then", 11, "in"),
            edge(12, 10, "scheduled_at", 11, "value")
        ]);
    });
    for budget in [1, 7, 1000] {
        let archive = Arc::new(Archive::default());
        let mut run = start(
            &doc,
            &snapshot,
            plan.clone(),
            archive.clone(),
            ReplayOptions::default(),
        );
        idle(&mut run, budget);
        run.trigger(id(1).parse().unwrap(), None, Some(100))
            .unwrap();
        idle(&mut run, budget);
        let request = run.take_effect().unwrap().unwrap();
        assert_eq!(&*request.inputs, &[Value::U64(41)]);
        run.complete_effect(request.id, Ok(&[Value::U64(42)]))
            .unwrap();
        run.advance_to(25).unwrap();
        idle(&mut run, budget);
        assert_eq!(result(run.inspect()), Some(&Value::U64(42)));
        assert_eq!(
            run.inspect().variable(id(102).parse().unwrap()),
            Some(&Value::Duration(20))
        );
        run.request_stop(StopMode::Graceful).unwrap();
        idle(&mut run, budget);
        let (original, log) = run.finish().unwrap();
        let replay = log.replay(&doc, &snapshot, archive).unwrap();
        assert_eq!(replay.inspect().trace(), original.trace());
        assert_eq!(replay.inspect().event_stats(), original.event_stats());
        assert_eq!(replay.inspect().last_event(), original.last_event());
        assert_eq!(
            replay.inspect().retention().reserved_bytes,
            original.retention().reserved_bytes
        );
        assert_eq!(result(replay.inspect()), Some(&Value::U64(42)));
    }
}
#[test]
fn host_sources_refused_ingress_and_revocation_preserve_recorded_order() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |wire| {
        wire["nodes"][0]["definition_id"] = json!("core/on-event");
        wire["nodes"][0]["config"] = json!({"event":EVENT});
        wire["events"] = json!([{"id":EVENT,"name":"pulse","payload":"core/u64@1","scope":"run"}]);
    });
    let archive = Arc::new(Archive::default());
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions::default(),
    );
    let token = run.register_event_source(EVENT.parse().unwrap()).unwrap();
    run.ingest_event(&token, 1, &Value::U64(4), Some(55))
        .unwrap();
    assert_eq!(
        run.ingest_event(&token, 1, &Value::U64(5), None),
        Err(ReplayError::Ingress(IngressError::SequenceNotIncreasing))
    );
    assert_eq!(
        run.ingest_event(&token, 2, &Value::Bool(true), None),
        Err(ReplayError::Ingress(IngressError::InvalidPayload))
    );
    assert!(run.revoke_event_source(&token).unwrap());
    assert!(!run.revoke_event_source(&token).unwrap());
    assert_eq!(
        run.ingest_event(&token, 2, &Value::U64(8), None),
        Err(ReplayError::Ingress(IngressError::RevokedSource))
    );
    idle(&mut run, 1);
    let request = run.take_effect().unwrap().unwrap();
    run.complete_effect(request.id, Err(FAULT)).unwrap();
    idle(&mut run, 1);
    run.request_stop(StopMode::Immediate).unwrap();
    idle(&mut run, 1);
    let (original, log) = run.finish().unwrap();
    let replay = log.replay(&doc, &snapshot, archive).unwrap();
    assert_eq!(replay.inspect().trace(), original.trace());
    assert_eq!(replay.inspect().event_stats(), original.event_stats());
}
#[test]
fn full_journal_refuses_host_mutation_and_can_seal_its_complete_prefix() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |_| {});
    let archive = Arc::new(Archive::default());
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions {
            max_entries: 1,
            max_bytes: 4096,
        },
    );
    run.advance_to(10).unwrap();
    assert_eq!(run.advance_to(20), Err(ReplayError::Full));
    assert_eq!(run.inspect().now(), 10);
    assert_eq!(run.stats().full_refusals, 1);
    assert!(!run.stats().incomplete);
    assert!(run.stats().reserved_bytes <= 4096);
    let (_, log) = run.finish().unwrap();
    let replay = log.replay(&doc, &snapshot, archive).unwrap();
    assert_eq!(replay.inspect().now(), 10);
}
#[test]
fn full_inside_pump_never_invokes_action_and_refuses_incomplete_replay() {
    let adapter = adapter(true);
    let (doc, snapshot, plan) = fixture(&adapter, |_| {});
    let archive = Arc::new(Archive::default());
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive,
        ReplayOptions {
            max_entries: 1,
            max_bytes: 4096,
        },
    );
    assert_eq!(
        run.pump(1000),
        Err(ReplayError::Run(RunError::ReplayFailed))
    );
    assert_eq!(adapter.count.load(Ordering::Relaxed), 0);
    assert!(run.stats().incomplete);
    assert_eq!(run.finish().unwrap_err(), ReplayError::Incomplete);
}
#[test]
fn missing_or_wrong_typed_archive_results_fail_closed_and_never_execute_actions() {
    let adapter = adapter(true);
    let (doc, snapshot, plan) = fixture(&adapter, |_| {});
    let archive = Arc::new(Archive::default());
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions::default(),
    );
    idle(&mut run, 1000);
    let (_, log) = run.finish().unwrap();
    archive.missing.store(true, Ordering::Relaxed);
    assert!(log.replay(&doc, &snapshot, archive.clone()).is_err());
    archive.missing.store(false, Ordering::Relaxed);
    archive.wrong.store(true, Ordering::Relaxed);
    assert!(log.replay(&doc, &snapshot, archive).is_err());
    assert_eq!(adapter.count.load(Ordering::Relaxed), 1);
}
#[test]
fn excluded_payloads_and_panics_never_produce_a_complete_recording() {
    for panic in [false, true] {
        let mut adapter = adapter(true);
        adapter.panic = panic;
        let (doc, snapshot, plan) = fixture(&adapter, |_| {});
        let archive = Arc::new(Archive::default());
        archive.exclude.store(!panic, Ordering::Relaxed);
        let mut run = start(&doc, &snapshot, plan, archive, ReplayOptions::default());
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.pump(1000)));
        if panic {
            assert!(attempt.is_err());
        } else {
            assert!(attempt.unwrap().is_err());
        }
        assert!(run.stats().incomplete);
        assert_eq!(run.finish().unwrap_err(), ReplayError::Incomplete);
    }
}
#[test]
fn log_debug_omits_payloads_archive_and_callback_context() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |wire| {
        wire["nodes"][0]["definition_id"] = json!("core/on-manual");
        wire["nodes"][0]["config"] = json!({"payload":"core/string@1"});
    });
    let archive = Arc::new(Archive::default());
    let weak = Arc::downgrade(&archive);
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions::default(),
    );
    let secret = "secret-never-in-journal";
    run.trigger(
        id(1).parse().unwrap(),
        Some(&Value::Str(Arc::new(secret.into()))),
        None,
    )
    .unwrap();
    assert!(!format!("{run:?}").contains(secret));
    let (_, log) = run.finish().unwrap();
    assert!(!format!("{log:?}").contains(secret));
    drop(archive);
    assert!(weak.upgrade().is_none());
    assert!(log.stats().entries > 0);
}
#[test]
fn external_cancel_tokens_and_unbounded_entry_storage_are_refused_before_start() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |_| {});
    let archive = Arc::new(Archive::default());
    let mut options = RunOptions::default();
    options.data.cancel = Some(orange_runtime::CancelToken::default());
    assert_eq!(
        RecordedRun::start(
            plan.clone(),
            &doc,
            &snapshot,
            0,
            options,
            ReplayOptions::default(),
            archive.clone()
        )
        .unwrap_err(),
        ReplayError::Unsupported
    );
    assert_eq!(
        RecordedRun::start(
            plan,
            &doc,
            &snapshot,
            0,
            RunOptions::default(),
            ReplayOptions {
                max_entries: usize::MAX,
                max_bytes: usize::MAX
            },
            archive
        )
        .unwrap_err(),
        ReplayError::Full
    );
}

#[test]
fn abort_at_zero_capacity_still_drives_normal_run_cleanup() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |_| {});
    let mut recorded = start(
        &doc,
        &snapshot,
        plan,
        Arc::new(Archive::default()),
        ReplayOptions {
            max_entries: 0,
            max_bytes: 0,
        },
    );
    assert_eq!(
        recorded.request_stop(StopMode::Immediate),
        Err(ReplayError::Full)
    );
    let mut run = recorded.abort().unwrap();
    for _ in 0..100 {
        if run.state() == RunState::Stopped {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.ownership().effects, 0);
    assert_eq!(adapter.count.load(Ordering::Relaxed), 0);
}

#[test]
fn late_completion_after_accepted_result_and_stop_cannot_publish_on_replay() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |_| {});
    let archive = Arc::new(Archive::default());
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions::default(),
    );
    idle(&mut run, 1);
    let request = run.take_effect().unwrap().unwrap();
    run.complete_effect(request.id, Ok(&[Value::U64(42)]))
        .unwrap();
    run.request_stop(StopMode::Immediate).unwrap();
    idle(&mut run, 1);
    assert!(
        run.complete_effect(request.id, Ok(&[Value::U64(99)]))
            .is_err()
    );
    let (original, log) = run.finish().unwrap();
    let replay = log.replay(&doc, &snapshot, archive).unwrap();
    assert_eq!(result(&original), None);
    assert_eq!(result(replay.inspect()), None);
    assert!(request.lease.is_cancelled());
    assert_eq!(original.trace(), replay.inspect().trace());
}

#[test]
fn async_arrival_order_is_recorded_across_two_manual_activations() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |wire| {
        wire["nodes"][0]["definition_id"] = json!("core/on-manual");
        wire["nodes"][0]["activation"]["mode"] = json!("parallel");
        wire["nodes"][0]["activation"]["max_parallel"] = json!(2);
    });
    let archive = Arc::new(Archive::default());
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions::default(),
    );
    run.trigger(id(1).parse().unwrap(), None, None).unwrap();
    run.trigger(id(1).parse().unwrap(), None, None).unwrap();
    idle(&mut run, 1);
    let first = run.take_effect().unwrap().unwrap();
    let second = run.take_effect().unwrap().unwrap();
    run.complete_effect(second.id, Ok(&[Value::U64(20)]))
        .unwrap();
    idle(&mut run, 1);
    assert_eq!(result(run.inspect()), Some(&Value::U64(20)));
    run.complete_effect(first.id, Ok(&[Value::U64(10)]))
        .unwrap();
    idle(&mut run, 1);
    let (original, log) = run.finish().unwrap();
    let replay = log.replay(&doc, &snapshot, archive).unwrap();
    assert_eq!(result(replay.inspect()), Some(&Value::U64(10)));
    assert_eq!(original.trace(), replay.inspect().trace());
}

#[test]
fn replay_rejects_changed_document_or_registry_before_any_callback() {
    let adapter = adapter(true);
    let (doc, snapshot, plan) = fixture(&adapter, |_| {});
    let archive = Arc::new(Archive::default());
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions::default(),
    );
    idle(&mut run, 1);
    let (_, log) = run.finish().unwrap();
    let mut wire: Json = serde_json::from_str(&to_json(&doc).unwrap()).unwrap();
    wire["nodes"][1]["inputs"]["value"]["value"] = json!("100");
    let changed = from_json(&wire.to_string()).unwrap();
    assert_eq!(
        log.replay(&changed, &snapshot, archive.clone())
            .unwrap_err(),
        ReplayError::Run(RunError::Data(orange_runtime::DataError::StaleDocument))
    );
    let (_, other, _) = fixture(&adapter, |_| {});
    assert_eq!(
        log.replay(&doc, &other, archive).unwrap_err(),
        ReplayError::Run(RunError::Data(orange_runtime::DataError::StaleRegistry))
    );
    assert_eq!(adapter.count.load(Ordering::Relaxed), 1);
}

#[test]
fn foreign_source_and_effect_id_refusals_replay_without_borrowing_other_run_authority() {
    let adapter = adapter(false);
    let (doc, snapshot, plan) = fixture(&adapter, |wire| {
        wire["nodes"][0]["definition_id"] = json!("core/on-event");
        wire["nodes"][0]["config"] = json!({"event":EVENT});
        wire["events"] = json!([{"id":EVENT,"name":"pulse","payload":"core/u64@1","scope":"run"}]);
    });
    let archive = Arc::new(Archive::default());
    let mut first = start(
        &doc,
        &snapshot,
        plan.clone(),
        archive.clone(),
        ReplayOptions::default(),
    );
    let mut second = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions::default(),
    );
    let source = first.register_event_source(EVENT.parse().unwrap()).unwrap();
    let foreign = second
        .register_event_source(EVENT.parse().unwrap())
        .unwrap();
    assert_eq!(
        first.ingest_event(&foreign, 1, &Value::U64(3), None),
        Err(ReplayError::Ingress(IngressError::WrongRun))
    );
    assert_eq!(
        first.revoke_event_source(&foreign),
        Err(ReplayError::Ingress(IngressError::WrongRun))
    );
    first
        .ingest_event(&source, 1, &Value::U64(3), None)
        .unwrap();
    second
        .ingest_event(&foreign, 1, &Value::U64(4), None)
        .unwrap();
    idle(&mut first, 1);
    idle(&mut second, 1);
    let own = first.take_effect().unwrap().unwrap();
    let other = second.take_effect().unwrap().unwrap();
    assert_eq!(
        first.complete_effect(other.id, Ok(&[Value::U64(99)])),
        Err(ReplayError::Effect(EffectError::WrongRun))
    );
    first
        .complete_effect(own.id, Ok(&[Value::U64(42)]))
        .unwrap();
    idle(&mut first, 1);
    let (original, log) = first.finish().unwrap();
    let replay = log.replay(&doc, &snapshot, archive).unwrap();
    assert_eq!(result(replay.inspect()), Some(&Value::U64(42)));
    assert_eq!(original.trace(), replay.inspect().trace());
}

#[test]
fn resumable_playback_steps_one_command_and_latches_divergence() {
    let adapter = adapter(true);
    let (doc, snapshot, plan) = fixture(&adapter, |_| {});
    let archive = Arc::new(Archive::default());
    let mut run = start(
        &doc,
        &snapshot,
        plan,
        archive.clone(),
        ReplayOptions::default(),
    );
    run.advance_to(10).unwrap();
    run.pump(1000).unwrap();
    run.advance_to(20).unwrap();
    let (_, log) = run.finish().unwrap();
    assert_eq!(log.seed(), 123);
    assert_eq!(log.entropy_algorithm(), orange_runtime::ENTROPY_ALGORITHM);
    assert_eq!(log.stats().version, 1);
    assert_eq!(
        log.begin_replay(&doc, &snapshot, archive.clone())
            .unwrap()
            .finish()
            .unwrap_err(),
        ReplayError::Incomplete
    );
    let mut playback = log.begin_replay(&doc, &snapshot, archive.clone()).unwrap();
    assert_eq!(playback.inspect().now(), 0);
    assert!(playback.step().unwrap());
    assert_eq!(playback.inspect().now(), 10);
    assert_eq!(result(playback.inspect()), None);
    assert!(playback.step().unwrap());
    assert_eq!(result(playback.inspect()), Some(&Value::U64(42)));
    assert!(!playback.is_complete());
    assert!(playback.step().unwrap());
    assert_eq!(playback.inspect().now(), 20);
    assert!(playback.is_complete());
    assert!(!playback.step().unwrap());
    playback.finish().unwrap();
    archive.wrong.store(true, Ordering::Relaxed);
    let mut failed = log.begin_replay(&doc, &snapshot, archive.clone()).unwrap();
    failed.step().unwrap();
    assert!(failed.step().is_err());
    archive.wrong.store(false, Ordering::Relaxed);
    assert_eq!(failed.step(), Err(ReplayError::Incomplete));
    assert!(!failed.is_complete());
    assert_eq!(adapter.count.load(Ordering::Relaxed), 1);
}

#[test]
fn caught_archive_panic_cannot_seal_or_resume_playback_even_at_the_last_entry() {
    for action in [false, true] {
        let adapter = adapter(action);
        let (doc, snapshot, plan) = fixture(&adapter, |wire| {
            if !action {
                wire["nodes"][0]["definition_id"] = json!("core/on-manual");
                wire["nodes"][0]["config"] = json!({"payload":"core/string@1"});
            }
        });
        let archive = Arc::new(Archive::default());
        let mut run = start(
            &doc,
            &snapshot,
            plan,
            archive.clone(),
            ReplayOptions::default(),
        );
        if action {
            run.pump(1000).unwrap();
        } else {
            run.trigger(
                id(1).parse().unwrap(),
                Some(&Value::Str(Arc::new("not-logged".into()))),
                None,
            )
            .unwrap();
        }
        let (_, log) = run.finish().unwrap();
        archive.panic_resolve.store(true, Ordering::Relaxed);
        let mut replay = log.begin_replay(&doc, &snapshot, archive).unwrap();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| replay.step())).is_err());
        assert!(!replay.is_complete());
        assert_eq!(replay.step(), Err(ReplayError::Incomplete));
        assert_eq!(replay.finish().unwrap_err(), ReplayError::Incomplete);
        assert_eq!(adapter.count.load(Ordering::Relaxed), usize::from(action));
    }
}
