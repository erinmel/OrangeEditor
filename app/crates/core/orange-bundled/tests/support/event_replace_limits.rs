//! Replace admission is atomic across payload limits and every fanout target.
use super::event_replace::{quiesce, release, replacement, suspended_fixture};
use super::*;
use orange_runtime::{RunLimits, StopMode};

fn reached(run: &Run, node: u32, outcome: ControlOutcome) -> bool {
    run.trace()
        .iter()
        .any(|event| event.node.to_string() == id(node) && event.outcome == outcome)
}

fn until(run: &mut Run, condition: impl Fn(&Run) -> bool) {
    for _ in 0..20_000 {
        if condition(run) {
            return;
        }
        run.pump(1).unwrap();
    }
    panic!(
        "Replace limit fixture did not reach boundary: {:?}",
        run.trace()
    );
}

fn assert_empty(run: &Run) {
    assert_eq!(run.ownership().activations, 0);
    assert_eq!(run.ownership().strands, 0);
    assert_eq!(run.ownership().regions, 0);
    assert_eq!(run.ownership().timers, 0);
    let events = run.event_stats();
    assert_eq!(events.pending, 0);
    assert_eq!(events.pending_bytes, 0);
    assert_eq!(events.live_payload_bytes, 0);
    assert_eq!(run.retention().reserved_bytes, 0);
}

#[test]
fn exact_old_plus_new_live_payload_limit_accepts_and_one_byte_less_refuses_without_cancel() {
    let (doc, plan) = suspended_fixture(&[2], 1);
    let mut baseline = run(&doc, plan.clone());
    quiesce(&mut baseline);
    let old_charge = baseline.event_stats().live_payload_bytes;
    assert!(old_charge > 0);
    assert_eq!(
        (
            baseline.event_stats().accepted,
            baseline.event_stats().delivered
        ),
        (1, 1)
    );
    release(baseline);

    for accepts in [true, false] {
        let options = RunOptions {
            limits: RunLimits {
                max_live_event_payload_bytes: 2 * old_charge - usize::from(!accepts),
                ..RunLimits::default()
            },
            ..RunOptions::default()
        };
        let mut run = run_with_options(&doc, plan.clone(), options);
        quiesce(&mut run);
        assert_eq!(run.event_stats().live_payload_bytes, old_charge);
        run.advance_to(5).unwrap();
        until(&mut run, |run| {
            reached(run, 4, ControlOutcome::Then) || reached(run, 4, ControlOutcome::Failed)
        });
        if accepts {
            assert!(reached(&run, 4, ControlOutcome::Then));
            assert_eq!(run.event_stats().accepted, 2);
            assert_eq!(run.event_stats().delivered, 1);
            assert_eq!(run.event_stats().pending, 1);
            assert_eq!(run.event_stats().live_payload_bytes, 2 * old_charge);
            assert!(reached(&run, 11, ControlOutcome::Cancelled));
            quiesce(&mut run);
            assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(1)));
            assert_eq!(run.event_stats().overflows, 0);
            run.advance_to(105).unwrap();
            quiesce(&mut run);
            assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
        } else {
            assert!(run.trace().iter().any(|event| {
                event.node.to_string() == id(4)
                    && event.outcome == ControlOutcome::Failed
                    && event
                        .fault
                        .as_ref()
                        .is_some_and(|f| f.fault.code == "core/LimitExceeded")
            }));
            quiesce(&mut run);
            // A connected Failed path proves any stop/cancel would be caused by Replace,
            // not an unrelated stop-run policy following a rejected Emit.
            assert_eq!(run.state(), RunState::Running);
            assert_eq!(
                run.variable(id(102).parse().unwrap()),
                Some(&Value::U64(99))
            );
            assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(0)));
            assert!(!reached(&run, 11, ControlOutcome::Cancelled));
            assert_eq!(
                (
                    run.event_stats().accepted,
                    run.event_stats().delivered,
                    run.event_stats().pending
                ),
                (1, 1, 0)
            );
            assert_eq!(run.event_stats().live_payload_bytes, old_charge);
            assert_eq!(run.event_stats().overflows, 1);
            assert_eq!(run.last_event().unwrap().ingestion_seq, 1);
            run.advance_to(100).unwrap();
            quiesce(&mut run);
            assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(1)));
            assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(0)));
        }
        assert_eq!(run.event_stats().live_payload_bytes, 0);
        release(run);
    }
}

fn fanout_fixture(
    after_new_root_starts: bool,
) -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    let mut replace = replacement(10, 8, "fault");
    replace["activation"]["order"] = json!(0);
    let mut failed = variable_node(7, "set-variable", 103);
    failed["inputs"] = json!({"value":{"type":"core/u64@1","value":"99"}});
    let mut nodes = vec![
        core(1, "on-start"),
        emit(2, 1),
        wait_node(3, 5),
        emit(4, 2),
        emit(6, 3),
        failed,
        replace,
        wait_node(11, 10),
        variable_node(12, "set-variable", 100),
        on_event(30, 1, 1, "fault"),
        wait_node(31, 100),
        variable_node(32, "set-variable", 102),
    ];
    let mut edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 6, "failed", 7, "in"),
        edge(5, 10, "then", 11, "in"),
        edge(6, 11, "then", 12, "in"),
        edge(7, 10, "payload", 12, "value"),
        edge(8, 30, "then", 31, "in"),
        edge(9, 31, "then", 32, "in"),
        edge(10, 30, "payload", 32, "value"),
    ];
    if after_new_root_starts {
        nodes.push(wait_node(5, 1));
        edges.push(edge(11, 4, "then", 5, "in"));
        edges.push(edge(12, 5, "then", 6, "in"));
    } else {
        edges.push(edge(11, 4, "then", 6, "in"));
    }
    let mut previous = 11;
    for step in 0..4 {
        let n = 20 + step;
        nodes.push(update_local(n, 101));
        edges.push(edge(
            100 + step,
            previous,
            if step == 0 { "cancelled" } else { "then" },
            n,
            "in",
        ));
        previous = n;
    }
    compile(
        nodes,
        edges,
        vec![
            variable(100, "run", Some(0)),
            variable(101, "run", Some(0)),
            variable(102, "run", Some(0)),
            variable(103, "run", Some(0)),
        ],
    )
}

fn fanout_refusal(after_new_root_starts: bool) {
    let (doc, plan) = fanout_fixture(after_new_root_starts);
    let mut run = run(&doc, plan);
    quiesce(&mut run);
    assert_eq!(
        (run.event_stats().accepted, run.event_stats().delivered),
        (2, 2)
    );
    run.advance_to(5).unwrap();
    if after_new_root_starts {
        quiesce(&mut run);
        assert_eq!(
            (run.event_stats().delivered, run.event_stats().pending),
            (3, 1)
        );
        assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(4)));
        run.advance_to(6).unwrap();
    }
    until(&mut run, |run| reached(run, 6, ControlOutcome::Failed));
    assert_eq!(run.event_stats().accepted, 4);
    assert_eq!(
        run.event_stats().dropped,
        0,
        "failed fanout must not evict the earlier Replace pending payload"
    );
    assert_eq!(run.last_event().unwrap().ingestion_seq, 2);
    if after_new_root_starts {
        assert_eq!(
            (run.event_stats().delivered, run.event_stats().pending),
            (3, 1)
        );
    } else {
        assert_eq!(
            (run.event_stats().delivered, run.event_stats().pending),
            (2, 2)
        );
    }
    quiesce(&mut run);
    assert_eq!(run.state(), RunState::Running);
    assert_eq!(
        run.variable(id(103).parse().unwrap()),
        Some(&Value::U64(99))
    );
    assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(4)));
    assert_eq!(
        run.trace()
            .iter()
            .filter(|event| event.node.to_string() == id(11)
                && event.outcome == ControlOutcome::Cancelled)
            .count(),
        1,
        "the refused C emission must not request cancellation of active replacement B"
    );
    run.advance_to(15).unwrap();
    quiesce(&mut run);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
    run.advance_to(100).unwrap();
    quiesce(&mut run);
    assert_eq!(run.variable(id(102).parse().unwrap()), Some(&Value::U64(1)));
    run.advance_to(200).unwrap();
    quiesce(&mut run);
    assert_eq!(run.variable(id(102).parse().unwrap()), Some(&Value::U64(2)));
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
    assert_eq!(
        (
            run.event_stats().accepted,
            run.event_stats().delivered,
            run.event_stats().pending
        ),
        (4, 4, 0)
    );
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    assert_eq!(run.event_stats().overflows, 1);
    release(run);
}

#[test]
fn later_fault_fanout_preserves_the_earlier_pending_replacement() {
    fanout_refusal(false);
}

#[test]
fn later_fault_fanout_does_not_cancel_the_earlier_active_replacement() {
    fanout_refusal(true);
}

#[test]
fn graceful_and_immediate_stop_finish_replace_bookkeeping_before_disposal() {
    let (doc, plan) = suspended_fixture(&[2], 2);
    for mode in [StopMode::Graceful, StopMode::Immediate] {
        let mut run = run(&doc, plan.clone());
        quiesce(&mut run);
        run.advance_to(5).unwrap();
        until(&mut run, |run| reached(run, 11, ControlOutcome::Cancelled));
        assert_eq!(
            (run.event_stats().delivered, run.event_stats().pending),
            (1, 1)
        );
        run.request_stop(mode).unwrap();
        quiesce(&mut run);
        if mode == StopMode::Graceful {
            assert_eq!(run.state(), RunState::Draining);
            assert_eq!(run.event_stats().delivered, 2);
            run.advance_to(105).unwrap();
            quiesce(&mut run);
            assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
            assert_eq!(run.event_stats().dropped, 0);
        } else {
            assert_eq!(run.event_stats().delivered, 1);
            assert_eq!(run.event_stats().dropped, 1);
            assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(0)));
        }
        assert_eq!(run.state(), RunState::Stopped);
        assert_empty(&run);
        run.dispose().unwrap();
        assert_eq!(run.state(), RunState::Disposed);
        assert_empty(&run);
    }
}

#[test]
fn stop_run_fault_clears_a_pending_replacement_while_old_cleanup_is_owned() {
    let mut start = core(1, "on-start");
    start["activation"]["on_fault"] = json!("stop-run");
    let (doc, plan) = compile(
        vec![
            start,
            emit(2, 1),
            wait_node(3, 5),
            emit(4, 2),
            variable_node(6, "get-variable", 103),
            variable_node(7, "set-variable", 104),
            replacement(10, 8, "fault"),
            wait_node(11, 100),
            variable_node(12, "set-variable", 100),
            update_local(20, 101),
            update_local(21, 101),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 4, "then", 7, "in"),
            edge(5, 6, "value", 7, "value"),
            edge(6, 10, "then", 11, "in"),
            edge(7, 11, "then", 12, "in"),
            edge(8, 10, "payload", 12, "value"),
            edge(9, 11, "cancelled", 20, "in"),
            edge(10, 20, "then", 21, "in"),
        ],
        vec![
            variable(100, "run", Some(0)),
            variable(101, "run", Some(0)),
            variable(103, "run", None),
            variable(104, "run", Some(0)),
        ],
    );
    let mut run = run(&doc, plan);
    quiesce(&mut run);
    run.advance_to(5).unwrap();
    until(&mut run, |run| reached(run, 4, ControlOutcome::Then));
    assert_eq!(
        (run.event_stats().delivered, run.event_stats().pending),
        (1, 1)
    );
    quiesce(&mut run);
    assert!(reached(&run, 7, ControlOutcome::Failed));
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.event_stats().delivered, 1);
    assert_eq!(run.event_stats().dropped, 1);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(0)));
    assert_eq!(run.variable(id(104).parse().unwrap()), Some(&Value::U64(0)));
    assert_empty(&run);
    run.dispose().unwrap();
    assert_eq!(run.state(), RunState::Disposed);
    assert_empty(&run);
}
