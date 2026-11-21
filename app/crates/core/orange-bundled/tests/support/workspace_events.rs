use super::*;
use orange_model::document::OverflowPolicy;
use orange_runtime::{
    Workspace, WorkspaceBus, WorkspaceBusError as Error, WorkspaceBusLimits, WorkspaceLimits,
    WorkspaceQueue,
};
type Fixture = (GraphDocument, Arc<orange_compiler::ControlPlan>);
fn build(nodes: Vec<Json>, edges: Vec<Json>, variables: Vec<Json>, ty: &str) -> Fixture {
    compile_with_event(
        nodes,
        edges,
        variables,
        json!({"id":EVENT,"name":"secret-label","payload":ty,"scope":"workspace"}),
    )
    .unwrap()
}
fn producer(values: &[u64]) -> Fixture {
    let mut nodes = vec![core(1, "on-start")];
    let mut edges = Vec::new();
    let mut previous = 1;
    for (i, value) in values.iter().enumerate() {
        let n = i as u32 + 2;
        nodes.push(emit(n, *value));
        edges.push(edge(n, previous, "then", n, "in"));
        previous = n;
    }
    build(nodes, edges, vec![], "core/u64@1")
}
fn receiver() -> Fixture {
    build(
        vec![
            on_event(1, 0, 16, "fault"),
            variable_node(2, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "payload", 2, "value"),
        ],
        vec![variable(100, "run", None)],
        "core/u64@1",
    )
}
fn host(f: &Fixture, limits: WorkspaceBusLimits) -> (Workspace, WorkspaceBus) {
    let w = Workspace::new(
        &f.0,
        f.1.data().snapshot().clone(),
        WorkspaceLimits::default(),
    )
    .unwrap();
    let b = WorkspaceBus::new(&f.0, f.1.data().snapshot().clone(), &w.snapshot(), limits).unwrap();
    (w, b)
}
fn start(f: &Fixture, w: &Workspace, b: &mut WorkspaceBus, q: WorkspaceQueue) -> Run {
    start_options(f, w, b, q, RunOptions::default())
}
fn start_options(
    f: &Fixture,
    w: &Workspace,
    b: &mut WorkspaceBus,
    q: WorkspaceQueue,
    o: RunOptions,
) -> Run {
    let mut r = Run::start_in_workspace(
        f.1.clone(),
        &f.0,
        f.1.data().snapshot(),
        9,
        o,
        &w.snapshot(),
    )
    .unwrap();
    r.attach_workspace_bus(b, q).unwrap();
    r
}
fn idle(r: &mut Run, b: &mut WorkspaceBus, budget: usize) {
    for _ in 0..20000 {
        if r.state() == RunState::Stopped || !r.pump_with_workspace_bus(b, budget).unwrap().ready {
            return;
        }
    }
    panic!("fixture did not idle");
}
fn drain(r: &Run, b: &mut WorkspaceBus) -> Vec<orange_runtime::WorkspaceDelivery> {
    let mut v = Vec::new();
    while let Some(m) = r.take_workspace_delivery(b).unwrap() {
        v.push(m);
    }
    v
}
#[test]
fn compiled_workspace_emit_reaches_another_run_on_a_later_pump_with_qualified_metadata() {
    for budget in [1, 64] {
        let p = producer(&[7]);
        let target = receiver();
        let (w, mut b) = host(&p, WorkspaceBusLimits::default());
        let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
        let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
        a.advance_to(40).unwrap();
        r.advance_to(9).unwrap();
        idle(&mut a, &mut b, budget);
        assert!(a.take_workspace_delivery(&mut b).unwrap().is_none()); // workspace broadcast targets other runs
        assert!(r.variable(id(100).parse().unwrap()).is_none());
        let m = r.take_workspace_delivery(&mut b).unwrap().unwrap();
        let e = m.envelope();
        assert_eq!((e.source_run, e.target_run), (a.id(), r.id()));
        assert_eq!(e.source_node.to_string(), id(2));
        assert_eq!(
            (
                e.source_seq,
                e.ingestion_seq,
                e.cause_depth,
                e.source_monotonic_ns
            ),
            (1, 1, 1, 40)
        );
        let metadata = r.ingest_workspace(m).unwrap();
        assert_eq!(metadata.monotonic_ns, 9);
        assert_eq!(metadata.cause, Some(e.cause));
        assert_eq!(
            metadata.origin,
            orange_runtime::EventOrigin::Workspace(e.origin())
        );
        assert!(r.variable(id(100).parse().unwrap()).is_none());
        idle(&mut r, &mut b, budget);
        assert_eq!(r.variable(id(100).parse().unwrap()), Some(&Value::U64(7)));
        assert_eq!(b.meter().retention().reserved_items, 0);
    }
}
#[test]
fn plain_pump_and_ordinary_host_ingress_cannot_bypass_workspace_transport() {
    let p = producer(&[7]);
    let target = receiver();
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
    assert!(matches!(
        r.pump(5),
        Err(orange_runtime::RunError::WorkspaceBus(Error::NotAttached))
    ));
    assert!(r.trace().is_empty());
    assert_eq!(
        r.register_event_source(EVENT.parse().unwrap()),
        Err(orange_runtime::IngressError::UnknownEvent)
    );
    let (_, mut other) = host(&p, WorkspaceBusLimits::default());
    assert!(matches!(
        r.pump_with_workspace_bus(&mut other, 5),
        Err(orange_runtime::RunError::WorkspaceBus(Error::WrongOwner))
    ));
}
#[test]
fn every_overflow_mode_has_finite_fifo_or_explicit_loss_behavior() {
    for (policy, expected, accepted) in [
        (OverflowPolicy::Fault, vec![1, 2], 2),
        (OverflowPolicy::DropNewest, vec![1, 2], 4),
        (OverflowPolicy::DropOldest, vec![3, 4], 4),
        (OverflowPolicy::CoalesceLatest, vec![3, 4], 4),
    ] {
        let p = producer(&[1, 2, 3, 4]);
        let target = receiver();
        let (w, mut b) = host(&p, WorkspaceBusLimits::default());
        let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
        let r = start(
            &target,
            &w,
            &mut b,
            WorkspaceQueue {
                capacity: 2,
                max_bytes: 32,
                overflow: policy,
            },
        );
        idle(&mut a, &mut b, 1);
        let got: Vec<_> = drain(&r, &mut b)
            .iter()
            .map(|m| match m.payload() {
                Value::U64(v) => *v,
                _ => panic!(),
            })
            .collect();
        assert_eq!(got, expected);
        assert_eq!(b.stats().accepted, accepted);
        assert_eq!(b.meter().retention().reserved_items, 0);
    }
}
#[test]
fn a_later_fault_recipient_rolls_back_all_earlier_lossy_dispositions() {
    let p = producer(&[1, 2]);
    let target = receiver();
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let r1 = start(
        &target,
        &w,
        &mut b,
        WorkspaceQueue {
            capacity: 1,
            max_bytes: 16,
            overflow: OverflowPolicy::DropOldest,
        },
    );
    let r2 = start(
        &target,
        &w,
        &mut b,
        WorkspaceQueue {
            capacity: 1,
            max_bytes: 16,
            overflow: OverflowPolicy::Fault,
        },
    );
    idle(&mut a, &mut b, 64);
    for r in [&r1, &r2] {
        let m = r.take_workspace_delivery(&mut b).unwrap().unwrap();
        assert_eq!(m.payload(), &Value::U64(1));
        assert_eq!(m.envelope().ingestion_seq, 1);
    }
    assert_eq!(b.stats().accepted, 1);
    assert_eq!(b.stats().dropped, 0);
    assert_eq!(b.stats().refused, 1);
}
#[test]
fn host_drained_deliveries_remain_in_the_global_item_bound() {
    let p = producer(&[1, 2]);
    let target = receiver();
    let (w, mut b) = host(
        &p,
        WorkspaceBusLimits {
            max_retained_items: 1,
            ..WorkspaceBusLimits::default()
        },
    );
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let r = start(&target, &w, &mut b, WorkspaceQueue::default());
    while b.stats().accepted == 0 {
        a.pump_with_workspace_bus(&mut b, 1).unwrap();
    }
    let held = r.take_workspace_delivery(&mut b).unwrap().unwrap();
    assert_eq!(b.stats().queued, 0);
    assert_eq!(b.meter().retention().reserved_items, 1);
    idle(&mut a, &mut b, 1);
    assert_eq!(b.stats().accepted, 1);
    assert_eq!(b.stats().refused, 1);
    drop(held);
    assert_eq!(b.meter().retention().reserved_items, 0);
}
#[test]
fn wrong_recipient_and_delivery_quota_refusals_return_the_authentic_message_for_retry() {
    let p = producer(&[7]);
    let target = receiver();
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
    idle(&mut a, &mut b, 1);
    let m = r.take_workspace_delivery(&mut b).unwrap().unwrap();
    let refused = a.ingest_workspace(m).unwrap_err();
    assert_eq!(refused.error, Error::WrongOwner);
    assert_eq!(b.meter().retention().reserved_items, 1);
    r.ingest_workspace(refused.delivery).unwrap();
    assert_eq!(b.meter().retention().reserved_items, 0);
}
#[test]
fn out_of_order_host_delivery_cannot_move_accepted_bus_sequence_backwards() {
    let p = producer(&[1, 2]);
    let target = receiver();
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
    idle(&mut a, &mut b, 1);
    let mut messages = drain(&r, &mut b);
    r.ingest_workspace(messages.pop().unwrap()).unwrap();
    let refused = r.ingest_workspace(messages.pop().unwrap()).unwrap_err();
    assert_eq!(refused.error, Error::SequenceNotIncreasing);
    assert_eq!(r.last_event().unwrap().source_seq, 1); // sequences belong to each source node
    assert_eq!(r.last_event().unwrap().ingestion_seq, 1);
}
#[test]
fn stop_drop_revoke_and_bus_drop_close_leases_and_refund_after_last_holder() {
    for mode in 0..4 {
        let p = producer(&[1]);
        let target = receiver();
        let (w, mut b) = host(&p, WorkspaceBusLimits::default());
        let meter = b.meter();
        let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
        let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
        idle(&mut a, &mut b, 1);
        let m = r.take_workspace_delivery(&mut b).unwrap().unwrap();
        match mode {
            0 => {
                r.stop().unwrap();
            }
            1 => {
                b.revoke_run(r.id());
            }
            2 => {
                drop(a);
                drop(r);
                drop(b);
                assert_eq!(meter.retention().reserved_items, 1);
                drop(m);
                assert_eq!(meter.retention().reserved_bytes, 0);
                continue;
            }
            _ => {
                drop(b);
                assert_eq!(r.ingest_workspace(m).unwrap_err().error, Error::Closed);
                drop(a);
                drop(r);
                assert_eq!(meter.retention().reserved_bytes, 0);
                continue;
            }
        }
        assert!(matches!(
            r.ingest_workspace(m).unwrap_err().error,
            Error::Closed | Error::NotRunning
        ));
        b.reap();
        drop(a);
        drop(r);
        drop(b);
        assert_eq!(meter.retention().reserved_items, 0);
        assert_eq!(meter.retention().reserved_bytes, 0);
    }
}
#[test]
fn run_ingress_quota_refusal_keeps_delivery_for_retry_after_earlier_work_retires() {
    let p = producer(&[1, 2]);
    let target = receiver();
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let mut options = RunOptions::default();
    options.limits.max_pending_events = 1;
    let mut r = start_options(&target, &w, &mut b, WorkspaceQueue::default(), options);
    idle(&mut a, &mut b, 1);
    let mut messages = drain(&r, &mut b).into_iter();
    r.ingest_workspace(messages.next().unwrap()).unwrap();
    let before = r.event_stats();
    let refused = r.ingest_workspace(messages.next().unwrap()).unwrap_err();
    assert_eq!(refused.error, Error::Limit);
    assert_eq!(r.event_stats(), before);
    assert_eq!(r.last_event().unwrap().ingestion_seq, 1);
    assert_eq!(b.meter().retention().reserved_items, 1);
    idle(&mut r, &mut b, 1);
    r.ingest_workspace(refused.delivery).unwrap();
    idle(&mut r, &mut b, 1);
    assert_eq!(r.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
}
#[test]
fn workspace_await_uses_existing_timeout_ordering_and_stop_owns_its_waiter() {
    let mut awaiter = core(2, "await-event");
    awaiter["config"] = json!({"event":EVENT});
    awaiter["inputs"] =
        json!({"timeout":{"type":"core/option@1<core/duration@1>","value":{"some":"5"}}});
    let target = build(
        vec![
            core(1, "on-start"),
            awaiter,
            variable_node(3, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "payload", 3, "value"),
        ],
        vec![variable(100, "run", None)],
        "core/u64@1",
    );
    for stop in [false, true] {
        let p = producer(&[19]);
        let (w, mut b) = host(&p, WorkspaceBusLimits::default());
        let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
        let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
        idle(&mut r, &mut b, 1);
        assert_eq!(r.ownership().event_waiters, 1);
        idle(&mut a, &mut b, 1);
        let m = r.take_workspace_delivery(&mut b).unwrap().unwrap();
        if stop {
            r.stop().unwrap();
            idle(&mut r, &mut b, 1);
            assert_eq!(r.ownership().event_waiters, 0);
            assert!(r.ingest_workspace(m).is_err());
        } else {
            r.advance_to(5).unwrap();
            r.ingest_workspace(m).unwrap();
            assert_eq!(r.ownership().event_waiters, 0);
            idle(&mut r, &mut b, 1);
            assert_eq!(r.variable(id(100).parse().unwrap()), Some(&Value::U64(19)));
        }
    }
}
#[test]
fn cross_run_causal_lineage_is_preserved_and_depth_is_not_reset_at_the_host_bus() {
    let mut a_nodes = vec![
        core(1, "on-start"),
        emit(2, 7),
        on_event(3, 0, 4, "fault"),
        emit(4, 7),
    ];
    let a_f = build(
        std::mem::take(&mut a_nodes),
        vec![edge(1, 1, "then", 2, "in"), edge(2, 3, "then", 4, "in")],
        vec![],
        "core/u64@1",
    );
    let b_f = build(
        vec![on_event(1, 0, 4, "fault"), emit(2, 7)],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
        "core/u64@1",
    );
    let (w, mut bus) = host(&a_f, WorkspaceBusLimits::default());
    let options = RunOptions {
        max_cause_depth: 2,
        ..RunOptions::default()
    };
    let mut a = start_options(
        &a_f,
        &w,
        &mut bus,
        WorkspaceQueue::default(),
        options.clone(),
    );
    let mut b = start_options(&b_f, &w, &mut bus, WorkspaceQueue::default(), options);
    idle(&mut a, &mut bus, 1);
    let message = b.take_workspace_delivery(&mut bus).unwrap().unwrap();
    assert_eq!(message.envelope().cause_depth, 1);
    b.ingest_workspace(message).unwrap();
    idle(&mut b, &mut bus, 1);
    let message = a.take_workspace_delivery(&mut bus).unwrap().unwrap();
    assert_eq!(message.envelope().cause_depth, 2);
    assert_eq!(message.envelope().source_run, b.id());
    a.ingest_workspace(message).unwrap();
    idle(&mut a, &mut bus, 1);
    assert_eq!(bus.stats().accepted, 2);
    assert_eq!(
        a.trace().back().unwrap().fault.as_ref().unwrap().fault.code,
        "core/RunawayEventChain"
    );
}
fn strings(values: &[&str]) -> Fixture {
    let mut nodes = vec![core(1, "on-start")];
    let mut edges = Vec::new();
    let mut prev = 1;
    for (i, value) in values.iter().enumerate() {
        let n = i as u32 + 2;
        nodes.push(emit_string(n, EVENT, value));
        edges.push(edge(n, prev, "then", n, "in"));
        prev = n;
    }
    build(nodes, edges, vec![], "core/string@1")
}
fn string_receiver() -> Fixture {
    build(
        vec![on_event(1, 0, 4, "fault")],
        vec![],
        vec![],
        "core/string@1",
    )
}
#[test]
fn byte_overflow_drops_only_the_minimal_oldest_prefix_and_oversized_items_are_hard_refusals() {
    let p = strings(&["aaaaaa", "bbbbbb", "cccccccccccc"]);
    let target = string_receiver();
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let r = start(
        &target,
        &w,
        &mut b,
        WorkspaceQueue {
            capacity: 10,
            max_bytes: 50,
            overflow: OverflowPolicy::DropOldest,
        },
    );
    idle(&mut a, &mut b, 1);
    let got: Vec<_> = drain(&r, &mut b)
        .iter()
        .map(|m| m.payload().clone())
        .collect();
    assert_eq!(
        got,
        vec![
            Value::Str(Arc::new("bbbbbb".into())),
            Value::Str(Arc::new("cccccccccccc".into()))
        ]
    );
    assert_eq!(b.stats().dropped, 1);
    let p = strings(&["a string that exceeds a tiny queue"]);
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let _r = start(
        &target,
        &w,
        &mut b,
        WorkspaceQueue {
            capacity: 1,
            max_bytes: 16,
            overflow: OverflowPolicy::DropNewest,
        },
    );
    idle(&mut a, &mut b, 1);
    assert_eq!(b.stats().accepted, 0);
    assert_eq!(b.stats().ingestion_seq, 0);
}
#[test]
fn drained_payload_bytes_remain_reserved_and_block_later_fanout() {
    let p = producer(&[1, 2]);
    let target = receiver();
    let (w, mut probe) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut probe, WorkspaceQueue::default());
    let r = start(&target, &w, &mut probe, WorkspaceQueue::default());
    while probe.stats().accepted == 0 {
        a.pump_with_workspace_bus(&mut probe, 1).unwrap();
    }
    let held = r.take_workspace_delivery(&mut probe).unwrap().unwrap();
    let cap = probe.meter().retention().reserved_bytes;
    drop(held);
    drop(a);
    drop(r);
    drop(probe);
    let (w, mut b) = host(
        &p,
        WorkspaceBusLimits {
            max_retained_bytes: cap,
            ..WorkspaceBusLimits::default()
        },
    );
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let r = start(&target, &w, &mut b, WorkspaceQueue::default());
    while b.stats().accepted == 0 {
        a.pump_with_workspace_bus(&mut b, 1).unwrap();
    }
    let held = r.take_workspace_delivery(&mut b).unwrap().unwrap();
    let before = b.meter().retention();
    idle(&mut a, &mut b, 1);
    assert_eq!(b.stats().accepted, 1);
    assert_eq!(b.meter().retention(), before);
    drop(held);
    assert!(b.meter().retention().reserved_bytes < before.reserved_bytes);
}
#[test]
fn queue_schema_endpoint_and_constructor_limits_refuse_without_consuming_attachment() {
    let p = producer(&[1]);
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut r = Run::start_in_workspace(
        p.1.clone(),
        &p.0,
        p.1.data().snapshot(),
        1,
        RunOptions::default(),
        &w.snapshot(),
    )
    .unwrap();
    for queue in [
        WorkspaceQueue {
            capacity: 0,
            ..WorkspaceQueue::default()
        },
        WorkspaceQueue {
            overflow: OverflowPolicy::Backpressure,
            ..WorkspaceQueue::default()
        },
    ] {
        assert_eq!(r.attach_workspace_bus(&mut b, queue), Err(Error::Contract));
        assert_eq!(b.stats().endpoints, 0);
    }
    r.attach_workspace_bus(&mut b, WorkspaceQueue::default())
        .unwrap();
    assert_eq!(b.stats().endpoints, 1);
    for limits in [
        WorkspaceBusLimits {
            max_events: 0,
            ..WorkspaceBusLimits::default()
        },
        WorkspaceBusLimits {
            max_schema_bytes: 0,
            ..WorkspaceBusLimits::default()
        },
        WorkspaceBusLimits {
            max_retained_bytes: 0,
            ..WorkspaceBusLimits::default()
        },
    ] {
        assert_eq!(
            WorkspaceBus::new(&p.0, p.1.data().snapshot().clone(), &w.snapshot(), limits)
                .unwrap_err(),
            Error::Limit
        );
    }
    let (_, mut full) = host(
        &p,
        WorkspaceBusLimits {
            max_endpoints: 0,
            ..WorkspaceBusLimits::default()
        },
    );
    let mut another = Run::start_in_workspace(
        p.1.clone(),
        &p.0,
        p.1.data().snapshot(),
        1,
        RunOptions::default(),
        &w.snapshot(),
    )
    .unwrap();
    assert_eq!(
        another.attach_workspace_bus(&mut full, WorkspaceQueue::default()),
        Err(Error::WrongOwner)
    ); // different workspace identity
    let mut full = WorkspaceBus::new(
        &p.0,
        p.1.data().snapshot().clone(),
        &w.snapshot(),
        WorkspaceBusLimits {
            max_endpoints: 0,
            ..WorkspaceBusLimits::default()
        },
    )
    .unwrap();
    assert_eq!(
        another.attach_workspace_bus(&mut full, WorkspaceQueue::default()),
        Err(Error::Limit)
    );
    let wrong = strings(&["wrong-type"]);
    let bad_bus = WorkspaceBus::new(
        &wrong.0,
        wrong.1.data().snapshot().clone(),
        &w.snapshot(),
        WorkspaceBusLimits::default(),
    )
    .unwrap();
    let mut bad_bus = bad_bus;
    assert_eq!(
        another.attach_workspace_bus(&mut bad_bus, WorkspaceQueue::default()),
        Err(Error::SchemaMismatch)
    );
}
#[test]
fn workspace_bus_and_delivery_debug_are_redacted_and_types_are_send_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<WorkspaceBus>();
    send_sync::<orange_runtime::WorkspaceDelivery>();
    let p = strings(&["secret-payload-not-debug"]);
    let target = string_receiver();
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let r = start(&target, &w, &mut b, WorkspaceQueue::default());
    idle(&mut a, &mut b, 1);
    let m = r.take_workspace_delivery(&mut b).unwrap().unwrap();
    for formatted in [
        format!("{m:?}"),
        format!("{b:?}"),
        format!("{:?}", b.meter()),
    ] {
        assert!(!formatted.contains("secret-payload-not-debug"));
        assert!(!formatted.contains("secret-label"));
    }
}
#[test]
fn endpoint_drop_reaps_queued_deliveries_and_reuses_only_physical_capacity() {
    let p = producer(&[1]);
    let target = receiver();
    let (w, mut b) = host(
        &p,
        WorkspaceBusLimits {
            max_endpoints: 2,
            ..WorkspaceBusLimits::default()
        },
    );
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let r = start(&target, &w, &mut b, WorkspaceQueue::default());
    idle(&mut a, &mut b, 1);
    assert_eq!(b.meter().retention().reserved_items, 1);
    let old_id = r.id();
    drop(r);
    assert_eq!(b.stats().endpoints, 1);
    b.reap();
    assert_eq!(b.meter().retention().reserved_items, 0);
    let replacement = start(&target, &w, &mut b, WorkspaceQueue::default());
    assert_ne!(replacement.id(), old_id);
    assert_eq!(b.stats().endpoints, 2);
    assert!(
        replacement
            .take_workspace_delivery(&mut b)
            .unwrap()
            .is_none()
    );
}
#[test]
fn bus_causal_and_payload_bounds_refuse_emission_before_global_sequence_publication() {
    for limits in [
        WorkspaceBusLimits {
            max_cause_depth: 0,
            ..WorkspaceBusLimits::default()
        },
        WorkspaceBusLimits {
            max_payload_bytes: 0,
            ..WorkspaceBusLimits::default()
        },
    ] {
        let p = producer(&[1]);
        let target = receiver();
        let (w, mut b) = host(&p, limits);
        let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
        let _r = start(&target, &w, &mut b, WorkspaceQueue::default());
        let before = b.meter().retention();
        idle(&mut a, &mut b, 1);
        assert_eq!(b.stats().accepted, 0);
        assert_eq!(b.stats().ingestion_seq, 0);
        assert_eq!(b.meter().retention(), before);
        assert!(a.trace().back().unwrap().fault.is_some());
    }
}
#[test]
fn workspace_event_plans_are_explicitly_outside_the_recorded_effect_replay_profile() {
    struct Archive;
    impl orange_runtime::ReplayArchive for Archive {
        fn capture(
            &self,
            _: &[Value],
        ) -> Result<orange_runtime::ReplayPayloadRef, orange_runtime::ReplayArchiveError> {
            panic!("must refuse before capture")
        }
        fn resolve(
            &self,
            _: orange_runtime::ReplayPayloadRef,
        ) -> Result<Arc<[Value]>, orange_runtime::ReplayArchiveError> {
            panic!("must refuse before resolve")
        }
    }
    let f = producer(&[1]);
    assert!(matches!(
        orange_runtime::RecordedRun::start(
            f.1.clone(),
            &f.0,
            f.1.data().snapshot(),
            1,
            RunOptions::default(),
            orange_runtime::ReplayOptions::default(),
            Arc::new(Archive)
        ),
        Err(orange_runtime::ReplayError::Unsupported)
    ));
}
#[test]
fn shared_value_commit_can_precede_a_workspace_event_and_receiver_reads_the_published_value() {
    let mut write = variable_node(2, "set-shared", 100);
    write["inputs"] = json!({"value":{"type":"core/u64@1","value":"21"}});
    let mut shared = variable(100, "workspace", Some(0));
    shared["persistent"] = json!(false);
    let p = build(
        vec![core(1, "on-start"), write, emit(3, 1)],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
        vec![shared.clone()],
        "core/u64@1",
    );
    let target = build(
        vec![
            on_event(1, 0, 4, "fault"),
            variable_node(2, "set-variable", 101),
            variable_node(3, "get-variable", 100),
        ],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 3, "value", 2, "value")],
        vec![shared, variable(101, "run", None)],
        "core/u64@1",
    );
    let (mut w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
    idle(&mut a, &mut b, 1);
    let receipt = w.apply_shared(a.take_shared().unwrap().unwrap()).unwrap();
    a.complete_shared(receipt).unwrap();
    idle(&mut a, &mut b, 1);
    r.publish_workspace(&w.snapshot()).unwrap();
    let message = r.take_workspace_delivery(&mut b).unwrap().unwrap();
    r.ingest_workspace(message).unwrap();
    idle(&mut r, &mut b, 1);
    assert_eq!(r.variable(id(101).parse().unwrap()), Some(&Value::U64(21)));
}
#[test]
fn graceful_stop_closes_bus_ingress_but_drains_already_accepted_run_deliveries() {
    let p = producer(&[1, 2]);
    let target = build(
        vec![
            on_event(1, 0, 4, "fault"),
            variable_node(2, "set-variable", 100),
            core(3, "on-stop"),
            emit(4, 99),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "payload", 2, "value"),
            edge(3, 3, "then", 4, "in"),
        ],
        vec![variable(100, "run", None)],
        "core/u64@1",
    );
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
    idle(&mut a, &mut b, 1);
    let mut messages = drain(&r, &mut b).into_iter();
    r.ingest_workspace(messages.next().unwrap()).unwrap();
    r.request_stop(orange_runtime::StopMode::Graceful).unwrap();
    assert_eq!(
        r.ingest_workspace(messages.next().unwrap())
            .unwrap_err()
            .error,
        Error::NotRunning
    );
    for _ in 0..1000 {
        if r.state() == RunState::Stopped {
            break;
        }
        r.pump(1).unwrap();
    }
    assert_eq!(r.state(), RunState::Stopped);
    assert_eq!(r.variable(id(100).parse().unwrap()), Some(&Value::U64(1)));
    assert!(r.trace().iter().any(|e| {
        e.fault
            .as_ref()
            .is_some_and(|f| f.fault.code == "core/RunStopping")
    }));
    assert_eq!(b.stats().accepted, 2);
}
#[test]
fn receiving_handler_drop_policy_is_separate_from_successful_bus_transport() {
    let p = producer(&[1, 2]);
    let mut entry = on_event(1, 0, 4, "fault");
    entry["activation"]["mode"] = json!("drop");
    let target = build(
        vec![entry, variable_node(2, "set-variable", 100)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "payload", 2, "value"),
        ],
        vec![variable(100, "run", None)],
        "core/u64@1",
    );
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
    idle(&mut a, &mut b, 1);
    for message in drain(&r, &mut b) {
        r.ingest_workspace(message).unwrap();
    }
    assert_eq!(r.event_stats().dropped, 1);
    assert_eq!(b.stats().dropped, 0);
    assert_eq!(b.stats().accepted, 2);
    idle(&mut r, &mut b, 1);
    assert_eq!(r.variable(id(100).parse().unwrap()), Some(&Value::U64(1)));
}
#[test]
fn a_paused_run_keeps_its_bus_endpoint_and_handles_deliveries_after_resume() {
    let p = producer(&[7]);
    let target = receiver();
    let (w, mut b) = host(&p, WorkspaceBusLimits::default());
    let mut a = start(&p, &w, &mut b, WorkspaceQueue::default());
    let mut r = start(&target, &w, &mut b, WorkspaceQueue::default());
    r.pause().unwrap();
    idle(&mut a, &mut b, 64);
    let m = r.take_workspace_delivery(&mut b).unwrap().unwrap();
    r.ingest_workspace(m).unwrap();
    assert_eq!(r.pump_with_workspace_bus(&mut b, 64).unwrap().operations, 0);
    assert!(r.variable(id(100).parse().unwrap()).is_none());
    r.resume().unwrap();
    r.pump_with_workspace_bus(&mut b, 1).unwrap();
    idle(&mut r, &mut b, 64);
    assert_eq!(r.variable(id(100).parse().unwrap()), Some(&Value::U64(7)));
}
