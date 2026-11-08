use super::*;
use orange_model::{GraphDocument, Value};
use orange_runtime::{
    Run, RunError, RunOptions, SharedError, StopMode, Workspace, WorkspaceLimits,
};
use std::sync::Arc;
fn vid(n: u32) -> orange_model::id::VariableId {
    id(n).parse().unwrap()
}
fn idle(run: &mut Run, budget: usize) {
    for _ in 0..10000 {
        if run.state() == orange_runtime::RunState::Stopped || !run.pump(budget).unwrap().ready {
            return;
        }
    }
    panic!("bounded fixture did not idle");
}
fn fixture(operation: &str) -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    let mut write = var_node(
        2,
        if operation == "set" {
            "set-shared"
        } else {
            "update-shared"
        },
        100,
    );
    write["inputs"] =
        json!({"value":{"type":"core/u64@1","value":if operation=="add" {"1"}else{"8"}}});
    if operation != "set" {
        write["config"]["operation"] = json!(operation);
    }
    if operation == "compare-and-set" {
        write["inputs"]["expected"] = json!({"type":"core/u64@1","value":"3"});
    }
    let mut nodes = vec![
        core(1, "on-start"),
        write,
        var_node(3, "get-variable", 100),
        var_node(4, "set-variable", 101),
    ];
    let mut edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 4, "in"),
        edge(3, 3, "value", 4, "value"),
    ];
    if operation != "set" {
        nodes.extend([
            var_node(5, "set-variable", 102),
            var_node(6, "set-variable", 103),
        ]);
        edges.extend([
            edge(4, 4, "then", 5, "in"),
            edge(5, 2, "old", 5, "value"),
            edge(6, 5, "then", 6, "in"),
            edge(7, 2, "new", 6, "value"),
        ]);
    }
    if operation == "compare-and-set" {
        nodes.push(var_node(7, "set-variable", 104));
        edges.extend([
            edge(8, 6, "then", 7, "in"),
            edge(9, 2, "exchanged", 7, "value"),
        ]);
    }
    state_plan(
        nodes,
        edges,
        vec![
            variable(100, "workspace", "core/u64@1", Some(json!("3"))),
            variable(101, "run", "core/u64@1", None),
            variable(102, "run", "core/u64@1", None),
            variable(103, "run", "core/u64@1", None),
            variable(104, "run", "core/bool@1", None),
        ],
    )
}
fn start(doc: &GraphDocument, plan: &Arc<orange_compiler::ControlPlan>, w: &Workspace) -> Run {
    Run::start_in_workspace(
        plan.clone(),
        doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
        &w.snapshot(),
    )
    .unwrap()
}
#[test]
fn two_compiled_runs_serialize_add_without_lost_updates_and_read_own_write_before_then() {
    for budget in [1, 100] {
        let (doc, plan) = fixture("add");
        assert!(
            plan.variables()
                .iter()
                .filter(|v| v.scope() == orange_model::document::VarScope::Workspace)
                .all(|v| v.initial().is_none())
        );
        let mut w = Workspace::new(
            &doc,
            plan.data().snapshot().clone(),
            WorkspaceLimits::default(),
        )
        .unwrap();
        let mut a = start(&doc, &plan, &w);
        let mut b = start(&doc, &plan, &w);
        idle(&mut a, budget);
        idle(&mut b, budget);
        assert!(a.take_effect().is_none());
        assert_eq!(a.ownership().effects, 1);
        let ar = a.take_shared().unwrap().unwrap();
        let br = b.take_shared().unwrap().unwrap();
        assert!(a.take_shared().unwrap().is_none());
        assert_eq!(w.snapshot().version(), 0);
        let receipt = w.apply_shared(ar).unwrap();
        assert_eq!(receipt.commit().unwrap().old(), Some(&Value::U64(3)));
        a.complete_shared(receipt).unwrap();
        assert!(a.variable(vid(101)).is_none()); // ingestion never runs graph work
        let receipt = w.apply_shared(br).unwrap();
        b.complete_shared(receipt).unwrap();
        idle(&mut a, budget);
        idle(&mut b, budget);
        assert_eq!(w.snapshot().get(vid(100)).unwrap(), Some(&Value::U64(5)));
        assert_eq!(a.variable(vid(101)), Some(&Value::U64(4)));
        assert_eq!(b.variable(vid(101)), Some(&Value::U64(5)));
        assert_eq!(a.variable(vid(102)), Some(&Value::U64(3)));
        assert_eq!(b.variable(vid(102)), Some(&Value::U64(4)));
        assert_eq!(a.variable(vid(103)), Some(&Value::U64(4)));
        assert_eq!(b.variable(vid(103)), Some(&Value::U64(5)));
        a.publish_workspace(&w.snapshot()).unwrap();
        assert_eq!(a.workspace_snapshot().unwrap().version(), 2);
    }
}
#[test]
fn competing_cas_reports_one_exchange_and_mismatch_does_not_advance_version() {
    let (doc, plan) = fixture("compare-and-set");
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut a = start(&doc, &plan, &w);
    let mut b = start(&doc, &plan, &w);
    idle(&mut a, 1);
    idle(&mut b, 1);
    let first = w.apply_shared(a.take_shared().unwrap().unwrap()).unwrap();
    let second = w.apply_shared(b.take_shared().unwrap().unwrap()).unwrap();
    a.complete_shared(first).unwrap();
    b.complete_shared(second).unwrap();
    idle(&mut a, 1);
    idle(&mut b, 1);
    assert_eq!(a.variable(vid(104)), Some(&Value::Bool(true)));
    assert_eq!(b.variable(vid(104)), Some(&Value::Bool(false)));
    assert_eq!(b.variable(vid(102)), Some(&Value::U64(8)));
    assert_eq!(w.snapshot().version(), 1);
}
#[test]
fn shared_set_suspends_and_continues_with_an_empty_typed_result() {
    let (doc, plan) = fixture("set");
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    idle(&mut run, 1);
    let request = run.take_shared().unwrap().unwrap();
    assert!(request.operation().is_none());
    let receipt = w.apply_shared(request).unwrap();
    run.complete_shared(receipt).unwrap();
    idle(&mut run, 1);
    assert_eq!(run.variable(vid(101)), Some(&Value::U64(8)));
}
#[test]
fn wrong_workspace_and_wrong_run_refusals_preserve_single_use_input_for_retry() {
    let (doc, plan) = fixture("add");
    let mut a = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut b = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &a);
    let mut other = start(&doc, &plan, &a);
    idle(&mut run, 1);
    let request = run.take_shared().unwrap().unwrap();
    let refused = b.apply_shared(request).unwrap_err();
    assert_eq!(refused.error, SharedError::WrongWorkspace);
    assert_eq!(a.snapshot().version(), 0);
    assert_eq!(b.snapshot().version(), 0);
    let receipt = a.apply_shared(refused.value).unwrap();
    let refused = other.complete_shared(receipt).unwrap_err();
    assert_eq!(refused.error, SharedError::WrongRun);
    run.complete_shared(refused.value).unwrap();
    idle(&mut run, 1);
    assert_eq!(run.variable(vid(101)), Some(&Value::U64(4)));
}
#[test]
fn cancellation_before_commit_refuses_and_after_commit_never_rolls_back_external_state() {
    for committed in [false, true] {
        let (doc, plan) = fixture("add");
        let mut w = Workspace::new(
            &doc,
            plan.data().snapshot().clone(),
            WorkspaceLimits::default(),
        )
        .unwrap();
        let mut run = start(&doc, &plan, &w);
        idle(&mut run, 1);
        let request = run.take_shared().unwrap().unwrap();
        if committed {
            let receipt = w.apply_shared(request).unwrap();
            run.request_stop(StopMode::Immediate).unwrap();
            idle(&mut run, 1);
            assert!(run.complete_shared(receipt).is_err());
            assert_eq!(w.snapshot().version(), 1);
        } else {
            run.request_stop(StopMode::Immediate).unwrap();
            idle(&mut run, 1);
            assert_eq!(
                w.apply_shared(request).unwrap_err().error,
                SharedError::Stale
            );
            assert_eq!(w.snapshot().version(), 0);
        }
        assert!(run.variable(vid(101)).is_none());
        assert_eq!(run.ownership().effects, 0);
    }
}
#[test]
fn plain_run_requires_workspace_and_foreign_or_older_publications_are_atomic_refusals() {
    let (doc, plan) = fixture("set");
    assert!(matches!(
        Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default()
        ),
        Err(RunError::Shared(SharedError::WrongWorkspace))
    ));
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let foreign = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    let old = w.snapshot();
    w.set(vid(100), &Value::U64(99)).unwrap();
    run.publish_workspace(&w.snapshot()).unwrap();
    assert_eq!(
        run.publish_workspace(&old),
        Err(SharedError::SnapshotRegression)
    );
    assert_eq!(
        run.publish_workspace(&foreign.snapshot()),
        Err(SharedError::WrongWorkspace)
    );
    assert_eq!(
        run.workspace_snapshot().unwrap().get(vid(100)).unwrap(),
        Some(&Value::U64(99))
    );
}
#[test]
fn shared_reads_freeze_the_potential_input_union_across_host_pump_slices() {
    let mut nodes = vec![
        core(1, "on-start"),
        var_node(2, "set-variable", 101),
        var_node(3, "get-variable", 100),
    ];
    let mut edges = vec![edge(1, 1, "then", 2, "in")];
    let mut previous = 3;
    for n in 10..60 {
        let mut add = math(n, "add", 0, 1);
        add["inputs"].as_object_mut().unwrap().remove("a");
        edges.push(edge(
            n,
            previous,
            if previous == 3 { "value" } else { "sum" },
            n,
            "a",
        ));
        nodes.push(add);
        previous = n;
    }
    edges.push(edge(70, previous, "sum", 2, "value"));
    let (doc, plan) = state_plan(
        nodes,
        edges,
        vec![
            variable(100, "workspace", "core/u64@1", Some(json!("3"))),
            variable(101, "run", "core/u64@1", None),
        ],
    );
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    run.pump(1).unwrap();
    run.pump(1).unwrap();
    assert!(run.variable(vid(101)).is_none());
    w.set(vid(100), &Value::U64(99)).unwrap();
    run.publish_workspace(&w.snapshot()).unwrap();
    idle(&mut run, 1);
    assert_eq!(run.variable(vid(101)), Some(&Value::U64(53)));
    assert_eq!(
        run.workspace_snapshot().unwrap().get(vid(100)).unwrap(),
        Some(&Value::U64(99))
    );
}
#[test]
fn newer_publication_is_not_replaced_by_an_older_successful_receipt() {
    let (doc, plan) = fixture("add");
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    idle(&mut run, 1);
    let receipt = w.apply_shared(run.take_shared().unwrap().unwrap()).unwrap();
    w.set(vid(100), &Value::U64(80)).unwrap();
    run.publish_workspace(&w.snapshot()).unwrap();
    run.complete_shared(receipt).unwrap();
    idle(&mut run, 1);
    assert_eq!(run.variable(vid(101)), Some(&Value::U64(80)));
    assert_eq!(run.variable(vid(103)), Some(&Value::U64(4)));
    assert_eq!(run.workspace_snapshot().unwrap().version(), 2);
}
#[test]
fn host_rejection_routes_a_validated_graph_fault_without_committing() {
    let (doc, plan) = fixture("add");
    let w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    idle(&mut run, 1);
    let request = run.take_shared().unwrap().unwrap();
    let bad = orange_compiler::KernelFault {
        code: "secret-invalid",
        message_key: "bad",
        message: "hidden",
    };
    let refused = run.fail_shared(request, bad).unwrap_err();
    assert_eq!(refused.error, SharedError::Contract);
    run.fail_shared(
        refused.value,
        orange_compiler::KernelFault {
            code: "core/Refused",
            message_key: "core/shared-refused",
            message: "The shared operation was refused.",
        },
    )
    .unwrap();
    idle(&mut run, 1);
    assert_eq!(w.snapshot().version(), 0);
    assert!(run.variable(vid(101)).is_none());
    assert_eq!(
        run.trace()
            .back()
            .unwrap()
            .fault
            .as_ref()
            .unwrap()
            .fault
            .code,
        "core/Refused"
    );
}
#[test]
fn host_held_request_keeps_a_run_budget_lease_after_cancellation_and_disposal() {
    let (doc, plan) = fixture("add");
    let w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    idle(&mut run, 1);
    let request = run.take_shared().unwrap().unwrap();
    run.stop().unwrap();
    idle(&mut run, 1);
    run.dispose().unwrap();
    assert!(request.lease().is_cancelled());
    assert!(run.retention().reserved_bytes > 0);
    assert!(run.workspace_snapshot().is_none());
    drop(request);
    assert_eq!(run.retention().reserved_bytes, 0);
}
#[test]
fn workspace_snapshot_read_is_typed_and_uninitialized_shared_get_has_original_source() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            var_node(2, "set-variable", 101),
            var_node(3, "get-variable", 100),
        ],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 3, "value", 2, "value")],
        vec![
            variable(100, "workspace", "core/u64@1", None),
            variable(101, "run", "core/u64@1", None),
        ],
    );
    let w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    idle(&mut run, 1);
    let f = run.trace().back().unwrap().fault.as_ref().unwrap();
    assert_eq!(f.source.to_string(), id(3));
    assert_eq!(f.fault.code, "core/UninitializedVariable");
}
#[test]
fn workspace_profile_cannot_be_started_as_effect_replay() {
    struct Archive;
    impl std::fmt::Debug for Archive {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Archive")
        }
    }
    impl orange_runtime::ReplayArchive for Archive {
        fn capture(
            &self,
            _: &[Value],
        ) -> Result<orange_runtime::ReplayPayloadRef, orange_runtime::ReplayArchiveError> {
            panic!("must not call archive")
        }
        fn resolve(
            &self,
            _: orange_runtime::ReplayPayloadRef,
        ) -> Result<Arc<[Value]>, orange_runtime::ReplayArchiveError> {
            panic!("must not call archive")
        }
    }
    let (doc, plan) = fixture("add");
    assert!(matches!(
        orange_runtime::RecordedRun::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
            orange_runtime::ReplayOptions::default(),
            Arc::new(Archive)
        ),
        Err(orange_runtime::ReplayError::Unsupported)
    ));
}
#[test]
fn shared_command_is_cancelled_by_race_region_retirement() {
    let mut fork = core(2, "fork");
    fork["config"] = json!({"join":"race"});
    fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
    let mut write = var_node(3, "set-shared", 100);
    write["inputs"] = json!({"value":{"type":"core/u64@1","value":"9"}});
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            fork,
            write,
            wait(4, 5),
            var_node(5, "set-variable", 101),
            var_node(6, "get-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "branch#0", 3, "in"),
            edge(3, 2, "branch#1", 4, "in"),
            edge(4, 3, "then", 5, "in"),
            edge(5, 6, "value", 5, "value"),
        ],
        vec![
            variable(100, "workspace", "core/u64@1", Some(json!("3"))),
            variable(101, "run", "core/u64@1", None),
        ],
    );
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    idle(&mut run, 1);
    let receipt = w.apply_shared(run.take_shared().unwrap().unwrap()).unwrap();
    run.advance_to(5).unwrap();
    idle(&mut run, 1);
    assert!(run.complete_shared(receipt).is_err());
    assert!(run.variable(vid(101)).is_none());
    assert_eq!(w.snapshot().get(vid(100)).unwrap(), Some(&Value::U64(9)));
    assert_eq!(run.ownership().effects, 0);
}
#[test]
fn snapshot_projection_and_host_request_admission_obey_run_budgets() {
    let (doc, plan) = fixture("add");
    let w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 0;
    assert!(matches!(
        Run::start_in_workspace(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            options,
            &w.snapshot()
        ),
        Err(RunError::Shared(SharedError::Limit))
    ));
    let mut run = start(&doc, &plan, &w);
    idle(&mut run, 1);
    let baseline = run.retention().reserved_bytes;
    // Recreate at exactly its parked reservation: no detached host-command credit.
    let mut options = RunOptions::default();
    options.limits.max_run_retained_bytes = baseline;
    let mut tight = Run::start_in_workspace(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        options,
        &w.snapshot(),
    )
    .unwrap();
    idle(&mut tight, 1);
    let before = tight.retention();
    assert_eq!(tight.take_shared().unwrap_err(), SharedError::Limit);
    assert_eq!(tight.retention(), before);
    assert_eq!(tight.ownership().effects, 1);
    // A refusal did not consume the command; stop still owns and retires it.
    tight.stop().unwrap();
    idle(&mut tight, 1);
    tight.dispose().unwrap();
    assert_eq!(tight.retention().reserved_bytes, 0);
}
#[test]
fn actor_quota_refusal_preserves_the_original_request_and_committed_state() {
    let (doc, plan) = fixture("add");
    let probe = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let cap = probe.meter().retention().reserved_bytes;
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits {
            max_retained_bytes: cap,
            ..WorkspaceLimits::default()
        },
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    idle(&mut run, 1);
    let request = run.take_shared().unwrap().unwrap();
    let before = w.meter().retention();
    let refused = w.apply_shared(request).unwrap_err();
    assert_eq!(refused.error, SharedError::Limit);
    assert_eq!(w.meter().retention(), before);
    assert_eq!(w.snapshot().version(), 0);
    assert!(!refused.value.lease().is_finished());
    run.fail_shared(
        refused.value,
        orange_compiler::KernelFault {
            code: "core/Refused",
            message_key: "core/shared-refused",
            message: "The shared operation was refused.",
        },
    )
    .unwrap();
    idle(&mut run, 1);
}
#[test]
fn shared_public_objects_are_send_sync_and_debug_omits_value_payloads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<orange_runtime::SharedRequest>();
    send_sync::<orange_runtime::SharedReceipt>();
    let mut set = var_node(2, "set-shared", 100);
    set["inputs"] = json!({"value":{"type":"core/string@1","value":"secret-shared-payload"}});
    let (doc, plan) = state_plan(
        vec![core(1, "on-start"), set],
        vec![edge(1, 1, "then", 2, "in")],
        vec![variable(100, "workspace", "core/string@1", None)],
    );
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut run = start(&doc, &plan, &w);
    idle(&mut run, 1);
    let request = run.take_shared().unwrap().unwrap();
    assert!(!format!("{request:?}").contains("secret-shared-payload"));
    let receipt = w.apply_shared(request).unwrap();
    assert!(!format!("{receipt:?}").contains("secret-shared-payload"));
    run.complete_shared(receipt).unwrap();
    idle(&mut run, 1);
}
#[test]
fn completion_quota_refusal_keeps_receipt_pending_without_rolling_back_the_host_commit() {
    let (doc, plan) = fixture("add");
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut options = RunOptions::default();
    options.limits.max_effect_bytes = 16;
    let mut run = Run::start_in_workspace(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        options,
        &w.snapshot(),
    )
    .unwrap();
    idle(&mut run, 1);
    let receipt = w.apply_shared(run.take_shared().unwrap().unwrap()).unwrap();
    let before = run.retention();
    let trace = run.trace().len();
    let refused = run.complete_shared(receipt).unwrap_err();
    assert_eq!(refused.error, SharedError::Limit);
    assert_eq!(run.retention(), before);
    assert_eq!(run.trace().len(), trace);
    assert_eq!(run.workspace_snapshot().unwrap().version(), 0);
    assert_eq!(w.snapshot().version(), 1);
    assert_eq!(run.ownership().effects, 1);
    assert!(run.take_shared().unwrap().is_none());
    // Retry preserves the immutable receipt. For a permanent refusal the host may
    // retire it with a bounded fault, without minting another mutation command.
    let refused = run.complete_shared(refused.value).unwrap_err();
    let fault = orange_compiler::KernelFault {
        code: "core/Refused",
        message_key: "core/shared-refused",
        message: "The shared operation was refused.",
    };
    // Even fault metadata may exceed this deliberately tiny effect limit.
    assert_eq!(
        run.fail_shared_receipt(refused.value, fault)
            .unwrap_err()
            .error,
        SharedError::Limit
    );
    run.stop().unwrap();
    idle(&mut run, 1);
    assert!(run.variable(vid(101)).is_none());
}
#[test]
fn host_can_retire_an_oversized_committed_receipt_with_a_small_graph_fault() {
    let mut update = var_node(2, "update-shared", 100);
    update["config"]["operation"] = json!("append");
    update["inputs"] = json!({"value":{"type":"core/string@1","value":"x"}});
    let (doc, plan) = state_plan(
        vec![core(1, "on-start"), update],
        vec![edge(1, 1, "then", 2, "in")],
        vec![variable(
            100,
            "workspace",
            "core/string@1",
            Some(json!("z".repeat(200))),
        )],
    );
    let mut w = Workspace::new(
        &doc,
        plan.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let mut options = RunOptions::default();
    options.limits.max_effect_bytes = 256;
    let mut run = Run::start_in_workspace(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        options,
        &w.snapshot(),
    )
    .unwrap();
    idle(&mut run, 1);
    let receipt = w.apply_shared(run.take_shared().unwrap().unwrap()).unwrap();
    let refused = run.complete_shared(receipt).unwrap_err();
    assert_eq!(refused.error, SharedError::Limit);
    run.fail_shared_receipt(
        refused.value,
        orange_compiler::KernelFault {
            code: "core/Refused",
            message_key: "core/shared-refused",
            message: "The shared operation was refused.",
        },
    )
    .unwrap();
    idle(&mut run, 1);
    assert_eq!(run.ownership().effects, 0);
    assert_eq!(
        run.trace()
            .back()
            .unwrap()
            .fault
            .as_ref()
            .unwrap()
            .fault
            .code,
        "core/Refused"
    );
    assert_eq!(w.snapshot().version(), 1);
}
