use super::*;
use orange_runtime::{Run, RunOptions, RunState};

fn pump_to_idle(run: &mut Run) {
    for _ in 0..10_000 {
        if !run.pump(1000).unwrap().ready {
            return;
        }
    }
    panic!("bounded fixture did not suspend or retire");
}
fn fork_node(join: &str, width: u16) -> Json {
    let mut node = core(2, "fork");
    node["config"] = json!({"join":join});
    node["variadic"] = json!({"branch":{"order":(0..width).collect::<Vec<_>>(),"next":width}});
    node
}

#[test]
fn stopping_many_waiting_roots_retires_each_timer_once() {
    let (mut nodes, mut edges) = (Vec::new(), Vec::new());
    for i in 0..32 {
        nodes.push(core(i * 2 + 1, "on-start"));
        nodes.push(wait(i * 2 + 2, 10));
        edges.push(edge(i + 1, i * 2 + 1, "then", i * 2 + 2, "in"));
    }
    let (doc, plan) = compiled(nodes, edges);
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    pump_to_idle(&mut run);
    assert_eq!(run.pending_continuations(), 32);
    run.stop().unwrap();
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.outcome == orange_runtime::ControlOutcome::Cancelled)
            .count(),
        32
    );
    let owners = run.ownership();
    assert_eq!(
        (
            owners.activations,
            owners.strands,
            owners.regions,
            owners.timers
        ),
        (0, 0, 0, 0)
    );
}

#[test]
fn race_cancellation_preserves_an_unrelated_activation_timer_and_order() {
    let (doc, plan) = compiled(
        vec![
            core(1, "on-start"),
            fork_node("race", 2),
            wait(3, 1),
            wait(4, 100),
            core(5, "on-start"),
            wait(6, 50),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "branch#0", 3, "in"),
            edge(3, 2, "branch#1", 4, "in"),
            edge(4, 5, "then", 6, "in"),
        ],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    pump_to_idle(&mut run);
    assert_eq!(run.pending_continuations(), 3);
    run.advance_to(1).unwrap();
    pump_to_idle(&mut run);
    assert_eq!(run.pending_continuations(), 1);
    let cancelled = run
        .trace()
        .iter()
        .filter(|e| e.outcome == orange_runtime::ControlOutcome::Cancelled)
        .map(|e| e.node)
        .collect::<Vec<_>>();
    assert_eq!(cancelled, vec![id(4).parse().unwrap()]);
    run.advance_to(50).unwrap();
    pump_to_idle(&mut run);
    assert_eq!(run.ownership().strands, 0);
    assert_eq!(run.pending_continuations(), 0);
    assert!(
        run.trace().iter().any(|e| e.node == id(6).parse().unwrap()
            && e.outcome == orange_runtime::ControlOutcome::Then)
    );
    run.stop().unwrap();
    assert_eq!(run.state(), RunState::Stopped);
}

#[test]
fn wide_pending_read_pull_can_stop_without_losing_queued_ownership() {
    let mut nodes = vec![
        core(1, "on-start"),
        fork_node("all", 128),
        branch(3, false),
        var_node(400, "get-variable", 900),
    ];
    let mut edges = vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "branch#0", 3, "in")];
    for i in 0..64 {
        nodes.push(json!({"id":id(500+i),"definition_id":"org.orange.math/not","definition_version":1,"config_version":1,"config":{},"signature":"not","provider":provider(),"inputs":{"value":{"type":"core/bool@1","value":false}}}));
        edges.push(edge(
            500 + i,
            if i == 0 { 400 } else { 499 + i },
            if i == 0 { "value" } else { "result" },
            500 + i,
            "value",
        ));
    }
    edges.push(edge(700, 563, "result", 3, "condition"));
    let (doc, plan) = state_plan(
        nodes,
        edges,
        vec![variable(900, "run", "core/bool@1", Some(json!(true)))],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    for _ in 0..10 {
        assert!(run.pump(1).unwrap().ready);
    }
    assert_eq!(run.ownership().strands, 129);
    run.stop().unwrap();
    let owners = run.ownership();
    assert_eq!(
        (
            owners.activations,
            owners.strands,
            owners.regions,
            owners.timers
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(run.state(), RunState::Stopped);
}
