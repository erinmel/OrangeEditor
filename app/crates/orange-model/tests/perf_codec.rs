//! Descriptive load/save timings; run explicitly with --release --ignored --nocapture.
//! Excludes file I/O, semantic validation and compilation. No NFR threshold assertion.
use std::hint::black_box;
use std::time::{Duration, Instant};

use orange_model::{from_json, to_json};
use serde_json::json;

fn id(prefix: u8, n: usize) -> String {
    format!("{prefix:08x}-0000-4000-8000-{n:012x}")
}

fn fixture(nodes: usize) -> String {
    let nodes: Vec<_> = (1..=nodes).map(|n| json!({
        "id":id(1,n),"definition_id":"org.example/math","definition_version":1,
        "config_version":1,"config":{"mode":"checked","future":{"label":"measurement", "n":n}},
        "inputs":{"a":{"type":"core/i64@1","value":n.to_string()}},"signature":"saved"
    })).collect();
    let edges: Vec<_> = (2..=nodes.len())
        .map(|n| {
            json!({
                "id":id(3,n),"from":{"node":id(1,n-1),"port":"result"},
                "to":{"node":id(1,n),"port":"b"}
            })
        })
        .collect();
    json!({"format":"orange.graph","format_version":1,"graph_id":id(2,1),"revision":0,
        "nodes":nodes,"edges":edges,"future":{"large":18446744073709551615_u64},
        "signatures":{"saved":{"kind":"pure","ports":[
            {"port_id":"a","direction":"input","kind":"data","type":"core/i64@1"},
            {"port_id":"b","direction":"input","kind":"data","type":"core/i64@1"},
            {"port_id":"result","direction":"output","kind":"data","type":"core/i64@1","source":"computed"}
        ]}}}).to_string()
}

fn median<T>(mut operation: impl FnMut() -> T) -> Duration {
    drop(black_box(operation())); // warmup
    let mut samples = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        let value = operation();
        samples.push(start.elapsed());
        drop(black_box(value)); // destruction is outside the timed interval
    }
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[test]
#[ignore = "descriptive release-only codec measurement"]
fn json_load_save_cost_by_document_size() {
    for count in [1_000, 10_000] {
        let source = fixture(count);
        let document = from_json(&source).unwrap();
        assert_eq!(document.root().nodes.len(), count);
        assert_eq!(document.root().edges.len(), count - 1);
        let saved = to_json(&document).unwrap();
        assert_eq!(document, from_json(&saved).unwrap());
        assert_eq!(saved, to_json(&from_json(&saved).unwrap()).unwrap());
        let load = median(|| from_json(black_box(&saved)).unwrap());
        let save = median(|| to_json(black_box(&document)).unwrap());
        println!(
            "nodes={count} edges={} bytes={} load_median={load:.3?} save_median={save:.3?}",
            count - 1,
            saved.len()
        );
    }
}
