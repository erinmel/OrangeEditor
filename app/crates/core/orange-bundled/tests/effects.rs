//! Synthetic trusted adapter exercised through public graph/compiler/runtime APIs.
use crate::data_graph::{catalog, edge, graph, id};
use orange_compiler::{
    ActionKernel, BindingRequest, CompileOptions, EffectBinding, EffectResolver, KernelFault,
    ResolveError, compile_control_with_effects,
};
use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder, RegistrySnapshot,
};
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use orange_runtime::{EffectError, Run, RunOptions, RunState};
use serde_json::{Value as Json, json};
use std::sync::Arc;

#[derive(Debug)]
struct Increment;
impl ActionKernel for Increment {
    fn execute(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        let Value::U64(value) = inputs[0] else {
            panic!("typed input")
        };
        outputs[0] = Value::U64(value + 1);
        Ok(())
    }
}
struct Adapter {
    action: bool,
    wrong_ports: bool,
}
impl EffectResolver for Adapter {
    fn resolve_effect(&self, request: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
        assert_eq!(request.provider().unwrap().id, "inert");
        Ok(EffectBinding {
            route: 42,
            context: Arc::new(request.provider().unwrap().clone()),
            inputs: vec!["value".parse().unwrap()].into(),
            outputs: vec![
                if self.wrong_ports { "wrong" } else { "result" }
                    .parse()
                    .unwrap(),
            ]
            .into(),
            action: self
                .action
                .then(|| Arc::new(Increment) as Arc<dyn ActionKernel>),
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
pub(super) fn setup(
    action: bool,
    options: RunOptions,
) -> (GraphDocument, Arc<RegistrySnapshot>, Run) {
    setup_edit(action, options, |_| {})
}
fn setup_edit(
    action: bool,
    options: RunOptions,
    edit: impl FnOnce(&mut Json),
) -> (GraphDocument, Arc<RegistrySnapshot>, Run) {
    let (package, base) = catalog();
    let definition = NodeDefinition::from_json(&json!({"definition_id":"acme.effects/increment","definition_version":1,"config_version":1,
        "kind":if action {"action"} else {"async"},"suspends":if action {"never"} else {"always"},"determinism":"external",
        "ports":[{"port_id":"value","direction":"input","kind":"data","type":"core/u64@1"},
                 {"port_id":"result","direction":"output","kind":"data","type":"core/u64@1","source":"execution","available_on":["then"]}]}).to_string()).unwrap();
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
    let effect = json!({"id":id(2),"definition_id":"acme.effects/increment","definition_version":1,"config_version":1,"config":{},"signature":"effect",
        "provider":{"package":key.package,"version":key.version,"id":key.id},"inputs":{"value":{"type":"core/u64@1","value":"41"}}});
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
    // This inert adapter has no graph-variable writes. Callable bodies require
    // explicit fresh leaf-effect evidence as well as their registered kind.
    let leaves = doc
        .functions()
        .values()
        .flat_map(|f| f.body.nodes.values())
        .filter(|n| n.definition.id.as_str() == "acme.effects/increment")
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
        &Adapter {
            action,
            wrong_ports: false,
        },
        CompileOptions::default(),
    )
    .unwrap();
    let run = Run::start(Arc::new(plan), &doc, &snapshot, 0, options).unwrap();
    (doc, snapshot, run)
}
fn idle(run: &mut Run) {
    for _ in 0..300 {
        if run.state() == RunState::Stopped || !run.pump(1).unwrap().ready {
            return;
        }
    }
    panic!("did not idle");
}
fn result(run: &Run) -> Option<&Value> {
    run.variable(id(100).parse().unwrap())
}

fn error_projection_run(options: RunOptions, cycle: bool) -> Run {
    setup_edit(true, options, |wire| {
        let mut count = core(2, "count");
        count["inputs"] = json!({"count":{"type":"core/i64@1","value":"-1"}});
        wire["nodes"][1] = count;
        wire["variables"][0]["type"] = json!("core/error@1");
        let mut wait = core(4, "wait");
        wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"1"}});
        wire["nodes"].as_array_mut().unwrap().push(wait);
        wire["edges"] = json!([
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "failed", 4, "in"),
            edge(3, 4, "then", 3, "in"),
            edge(4, 2, "error", 3, "value")
        ]);
        if cycle {
            wire["edges"]
                .as_array_mut()
                .unwrap()
                .push(edge(5, 3, "then", 2, "in"));
        }
    })
    .2
}

#[test]
fn implicit_error_projection_respects_state_and_aggregate_admission_before_waiting() {
    use orange_runtime::{RunError, RunLimits};
    let bytes = 32 + "core/OutOfRange".len();
    for (state, aggregate, expected) in [
        (bytes - 1, 4096, Some(RunError::ResourceLimit)),
        (bytes, bytes - 1, Some(RunError::AggregateLimit)),
        (bytes, bytes, None),
    ] {
        let mut run = error_projection_run(
            RunOptions {
                limits: RunLimits {
                    max_state_bytes: state,
                    max_run_retained_bytes: aggregate,
                    ..Default::default()
                },
                ..Default::default()
            },
            false,
        );
        let mut failure = None;
        for _ in 0..32 {
            match run.pump(1) {
                Err(error) => {
                    failure = Some(error);
                    break;
                }
                Ok(progress) if !progress.ready => break,
                Ok(_) => {}
            }
        }
        assert_eq!(failure, expected);
        if expected.is_none() {
            assert_eq!(run.pump(0).unwrap().next_wakeup, Some(1));
            assert_eq!(run.retention().reserved_bytes, bytes);
            assert!(run.trace().iter().any(|e| {
                e.node.to_string() == id(2)
                    && e.fault
                        .as_ref()
                        .is_some_and(|f| f.source.to_string() == id(2))
            }));
        } else {
            assert!(!run.trace().iter().any(|e| e.node.to_string() == id(4)));
        }
        if run.state() != RunState::Stopped {
            run.stop().unwrap();
        }
        idle(&mut run);
        assert_eq!(run.retention().reserved_bytes, 0);
    }
}

#[test]
fn repeated_error_publication_credits_replaced_storage_and_stop_releases_it() {
    let bytes = 32 + "core/OutOfRange".len();
    let mut run = error_projection_run(
        RunOptions {
            limits: orange_runtime::RunLimits {
                max_state_bytes: bytes,
                max_run_retained_bytes: 2 * bytes,
                ..Default::default()
            },
            ..Default::default()
        },
        true,
    );
    idle(&mut run);
    assert_eq!(run.retention().reserved_bytes, bytes);
    for now in 1..=16 {
        run.advance_to(now).unwrap();
        idle(&mut run);
        assert_eq!(run.retention().reserved_bytes, 2 * bytes);
        let Some(Value::Err(error)) = result(&run) else {
            panic!("error must stay data")
        };
        assert_eq!(error.code, "core/OutOfRange");
        assert_eq!(error.node, Some(id(2).parse().unwrap()));
    }
    run.stop().unwrap();
    idle(&mut run);
    run.dispose().unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);
}
#[test]
fn async_request_once_then_budgeted_typed_completion() {
    let (_, _, mut run) = setup(false, RunOptions::default());
    idle(&mut run);
    let request = run.take_effect().unwrap();
    assert_eq!(request.route, 42);
    assert_eq!(
        request
            .context
            .downcast_ref::<ProviderKey>()
            .unwrap()
            .version,
        "1.0.0"
    );
    assert_eq!(&*request.inputs, &[Value::U64(41)]);
    assert!(run.take_effect().is_none());
    assert_eq!(result(&run), None);
    assert_eq!(
        run.complete_effect(request.id, Ok(&[Value::Bool(true)])),
        Err(EffectError::Contract)
    );
    run.complete_effect(request.id, Ok(&[Value::U64(42)]))
        .unwrap();
    assert_eq!(result(&run), None);
    assert!(run.pump(0).unwrap().ready);
    assert_eq!(
        run.complete_effect(request.id, Ok(&[Value::U64(99)])),
        Err(EffectError::Stale)
    );
    idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(42)));
    assert_eq!(run.ownership().effects, 0);
    assert!(request.lease.is_finished());
    assert!(!request.lease.is_cancelled());
}
#[test]
fn blocked_host_does_not_block_stop_and_late_wrong_run_refuse() {
    let (_, _, mut run) = setup(false, RunOptions::default());
    idle(&mut run);
    let request = run.take_effect().unwrap();
    let (_, _, mut other) = setup(false, RunOptions::default());
    assert_eq!(
        other.complete_effect(request.id, Ok(&[Value::U64(1)])),
        Err(EffectError::WrongRun)
    );
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert!(request.lease.is_cancelled());
    assert!(
        run.complete_effect(request.id, Ok(&[Value::U64(1)]))
            .is_err()
    );
    assert_eq!(run.ownership().effects, 0);
    run.dispose().unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);
}
#[test]
fn synchronous_action_executes_only_from_control() {
    let (_, _, mut run) = setup(true, RunOptions::default());
    assert_eq!(result(&run), None);
    idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(42)));
    assert!(run.take_effect().is_none());
}
#[test]
fn refused_result_quota_is_retryable_and_fault_uses_failed_semantics() {
    let options = RunOptions {
        limits: orange_runtime::RunLimits {
            max_effect_bytes: 16,
            ..Default::default()
        },
        ..Default::default()
    };
    let (_, _, mut run) = setup(false, options);
    idle(&mut run);
    let request = run.take_effect().unwrap();
    assert_eq!(
        run.complete_effect(request.id, Ok(&[Value::U64(42)])),
        Err(EffectError::Limit)
    );
    assert_eq!(run.ownership().effects, 1);
    run.stop().unwrap();
    idle(&mut run);
    let (_, _, mut run) = setup(false, RunOptions::default());
    idle(&mut run);
    let request = run.take_effect().unwrap();
    assert_eq!(
        run.complete_effect(
            request.id,
            Err(KernelFault {
                code: orange_compiler::FaultText::Static("invalid code"),
                message_key: orange_compiler::FaultText::Static("core/test"),
                message: "Safe static fault."
            })
        ),
        Err(EffectError::Contract)
    );
    run.complete_effect(
        request.id,
        Err(KernelFault {
            code: orange_compiler::FaultText::Static("core/Test"),
            message_key: orange_compiler::FaultText::Static("core/test"),
            message: "Inert test failure.",
        }),
    )
    .unwrap();
    idle(&mut run);
    assert_eq!(result(&run), None);
    assert_eq!(run.ownership().effects, 0);
}

#[test]
fn cancellation_runs_connected_cleanup() {
    let (_, _, mut run) = setup_edit(false, RunOptions::default(), |wire| {
        let mut cleanup = core(4, "set-variable");
        cleanup["config"] = json!({"variable":id(100)});
        cleanup["inputs"] = json!({"value":{"type":"core/u64@1","value":"9"}});
        wire["nodes"].as_array_mut().unwrap().push(cleanup);
        wire["edges"]
            .as_array_mut()
            .unwrap()
            .push(edge(4, 2, "cancelled", 4, "in"));
    });
    idle(&mut run);
    let request = run.take_effect().unwrap();
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(9)));
    assert!(request.lease.is_cancelled());
}

#[test]
fn reentry_gets_a_new_id() {
    let (_, _, mut run) = setup_edit(false, RunOptions::default(), |wire| {
        let mut wait = core(4, "wait");
        wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"1"}});
        wire["nodes"].as_array_mut().unwrap().push(wait);
        wire["edges"]
            .as_array_mut()
            .unwrap()
            .extend([edge(4, 3, "then", 4, "in"), edge(5, 4, "then", 2, "in")]);
    });
    idle(&mut run);
    let first = run.take_effect().unwrap();
    run.complete_effect(first.id, Ok(&[Value::U64(42)]))
        .unwrap();
    idle(&mut run);
    run.advance_to(1).unwrap();
    idle(&mut run);
    let second = run.take_effect().unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(
        run.complete_effect(first.id, Ok(&[Value::U64(99)])),
        Err(EffectError::Stale)
    );
    run.stop().unwrap();
    idle(&mut run);
}

#[test]
fn ordinary_compile_and_mismatched_effect_bindings_refuse() {
    let (doc, snapshot, _) = setup(false, RunOptions::default());
    let (package, _) = catalog();
    let analysis = analyze(&doc, &snapshot);
    assert!(matches!(
        orange_compiler::compile_control(
            &doc,
            snapshot.clone(),
            &analysis,
            &package,
            CompileOptions::default()
        ),
        Err(orange_compiler::CompileError::UnsupportedNode { .. })
    ));
    for adapter in [
        Adapter {
            action: false,
            wrong_ports: true,
        },
        Adapter {
            action: true,
            wrong_ports: false,
        },
    ] {
        assert!(matches!(
            compile_control_with_effects(
                &doc,
                snapshot.clone(),
                &analysis,
                &package,
                &adapter,
                CompileOptions::default()
            ),
            Err(orange_compiler::CompileError::KernelContract { .. })
        ));
    }
}

#[test]
fn pending_capacity_is_enforced_before_publishing_a_request() {
    let options = RunOptions {
        limits: orange_runtime::RunLimits {
            max_pending_effects: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    let (_, _, mut run) = setup(false, options);
    let mut fault = None;
    for _ in 0..100 {
        match run.pump(1) {
            Ok(_) => {}
            Err(error) => {
                fault = Some(error);
                break;
            }
        }
    }
    assert_eq!(fault, Some(orange_runtime::RunError::ResourceLimit));
    assert!(run.take_effect().is_none());
    assert_eq!(run.ownership().effects, 0);
}

#[test]
fn race_cancels_pending_effect_and_connected_cleanup_before_completion() {
    let (_, _, mut run) = setup_edit(false, RunOptions::default(), |wire| {
        let mut fork = core(4, "fork");
        fork["config"] = json!({"join":"race"});
        fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
        let mut wait = core(5, "wait");
        wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"1"}});
        let mut cleanup = core(6, "set-variable");
        cleanup["config"] = json!({"variable":id(100)});
        cleanup["inputs"] = json!({"value":{"type":"core/u64@1","value":"9"}});
        wire["nodes"]
            .as_array_mut()
            .unwrap()
            .extend([fork, wait, cleanup]);
        let edges = wire["edges"].as_array_mut().unwrap();
        edges[0] = edge(1, 1, "then", 4, "in");
        edges.extend([
            edge(4, 4, "branch#0", 2, "in"),
            edge(5, 4, "branch#1", 5, "in"),
            edge(6, 2, "cancelled", 6, "in"),
        ]);
    });
    idle(&mut run);
    let request = run.take_effect().unwrap();
    run.advance_to(1).unwrap();
    idle(&mut run);
    assert!(request.lease.is_cancelled());
    assert_eq!(result(&run), Some(&Value::U64(9)));
    assert_eq!(run.ownership().effects, 0);
    assert_eq!(
        run.complete_effect(request.id, Ok(&[Value::U64(42)])),
        Err(EffectError::Stale)
    );
}

#[test]
fn manual_to_event_await_to_watch_to_inert_effect_then_stop() {
    let (_, _, mut run) = setup_edit(false, RunOptions::default(), |wire| {
        wire["nodes"][0]["definition_id"] = json!("core/on-manual");
        let event = "40000000-0000-4000-8000-000000000001";
        wire["events"] = json!([{"id":event,"name":"pulse","payload":"core/u64@1","scope":"run"}]);
        wire["variables"].as_array_mut().unwrap().push(json!({"id":id(101),"name":"watched","type":"core/u64@1","scope":"run","init":{"type":"core/u64@1","value":"0"}}));
        let mut fork = core(4, "fork");
        fork["config"] = json!({"join":"all"});
        fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
        let mut awaiter = core(5, "await-event");
        awaiter["config"] = json!({"event":event});
        let mut set = core(6, "set-variable");
        set["config"] = json!({"variable":id(101)});
        let mut emit = core(7, "emit");
        emit["config"] = json!({"event":event});
        emit["inputs"] = json!({"payload":{"type":"core/u64@1","value":"41"}});
        let mut watch = core(8, "on-watch");
        watch["config"] = json!({"mode":"changed"});
        watch["type_pins"] = json!({"T":"core/u64@1"});
        watch["activation"] =
            json!({"mode":"queue","capacity":4,"overflow":"fault","on_fault":"continue","order":0});
        let mut get = core(9, "get-variable");
        get["config"] = json!({"variable":id(101)});
        wire["nodes"]
            .as_array_mut()
            .unwrap()
            .extend([fork, awaiter, set, emit, watch, get]);
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
            edge(10, 2, "result", 3, "value")
        ]);
    });
    idle(&mut run);
    assert!(run.take_effect().is_none());
    run.trigger(id(1).parse().unwrap(), None, None).unwrap();
    idle(&mut run);
    let request = run.take_effect().unwrap();
    assert_eq!(&*request.inputs, &[Value::U64(41)]);
    assert!(run.take_effect().is_none());
    run.complete_effect(request.id, Ok(&[Value::U64(42)]))
        .unwrap();
    idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(42)));
    run.stop().unwrap();
    idle(&mut run);
    run.dispose().unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);
}

#[test]
fn async_function_effect_returns_typed_results_and_retires_its_projection() {
    let (_, _, mut run) = setup_edit(false, RunOptions::default(), |wire| {
        let mut effect = wire["nodes"][1].clone();
        effect["id"] = json!(id(12));
        let mut call = core(2, "call");
        call["config"] = json!({"function":id(200)});
        wire["nodes"][1] = call;
        wire["functions"] = json!([{"id":id(200),"name":"effect","kind":"async","inputs":[],
            "outputs":[{"port_id":"result","direction":"output","kind":"data","type":"core/u64@1"}],
            "nodes":[core(11,"function-entry"),effect,core(13,"return")],
            "edges":[edge(11,11,"then",12,"in"),edge(12,12,"then",13,"in"),edge(13,12,"result",13,"result")]}]);
    });
    idle(&mut run);
    let request = run.take_effect().unwrap();
    run.complete_effect(request.id, Ok(&[Value::U64(42)]))
        .unwrap();
    idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(42)));
    assert_eq!(run.ownership().effects, 0);
    assert_eq!(run.retention().reserved_bytes, 0);
}

#[test]
fn returning_a_captured_error_refunds_the_callees_error_projection() {
    let (_, _, mut run) = setup_edit(false, RunOptions::default(), |wire| {
        let mut effect = wire["nodes"][1].clone();
        effect["id"] = json!(id(12));
        let mut call = core(2, "call");
        call["config"] = json!({"function":id(200)});
        wire["nodes"][1] = call;
        wire["variables"][0]["type"] = json!("core/error@1");
        let mut wait = core(4, "wait");
        wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"1"}});
        wire["nodes"].as_array_mut().unwrap().push(wait);
        wire["edges"] = json!([
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 4, "in"),
            edge(3, 4, "then", 3, "in"),
            edge(4, 2, "result", 3, "value")
        ]);
        let mut get = core(15, "get-variable");
        get["config"] = json!({"variable":id(100)});
        wire["functions"] = json!([{"id":id(200),"name":"captured","kind":"async","inputs":[],
            "outputs":[{"port_id":"result","direction":"output","kind":"data","type":"core/error@1"}],
            "nodes":[core(11,"function-entry"),effect,core(13,"return"),core(14,"return"),get],
            "edges":[edge(11,11,"then",12,"in"),edge(12,12,"failed",13,"in"),edge(13,12,"error",13,"result"),
                edge(14,12,"then",14,"in"),edge(15,15,"value",14,"result")]}]);
    });
    idle(&mut run);
    let request = run.take_effect().unwrap();
    run.complete_effect(
        request.id,
        Err(KernelFault {
            code: "acme/Failed".into(),
            message_key: "acme/failed".into(),
            message: "Safe failure.",
        }),
    )
    .unwrap();
    idle(&mut run);
    assert_eq!(run.pump(0).unwrap().next_wakeup, Some(1));
    // Only the return's data cache and caller's result remain. The callee's
    // execution projection must release its separate nominal reservation.
    assert_eq!(
        run.retention().reserved_bytes,
        2 * (32 + "acme/Failed".len())
    );
    run.stop().unwrap();
    idle(&mut run);
    run.dispose().unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);
}

#[test]
fn stop_after_result_acceptance_discards_it_before_publication() {
    let (_, _, mut run) = setup(false, RunOptions::default());
    idle(&mut run);
    let request = run.take_effect().unwrap();
    run.complete_effect(request.id, Ok(&[Value::U64(42)]))
        .unwrap();
    run.stop().unwrap();
    idle(&mut run);
    assert_eq!(result(&run), None);
    assert!(request.lease.is_cancelled());
    assert_eq!(run.retention().reserved_bytes, 0);
}

#[test]
fn return_cancels_a_detached_effect_before_resuming_its_caller() {
    let (_, _, mut run) = setup_edit(false, RunOptions::default(), |wire| {
        let mut effect = wire["nodes"][1].clone();
        effect["id"] = json!(id(12));
        let mut call = core(2, "call");
        call["config"] = json!({"function":id(200)});
        wire["nodes"][1] = call;
        wire["nodes"][2]["inputs"] = json!({"value":{"type":"core/u64@1","value":"42"}});
        wire["edges"] = json!([edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")]);
        let mut fork = core(14, "fork");
        fork["config"] = json!({"join":"detach"});
        fork["variadic"] = json!({"branch":{"next":1,"order":[0]}});
        let mut wait = core(15, "wait");
        wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"1"}});
        wire["functions"] = json!([{"id":id(200),"name":"detached","kind":"async","inputs":[],"outputs":[],
            "nodes":[core(11,"function-entry"),effect,core(13,"return"),fork,wait],
            "edges":[edge(11,11,"then",14,"in"),edge(12,14,"branch#0",12,"in"),edge(13,14,"completed",15,"in"),edge(14,15,"then",13,"in")]}]);
    });
    idle(&mut run);
    let request = run.take_effect().unwrap();
    run.advance_to(1).unwrap();
    idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(42)));
    assert!(request.lease.is_cancelled());
    assert_eq!(run.ownership().effects, 0);
    assert_eq!(
        run.complete_effect(request.id, Ok(&[Value::U64(1)])),
        Err(EffectError::Stale)
    );
}

#[test]
fn a_paused_run_issues_no_effect_and_holds_an_admitted_completion() {
    let (_, _, mut run) = setup(false, RunOptions::default());
    idle(&mut run);
    run.pause().unwrap();
    assert!(run.take_effect().is_none());
    run.resume().unwrap();
    let request = run.take_effect().unwrap();
    assert!(!request.lease.is_paused());

    run.pause().unwrap();
    assert!(request.lease.is_paused());
    run.complete_effect(request.id, Ok(&[Value::U64(42)]))
        .unwrap();
    assert_eq!(run.pump(64).unwrap().operations, 0);
    assert_eq!(result(&run), None);

    run.resume().unwrap();
    assert!(!request.lease.is_paused());
    idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(42)));
    // Finished work is never reported as held, even if the run pauses again.
    run.pause().unwrap();
    assert!(request.lease.is_finished() && !request.lease.is_paused());
}
