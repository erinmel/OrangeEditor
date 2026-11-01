//! Reproducible `DocIndex` transaction/update measurements for review R6.

use crate::counter::{self, Snapshot};
use crate::harness::Config;
use orange_model::document::{FunctionDecl, literal::Literal};
use orange_model::id::FunctionId;
use orange_model::{DocCommand, DocIndex, GraphDocument, from_json};
use serde_json::{Value as Json, json};
use std::time::{Duration, Instant};

const ROOT_BASE: u32 = 100_000;
const FUNCTION_BASE: u32 = 300_000;
const FUNCTION_ID_BASE: u32 = 900_000;

struct Fixture {
    document: GraphDocument,
    changed_function: Option<String>,
    function: Option<FunctionDecl>,
}

fn graph_id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}

fn node(n: u32) -> Json {
    json!({
        "id": graph_id(n),
        "definition_id": "org.example/measurement-node",
        "definition_version": 1,
        "config_version": 1,
        "config": {},
        "inputs": {"value": {"type": "core/u64@1", "value": "0"}}
    })
}

fn fixture(root_nodes: usize, functions: usize, nodes_per_function: usize) -> Fixture {
    let roots: Vec<_> = (0..root_nodes)
        .map(|n| node(ROOT_BASE + n as u32))
        .collect();
    let functions_json: Vec<_> = (0..functions)
        .map(|function| {
            let nodes: Vec<_> = (0..nodes_per_function)
                .map(|n| node(FUNCTION_BASE + function as u32 * 2_000 + n as u32))
                .collect();
            let edges: Vec<_> = (0..nodes_per_function.saturating_sub(1))
                .map(|n| {
                    json!({
                        "id": format!("30000000-0000-4000-8000-{:012x}", function * 2_000 + n),
                        "from": {"node": graph_id(FUNCTION_BASE + function as u32 * 2_000 + n as u32), "port": "out"},
                        "to": {"node": graph_id(FUNCTION_BASE + function as u32 * 2_000 + n as u32 + 1), "port": "in"}
                    })
                })
                .collect();
            json!({
                "id": graph_id(FUNCTION_ID_BASE + function as u32),
                "name": format!("f{function}"),
                "kind": "pure",
                "nodes": nodes,
                "edges": edges
            })
        })
        .collect();
    let changed_function = (functions > 0).then(|| graph_id(FUNCTION_ID_BASE));
    let value = json!({
        "format": "orange.graph",
        "format_version": 1,
        "graph_id": graph_id(1),
        "revision": 0,
        "nodes": roots,
        "edges": [],
        "functions": functions_json
    });
    let document = from_json(&value.to_string()).expect("index fixture");
    let function = changed_function.as_ref().map(|id| {
        document
            .functions()
            .get(&id.parse::<FunctionId>().expect("function id"))
            .expect("fixture function")
            .clone()
    });
    Fixture {
        document,
        changed_function,
        function,
    }
}

struct Memory {
    retained: usize,
    peak: usize,
    released: usize,
}

fn index_memory(document: &GraphDocument) -> Memory {
    let baseline = counter::live_bytes();
    let window = Snapshot::start();
    let index = DocIndex::build(document);
    let retained = counter::live_bytes().saturating_sub(baseline);
    let peak = window.finish().peak_live_bytes;
    let before_drop = counter::live_bytes();
    drop(index);
    Memory {
        retained,
        peak,
        released: before_drop.saturating_sub(counter::live_bytes()),
    }
}

#[derive(Clone, Copy)]
enum EditCase {
    RootLocal,
    FunctionLocal,
    FunctionReplace,
    FunctionRemove,
    FunctionRestore,
}

fn samples(config: Config, mut fixture: Fixture, edit_case: EditCase) -> Vec<Duration> {
    let mut index = DocIndex::build(&fixture.document);
    let mut literal_value = 0_u64;
    let function_id = fixture
        .changed_function
        .as_ref()
        .map(|id| id.parse().expect("function id"));
    if matches!(edit_case, EditCase::FunctionRestore) {
        let committed = fixture
            .document
            .apply([DocCommand::RemoveFunction(
                function_id.expect("function id"),
            )])
            .expect("prepare restore fixture");
        index.update(&fixture.document, &committed.change_set);
        assert_eq!(index, DocIndex::build(&fixture.document));
    }
    let mut alternate = false;
    let mut times = Vec::with_capacity(config.warmup + config.runs);
    for sample in 0..config.warmup + config.runs {
        let target = match edit_case {
            EditCase::RootLocal => graph_id(ROOT_BASE).parse().expect("root node id"),
            _ => graph_id(FUNCTION_BASE).parse().expect("function node id"),
        };
        let command = match edit_case {
            EditCase::RootLocal | EditCase::FunctionLocal => {
                literal_value ^= 1;
                DocCommand::SetLiteral {
                    port: orange_model::document::PortRef {
                        node: target,
                        port: "value".parse().expect("port id"),
                    },
                    literal: Some(
                        serde_json::from_value::<Literal>(json!({
                            "type": "core/u64@1",
                            "value": literal_value.to_string()
                        }))
                        .expect("literal"),
                    ),
                }
            }
            EditCase::FunctionReplace => {
                let mut replacement = fixture.function.as_ref().expect("function").clone();
                replacement.name = if alternate { "f0a" } else { "f0b" }.to_owned();
                alternate = !alternate;
                DocCommand::UpsertFunction(replacement)
            }
            EditCase::FunctionRemove => {
                DocCommand::RemoveFunction(function_id.expect("function id"))
            }
            EditCase::FunctionRestore => {
                DocCommand::UpsertFunction(fixture.function.as_ref().expect("function").clone())
            }
        };
        let start = Instant::now();
        let committed = fixture.document.apply([command]).expect("committed edit");
        index.update(&fixture.document, &committed.change_set);
        let elapsed = start.elapsed();
        if sample >= config.warmup {
            times.push(elapsed);
        }
        assert!(index.is_current(&fixture.document));
        assert_eq!(index, DocIndex::build(&fixture.document));
        if matches!(edit_case, EditCase::FunctionRemove) {
            let committed = fixture
                .document
                .apply([DocCommand::UpsertFunction(
                    fixture.function.as_ref().expect("function").clone(),
                )])
                .expect("restore function");
            index.update(&fixture.document, &committed.change_set);
            assert_eq!(index, DocIndex::build(&fixture.document));
        }
        if matches!(edit_case, EditCase::FunctionRestore) {
            let committed = fixture
                .document
                .apply([DocCommand::RemoveFunction(
                    function_id.expect("function id"),
                )])
                .expect("prepare next restore");
            index.update(&fixture.document, &committed.change_set);
            assert_eq!(index, DocIndex::build(&fixture.document));
        }
    }
    times
}

fn percentile(values: &[Duration], percent: usize) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

fn report(name: &str, fixture: Fixture, config: Config, edit_case: EditCase) {
    let memory = index_memory(&fixture.document);
    let times = samples(config, fixture, edit_case);
    let min = times.iter().min().copied().unwrap_or_default();
    let max = times.iter().max().copied().unwrap_or_default();
    let us = |duration: Duration| duration.as_secs_f64() * 1e6;
    println!(
        "index {name}: samples={} p50={:.3}us p95={:.3}us min={:.3}us max={:.3}us; retained={}B peak={}B drop_released={}B DocIndex_inline={}B",
        times.len(),
        us(percentile(&times, 50)),
        us(percentile(&times, 95)),
        us(min),
        us(max),
        memory.retained,
        memory.peak,
        memory.released,
        std::mem::size_of::<DocIndex>()
    );
}

pub fn run(config: Config) {
    for (name, roots, functions, members) in [
        ("root-1k-small-function", 1_000, 1, 4),
        ("root-10k-small-function", 10_000, 1, 4),
        ("root-1k-many-functions-64x16", 1_000, 64, 16),
        ("root-10k-many-functions-128x16", 10_000, 128, 16),
        ("root-10k-large-function-1024", 10_000, 1, 1_024),
    ] {
        report(
            &format!("{name}/root-edit"),
            fixture(roots, functions, members),
            config,
            EditCase::RootLocal,
        );
        if functions > 0 {
            report(
                &format!("{name}/function-local-edit"),
                fixture(roots, functions, members),
                config,
                EditCase::FunctionLocal,
            );
            report(
                &format!("{name}/function-replace"),
                fixture(roots, functions, members),
                config,
                EditCase::FunctionReplace,
            );
            report(
                &format!("{name}/function-remove"),
                fixture(roots, functions, members),
                config,
                EditCase::FunctionRemove,
            );
            report(
                &format!("{name}/function-restore"),
                fixture(roots, functions, members),
                config,
                EditCase::FunctionRestore,
            );
        }
    }
}
