//! Scratch-only retained AnalysisSession ownership measurement.
//!
//! Invoked by `tools/measure-session.ps1` after that script copies the source tree and
//! injects consuming ownership partitioners into the copied orange-model source.

#[path = "counter.rs"]
mod counter;
#[path = "graph.rs"]
// The shared fixture module also serves unrelated runtime measurements.
#[allow(dead_code, unused_imports)]
mod graph;

use counter::Snapshot;
use graph::{Source, edge, id, math};
use orange_model::document::NodeInstance;
use orange_model::id::NodeId;
use orange_model::validation::{AnalysisOptions, AnalysisSession};
use orange_model::{BodyRef, DocCommand, GraphDocument};
use serde_json::json;
use std::collections::BTreeMap;

#[global_allocator]
static ALLOCATOR: counter::CountingAllocator = counter::CountingAllocator;

fn arguments() -> (usize, bool, bool, usize, usize) {
    let mut args = std::env::args().skip(1);
    let nodes = args
        .next()
        .expect("node count argument")
        .parse::<usize>()
        .expect("node count must be an integer");
    let mode = args.next().expect("valid, warnings or error argument");
    let warmup = parse_option(&mut args, "--warmup", 2);
    let runs = parse_option(&mut args, "--runs", 5);
    assert!(
        nodes > 0 && runs > 0,
        "node count and runs must be positive"
    );
    assert!(
        matches!(mode.as_str(), "valid" | "warnings" | "error"),
        "mode must be valid, warnings or error"
    );
    (nodes, mode == "error", mode == "warnings", warmup, runs)
}

fn parse_option(args: &mut impl Iterator<Item = String>, name: &str, default: usize) -> usize {
    let Some(option) = args.next() else {
        return default;
    };
    assert_eq!(option, name, "unexpected option");
    args.next()
        .expect("option value")
        .parse::<usize>()
        .expect("option value must be an integer")
}

fn graph(
    nodes: usize,
    with_error: bool,
    with_warnings: bool,
) -> (
    GraphDocument,
    std::sync::Arc<orange_model::catalog::RegistrySnapshot>,
) {
    let mut saved_nodes = Vec::with_capacity(nodes + usize::from(with_error));
    let mut saved_edges = Vec::with_capacity(nodes.saturating_sub(1));
    for index in 0..nodes {
        let current = 10 + index as u32;
        let mut node = math(current, "add", index as u64, 1);
        if index > 0 && !with_warnings {
            node["inputs"]
                .as_object_mut()
                .expect("literal inputs")
                .remove("a");
        }
        saved_nodes.push(node);
        if index > 0 {
            saved_edges.push(edge(
                100_000 + index as u32,
                current - 1,
                "sum",
                current,
                "a",
            ));
        }
    }
    let (mut document, registry) = Source::new(saved_nodes, saved_edges).analyzed();
    if with_error {
        let unknown: NodeInstance = serde_json::from_value(json!({
            "id": id(50_000),
            "definition_id": "org.orange.measurement/missing-definition",
            "definition_version": 1,
            "config_version": 1,
            "config": {}
        }))
        .expect("unknown definition node is structurally valid");
        let _ = document
            .apply([DocCommand::AddNode {
                body: BodyRef::Root,
                node: unknown,
            }])
            .expect("add error-bearing node");
    }
    // Build the document's lazy body directory before establishing the owner baseline.
    let first: NodeId = id(10).parse().expect("first add ID");
    assert!(document.locate_node(first).is_some());
    (document, registry)
}

fn category(name: &str) -> usize {
    if name == "published.signatures" || name == "retained.args" {
        0 // resolved signatures and retained inference arguments
    } else if name == "published.diagnostics"
        || name.starts_with("retained.node_diagnostics")
        || name.starts_with("retained.inputs")
        || name.starts_with("retained.config")
        || name.starts_with("retained.functions")
        || name.starts_with("retained.base")
        || name.starts_with("retained.flow")
        || name.starts_with("retained.state")
    {
        1 // diagnostic buckets
    } else if name == "published.report_rest" {
        2 // incomplete markers, function summaries, witnesses and regions
    } else if name == "retained.metadata" {
        3
    } else {
        4 // local cache, index, effects and session tracking
    }
}

fn measure(nodes: usize, with_error: bool, with_warnings: bool, warmup: usize, runs: usize) {
    let (document, registry) = graph(nodes, with_error, with_warnings);
    let mut release_rows: Vec<(&'static str, usize)> = Vec::with_capacity(24);
    let expected_owner_baseline = counter::live_bytes();
    let mut allocation_sum = 0_u64;
    let mut allocated_bytes_sum = 0_u64;
    let mut retained_sum = 0_usize;
    let mut peak_max = 0_usize;
    let mut diagnostic_count = 0_usize;
    let mut measured_runs = 0_usize;
    let mut final_partition_overhead = 0_i128;
    let mut final_container_release = 0_usize;
    let mut final_residual = 0_i128;
    let mut category_release = [0_usize; 5];

    for iteration in 0..warmup + runs {
        let before_session = counter::live_bytes();
        let window = Snapshot::start();
        let session = AnalysisSession::new(
            &document,
            registry.as_ref(),
            BTreeMap::new(),
            AnalysisOptions::default(),
        );
        let session_diagnostics = session.analysis().report().diagnostics.len();
        if with_error {
            assert!(session_diagnostics > 0);
        } else {
            assert!(session.analysis().report().is_valid());
            assert_eq!(
                session_diagnostics,
                if with_warnings { nodes - 1 } else { 0 }
            );
        }
        let retained_live = counter::live_bytes().saturating_sub(before_session);
        let counts = window.finish();
        let before_partition = counter::live_bytes();
        let parts = session.__measurement_parts();
        let after_partition = counter::live_bytes();
        let partition_overhead = after_partition as i128 - before_partition as i128;
        let mut iteration_release = [0_usize; 5];
        release_rows.clear();
        let mut parts = parts.into_iter();
        while let Some((name, part)) = parts.next() {
            let before = counter::live_bytes();
            drop(part);
            let released = before.saturating_sub(counter::live_bytes());
            release_rows.push((name, released));
            iteration_release[category(name)] += released;
        }
        let before_container_drop = counter::live_bytes();
        drop(parts);
        let container_release = before_container_drop.saturating_sub(counter::live_bytes());
        let residual = counter::live_bytes() as i128 - expected_owner_baseline as i128;

        if iteration >= warmup {
            measured_runs += 1;
            allocation_sum += counts.allocations;
            allocated_bytes_sum += counts.allocated_bytes;
            retained_sum += retained_live;
            peak_max = peak_max.max(counts.peak_live_bytes);
            diagnostic_count = session_diagnostics;
            final_partition_overhead = partition_overhead;
            final_container_release = container_release;
            final_residual = residual;
            category_release = iteration_release;
        }
    }

    let mode = if with_error {
        "error-bearing"
    } else if with_warnings {
        "warnings"
    } else {
        "clean"
    };
    println!(
        "case nodes={nodes} mode={mode} warmup={warmup} runs={measured_runs} diagnostics={diagnostic_count} AnalysisSession::new allocations/run={} bytes/run={} retained_before_partition_avg={}B peak_live_max={}B partition_overhead_last={}B",
        allocation_sum / measured_runs as u64,
        allocated_bytes_sum / measured_runs as u64,
        retained_sum / measured_runs,
        peak_max,
        final_partition_overhead,
    );
    println!(
        "partition drop order and immediate live-byte release (shared storage appears when its last owner is dropped):"
    );
    for (name, released) in &release_rows {
        println!("  {name}: {released}B");
    }
    println!(
        "partition category release totals: signatures+inference={}B diagnostics={}B report-rest={}B retained-metadata={}B other-session-state={}B",
        category_release[0],
        category_release[1],
        category_release[2],
        category_release[3],
        category_release[4]
    );
    println!(
        "partition-container released={}B; final live delta from warmed document+registry baseline={}B (zero means all session-owned heap was retired)",
        final_container_release, final_residual
    );
}

fn main() {
    let (nodes, with_error, with_warnings, warmup, runs) = arguments();
    measure(nodes, with_error, with_warnings, warmup, runs);
}
