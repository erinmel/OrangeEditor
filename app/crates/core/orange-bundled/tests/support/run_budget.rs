//! Run-owned aggregate nominal reservation budget: activations that each fit their
//! per-activation limits are refused together, a refusal changes nothing, and
//! retirement, stop and disposal return capacity.
use super::*;
use crate::budget_drive::{HUGE, boundary, drive, release};
use orange_runtime::{DataError, Run, RunError, RunOptions, RunRetention, RunState, StopMode};
use std::sync::Arc;

type Planned = (
    orange_model::GraphDocument,
    Arc<orange_compiler::ControlPlan>,
);

fn start(n: u32, order: u32) -> Json {
    let mut node = core(n, "on-start");
    node["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":order});
    node
}
fn update(n: u32, var: u32, operation: &str, ty: &str, value: Json) -> Json {
    let mut node = var_node(n, "update-variable", var);
    node["config"]["operation"] = json!(operation);
    node["inputs"] = json!({"value":{"type":ty,"value":value}});
    node
}
fn make(
    planned: &Planned,
    tune: impl Fn(&mut RunOptions),
) -> impl Fn(usize) -> Result<Run, RunError> {
    let (doc, plan) = (planned.0.clone(), planned.1.clone());
    move |limit| {
        let mut options = RunOptions::default();
        options.limits.max_run_retained_bytes = limit;
        tune(&mut options);
        Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options)
    }
}
/// `count` independent roots: start, bump the local cell, then wait.
fn roots(count: u32) -> Planned {
    let (mut nodes, mut edges) = (vec![], vec![]);
    for i in 0..count {
        let base = 10 * (i + 1);
        nodes.extend([
            start(base, i),
            update(base + 1, 100, "add", "core/u64@1", json!("1")),
            wait(base + 2, 10),
        ]);
        edges.push(edge(base, base, "then", base + 1, "in"));
        edges.push(edge(base + 1, base + 1, "then", base + 2, "in"));
    }
    state_plan(
        nodes,
        edges,
        vec![variable(100, "local", "core/u64@1", Some(json!("0")))],
    )
}

#[test]
fn roots_that_each_fit_their_limits_are_refused_together_and_return_capacity() {
    let planned = roots(3);
    // One 16-byte local cell per activation: every root fits max_state_bytes alone.
    let tune = |o: &mut RunOptions| o.limits.max_state_bytes = 16;
    let one = roots(1);
    assert!(make(&one, tune)(HUGE).is_ok());

    let start = make(&planned, tune);
    assert_eq!(boundary(&start), RunError::AggregateLimit);
    let mut run = start(48).unwrap();
    assert_eq!(
        run.retention(),
        RunRetention {
            reserved_bytes: 48,
            limit_bytes: 48
        }
    );
    // Cancellation: stopping while every root waits returns all of it.
    run.pump(100).unwrap();
    assert_eq!(run.retention().reserved_bytes, 48);
    run.request_stop(StopMode::Immediate).unwrap();
    for _ in 0..100 {
        if run.state() == RunState::Stopped {
            break;
        }
        run.pump(100).unwrap();
    }
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.ownership().activations, 0);
    assert_eq!(run.retention().reserved_bytes, 0);
    run.dispose().unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);

    // Completion alone retires roots and returns their capacity before stop.
    let mut run = start(48).unwrap();
    drive(&mut run, 48).unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);
    release(run);
}

#[test]
fn growing_local_cells_across_roots_are_refused_with_the_typed_error() {
    // Each root replaces its own string cell three times; every cell fits alone.
    let (mut nodes, mut edges) = (vec![], vec![]);
    for i in 0..2 {
        let base = 10 * (i + 1);
        nodes.extend([start(base, i), count(base + 1, 3)]);
        nodes.push(update(
            base + 2,
            100,
            "append",
            "core/string@1",
            json!("0123456789"),
        ));
        edges.push(edge(base, base, "then", base + 1, "in"));
        edges.push(edge(base + 1, base + 1, "body", base + 2, "in"));
    }
    let planned = state_plan(
        nodes,
        edges,
        vec![variable(100, "local", "core/string@1", Some(json!("")))],
    );
    let tune = |o: &mut RunOptions| o.limits.max_state_bytes = 4096;
    assert_eq!(boundary(make(&planned, tune)), RunError::AggregateLimit);
}

#[test]
fn pending_data_pulls_charge_their_cached_slots_and_refuse_atomically() {
    // A chain of dependent additions caches one dynamic slot per node while the
    // one-unit pump leaves the pull pending between slices.
    let mut nodes = vec![
        start(1, 0),
        var_node(2, "get-variable", 100),
        var_node(3, "set-variable", 101),
    ];
    let mut edges = vec![edge(1, 1, "then", 3, "in"), edge(2, 2, "value", 10, "a")];
    for i in 0..12 {
        nodes.push(math(10 + i, "add", 0, 1));
        if i > 0 {
            edges.push(edge(20 + i, 9 + i, "sum", 10 + i, "a"));
        }
    }
    edges.push(edge(50, 21, "sum", 3, "value"));
    let planned = state_plan(
        nodes,
        edges,
        vec![
            variable(100, "run", "core/u64@1", Some(json!("1"))),
            variable(101, "run", "core/u64@1", None),
        ],
    );
    let make = make(&planned, |_| {});
    assert_eq!(boundary(&make), RunError::Data(DataError::AggregateLimit));
    let mut run = make(HUGE).unwrap();
    drive(&mut run, HUGE).unwrap();
    assert_eq!(
        run.variable(id(101).parse().unwrap()),
        Some(&orange_model::Value::U64(13))
    );
    // A refused pull publishes nothing: the target keeps its value and the fault is typed.
    let mut run = make(16).unwrap();
    let error = RunError::Data(DataError::AggregateLimit);
    assert_eq!(drive(&mut run, 16), Err(error));
    assert_eq!(run.last_error(), Some(error));
    assert_eq!(run.variable(id(101).parse().unwrap()), None);
    release(run);
}

#[test]
fn iteration_frames_are_replaced_in_place_under_the_aggregate_budget() {
    let mut list = core(2, "for-each");
    list["config"] = json!({"mode":"list"});
    list["type_pins"] = json!({"T":"core/u64@1"});
    let items: Vec<String> = (0..64).map(|i| i.to_string()).collect();
    list["inputs"] = json!({"items":{"type":"core/list@1<core/u64@1>","value":items}});
    let planned = state_plan(
        vec![
            start(1, 0),
            list,
            update(3, 100, "add", "core/u64@1", json!("0")),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 2, "element", 3, "value"),
        ],
        vec![variable(100, "run", "core/u64@1", Some(json!("0")))],
    );
    // Replacing the iteration frame in place credits its own hold, so the replacement
    // is not charged twice: the smallest admitting limit completes all 64 iterations.
    let refusal = boundary(make(&planned, |_| {}));
    assert!(matches!(
        refusal,
        RunError::AggregateLimit | RunError::Data(DataError::AggregateLimit)
    ));
}
