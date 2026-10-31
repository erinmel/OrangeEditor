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

/// Count(100) whose body sets a variable from a 384-add chain in which every add reads
/// its own Get Variable, so each pull's first poll snapshots 385 host inputs at once. The
/// default compile limits refuse a 500-add chain of this shape.
pub fn host_inputs_384(config: Config) -> Row {
    const INPUTS: u32 = 384;
    const ROUNDS: u64 = 100;
    const ONE: u32 = 101;
    let mut nodes = vec![
        core(1, "on-start"),
        count(2, ROUNDS),
        var_node(3, "get-variable", TOTAL),
        var_node(4, "set-variable", TOTAL),
    ];
    let mut edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "body", 4, "in"),
        edge(3, 3, "value", 10_000, "a"),
        edge(4, 10_000 + INPUTS - 1, "sum", 4, "value"),
    ];
    for link in 0..INPUTS {
        let add = 10_000 + link;
        let get = 20_000 + link;
        nodes.push(math(add, "add", 0, 0));
        nodes.push(var_node(get, "get-variable", ONE));
        edges.push(edge(get, get, "value", add, "b"));
        if link > 0 {
            edges.push(edge(add, add - 1, "sum", add, "a"));
        }
    }
    let mut source = Source::new(nodes, edges);
    source.variables = vec![
        run_variable(TOTAL, U64, Some(json!("0"))),
        run_variable(ONE, U64, Some(json!("1"))),
    ];
    let (document, plan) = source.control();
    measure(
        format!("host inputs ({INPUTS} gets per pull)"),
        ROUNDS,
        "iterations",
        config,
        || RunSession::start(&plan, &document, total(), config.budget),
        |(value, wakeups)| {
            assert_eq!(value, Some(Value::U64(ROUNDS * u64::from(INPUTS))));
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

/// A Fork race over `branches` branches: one finishes at once and every other waits, so
/// each iteration cancels `branches - 1` waiting strands in one ownership traversal.
fn fork_fanout(config: Config, branches: u32) -> Row {
    const FANOUT_ITERATIONS: u64 = 100;
    let order: Vec<u32> = (0..branches).collect();
    let mut fork = core(3, "fork");
    fork["config"] = json!({"join":"race"});
    fork["variadic"] = json!({"branch":{"next":branches,"order":order}});
    let mut nodes = vec![
        core(1, "on-start"),
        count(2, FANOUT_ITERATIONS),
        fork,
        update(5, TOTAL, "add", U64, json!("1")),
    ];
    let mut edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "body", 3, "in"),
        edge(3, 3, "branch#0", 5, "in"),
    ];
    for branch in 1..branches {
        let mut wait = core(100 + branch, "wait");
        wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"1000000000"}});
        nodes.push(wait);
        edges.push(edge(
            100 + branch,
            3,
            &format!("branch#{branch}"),
            100 + branch,
            "in",
        ));
    }
    let mut source = Source::new(nodes, edges);
    source.variables = vec![run_variable(TOTAL, U64, Some(json!("0")))];
    let (document, plan) = source.control();
    measure(
        format!("fork race fan-out {branches}"),
        FANOUT_ITERATIONS,
        "iterations",
        config,
        || RunSession::start(&plan, &document, total(), config.budget),
        |(value, wakeups)| {
            assert_eq!(value, Some(Value::U64(FANOUT_ITERATIONS)));
            assert_eq!(wakeups, 0);
        },
    )
}

pub fn fork_fanout_256(config: Config) -> Row {
    fork_fanout(config, 256)
}

/// The widest race the default run limits admit: 4,000 branches of 4,096 strands.
pub fn fork_fanout_4000(config: Config) -> Row {
    fork_fanout(config, 4000)
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

/// Comparing two separate 65,536-element string lists whose elements share one string, as
/// p50 over `config.runs` of 16 comparisons.
pub fn string_list_equality(config: Config) -> std::time::Duration {
    use orange_model::value::ListRepr;
    use std::sync::Arc;
    use std::time::Instant;
    let shared = Arc::new("shared".to_owned());
    let list = || ListRepr::Str(vec![Arc::clone(&shared); 1 << 16].into());
    let (a, b) = (list(), list());
    let mut samples: Vec<_> = (0..config.runs)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..16 {
                assert!(std::hint::black_box(&a) == std::hint::black_box(&b));
            }
            start.elapsed()
        })
        .collect();
    samples.sort();
    samples[samples.len() / 2]
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

/// Session update latency for one literal edit on `chains` independent add chains of
/// `length` nodes, as (p50, p95) over `config.runs` edits after `config.warmup`.
/// Session update latency when an exec edge in the middle of 1,000 Set Variable nodes,
/// split evenly across `entries` On Start activations that each run theirs in turn, every
/// Set fed by a 9-add chain (about 10,000 nodes), is alternately removed and restored.
/// (p50, p95) over `config.runs` edits after `config.warmup`.
/// With `paste`, edits instead alternately add a Set Variable after the last Set of the
/// middle activation, wired to it and to that Set's add chain, and remove it again.
pub fn session_exec_edit(
    config: Config,
    entries: u32,
    paste: bool,
) -> (std::time::Duration, std::time::Duration) {
    use orange_model::DocCommand;
    use orange_model::validation::{AnalysisOptions, AnalysisSession};
    use std::collections::BTreeMap;
    use std::time::Instant;
    const SETS: u32 = 1000;
    const ADDS: u32 = 9;
    let per_entry = SETS / entries;
    let mut nodes = Vec::new();
    for entry in 0..entries {
        let mut start = core(30_000 + entry, "on-start");
        start["activation"]["order"] = json!(entry);
        nodes.push(start);
    }
    let mut edges = Vec::new();
    for set in 0..SETS {
        let node = 10 + set;
        nodes.push(var_node(node, "set-variable", TOTAL));
        let previous = if set % per_entry == 0 {
            30_000 + set / per_entry
        } else {
            node - 1
        };
        edges.push(edge(node, previous, "then", node, "in"));
        let first = 10_000 + set * ADDS;
        for link in 0..ADDS {
            nodes.push(math(first + link, "add", 0, 1));
            if link > 0 {
                edges.push(edge(
                    first + link,
                    first + link - 1,
                    "sum",
                    first + link,
                    "a",
                ));
            }
        }
        edges.push(edge(20_000 + set, first + ADDS - 1, "sum", node, "value"));
    }
    let mut source = Source::new(nodes, edges);
    source.variables = vec![run_variable(TOTAL, U64, Some(json!("0")))];
    let (mut document, registry) = source.analyzed();
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    // The middle Set of the middle activation, and the node before it.
    let middle = 10 + (entries / 2) * per_entry + per_entry / 2;
    let wired: orange_model::document::Edge =
        serde_json::from_value(edge(middle, middle - 1, "then", middle, "in")).expect("edge");
    let last = 10 + (entries / 2) * per_entry + per_entry - 1;
    let pasted = 9;
    let mut samples = Vec::new();
    for sample in 0..config.warmup + config.runs {
        let commands = match (paste, sample % 2 == 0) {
            (false, true) => vec![DocCommand::Disconnect { edge: wired.id }],
            (false, false) => vec![DocCommand::Connect {
                edge: wired.clone(),
            }],
            (true, true) => vec![
                DocCommand::AddNode {
                    body: orange_model::BodyRef::Root,
                    node: serde_json::from_value(var_node(pasted, "set-variable", TOTAL))
                        .expect("node"),
                },
                DocCommand::Connect {
                    edge: serde_json::from_value(edge(pasted, last, "then", pasted, "in"))
                        .expect("edge"),
                },
                DocCommand::Connect {
                    edge: serde_json::from_value(edge(
                        30_000,
                        10_000 + (last - 10) * ADDS,
                        "sum",
                        pasted,
                        "value",
                    ))
                    .expect("edge"),
                },
            ],
            (true, false) => vec![DocCommand::RemoveNodes {
                ids: vec![id(pasted).parse().expect("node ID")],
            }],
        };
        let committed = document.apply(commands).expect("edit");
        let start = Instant::now();
        session.update(
            &document,
            &registry,
            &committed.change_set,
            AnalysisOptions::default(),
        );
        if sample >= config.warmup {
            samples.push(start.elapsed());
        }
    }
    samples.sort();
    let at = |q: usize| samples[(samples.len() * q / 100).min(samples.len() - 1)];
    (at(50), at(95))
}

/// The edit `session_edit` repeats on the middle node.
#[derive(Clone, Copy)]
pub enum SessionEdit {
    /// A new `b` literal each time.
    Literal,
    /// Alternately removes and restores the edge into `a`.
    Wire,
    /// Alternately pins `T` to the chain's type and clears the pin.
    Pin,
    /// Alternately sets saturating and wrapping integer overflow.
    Config,
    /// Alternately adds an add node fed by the middle node and removes it again.
    Node,
    /// [`SessionEdit::Wire`] while the first node holds an invalid literal, an input error.
    WireBesideError,
    /// [`SessionEdit::Wire`] while an unresolvable node feeds the first node, leaving both
    /// incomplete.
    WireBesideIncomplete,
    /// [`SessionEdit::Wire`] with every chain's last node exported.
    WireIntoExport,
    /// [`SessionEdit::Wire`], each time right after two untimed edits that run the full
    /// analysis: closing a cycle in the first chain and opening it again.
    WireAfterFull,
    /// [`SessionEdit::Wire`] in a graph that also holds one Repeat zone, apart from the
    /// chains: begin, one body add, end and an add reading the result (V12c).
    WireBesideZone,
}

pub fn session_edit(
    config: Config,
    chains: u32,
    length: u32,
    kind: SessionEdit,
) -> (std::time::Duration, std::time::Duration) {
    use orange_model::DocCommand;
    use orange_model::document::PortRef;
    use orange_model::validation::{AnalysisOptions, AnalysisSession};
    use std::collections::BTreeMap;
    use std::time::Instant;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for chain in 0..chains {
        let first = 10 + chain * length;
        nodes.push(math(first, "add", 0, 1));
        for link in 1..length {
            nodes.push(math(first + link, "add", 0, 1));
            edges.push(edge(
                first + link,
                first + link - 1,
                "sum",
                first + link,
                "a",
            ));
        }
    }
    if matches!(kind, SessionEdit::WireBesideZone) {
        let marker = |n: u32, definition: &str| {
            json!({"id": id(n), "definition_id": definition, "definition_version": 1,
                "config_version": 1, "config": {"zone": "z"}, "type_pins": {"A": U64}})
        };
        let mut begin = marker(1, "core/repeat-begin");
        begin["inputs"] = json!({"count": {"type": "core/i64@1", "value": "3"},
            "initial": literal(0)});
        nodes.extend([
            begin,
            math(2, "add", 0, 1),
            marker(3, "core/repeat-end"),
            math(4, "add", 0, 1),
        ]);
        edges.extend([
            edge(1, 1, "carried", 2, "a"),
            edge(2, 2, "sum", 3, "next"),
            edge(3, 3, "result", 4, "a"),
        ]);
    }
    let mut source = Source::new(nodes, edges);
    if matches!(kind, SessionEdit::WireIntoExport) {
        source.exports = (0..chains)
            .map(|chain| export(10 + chain * length + length - 1, "sum"))
            .collect();
    }
    let (mut document, registry) = source.analyzed();
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let middle = 10 + (chains / 2) * length + length / 2;
    let port = PortRef {
        node: id(middle).parse().expect("node ID"),
        port: "b".parse().expect("port ID"),
    };
    let wired: orange_model::document::Edge =
        serde_json::from_value(edge(middle, middle - 1, "sum", middle, "a")).expect("edge");
    if matches!(kind, SessionEdit::WireBesideIncomplete) {
        let mut unknown = math(9, "add", 0, 1);
        unknown["definition_id"] = json!("org.orange.math/unknown");
        let committed = document
            .apply([
                DocCommand::AddNode {
                    body: orange_model::BodyRef::Root,
                    node: serde_json::from_value(unknown).expect("node"),
                },
                DocCommand::Connect {
                    edge: serde_json::from_value(edge(9, 9, "sum", 10, "a")).expect("edge"),
                },
            ])
            .expect("edit");
        session.update(
            &document,
            &registry,
            &committed.change_set,
            AnalysisOptions::default(),
        );
        assert!(!session.analysis().report().incomplete_nodes.is_empty());
    }
    if matches!(kind, SessionEdit::WireBesideError) {
        let invalid = serde_json::from_value(json!({"type": U64, "value": "x"})).expect("literal");
        let committed = document
            .apply([DocCommand::SetLiteral {
                port: PortRef {
                    node: id(10).parse().expect("node ID"),
                    port: "b".parse().expect("port ID"),
                },
                literal: Some(invalid),
            }])
            .expect("edit");
        session.update(
            &document,
            &registry,
            &committed.change_set,
            AnalysisOptions::default(),
        );
        assert!(!session.analysis().report().is_valid());
    }
    let mut samples = Vec::new();
    let cycle: orange_model::document::Edge =
        serde_json::from_value(edge(8, 10 + length - 1, "sum", 10, "b")).expect("edge");
    for sample in 0..config.warmup + config.runs {
        if matches!(kind, SessionEdit::WireAfterFull) {
            for command in [
                DocCommand::Connect {
                    edge: cycle.clone(),
                },
                DocCommand::Disconnect { edge: cycle.id },
            ] {
                let committed = document.apply([command]).expect("edit");
                session.update(
                    &document,
                    &registry,
                    &committed.change_set,
                    AnalysisOptions::default(),
                );
            }
        }
        let command = match (kind, sample % 2 == 0) {
            (SessionEdit::Literal, _) => {
                let literal = serde_json::from_value(literal(2 + sample as u64)).expect("literal");
                DocCommand::SetLiteral {
                    port: port.clone(),
                    literal: Some(literal),
                }
            }
            (
                SessionEdit::Wire
                | SessionEdit::WireBesideError
                | SessionEdit::WireBesideIncomplete
                | SessionEdit::WireIntoExport
                | SessionEdit::WireAfterFull
                | SessionEdit::WireBesideZone,
                true,
            ) => DocCommand::Disconnect { edge: wired.id },
            (
                SessionEdit::Wire
                | SessionEdit::WireBesideError
                | SessionEdit::WireBesideIncomplete
                | SessionEdit::WireIntoExport
                | SessionEdit::WireAfterFull
                | SessionEdit::WireBesideZone,
                false,
            ) => DocCommand::Connect {
                edge: wired.clone(),
            },
            (SessionEdit::Config, saturating) => DocCommand::SetConfig {
                node: port.node,
                config_version: 1,
                config: serde_json::from_value(json!({
                    "int_overflow": if saturating { "saturating" } else { "wrapping" }
                }))
                .expect("config"),
            },
            (SessionEdit::Node, true) => {
                let added = 9;
                let node = serde_json::from_value(math(added, "add", 0, 1)).expect("node");
                let wire =
                    serde_json::from_value(edge(added, middle, "sum", added, "a")).expect("edge");
                let committed = document
                    .apply([
                        DocCommand::AddNode {
                            body: orange_model::BodyRef::Root,
                            node,
                        },
                        DocCommand::Connect { edge: wire },
                    ])
                    .expect("edit");
                let start = Instant::now();
                session.update(
                    &document,
                    &registry,
                    &committed.change_set,
                    AnalysisOptions::default(),
                );
                if sample >= config.warmup {
                    samples.push(start.elapsed());
                }
                continue;
            }
            (SessionEdit::Node, false) => DocCommand::RemoveNodes {
                ids: vec![id(9).parse().expect("node ID")],
            },
            (SessionEdit::Pin, pin) => DocCommand::SetTypePins {
                node: port.node,
                pins: if pin {
                    [("T".to_owned(), U64.parse().expect("type"))].into()
                } else {
                    Default::default()
                },
            },
        };
        let committed = document.apply([command]).expect("edit");
        let start = Instant::now();
        session.update(
            &document,
            &registry,
            &committed.change_set,
            AnalysisOptions::default(),
        );
        if sample >= config.warmup {
            samples.push(start.elapsed());
        }
    }
    assert_eq!(
        session.analysis().report().is_valid(),
        !matches!(
            kind,
            SessionEdit::WireBesideError | SessionEdit::WireBesideIncomplete
        )
    );
    if matches!(kind, SessionEdit::WireBesideZone) {
        assert_eq!(session.analysis().report().regions.zones.iter().len(), 1);
    }
    samples.sort();
    let at = |q: usize| samples[(samples.len() * q / 100).min(samples.len() - 1)];
    (at(50), at(95))
}

/// The kinds of W9 edit, each reported on its own.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum W9Edit {
    /// Removes an add input's edge.
    Disconnect,
    /// Restores a removed add input's edge.
    Connect,
    /// A new literal on an add's `b`.
    Literal,
    /// Sets an add's `T` to `core/u64@1` or clears it.
    TypeArgs,
    /// Removes or restores the exec edge into a Set.
    Exec,
    /// A new literal on an add's `b` in a called function's body.
    FunctionLiteral,
    /// Rewires a cluster's first add to read its last add, closing a data cycle.
    CycleClose,
    /// Undoes the cycle-closing edit just before it.
    CycleOpen,
}

impl W9Edit {
    pub const ALL: [Self; 8] = [
        Self::Disconnect,
        Self::Connect,
        Self::Literal,
        Self::TypeArgs,
        Self::Exec,
        Self::FunctionLiteral,
        Self::CycleClose,
        Self::CycleOpen,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Disconnect => "disconnect",
            Self::Connect => "connect",
            Self::Literal => "literal",
            Self::TypeArgs => "type argument",
            Self::Exec => "exec edge",
            Self::FunctionLiteral => "function-body literal",
            Self::CycleClose => "cycle close",
            Self::CycleOpen => "cycle open",
        }
    }
}

/// Latency and handling of one group of W9 edits.
pub struct W9Bucket {
    pub edits: usize,
    pub p50: std::time::Duration,
    pub p95: std::time::Duration,
    pub p99: std::time::Duration,
    pub max: std::time::Duration,
    /// Edits applied in place; the rest ran the full analysis.
    pub scoped: usize,
    /// The most nodes one scoped edit re-resolved (`EditStats::input_nodes`).
    pub max_resolved: usize,
}

/// One W9 run.
pub struct W9 {
    /// Root nodes and edges before the edits.
    pub nodes: usize,
    pub edges: usize,
    /// The 1,000 seeded connect, disconnect, literal and type-argument edits, the same
    /// sequence as before the other kinds were added.
    pub core: W9Bucket,
    /// Every edit by kind, including the added reconnections in `Connect`.
    pub kinds: Vec<(W9Edit, W9Bucket)>,
}

/// The W9 graph's cluster shape.
#[derive(Clone, Copy)]
pub struct W9Shape {
    /// Each cluster's first add reads the previous cluster's last add instead of the Get,
    /// so all adds form one generic component.
    pub connected: bool,
    /// Adds per cluster: eight give 18,000 edges, nine W4's 20,000.
    pub adds: u32,
    /// One Get per exec chain, read by its 100 clusters, instead of one per cluster.
    pub shared_get: bool,
}

/// W9 on a W4-sized graph (06): ten On Start entries, each with an exec chain through the
/// Sets of 100 clusters of adds, each add reading the two values before it, and a Set
/// storing the last; four Gets also feed a Call of a small pure function. Then 1,000
/// seeded random edits, a quarter each literal, type argument, and disconnect or connect
/// of an add input (nearly always a disconnect). After them in turn, from a second seed: a
/// function-body literal, a reconnection of a removed add input and an exec edge toggle,
/// one each per four seeded edits; and after every 50th, closing a data cycle in a
/// cluster and opening it again.
pub fn w9_session_edits(shape: W9Shape) -> W9 {
    use orange_model::DocCommand;
    use orange_model::document::{Edge, PortRef};
    use orange_model::validation::{AnalysisOptions, AnalysisSession, SessionWork};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};
    const CLUSTERS: u32 = 1000;
    const ENTRIES: u32 = 10;
    const FUNCTIONS: u32 = 4;
    let adds = shape.adds;
    // Cluster `c` holds the IDs from `10 + c * stride`: its own Get if it has one, its
    // adds and last its Set.
    let own_get = u32::from(!shape.shared_get);
    let stride = if own_get + adds < 10 { 10 } else { 20 };
    let base = |cluster: u32| 10 + cluster * stride;
    let add_of = |cluster: u32, k: u32| base(cluster) + own_get + k;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for entry in 0..ENTRIES {
        let mut start = core(30_000 + entry, "on-start");
        start["activation"]["order"] = json!(entry);
        nodes.push(start);
        if shape.shared_get {
            nodes.push(var_node(40_000 + entry, "get-variable", TOTAL));
        }
    }
    // Every add input's edge, keyed by (add, input), and every Set's exec edge, so edits
    // can remove and restore them.
    let mut inputs: BTreeMap<(u32, &str), Json> = BTreeMap::new();
    let mut exec = Vec::new();
    let per_entry = CLUSTERS / ENTRIES;
    for cluster in 0..CLUSTERS {
        let get = if shape.shared_get {
            40_000 + cluster / per_entry
        } else {
            base(cluster)
        };
        let add = |k: u32| add_of(cluster, k);
        let set = add(adds);
        let n = 100_000 + cluster * 20;
        if !shape.shared_get {
            nodes.push(var_node(get, "get-variable", TOTAL));
        }
        for k in 0..adds {
            nodes.push(math(add(k), "add", 0, 1));
        }
        nodes.push(var_node(set, "set-variable", TOTAL));
        for k in 0..adds {
            let (a, b) = match k {
                0 if shape.connected && cluster > 0 => {
                    let last = add_of(cluster - 1, adds - 1);
                    ((last, "sum"), (last, "sum"))
                }
                0 => ((get, "value"), (get, "value")),
                1 => ((add(0), "sum"), (get, "value")),
                _ => ((add(k - 1), "sum"), (add(k - 2), "sum")),
            };
            inputs.insert((add(k), "a"), edge(n + 2 * k, a.0, a.1, add(k), "a"));
            inputs.insert((add(k), "b"), edge(n + 2 * k + 1, b.0, b.1, add(k), "b"));
        }
        edges.push(edge(n + 2 * adds, add(adds - 1), "sum", set, "value"));
        let previous = if cluster % per_entry == 0 {
            (30_000 + cluster / per_entry, "then")
        } else {
            (set - stride, "then")
        };
        exec.push(edge(n + 2 * adds + 1, previous.0, previous.1, set, "in"));
        edges.push(exec[exec.len() - 1].clone());
    }
    edges.extend(inputs.values().cloned());
    // Each pure function is called once, reading the Get of the first cluster of one entry.
    for f in 0..FUNCTIONS {
        let get = if shape.shared_get {
            40_000 + f
        } else {
            base(f * per_entry)
        };
        let mut call = core(70_000 + f, "call");
        call["config"] = json!({"function": id(60_000 + f * 20)});
        nodes.push(call);
        edges.push(edge(310_000 + f, get, "value", 70_000 + f, "value"));
    }
    let mut source = Source::new(nodes, edges);
    source.variables = vec![run_variable(TOTAL, U64, Some(json!("0")))];
    source.functions = (0..FUNCTIONS).map(w9_function).collect();
    let (mut document, registry) = source.analyzed();
    let (node_count, edge_count) = (document.root().nodes.len(), document.root().edges.len());
    let options = AnalysisOptions::default();
    let mut session = AnalysisSession::new(&document, &registry, BTreeMap::new(), options);
    let parse = |edge: Json| -> Edge { serde_json::from_value(edge).expect("edge") };
    let edges: Vec<(u32, &str, Edge)> = inputs
        .into_iter()
        .map(|((node, input), edge)| (node, input, parse(edge)))
        .collect();
    let mut wired = vec![true; edges.len()];
    let exec: Vec<Edge> = exec.into_iter().map(parse).collect();
    let mut exec_wired = vec![true; exec.len()];
    let random = |seed: &mut u64, bound: usize| {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*seed >> 33) as usize % bound
    };
    // The four NFR-15 kinds draw from the original seed, so their sequence is unchanged;
    // the other kinds draw from their own.
    let (mut seed, mut extra_seed): (u64, u64) = (0x5eed_0009, 0x5eed_0f09);
    let mut next = |bound: usize| random(&mut seed, bound);
    let mut extra = |bound: usize| random(&mut extra_seed, bound);
    let node_id = |node: u32| id(node).parse().expect("node ID");
    let b_literal = |node: u32, value: usize| DocCommand::SetLiteral {
        port: PortRef {
            node: node_id(node),
            port: "b".parse().expect("port ID"),
        },
        literal: Some(serde_json::from_value(literal(value as u64)).expect("literal")),
    };
    // Kind, latency, scoped, nodes re-resolved, and whether the edit is a seeded one.
    type Record = (W9Edit, Duration, bool, usize, bool);
    let mut records: Vec<Record> = Vec::new();
    let mut timed = |commands: Vec<DocCommand>, kind: W9Edit, seeded: bool| {
        let committed = document.apply(commands).expect("edit");
        let start = Instant::now();
        let stats = session.update(&document, &registry, &committed.change_set, options);
        let elapsed = start.elapsed();
        let scoped = stats.work == SessionWork::Scoped;
        records.push((kind, elapsed, scoped, stats.input_nodes, seeded));
        session.analysis().report().is_valid()
    };
    for edit in 0..1000 {
        let add = add_of(next(CLUSTERS as usize) as u32, next(adds as usize) as u32);
        let (command, kind) = match next(4) {
            0 | 1 => {
                let at = next(edges.len());
                wired[at] = !wired[at];
                if wired[at] {
                    let edge = edges[at].2.clone();
                    (DocCommand::Connect { edge }, W9Edit::Connect)
                } else {
                    let edge = edges[at].2.id;
                    (DocCommand::Disconnect { edge }, W9Edit::Disconnect)
                }
            }
            2 => (b_literal(add, next(1000)), W9Edit::Literal),
            _ => {
                let args = if next(2) == 0 {
                    [(
                        "T".parse().expect("type parameter"),
                        U64.parse().expect("type"),
                    )]
                    .into()
                } else {
                    BTreeMap::new()
                };
                let node = node_id(add);
                (DocCommand::SetTypeArgs { node, args }, W9Edit::TypeArgs)
            }
        };
        assert!(timed(vec![command], kind, true));
        if edit % 4 == 1 {
            let node = 60_000 + extra(FUNCTIONS as usize) as u32 * 20 + 2 + extra(8) as u32;
            assert!(timed(
                vec![b_literal(node, extra(1000))],
                W9Edit::FunctionLiteral,
                false
            ));
        }
        if edit % 4 == 2 {
            // The seeded toggles almost never pick a removed edge again, so restore one.
            let removed: Vec<usize> = (0..wired.len()).filter(|&at| !wired[at]).collect();
            if !removed.is_empty() {
                let at = removed[extra(removed.len())];
                wired[at] = true;
                let edge = edges[at].2.clone();
                assert!(timed(
                    vec![DocCommand::Connect { edge }],
                    W9Edit::Connect,
                    false
                ));
            }
        }
        if edit % 4 == 3 {
            let at = extra(exec.len());
            exec_wired[at] = !exec_wired[at];
            let command = if exec_wired[at] {
                DocCommand::Connect {
                    edge: exec[at].clone(),
                }
            } else {
                DocCommand::Disconnect { edge: exec[at].id }
            };
            assert!(timed(vec![command], W9Edit::Exec, false));
        }
        if edit % 50 == 49 {
            // Rewire a cluster's first add to read its last one, then undo it.
            let cluster = extra(CLUSTERS as usize) as u32;
            let first = add_of(cluster, 0);
            let at = edges
                .binary_search_by_key(&(first, "b"), |(node, input, _)| (*node, *input))
                .expect("first add's b");
            let cycle = parse(edge(
                200_000 + cluster,
                add_of(cluster, adds - 1),
                "sum",
                first,
                "b",
            ));
            let mut close = vec![DocCommand::Connect {
                edge: cycle.clone(),
            }];
            let mut open = vec![DocCommand::Disconnect { edge: cycle.id }];
            if wired[at] {
                close.insert(
                    0,
                    DocCommand::Disconnect {
                        edge: edges[at].2.id,
                    },
                );
                open.push(DocCommand::Connect {
                    edge: edges[at].2.clone(),
                });
            }
            assert!(!timed(close, W9Edit::CycleClose, false));
            assert!(timed(open, W9Edit::CycleOpen, false));
        }
    }
    let bucket = |keep: &dyn Fn(&Record) -> bool| {
        let mut kept: Vec<_> = records.iter().filter(|record| keep(record)).collect();
        kept.sort_by_key(|record| record.1);
        let at = |q: usize| {
            kept.get((kept.len() * q / 100).min(kept.len().saturating_sub(1)))
                .map_or(Duration::ZERO, |record| record.1)
        };
        W9Bucket {
            edits: kept.len(),
            p50: at(50),
            p95: at(95),
            p99: at(99),
            max: kept.last().map_or(Duration::ZERO, |record| record.1),
            scoped: kept.iter().filter(|record| record.2).count(),
            max_resolved: kept
                .iter()
                .filter(|record| record.2)
                .map(|record| record.3)
                .max()
                .unwrap_or(0),
        }
    };
    W9 {
        nodes: node_count,
        edges: edge_count,
        core: bucket(&|record| record.4),
        kinds: W9Edit::ALL
            .into_iter()
            .map(|kind| (kind, bucket(&|record| record.0 == kind)))
            .collect(),
    }
}

/// A W9 pure function: its argument through eight adds, each reading the one before it,
/// to its result.
fn w9_function(f: u32) -> Json {
    let at = 60_000 + f * 20;
    let (entry, first, result) = (at + 1, at + 2, at + 10);
    let mut nodes = vec![
        core(entry, "function-entry"),
        core(result, "function-outputs"),
    ];
    let mut edges = vec![
        edge(at + 2, entry, "value", first, "a"),
        edge(at + 10, first + 7, "sum", result, "result"),
    ];
    for k in 0..8 {
        nodes.push(math(first + k, "add", 0, 1));
        if k > 0 {
            edges.push(edge(first + k, first + k - 1, "sum", first + k, "a"));
        }
    }
    json!({"id": id(at), "name": format!("f{f}"), "kind": "pure",
        "inputs": [port("value", "input", U64)], "outputs": [port("result", "output", U64)],
        "nodes": nodes, "edges": edges})
}

#[cfg(test)]
mod tests {
    /// The counting allocator is process-wide, so measurements must not run side by side.
    static MEASURING: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        MEASURING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reserving the 10,000-item result before refusing it took about 80 KB.
    #[test]
    fn an_impossible_map_result_is_refused_before_reserving_it() {
        let _measuring = exclusive();
        let peak = super::oversized_map_zone_peak_bytes();
        assert!(
            peak < 16 * 1024,
            "refusing the map zone peaked at {peak} bytes"
        );
    }

    /// Decoding copied the whole 8 MiB literal before refusing it.
    #[test]
    fn an_oversized_json_literal_is_refused_without_copying_it() {
        let _measuring = exclusive();
        let allocated = super::oversized_json_literal_allocated_bytes();
        assert!(
            allocated < 64 * 1024,
            "refusing the JSON literal allocated {allocated} bytes"
        );
    }

    /// A 256-branch race once spawned and cancelled all its strands inside one pump of
    /// budget 64; ownership work now spends the budget, so each pump stops after it.
    #[test]
    fn wide_fork_ownership_work_spends_the_pump_budget() {
        let _measuring = exclusive();
        let config = crate::harness::Config {
            budget: 64,
            warmup: 0,
            runs: 1,
        };
        let row = super::fork_fanout_256(config);
        // At least one pump for each spawn and each cancellation of the 100 iterations.
        assert!(row.pumps >= 200, "{} pumps", row.pumps);
    }

    /// Before disposal released its pools, a disposed function-call run held about 6 KiB.
    #[test]
    fn a_disposed_run_holds_almost_no_heap() {
        let _measuring = exclusive();
        let retained = super::disposed_run_retained_bytes();
        assert!(
            retained < 1024,
            "a disposed run still holds {retained} bytes"
        );
    }
}
