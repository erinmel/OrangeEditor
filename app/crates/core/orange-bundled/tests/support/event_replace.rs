//! Replace keeps one latest pending delivery and retires the old root before dispatch.
use super::*;
use orange_model::document::ReentrancyMode;
use orange_runtime::StopMode;

pub(super) fn replacement(n: u32, capacity: u32, overflow: &str) -> Json {
    let mut node = on_event(n, 0, capacity, overflow);
    node["activation"]["mode"] = json!("replace");
    node
}

pub(super) fn quiesce(run: &mut Run) {
    for _ in 0..20_000 {
        if run.state() == RunState::Stopped || !run.pump(1).unwrap().ready {
            return;
        }
    }
    panic!("Replace fixture stayed ready: {:?}", run.event_stats());
}

pub(super) fn release(mut run: Run) {
    if run.state() != RunState::Stopped {
        run.request_stop(StopMode::Immediate).unwrap();
        quiesce(&mut run);
    }
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.ownership().activations, 0);
    assert_eq!(run.ownership().strands, 0);
    assert_eq!(run.ownership().regions, 0);
    assert_eq!(run.ownership().timers, 0);
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    assert_eq!(run.retention().reserved_bytes, 0);
    run.dispose().unwrap();
}

fn set_value(n: u32, variable: u32, value: u64) -> Json {
    let mut node = variable_node(n, "set-variable", variable);
    node["inputs"] = json!({"value":{"type":"core/u64@1","value":value.to_string()}});
    node
}

pub(super) fn suspended_fixture(
    later_values: &[u64],
    cleanup_steps: usize,
) -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    assert!(!later_values.is_empty() && later_values.len() <= 3);
    assert!(cleanup_steps > 0 && cleanup_steps <= 8);
    let mut nodes = vec![
        core(1, "on-start"),
        emit(2, 1),
        wait_node(3, 5),
        replacement(10, 8, "fault"),
        wait_node(11, 100),
        variable_node(12, "set-variable", 100),
        set_value(30, 102, 99),
    ];
    let mut edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 10, "then", 11, "in"),
        edge(4, 11, "then", 12, "in"),
        edge(5, 10, "payload", 12, "value"),
        edge(6, 4, "failed", 30, "in"),
    ];
    let mut predecessor = 3;
    for (i, value) in later_values.iter().enumerate() {
        let n = 4 + i as u32;
        nodes.push(emit(n, *value));
        edges.push(edge(edges.len() as u32 + 1, predecessor, "then", n, "in"));
        predecessor = n;
    }
    predecessor = 11;
    for i in 0..cleanup_steps {
        let n = 20 + i as u32;
        nodes.push(update_local(n, 101));
        let output = if i == 0 { "cancelled" } else { "then" };
        edges.push(edge(edges.len() as u32 + 1, predecessor, output, n, "in"));
        predecessor = n;
    }
    compile(
        nodes,
        edges,
        vec![
            variable(100, "run", Some(0)),
            variable(101, "run", Some(0)),
            variable(102, "run", Some(0)),
        ],
    )
}

fn reached(run: &Run, node: u32, outcome: ControlOutcome) -> bool {
    run.trace()
        .iter()
        .any(|item| item.node.to_string() == id(node) && item.outcome == outcome)
}

fn pump_until(run: &mut Run, condition: impl Fn(&Run) -> bool) {
    for _ in 0..20_000 {
        if condition(run) {
            return;
        }
        run.pump(1).unwrap();
    }
    panic!("Replace fixture did not reach condition: {:?}", run.trace());
}

#[test]
fn self_emit_defers_cancellation_until_actor_ownership_and_terminal_roots_retire_naturally() {
    for terminal in [false, true] {
        let mut nodes = vec![
            core(1, "on-start"),
            emit(2, 1),
            replacement(10, 8, "fault"),
            emit(11, 2),
        ];
        let mut edges = vec![edge(1, 1, "then", 2, "in"), edge(2, 10, "then", 11, "in")];
        if !terminal {
            nodes.push(set_value(12, 100, 9));
            edges.push(edge(3, 11, "then", 12, "in"));
        }
        let (doc, plan) = compile(nodes, edges, vec![variable(100, "run", Some(0))]);
        let mut run = run(&doc, plan);
        pump_until(&mut run, |run| run.event_stats().delivered >= 2);
        assert_eq!(run.event_stats().delivered, 2, "terminal={terminal}");
        assert_eq!(
            run.event_stats().pending,
            0,
            "latest delivery should have started"
        );
        if !terminal {
            assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(0)));
            assert!(!reached(&run, 12, ControlOutcome::Then));
        }
        assert!(reached(&run, 11, ControlOutcome::Then));
        release(run);
    }
}

#[test]
fn a_replacement_waits_for_the_old_root_cleanup_and_keeps_its_payload_reserved() {
    let (doc, plan) = suspended_fixture(&[2], 1);
    let mut run = run(&doc, plan);
    quiesce(&mut run);
    let charge = run.event_stats().live_payload_bytes;
    assert!(charge > 0);
    run.advance_to(5).unwrap();
    pump_until(&mut run, |run| reached(run, 11, ControlOutcome::Cancelled));
    assert!(
        !reached(&run, 20, ControlOutcome::Then),
        "cleanup ran inline in Emit"
    );
    assert_eq!(
        (
            run.event_stats().accepted,
            run.event_stats().delivered,
            run.event_stats().pending
        ),
        (2, 1, 1)
    );
    assert_eq!(run.event_stats().live_payload_bytes, 2 * charge);
    assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(0)));
    quiesce(&mut run);
    assert_eq!(run.event_stats().delivered, 2);
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(1)));
    let cleanup = run
        .trace()
        .iter()
        .position(|e| e.node.to_string() == id(20))
        .unwrap();
    let replacement = run
        .trace()
        .iter()
        .enumerate()
        .filter(|(_, e)| e.node.to_string() == id(10))
        .nth(1)
        .unwrap()
        .0;
    assert!(cleanup < replacement, "new root entered before old cleanup");
    assert_eq!(run.event_stats().live_payload_bytes, charge);
    run.advance_to(105).unwrap();
    quiesce(&mut run);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
    assert_eq!(run.cleanup_truncated(), 0);
    release(run);
}

#[test]
fn newer_pending_replacements_do_not_recancel_or_replenish_active_cleanup() {
    let (doc, plan) = suspended_fixture(&[2, 3, 4], 8);
    let mut run = run(&doc, plan);
    quiesce(&mut run);
    run.advance_to(5).unwrap();
    quiesce(&mut run);
    let stats = run.event_stats();
    assert_eq!(
        (stats.accepted, stats.delivered, stats.pending),
        (4, 2, 0),
        "{:?}",
        run.trace()
    );
    assert_eq!((stats.dropped, stats.overflows), (2, 0));
    assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(8)));
    assert_eq!(run.cleanup_truncated(), 0);
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.node.to_string() == id(11) && e.outcome == ControlOutcome::Cancelled)
            .count(),
        1
    );
    let cancelled = run
        .trace()
        .iter()
        .position(|e| e.node.to_string() == id(11) && e.outcome == ControlOutcome::Cancelled)
        .unwrap();
    let last_cleanup = run
        .trace()
        .iter()
        .position(|e| e.node.to_string() == id(27) && e.outcome == ControlOutcome::Then)
        .unwrap();
    for emitter in [5, 6] {
        let emission = run
            .trace()
            .iter()
            .position(|e| e.node.to_string() == id(emitter))
            .unwrap();
        assert!(
            cancelled < emission && emission < last_cleanup,
            "latest update did not overlap old cleanup"
        );
    }
    run.advance_to(105).unwrap();
    quiesce(&mut run);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(4)));
    release(run);
}

#[test]
fn zero_cleanup_fuel_or_deadline_retires_the_old_root_without_leaking_latest() {
    let (doc, plan) = suspended_fixture(&[2], 2);
    for options in [
        RunOptions {
            max_cleanup_operations: 0,
            ..RunOptions::default()
        },
        RunOptions {
            cleanup_timeout_ns: 0,
            ..RunOptions::default()
        },
    ] {
        let mut run = run_with_options(&doc, plan.clone(), options);
        quiesce(&mut run);
        run.advance_to(5).unwrap();
        quiesce(&mut run);
        assert_eq!(
            (run.event_stats().delivered, run.event_stats().pending),
            (2, 0)
        );
        assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(0)));
        assert_eq!(run.cleanup_truncated(), 1);
        run.advance_to(105).unwrap();
        quiesce(&mut run);
        assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
        release(run);
    }
}

#[test]
fn replace_keeps_only_latest_before_dispatch_for_every_capacity_and_overflow() {
    for capacity in [1, 8] {
        for overflow in ["fault", "drop-newest", "drop-oldest", "coalesce-latest"] {
            let (doc, plan) = compile(
                vec![
                    core(1, "on-start"),
                    emit(2, 1),
                    emit(3, 2),
                    emit(4, 3),
                    replacement(10, capacity, overflow),
                    variable_node(11, "set-variable", 100),
                ],
                vec![
                    edge(1, 1, "then", 2, "in"),
                    edge(2, 2, "then", 3, "in"),
                    edge(3, 3, "then", 4, "in"),
                    edge(4, 10, "then", 11, "in"),
                    edge(5, 10, "payload", 11, "value"),
                ],
                vec![variable(100, "run", None)],
            );
            assert_eq!(
                plan.events()[0].handlers()[0].mode(),
                ReentrancyMode::Replace
            );
            assert_eq!(plan.events()[0].handlers()[0].max_parallel(), 1);
            let mut options = RunOptions::default();
            options.limits.max_activations = 1;
            let mut run = run_with_options(&doc, plan, options);
            for _ in 0..200 {
                if run
                    .trace()
                    .iter()
                    .any(|event| event.node.to_string() == id(4))
                {
                    break;
                }
                run.pump(1).unwrap();
            }
            let stats = run.event_stats();
            assert_eq!(
                (stats.accepted, stats.delivered, stats.pending),
                (3, 0, 1),
                "{capacity}/{overflow}"
            );
            assert_eq!(
                (stats.dropped, stats.overflows),
                (2, 0),
                "{capacity}/{overflow}"
            );
            assert_eq!(stats.live_payload_bytes, stats.pending_bytes);
            quiesce(&mut run);
            assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(3)));
            assert_eq!(run.event_stats().delivered, 1);
            release(run);
        }
    }
}
