//! Representative workloads. Each checks its own result after every run, so a number
//! is only reported for a workload that did the work it names.

use crate::graph::*;
use crate::harness::{Config, DataInput, DataSession, Row, RunSession, measure};
use orange_compiler::{ControlPlan, DataPlan};
use orange_model::id::VariableId;
use orange_model::value::{ListRepr, MapRepr};
use orange_model::{GraphDocument, Value};
use orange_runtime::{DataActivation, DataError, DataLimits, DataOptions, DataPoll};
use serde_json::{Value as Json, json};
use std::sync::Arc;

const ITERATIONS: u64 = 1000;
const TOTAL: u32 = 100;

fn total() -> VariableId {
    id(TOTAL).parse().expect("variable ID")
}

/// Count(1000) whose body sets a run variable through a 16-deep `add` chain fed by Get.
pub fn scalar_chain(config: Config) -> Row {
    const DEPTH: u32 = 16;
    let mut nodes = vec![
        core(1, "on-start"),
        count(2, ITERATIONS),
        var_node(3, "get-variable", TOTAL),
        var_node(4, "set-variable", TOTAL),
    ];
    let mut edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "body", 4, "in"),
        edge(3, 3, "value", 10, "a"),
        edge(4, 10 + DEPTH - 1, "sum", 4, "value"),
    ];
    for link in 0..DEPTH {
        nodes.push(math(10 + link, "add", 0, 1));
        if link > 0 {
            edges.push(edge(10 + link, 10 + link - 1, "sum", 10 + link, "a"));
        }
    }
    let mut source = Source::new(nodes, edges);
    source.variables = vec![run_variable(TOTAL, U64, Some(json!("0")))];
    let (document, plan) = source.control();
    measure(
        format!("scalar chain ({DEPTH} adds)"),
        ITERATIONS,
        "iterations",
        config,
        || RunSession::start(&plan, &document, total(), config.budget),
        |(value, wakeups)| {
            assert_eq!(value, Some(Value::U64(ITERATIONS * u64::from(DEPTH))));
            assert_eq!(wakeups, 0);
        },
    )
}

/// Count(1000) whose body calls a graph-local action function and stores its result, so
/// every iteration enters one call frame, writes a fresh local cell and returns. The
/// function sets a local to its argument, then returns that local plus one.
fn function_call_graph() -> (GraphDocument, Arc<ControlPlan>) {
    const LOCAL: u32 = 101;
    let function = json!({"id":id(200),"name":"increment","kind":"action",
        "inputs":[port("value","input",U64)],"outputs":[port("result","output",U64)],
        "nodes":[core(201,"function-entry"),var_node(202,"set-variable",LOCAL),var_node(203,"get-variable",LOCAL),
            math(204,"add",0,1),core(205,"return")],
        "edges":[edge(201,201,"then",202,"in"),edge(202,201,"value",202,"value"),edge(203,202,"then",205,"in"),
            edge(204,203,"value",204,"a"),edge(205,204,"sum",205,"result")]});
    let mut local = run_variable(LOCAL, U64, Some(json!("0")));
    local["scope"] = json!("local");
    let mut call = core(4, "call");
    call["config"] = json!({"function":id(200)});
    let mut source = Source::new(
        vec![
            core(1, "on-start"),
            count(2, ITERATIONS),
            var_node(3, "get-variable", TOTAL),
            call,
            var_node(5, "set-variable", TOTAL),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 4, "in"),
            edge(3, 3, "value", 4, "value"),
            edge(4, 4, "then", 5, "in"),
            edge(5, 4, "result", 5, "value"),
        ],
    );
    source.variables = vec![run_variable(TOTAL, U64, Some(json!("0"))), local];
    source.functions = vec![function];
    source.control()
}

pub fn function_call(config: Config) -> Row {
    let (document, plan) = function_call_graph();
    measure(
        "function call + return".into(),
        ITERATIONS,
        "calls",
        config,
        || RunSession::start(&plan, &document, total(), config.budget),
        |(value, wakeups)| {
            assert_eq!(value, Some(Value::U64(ITERATIONS)));
            assert_eq!(wakeups, 0);
        },
    )
}

fn for_each_list(n: u32, element: &str, items: Json) -> Json {
    let mut node = core(n, "for-each");
    node["config"] = json!({"mode":"list"});
    node["type_pins"] = json!({"T":element});
    node["inputs"] = json!({"items":{"type":format!("core/list@1<{element}>"),"value":items}});
    node
}

/// For-each over 32 lists, each with an inner For-each over 32 items that adds the item.
pub fn nested_for_each(config: Config) -> Row {
    const SIDE: u64 = 32;
    let outer: Vec<Vec<String>> = (0..SIDE)
        .map(|row| (0..SIDE).map(|i| (row * SIDE + i).to_string()).collect())
        .collect();
    let mut source = Source::new(
        vec![
            core(1, "on-start"),
            for_each_list(2, "core/list@1<core/u64@1>", json!(outer)),
            for_each_list(3, U64, json!([])),
            update(4, TOTAL, "add", U64, json!("0")),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 2, "element", 3, "items"),
            edge(4, 3, "body", 4, "in"),
            edge(5, 3, "element", 4, "value"),
        ],
    );
    source.variables = vec![run_variable(TOTAL, U64, Some(json!("0")))];
    let (document, plan) = source.control();
    let items = SIDE * SIDE;
    measure(
        format!("nested for-each ({SIDE}x{SIDE})"),
        items,
        "inner items",
        config,
        || RunSession::start(&plan, &document, total(), config.budget),
        |(value, wakeups)| {
            assert_eq!(value, Some(Value::U64(items * (items - 1) / 2)));
            assert_eq!(wakeups, 0);
        },
    )
}

/// Count(1000) whose body is a Race Fork: branch 0 waits one virtual second, branch 1
/// increments synchronously and wins, so every iteration cancels the waiting region.
pub fn fork_cancel(config: Config) -> Row {
    let mut fork = core(3, "fork");
    fork["config"] = json!({"join":"race"});
    fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
    let mut wait = core(4, "wait");
    wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"1000000000"}});
    let mut source = Source::new(
        vec![
            core(1, "on-start"),
            count(2, ITERATIONS),
            fork,
            wait,
            update(5, TOTAL, "add", U64, json!("1")),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "branch#0", 4, "in"),
            edge(4, 3, "branch#1", 5, "in"),
        ],
    );
    source.variables = vec![run_variable(TOTAL, U64, Some(json!("0")))];
    let (document, plan) = source.control();
    measure(
        "fork race + cancel region".into(),
        ITERATIONS,
        "iterations",
        config,
        || RunSession::start(&plan, &document, total(), config.budget),
        |(value, wakeups)| {
            assert_eq!(value, Some(Value::U64(ITERATIONS)));
            // A surviving loser would leave its timer for the host to advance.
            assert_eq!(wakeups, 0);
        },
    )
}

const STRING_MAP: &str = "core/map@1<core/string@1,core/u64@1>";

fn map_entries(entries: u64) -> Json {
    json!(
        (0..entries)
            .map(|i| [format!("k{i:05}"), i.to_string()])
            .collect::<Vec<_>>()
    )
}

/// The `map_entries` value as the runtime represents it.
fn map_value(entries: u64) -> Value {
    let entries: Vec<_> = (0..entries)
        .map(|i| (Value::Str(Arc::new(format!("k{i:05}"))), Value::U64(i)))
        .collect();
    Value::Map(Arc::new(MapRepr::new(entries).expect("keys are distinct")))
}

/// Count(1000) whose body compare-and-sets a map variable to an equal literal, with
/// `expected` read from the variable, so every update compares and exchanges the map.
fn map_cas(config: Config, entries: u64) -> Row {
    let mut cas = update(
        3,
        TOTAL,
        "compare-and-set",
        STRING_MAP,
        map_entries(entries),
    );
    cas["inputs"]["expected"] = json!({"type":STRING_MAP,"value":[]});
    let mut source = Source::new(
        vec![
            core(1, "on-start"),
            count(2, ITERATIONS),
            cas,
            var_node(4, "get-variable", TOTAL),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 4, "value", 3, "expected"),
        ],
    );
    source.variables = vec![run_variable(TOTAL, STRING_MAP, Some(map_entries(entries)))];
    let (document, plan) = source.control();
    let expected = map_value(entries);
    measure(
        format!("map CAS ({entries} entries)"),
        ITERATIONS,
        "updates",
        config,
        || RunSession::start(&plan, &document, total(), config.budget),
        |(value, wakeups)| {
            assert_eq!(value.as_ref(), Some(&expected));
            assert_eq!(wakeups, 0);
        },
    )
}

pub fn map_cas_16(config: Config) -> Row {
    map_cas(config, 16)
}

pub fn map_cas_1000(config: Config) -> Row {
    map_cas(config, 1000)
}

/// A data plan that exports a map host input; every epoch publishes the same shared map
/// value, so the cost is input admission and export, not building the map.
fn host_map(config: Config, entries: u64) -> Row {
    const EPOCHS: u64 = 1000;
    let mut source = Source::new(vec![var_node(1, "get-variable", TOTAL)], vec![]);
    source.exports = vec![export(1, "value")];
    source.variables = vec![run_variable(TOTAL, STRING_MAP, None)];
    let (document, plan, index) = source.data(1);
    let value = map_value(entries);
    let input = DataInput {
        index,
        value: &value,
        epochs: EPOCHS,
    };
    measure(
        format!("host map input ({entries} entries)"),
        EPOCHS,
        "epochs",
        config,
        || {
            DataSession::start(
                &plan,
                &document,
                &input,
                config.budget,
                DataLimits::default(),
            )
        },
        |result| assert_eq!(result.as_ref(), Some(&value)),
    )
}

pub fn host_map_16(config: Config) -> Row {
    host_map(config, 16)
}

pub fn host_map_1000(config: Config) -> Row {
    host_map(config, 1000)
}

fn zone(n: u32, end: bool, pins: Json) -> Json {
    let definition = if end {
        "core/map-end"
    } else {
        "core/map-begin"
    };
    json!({"id":id(n),"definition_id":definition,"definition_version":1,"config_version":1,
        "config":{"zone":"z"},"type_pins":pins,"inputs":{}})
}

const MAP_ITEMS: u64 = 10_000;

/// A Map zone adding one to each item of a `u64` host list: the document, plan and the
/// host input's index.
fn map_zone_graph() -> (GraphDocument, Arc<DataPlan>, usize) {
    let mut source = Source::new(
        vec![
            zone(1, false, json!({"T":U64})),
            math(2, "add", 0, 1),
            zone(3, true, json!({"U":U64})),
            var_node(4, "get-variable", TOTAL),
        ],
        vec![
            edge(1, 4, "value", 1, "items"),
            edge(2, 1, "element", 2, "a"),
            edge(3, 2, "sum", 3, "item"),
        ],
    );
    source.exports = vec![export(3, "result")];
    source.variables = vec![run_variable(TOTAL, "core/list@1<core/u64@1>", None)];
    source.data(4)
}

fn u64_list(items: std::ops::Range<u64>) -> Value {
    Value::List(Arc::new(ListRepr::U64(items.collect())))
}

/// The Map zone over a 10,000-item host list, pulled in ten epochs.
pub fn map_zone(config: Config) -> Row {
    const ITEMS: u64 = MAP_ITEMS;
    const EPOCHS: u64 = 10;
    let (document, plan, index) = map_zone_graph();
    let value = u64_list(0..ITEMS);
    let expected = u64_list(1..ITEMS + 1);
    let input = DataInput {
        index,
        value: &value,
        epochs: EPOCHS,
    };
    // Each epoch evaluates every item; the default per-epoch operation limit is smaller.
    let limits = DataLimits {
        max_operations: 1_000_000,
        ..DataLimits::default()
    };
    measure(
        format!("map zone ({ITEMS} items)"),
        ITEMS * EPOCHS,
        "items",
        config,
        || DataSession::start(&plan, &document, &input, config.budget, limits),
        |result| assert_eq!(result.as_ref(), Some(&expected)),
    )
}

/// Heap the Map zone allocates while being refused when the retained-byte limit leaves
/// about 1 KiB beyond its 10,000-item source: the result cannot fit, so nothing
/// result-sized should be reserved.
pub fn oversized_map_zone_peak_bytes() -> usize {
    let (document, plan, index) = map_zone_graph();
    let value = u64_list(0..MAP_ITEMS);
    let options = DataOptions {
        limits: DataLimits {
            max_operations: 1_000_000,
            max_retained_bytes: 161_024,
            ..DataLimits::default()
        },
        cancel: None,
    };
    let mut activation =
        DataActivation::start(plan.clone(), &document, plan.snapshot(), 17, 1, options)
            .expect("activation starts");
    activation.begin_epoch().expect("epoch");
    activation.set_input(index, value).expect("host input");
    let window = crate::counter::Snapshot::start();
    loop {
        match activation.poll_export(0, &mut 64) {
            Ok(DataPoll::Pending) => continue,
            Err(DataError::ResourceLimit) => break,
            Err(error) => panic!("expected the result to exceed the limit, got {error:?}"),
            Ok(DataPoll::Ready(_)) => panic!("the result unexpectedly fit"),
        }
    }
    window.finish().peak_live_bytes
}

/// Heap bytes allocated while refusing an 8 MiB `core/json@1` literal against the 1 MiB
/// JSON text budget. Refusal should not copy the rejected text.
pub fn oversized_json_literal_allocated_bytes() -> u64 {
    use orange_model::document::literal::Literal;
    use orange_model::value::literal_codec::{SchemaSet, decode};
    let text = format!("\"{}\"", "x".repeat(8 << 20));
    let literal = Literal::value("core/json@1".parse().expect("type"), text.into());
    let schemas = SchemaSet::default();
    let window = crate::counter::Snapshot::start();
    let result = decode(&literal, &schemas);
    let allocated = window.finish().allocated_bytes;
    assert!(result.is_err(), "the oversized literal was accepted");
    allocated
}

/// Heap bytes a run still holds after the function-call workload, stop and dispose; they are
/// released only when the disposed run is dropped. Disposal should release its storage.
pub fn disposed_run_retained_bytes() -> usize {
    let (document, plan) = function_call_graph();
    let mut session = RunSession::start(&plan, &document, total(), 64);
    while crate::harness::Session::step(&mut session) {}
    let mut run = session.into_run();
    run.stop().expect("stop");
    run.dispose().expect("dispose");
    let held = crate::counter::live_bytes();
    drop(run);
    held.saturating_sub(crate::counter::live_bytes())
}

/// Background analysis on a 2,000-node add chain, as p50 over `config.runs`.
pub struct AnalysisWorkerTimes {
    /// Building the immutable document snapshot the host submits.
    pub snapshot: std::time::Duration,
    /// Handing a snapshot to the worker.
    pub submit: std::time::Duration,
    /// A full analysis from submission to result.
    pub full: std::time::Duration,
    /// A resubmission made right after an earlier one, which first waits for the
    /// superseded analysis to stop at its next pass boundary.
    pub superseded: std::time::Duration,
    /// Requesting shutdown while an analysis runs.
    pub shutdown: std::time::Duration,
    /// Joining the thread after that request.
    pub join: std::time::Duration,
}

pub fn analysis_worker_latency(config: Config) -> AnalysisWorkerTimes {
    use orange_model::validation::{AnalysisLimits, AnalysisWorker};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    const LINKS: u32 = 2000;
    let mut nodes = vec![math(10, "add", 0, 1)];
    let mut edges = Vec::new();
    for link in 1..LINKS {
        nodes.push(math(10 + link, "add", 0, 1));
        edges.push(edge(link, 9 + link, "sum", 10 + link, "a"));
    }
    let (document, registry) = Source::new(nodes, edges).analyzed();
    let spawn = || AnalysisWorker::spawn(AnalysisLimits::default()).expect("worker thread");
    let median = |mut samples: Vec<Duration>| {
        samples.sort();
        samples[samples.len() / 2]
    };
    let timed = |samples: &mut Vec<Duration>, start: Instant| samples.push(start.elapsed());
    let (mut snapshot, mut submit, mut full, mut superseded, mut shutdown, mut join) =
        Default::default();
    let mut worker = spawn();
    for _ in 0..config.runs {
        let start = Instant::now();
        let copy = Arc::new(document.clone());
        timed(&mut snapshot, start);
        let start = Instant::now();
        worker
            .submit(Arc::clone(&copy), &registry, Default::default())
            .expect("submit");
        timed(&mut submit, start);
        assert!(worker.wait().is_some());
        timed(&mut full, start);
        worker
            .submit(Arc::clone(&copy), &registry, Default::default())
            .expect("submit");
        let start = Instant::now();
        worker
            .submit(Arc::clone(&copy), &registry, Default::default())
            .expect("submit");
        assert!(worker.wait().is_some());
        timed(&mut superseded, start);

        let mut stopping = spawn();
        stopping
            .submit(copy, &registry, Default::default())
            .expect("submit");
        let start = Instant::now();
        stopping.request_shutdown();
        timed(&mut shutdown, start);
        let start = Instant::now();
        stopping.join().expect("join");
        timed(&mut join, start);
    }
    AnalysisWorkerTimes {
        snapshot: median(snapshot),
        submit: median(submit),
        full: median(full),
        superseded: median(superseded),
        shutdown: median(shutdown),
        join: median(join),
    }
}

#[cfg(test)]
mod tests {
    /// Reserving the 10,000-item result before refusing it took about 80 KB.
    #[test]
    fn an_impossible_map_result_is_refused_before_reserving_it() {
        let peak = super::oversized_map_zone_peak_bytes();
        assert!(
            peak < 16 * 1024,
            "refusing the map zone peaked at {peak} bytes"
        );
    }

    /// Decoding copied the whole 8 MiB literal before refusing it.
    #[test]
    fn an_oversized_json_literal_is_refused_without_copying_it() {
        let allocated = super::oversized_json_literal_allocated_bytes();
        assert!(
            allocated < 64 * 1024,
            "refusing the JSON literal allocated {allocated} bytes"
        );
    }

    /// Before disposal released its pools, a disposed function-call run held about 6 KiB.
    #[test]
    fn a_disposed_run_holds_almost_no_heap() {
        let retained = super::disposed_run_retained_bytes();
        assert!(
            retained < 1024,
            "a disposed run still holds {retained} bytes"
        );
    }
}
