//! Machine-readable whole-session measurements. Compilation and fixture construction are
//! outside the window; start, every budgeted pump, stop/dispose and drop are inside it.

use crate::harness::{Config, Row};
use serde_json::{Value, json};
use std::time::Duration;

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

fn report(row: Row) -> Value {
    let allocations = cfg!(feature = "allocation-counting").then(|| {
        json!({
            "allocations": row.counts.allocations,
            "allocated_bytes": row.counts.allocated_bytes,
            "peak_live_bytes": row.counts.peak_live_bytes,
            "varies": row.counts_vary,
        })
    });
    json!({
        "workload": row.name,
        "units": row.units,
        "unit": row.unit,
        "pumps_per_run": row.pumps,
        "allocation_counts": allocations,
        "total_ms": {
            "p50": milliseconds(row.total_p50),
            "p95": milliseconds(row.total_p95),
            "p99": milliseconds(row.total_p99),
            "max": milliseconds(row.total_max),
            "samples": row.total_samples.into_iter().map(milliseconds).collect::<Vec<_>>(),
        },
    })
}

pub fn run(config: Config, filter: &str) {
    let rows: Vec<_> = crate::WORKLOADS
        .iter()
        .filter(|(name, _)| name.contains(filter))
        .map(|(_, workload)| report(workload(config)))
        .collect();
    assert!(!rows.is_empty(), "no execution workload matches {filter:?}");
    let output = json!({
        "schema": 1,
        "architecture": std::env::consts::ARCH,
        "os": std::env::consts::OS,
        "debug_assertions": cfg!(debug_assertions),
        "allocation_counting": cfg!(feature = "allocation-counting"),
        "budget": config.budget,
        "warmup_runs": config.warmup,
        "warmup_seconds": config.warmup_seconds,
        "runs": config.runs,
        "window": "session start, all pumps, stop/dispose/drop; excludes document/plan construction; timers use virtual time",
        "workloads": rows,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("report JSON")
    );
}
