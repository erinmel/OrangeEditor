//! `core/on-variable-changed` entries (C06): one coalesced, equal-aware notification per
//! changed run variable at each segment end, through shared handler admission.
use super::*;
use orange_runtime::{EventOrigin, EventTarget};

const WATCHED: u32 = 100;
const SEEN: u32 = 101;
const PREVIOUS: u32 = 102;
const COUNT: u32 = 103;

fn set_to(n: u32, value: u64) -> Json {
    let mut node = variable_node(n, "set-variable", WATCHED);
    node["inputs"] = json!({"value": {"type": "core/u64@1", "value": value.to_string()}});
    node
}

fn on_changed(n: u32, variable: u32) -> Json {
    let mut node = variable_node(n, "on-variable-changed", variable);
    node["activation"] = activation(0, 8, "fault");
    node
}

/// Start runs `writes` in sequence; the handler records `value`, `previous` and a count.
fn changed_graph(writes: Vec<Json>) -> (Vec<Json>, Vec<Json>) {
    let mut nodes = vec![core(1, "on-start")];
    let mut edges = vec![];
    let mut from = 1;
    for (i, write) in writes.into_iter().enumerate() {
        let n = write["id"].as_str().unwrap().to_owned();
        let to = u32::from_str_radix(&n[n.len() - 12..], 16).unwrap();
        edges.push(edge(i as u32 + 1, from, "then", to, "in"));
        nodes.push(write);
        from = to;
    }
    nodes.extend([
        on_changed(10, WATCHED),
        variable_node(11, "set-variable", SEEN),
        variable_node(12, "set-variable", PREVIOUS),
        update_local(13, COUNT),
    ]);
    edges.extend([
        edge(50, 10, "then", 11, "in"),
        edge(51, 10, "value", 11, "value"),
        edge(52, 11, "then", 12, "in"),
        edge(53, 10, "previous", 12, "value"),
        edge(54, 12, "then", 13, "in"),
    ]);
    (nodes, edges)
}

fn variables() -> Vec<Json> {
    vec![
        variable(WATCHED, "run", Some(0)),
        variable(SEEN, "run", None),
        variable(PREVIOUS, "run", None),
        variable(COUNT, "run", Some(0)),
    ]
}

fn read(run: &Run, n: u32) -> Option<u64> {
    match run.variable(id(n).parse().unwrap()) {
        Some(Value::U64(value)) => Some(*value),
        None => None,
        other => panic!("unexpected value {other:?}"),
    }
}

fn observed(run: &Run) -> (Option<u64>, Option<u64>, Option<u64>) {
    (read(run, SEEN), read(run, PREVIOUS), read(run, COUNT))
}

fn started(writes: Vec<Json>, options: RunOptions) -> Run {
    let (nodes, edges) = changed_graph(writes);
    let (doc, plan) = compile(nodes, edges, variables());
    run_with_options(&doc, plan, options)
}

#[test]
fn one_segment_coalesces_writes_into_one_notification_with_its_start_value() {
    let mut run = started(
        vec![set_to(2, 5), set_to(3, 7), set_to(4, 7)],
        RunOptions::default(),
    );
    pump_until_idle(&mut run);
    assert_eq!(observed(&run), (Some(7), Some(0), Some(1)));
    let last = run.last_event().unwrap();
    assert_eq!(
        last.target,
        EventTarget::VariableChanged(id(10).parse().unwrap())
    );
    assert_eq!(last.origin, EventOrigin::Change);
    assert_eq!((last.source_seq, last.cause_depth), (1, 1));
    assert!(last.cause.is_some());
}

#[test]
fn equal_and_restored_values_are_silent() {
    for writes in [vec![set_to(2, 0)], vec![set_to(2, 5), set_to(3, 0)]] {
        let mut run = started(writes, RunOptions::default());
        pump_until_idle(&mut run);
        assert_eq!(observed(&run), (None, None, Some(0)));
        assert_eq!(run.event_stats().accepted, 0);
    }
}

#[test]
fn host_pump_slices_do_not_split_the_segment() {
    let mut run = started(
        vec![set_to(2, 5), set_to(3, 6), set_to(4, 7)],
        RunOptions::default(),
    );
    for _ in 0..200 {
        let progress = run.pump(1).unwrap();
        if !progress.ready {
            break;
        }
    }
    assert_eq!(observed(&run), (Some(7), Some(0), Some(1)));
}

#[test]
fn a_suspension_ends_the_segment_so_each_side_notifies() {
    let mut run = started(
        vec![set_to(2, 1), wait_node(3, 5), set_to(4, 2)],
        RunOptions::default(),
    );
    pump_until_idle(&mut run);
    assert_eq!(observed(&run), (Some(1), Some(0), Some(1)));
    run.advance_to(5).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(observed(&run), (Some(2), Some(1), Some(2)));
    assert_eq!(run.last_event().unwrap().source_seq, 2);
}

#[test]
fn rewriting_the_delivered_value_does_not_notify_again() {
    // G-16: On V changed → Set V to the same value → no further notification.
    let (mut nodes, mut edges) = changed_graph(vec![set_to(2, 4)]);
    nodes.push(variable_node(14, "set-variable", WATCHED));
    edges.extend([
        edge(55, 13, "then", 14, "in"),
        edge(56, 10, "value", 14, "value"),
    ]);
    let (doc, plan) = compile(nodes, edges, variables());
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    assert_eq!(observed(&run), (Some(4), Some(0), Some(1)));
}

#[test]
fn a_self_changing_handler_chain_stops_at_the_causal_depth_limit() {
    let nodes = vec![
        core(1, "on-start"),
        set_to(2, 1),
        on_changed(10, WATCHED),
        update_local(11, WATCHED),
    ];
    let edges = vec![edge(1, 1, "then", 2, "in"), edge(2, 10, "then", 11, "in")];
    let (doc, plan) = compile(nodes, edges, variables());
    let options = RunOptions {
        max_cause_depth: 3,
        ..Default::default()
    };
    let mut run = run_with_options(&doc, plan, options);
    pump_until_idle(&mut run);
    // Depths 1, 2 and 3 run; the fourth notification is refused without faulting the run.
    assert_eq!(read(&run, WATCHED), Some(4));
    assert_eq!(run.event_stats().accepted, 3);
    assert_eq!(run.event_stats().overflows, 1);
    assert_eq!(run.state(), RunState::Running);
}

#[test]
fn the_watched_variable_needs_run_scope_an_initial_value_and_eq() {
    let (package, registry) = catalog();
    let mut float = variable(WATCHED, "run", None);
    float["type"] = json!("core/f64@1");
    float["init"] = json!({"type": "core/f64@1", "value": "0"});
    for declaration in [
        variable(WATCHED, "local", Some(0)),
        variable(WATCHED, "run", None),
        float,
    ] {
        let source = graph(&package, vec![on_changed(10, WATCHED)], vec![], vec![]);
        let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
        saved["variables"] = json!([declaration.clone()]);
        let document = from_json(&saved.to_string()).unwrap();
        let analysis = analyze(&document, &registry);
        let codes: Vec<_> = analysis
            .report()
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect();
        assert!(
            codes.contains(&"E-VARIABLE-CHANGED"),
            "{declaration}: {codes:?}"
        );
    }
}
