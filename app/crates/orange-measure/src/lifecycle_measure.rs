//! Snapshot/session ownership and wide-fork lifecycle measurements for B0.

use crate::counter::{self, Snapshot};
use crate::graph::{Source, U64, core, count, edge, id, literal, math, run_variable, update};
use crate::harness::Config;
use orange_compiler::ControlPlan;
use orange_model::document::PortRef;
use orange_model::validation::{AnalysisOptions, AnalysisSession};
use orange_model::{DocCommand, GraphDocument};
use orange_runtime::{Run, RunOptions, RunState, StopMode};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn document_10k() -> (
    GraphDocument,
    std::sync::Arc<orange_model::catalog::RegistrySnapshot>,
) {
    let nodes = (0..10_000)
        .map(|n| math(10 + n, "add", n as u64, 1))
        .collect();
    Source::new(nodes, Vec::new()).analyzed()
}

fn wide_fork() -> (GraphDocument, Arc<ControlPlan>) {
    const BRANCHES: u32 = 4_000;
    const TOTAL: u32 = 9_000;
    let order: Vec<u32> = (0..BRANCHES).collect();
    let mut fork = core(3, "fork");
    fork["config"] = serde_json::json!({"join": "race"});
    fork["variadic"] = serde_json::json!({"branch": {"next": BRANCHES, "order": order}});
    let mut nodes = vec![
        core(1, "on-start"),
        count(2, 100),
        fork,
        update(5, TOTAL, "add", U64, serde_json::json!("1")),
    ];
    let mut edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "body", 3, "in"),
        edge(3, 3, "branch#0", 5, "in"),
    ];
    for branch in 1..BRANCHES {
        let mut wait = core(100 + branch, "wait");
        wait["inputs"] =
            serde_json::json!({"duration": {"type": "core/duration@1", "value": "1000000000"}});
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
    source.variables = vec![run_variable(TOTAL, U64, Some(serde_json::json!("0")))];
    source.control()
}

#[derive(Default)]
struct ForkStats {
    samples: usize,
    total_allocations: u64,
    total_bytes: u64,
    peak_live: usize,
    retained_after_fork: usize,
    fork_p50: Duration,
    fork_p95: Duration,
    pumps: usize,
    stop_allocations: u64,
    stop_bytes: u64,
    stop_pump_allocations: u64,
    stop_pump_bytes: u64,
    dispose_allocations: u64,
    dispose_bytes: u64,
    dispose_released: usize,
    drop_released: usize,
    drop_allocations: u64,
    drop_bytes: u64,
}

fn percentile(values: &[Duration], percent: usize) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

fn measure_wide_fork(config: Config) {
    let (document, plan) = wide_fork();
    let total = id(9_000).parse().expect("total variable identity");
    let mut durations = Vec::with_capacity(config.runs);
    let mut stats = ForkStats::default();
    for iteration in 0..config.warmup + config.runs {
        let before = counter::live_bytes();
        let window = Snapshot::start();
        let mut run = Run::start(
            plan.clone(),
            &document,
            plan.data().snapshot(),
            42,
            RunOptions::default(),
        )
        .expect("wide fork run starts");
        let fork_start = Instant::now();
        let mut pumps = 0;
        loop {
            let progress = run.pump(64).expect("wide fork pump");
            pumps += 1;
            if !progress.ready && progress.next_wakeup.is_none() {
                break;
            }
        }
        let fork_elapsed = fork_start.elapsed();
        let retained = counter::live_bytes().saturating_sub(before);
        let work_counts = window.finish();
        assert_eq!(run.variable(total), Some(&orange_model::Value::U64(100)));
        assert_eq!(run.ownership().timers, 0);
        if iteration >= config.warmup {
            durations.push(fork_elapsed);
            stats.samples += 1;
            stats.total_allocations += work_counts.allocations;
            stats.total_bytes += work_counts.allocated_bytes;
            stats.peak_live = stats.peak_live.max(work_counts.peak_live_bytes);
            stats.retained_after_fork = stats.retained_after_fork.max(retained);
            stats.pumps += pumps;
        }

        let stop_window = Snapshot::start();
        run.request_stop(StopMode::Graceful)
            .expect("stop requested");
        let stop_counts = stop_window.finish();
        let stop_pump_window = Snapshot::start();
        while run.state() != RunState::Stopped {
            run.pump(64).expect("stop drain pump");
        }
        let stop_pump_counts = stop_pump_window.finish();
        let before_dispose = counter::live_bytes();
        let dispose_window = Snapshot::start();
        run.dispose().expect("run disposed");
        let dispose_counts = dispose_window.finish();
        let dispose_released = before_dispose.saturating_sub(counter::live_bytes());
        let before_drop = counter::live_bytes();
        let drop_window = Snapshot::start();
        drop(run);
        let drop_counts = drop_window.finish();
        let released = before_drop.saturating_sub(counter::live_bytes());
        if iteration >= config.warmup {
            stats.stop_allocations += stop_counts.allocations;
            stats.stop_bytes += stop_counts.allocated_bytes;
            stats.stop_pump_allocations += stop_pump_counts.allocations;
            stats.stop_pump_bytes += stop_pump_counts.allocated_bytes;
            stats.dispose_allocations += dispose_counts.allocations;
            stats.dispose_bytes += dispose_counts.allocated_bytes;
            stats.dispose_released = stats.dispose_released.max(dispose_released);
            stats.drop_released = stats.drop_released.max(released);
            stats.drop_allocations += drop_counts.allocations;
            stats.drop_bytes += drop_counts.allocated_bytes;
        }
    }
    stats.fork_p50 = percentile(&durations, 50);
    stats.fork_p95 = percentile(&durations, 95);
    println!(
        "wide fork 4k: fixture=100 4000-branch races in one Run, warmup={} measured={} pumps/run={:.1} start+fork allocations/run={:.0} cumulative_bytes/run={:.0} peak_live={}B steady_retained_after_fork={}B fork_p50={:.3}ms fork_p95={:.3}ms; request_stop allocations/run={:.1} bytes/run={:.1}; stop-pump allocations/run={:.1} bytes/run={:.1}; dispose allocations/run={:.1} bytes/run={:.1} released_max={}B; drop released_max={}B allocations/run={:.1} bytes/run={:.1}",
        config.warmup,
        stats.samples,
        stats.pumps as f64 / stats.samples as f64,
        stats.total_allocations as f64 / stats.samples as f64,
        stats.total_bytes as f64 / stats.samples as f64,
        stats.peak_live,
        stats.retained_after_fork,
        stats.fork_p50.as_secs_f64() * 1e3,
        stats.fork_p95.as_secs_f64() * 1e3,
        stats.stop_allocations as f64 / stats.samples as f64,
        stats.stop_bytes as f64 / stats.samples as f64,
        stats.stop_pump_allocations as f64 / stats.samples as f64,
        stats.stop_pump_bytes as f64 / stats.samples as f64,
        stats.dispose_allocations as f64 / stats.samples as f64,
        stats.dispose_bytes as f64 / stats.samples as f64,
        stats.dispose_released,
        stats.drop_released,
        stats.drop_allocations as f64 / stats.samples as f64,
        stats.drop_bytes as f64 / stats.samples as f64,
    );
}

fn snapshot_memory(document: &GraphDocument) {
    let baseline = counter::live_bytes();
    let window = Snapshot::start();
    let begin = Instant::now();
    let copy = document.clone();
    let clone_time = begin.elapsed();
    let held = counter::live_bytes().saturating_sub(baseline);
    let peak = window.finish().peak_live_bytes;
    let before_drop = counter::live_bytes();
    let begin = Instant::now();
    drop(copy);
    let drop_time = begin.elapsed();
    println!(
        "document clone: retained={}B peak={}B drop_released={}B clone_us={:.3} drop_us={:.3}",
        held,
        peak,
        before_drop.saturating_sub(counter::live_bytes()),
        clone_time.as_secs_f64() * 1e6,
        drop_time.as_secs_f64() * 1e6
    );
}

pub fn run(config: Config) {
    let process_baseline = counter::live_bytes();
    let (mut document, registry) = document_10k();
    let root_live = counter::live_bytes().saturating_sub(process_baseline);
    let snapshot = Snapshot::start();
    let cloned_registry = registry.clone();
    let registry_clone_counts = snapshot.finish();
    drop(cloned_registry);
    println!(
        "valid 10k graph + registry retained={}B; registry Arc clone allocations={} bytes={} peak={}B",
        root_live,
        registry_clone_counts.allocations,
        registry_clone_counts.allocated_bytes,
        registry_clone_counts.peak_live_bytes
    );

    snapshot_memory(&document);

    let baseline = counter::live_bytes();
    let window = Snapshot::start();
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let session_held = counter::live_bytes().saturating_sub(baseline);
    let session_peak = window.finish().peak_live_bytes;
    println!(
        "AnalysisSession construction: retained={}B peak={}B",
        session_held, session_peak
    );

    let port = PortRef {
        node: id(10).parse().expect("edit node id"),
        port: "a".parse().expect("port id"),
    };
    let command = DocCommand::SetLiteral {
        port,
        literal: Some(serde_json::from_value(literal(99)).expect("literal")),
    };
    let update_window = Snapshot::start();
    let committed = document.apply([command]).expect("literal edit");
    let stats = session.update(
        &document,
        &registry,
        &committed.change_set,
        AnalysisOptions::default(),
    );
    let update_counts = update_window.finish();
    println!(
        "AnalysisSession update: work={:?} input_nodes={} allocations={} bytes={} peak={}B",
        stats.work,
        stats.input_nodes,
        update_counts.allocations,
        update_counts.allocated_bytes,
        update_counts.peak_live_bytes
    );

    let before_drop = counter::live_bytes();
    let drop_window = Snapshot::start();
    drop(session);
    let drop_counts = drop_window.finish();
    let session_drop_release = before_drop.saturating_sub(counter::live_bytes());
    println!(
        "AnalysisSession drop: allocations={} bytes={} peak={}B released={}B",
        drop_counts.allocations,
        drop_counts.allocated_bytes,
        drop_counts.peak_live_bytes,
        session_drop_release
    );

    measure_wide_fork(config);
    drop(document);
}
