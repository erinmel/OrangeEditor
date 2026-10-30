//! Measurement baselines for core workloads: allocation count, allocated bytes, peak
//! live bytes and the per-pump wall-time distribution.
//!
//! ```text
//! cargo run --release -p orange-measure --locked -- [--budget N] [--warmup N] [--runs N] [FILTER]
//! ```
//!
//! `FILTER` keeps workloads whose name contains it. Numbers describe the machine they ran
//! on; they are evidence for choosing optimizations, not acceptance thresholds.

mod counter;
mod graph;
mod harness;
mod workloads;

use harness::{Config, Row};

#[global_allocator]
static ALLOCATOR: counter::CountingAllocator = counter::CountingAllocator;

type Workload = (&'static str, fn(Config) -> Row);

const WORKLOADS: &[Workload] = &[
    ("scalar-chain", workloads::scalar_chain),
    ("host-inputs-384", workloads::host_inputs_384),
    ("function-call", workloads::function_call),
    ("nested-for-each", workloads::nested_for_each),
    ("fork-cancel", workloads::fork_cancel),
    ("fork-fanout-256", workloads::fork_fanout_256),
    ("fork-fanout-4000", workloads::fork_fanout_4000),
    ("map-cas-16", workloads::map_cas_16),
    ("map-cas-1000", workloads::map_cas_1000),
    ("host-map-16", workloads::host_map_16),
    ("host-map-1000", workloads::host_map_1000),
    ("map-zone", workloads::map_zone),
];

fn main() {
    let mut config = Config {
        budget: 64,
        warmup: 3,
        runs: 15,
    };
    let mut filter = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut number = |name: &str| -> usize {
            args.next()
                .and_then(|value| value.parse().ok())
                .filter(|value| *value > 0)
                .unwrap_or_else(|| panic!("{name} needs a positive integer"))
        };
        match arg.as_str() {
            "--budget" => config.budget = number("--budget"),
            "--warmup" => config.warmup = number("--warmup"),
            "--runs" => config.runs = number("--runs"),
            _ => filter = arg,
        }
    }
    let profile = if cfg!(debug_assertions) {
        "debug (not representative; use --release)"
    } else {
        "release"
    };
    println!(
        "profile={profile} target={}-{} budget={} warmup={} runs={}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        config.budget,
        config.warmup,
        config.runs
    );
    let rows: Vec<Row> = WORKLOADS
        .iter()
        .filter(|(name, _)| name.contains(filter.as_str()))
        .map(|(_, workload)| workload(config))
        .collect();
    harness::print_table(&rows);
    if "oversized-map-zone".contains(filter.as_str()) {
        println!(
            "oversized map zone: {} peak heap bytes before ResourceLimit",
            workloads::oversized_map_zone_peak_bytes()
        );
    }
    if "oversized-json-literal".contains(filter.as_str()) {
        println!(
            "oversized json literal: {} heap bytes allocated before TooLarge",
            workloads::oversized_json_literal_allocated_bytes()
        );
    }
    if "string-list-eq".contains(filter.as_str()) {
        println!(
            "string list equality (65536 shared elements, 16 compares) p50: {:.3} ms",
            workloads::string_list_equality(config).as_secs_f64() * 1e3
        );
    }
    if "disposed-run".contains(filter.as_str()) {
        println!(
            "disposed run: {} heap bytes released only on drop",
            workloads::disposed_run_retained_bytes()
        );
    }
    if "session-edit".contains(filter.as_str()) {
        let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1e3;
        for (chains, length) in [(1000, 10), (1, 10_000)] {
            let (p50, p95) = workloads::session_edit(config, chains, length);
            println!(
                "session literal edit ({chains} chains of {length} nodes) p50: {:.3} ms, p95: {:.3} ms",
                ms(p50),
                ms(p95)
            );
        }
    }
    if "analysis-worker".contains(filter.as_str()) {
        let times = workloads::analysis_worker_latency(config);
        let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1e3;
        println!(
            "analysis worker (2000 nodes) p50: snapshot {:.3} ms, submit {:.3} ms, \
             full {:.3} ms, resubmission {:.3} ms, shutdown request {:.3} ms, join {:.3} ms",
            ms(times.snapshot),
            ms(times.submit),
            ms(times.full),
            ms(times.superseded),
            ms(times.shutdown),
            ms(times.join)
        );
    }
}
