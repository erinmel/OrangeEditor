//! Bounded JSON import profiles used by the R6/B0 evidence harness.

use crate::counter::{self, Snapshot};
use crate::harness::Config;
use orange_model::{ImportLimits, from_json_with_limits};
use serde_json::{Map, Value as Json, json};
use std::time::{Duration, Instant};

fn id(n: usize) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}

fn node(n: usize) -> Json {
    json!({
        "id": id(n),
        "definition_id": "org.example/import-node",
        "definition_version": 1,
        "config_version": 1,
        "config": {}
    })
}

fn encoded(profile: &str) -> (String, ImportLimits, &'static str) {
    let mut value = json!({
        "format": "orange.graph",
        "format_version": 1,
        "graph_id": id(1),
        "revision": 0,
        "nodes": (0..1_000).map(|n| node(n + 10)).collect::<Vec<_>>(),
        "edges": [],
        "functions": []
    });
    let limits = match profile {
        "rejected-node-quota" => ImportLimits::new(8 * 1024 * 1024, 100, 10_000),
        _ => ImportLimits::default(),
    };
    if matches!(profile, "extension-array-262k" | "extension-array-1m") {
        let count = if profile == "extension-array-262k" {
            262_144
        } else {
            1_000_000
        };
        let envelope = value.to_string();
        let mut text = String::with_capacity(envelope.len() + count * 2 + 64);
        text.push_str(&envelope[..envelope.len() - 1]);
        text.push_str(",\"profile_data\":{\"items\":[");
        for index in 0..count {
            if index > 0 {
                text.push(',');
            }
            text.push('0');
        }
        text.push_str("]}}");
        return (text, limits, "observe");
    }
    match profile {
        "key-heavy" => {
            let mut keys = Map::new();
            for key in 0..4_096 {
                keys.insert(format!("extension-key-{key:04}"), json!([key, "v", true]));
            }
            value["profile_data"] = Json::Object(keys);
        }
        "value-heavy-escaped" => {
            value["profile_data"] = json!({
                "escaped": "line\\nquote\\\"slash\\\\tab\\t".repeat(8_192),
                "unicode": "áéíóú🟠".repeat(4_096)
            });
        }
        _ => {}
    }
    let text = value.to_string();
    let outcome = if profile == "rejected-node-quota" {
        "expected-error"
    } else {
        "expected-ok"
    };
    (text, limits, outcome)
}

fn percentile(values: &[Duration], percent: usize) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

fn measure(profile: &str, config: Config) {
    let (text, limits, expected) = encoded(profile);
    let encoded_bytes = text.len();
    let measured = config.runs;
    let mut durations = Vec::with_capacity(measured);
    let mut allocation_total = 0_u64;
    let mut cumulative_bytes = 0_u64;
    let mut max_peak = 0_usize;
    let mut max_live = 0_usize;
    let mut max_drop_release = 0_usize;
    let mut accepted_outcome = false;
    for sample in 0..config.warmup + measured {
        let before_live = counter::live_bytes();
        let window = Snapshot::start();
        let start = Instant::now();
        let result = from_json_with_limits(&text, limits);
        let elapsed = start.elapsed();
        let live_after_parse = counter::live_bytes().saturating_sub(before_live);
        let accepted = result.is_ok();
        if expected != "observe" && (expected == "expected-ok") != accepted {
            panic!(
                "{profile} produced {}, expected {expected}",
                if accepted { "accepted" } else { "rejected" }
            );
        }
        drop(result);
        let drop_release =
            live_after_parse.saturating_sub(counter::live_bytes().saturating_sub(before_live));
        let counts = window.finish();
        if sample >= config.warmup {
            accepted_outcome = accepted;
            durations.push(elapsed);
            allocation_total += counts.allocations;
            cumulative_bytes += counts.allocated_bytes;
            max_peak = max_peak.max(counts.peak_live_bytes);
            max_live = max_live.max(live_after_parse);
            max_drop_release = max_drop_release.max(drop_release);
        }
    }
    let ms = |duration: Duration| duration.as_secs_f64() * 1e3;
    let min = durations.iter().min().copied().unwrap_or_default();
    let max = durations.iter().max().copied().unwrap_or_default();
    let outcome = if accepted_outcome {
        "accepted"
    } else {
        "rejected"
    };
    println!(
        "import {profile}: outcome={outcome} encoded={}B samples={} warmup={} p50={:.3}ms p95={:.3}ms min={:.3}ms max={:.3}ms allocations+drop/run={:.0} cumulative_allocated+drop={}B peak_live={}B live_after_parse={}B released_on_drop={}B",
        encoded_bytes,
        durations.len(),
        config.warmup,
        ms(percentile(&durations, 50)),
        ms(percentile(&durations, 95)),
        ms(min),
        ms(max),
        allocation_total as f64 / measured as f64,
        cumulative_bytes,
        max_peak,
        max_live,
        max_drop_release
    );
}

pub fn run(config: Config) {
    for profile in [
        "node-heavy",
        "key-heavy",
        "value-heavy-escaped",
        "extension-array-262k",
        "extension-array-1m",
        "rejected-node-quota",
    ] {
        measure(profile, config);
    }
}
