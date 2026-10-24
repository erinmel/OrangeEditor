//! Release timing of one semantic edit, and of validation, in large documents (NFR-15
//! evidence aid).
//!
//! Run with `cargo test --release -p orange-model --test perf_edit_baseline -- --ignored
//! --nocapture`. It asserts nothing: it prints timings for the machine it runs on.

mod common;

use std::time::{Duration, Instant};

use common::generic::{ADD, registry_with_provider};
use common::minimal_document;
use orange_model::DocIndex;
use orange_model::catalog::ProviderKey;
use orange_model::document::Edge;
use orange_model::inference::{apply_inferred, apply_inferred_indexed};
use orange_model::validation::validate;
use orange_model::{DocCommand, GraphDocument, from_json};
use serde_json::{Value as Json, json};

const DIGEST: &str = "sha256:abababababababababababababababababababababababababababababababab";
const RUNS: u32 = 5;

fn key() -> ProviderKey {
    ProviderKey {
        package: "org.test".to_owned(),
        version: "1.0.0".to_owned(),
        content_digest: DIGEST.to_owned(),
        id: "add".to_owned(),
    }
}

fn node_id(n: usize) -> String {
    format!("10000000-0000-4000-8000-{n:012}")
}

fn add_node(n: usize) -> Json {
    json!({"id": node_id(n), "definition_id": ADD, "definition_version": 1, "config_version": 1,
        "config": {}, "type_pins": {"T": "core/i64@1"},
        "provider": {"package": "org.test", "version": "1.0.0", "id": "add"}})
}

fn edge_json(n: usize, from: usize, to: usize, port: &str) -> Json {
    json!({"id": format!("20000000-0000-4000-8000-{n:012}"),
        "from": {"node": node_id(from), "port": "sum"}, "to": {"node": node_id(to), "port": port}})
}

/// A chain of `n` `add` nodes plus two isolated nodes `n` and `n + 1`.
fn chain(n: usize) -> GraphDocument {
    let nodes: Vec<_> = (0..n + 2).map(add_node).collect();
    let edges: Vec<_> = (1..n).map(|i| edge_json(i, i - 1, i, "a")).collect();
    from_json(&minimal_document(json!({
        "package_lock": [{"id": "org.test", "version": "1.0.0", "content_digest": DIGEST}],
        "nodes": nodes,
        "edges": edges,
    })))
    .unwrap()
}

/// Best of `RUNS` runs of `f` on a fresh clone of `doc`; the clone is not timed.
fn best(doc: &GraphDocument, mut f: impl FnMut(&mut GraphDocument)) -> Duration {
    (0..RUNS)
        .map(|_| {
            let mut copy = doc.clone();
            let start = Instant::now();
            f(&mut copy);
            start.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
#[ignore = "release timing; run with --ignored --nocapture"]
fn edit_cost_by_document_size() {
    let registry = registry_with_provider(key());
    for n in [1_000usize, 10_000] {
        let doc = chain(n);
        for (label, from, to) in [("tail of chain", n - 1, n), ("isolated pair", n, n + 1)] {
            let edge: Edge = serde_json::from_value(edge_json(n + 10, from, to, "b")).unwrap();
            let time = best(&doc, |copy| {
                let _ = apply_inferred(
                    copy,
                    [DocCommand::Connect { edge: edge.clone() }],
                    &registry,
                )
                .unwrap();
            });
            println!("n={n:>6} apply_inferred connect ({label}): {time:.3?}");
            let time = (0..RUNS)
                .map(|_| {
                    let mut copy = doc.clone();
                    let mut index = DocIndex::build(&copy);
                    // Like the index, the document's node lookup is built once per document.
                    let _ = copy.node(node_id(0).parse().unwrap());
                    let start = Instant::now();
                    let _ = apply_inferred_indexed(
                        &mut copy,
                        &mut index,
                        [DocCommand::Connect { edge: edge.clone() }],
                        &registry,
                    )
                    .unwrap();
                    start.elapsed()
                })
                .min()
                .unwrap();
            println!("n={n:>6} apply_inferred_indexed connect ({label}): {time:.3?}");
        }
        let time = best(&doc, |copy| {
            std::hint::black_box(validate(copy, &registry));
        });
        println!("n={n:>6} validate: {time:.3?}");
    }
}

fn function_id(n: usize) -> String {
    format!("30000000-0000-4000-8000-{n:012}")
}

/// `depth` pure `x -> y` functions, each passing its input through a call to the next.
fn call_chain(depth: usize) -> GraphDocument {
    let marker = |id: &str, definition: &str| {
        json!({"id": id, "definition_id": definition, "definition_version": 1,
            "config_version": 1, "config": {}})
    };
    let port = |id: &str, direction: &str| json!({"port_id": id, "direction": direction, "kind": "data", "type": "core/i64@1"});
    let edge = |n: usize, from: &str, from_port: &str, to: &str, to_port: &str| {
        json!({"id": format!("20000000-0000-4000-8000-{n:012}"),
            "from": {"node": from, "port": from_port}, "to": {"node": to, "port": to_port}})
    };
    let functions: Vec<_> = (0..depth)
        .map(|i| {
            let (entry, out, call) = (node_id(3 * i), node_id(3 * i + 1), node_id(3 * i + 2));
            let mut nodes = vec![
                marker(&entry, "core/function-entry"),
                marker(&out, "core/function-outputs"),
            ];
            let edges = if i + 1 < depth {
                let mut call_node = marker(&call, "core/call");
                call_node["config"] = json!({"function": function_id(i + 1)});
                nodes.push(call_node);
                vec![
                    edge(2 * i, &entry, "x", &call, "x"),
                    edge(2 * i + 1, &call, "y", &out, "y"),
                ]
            } else {
                vec![edge(2 * i, &entry, "x", &out, "y")]
            };
            json!({"id": function_id(i), "name": format!("f{i}"), "kind": "pure",
                "nodes": nodes, "edges": edges,
                "inputs": [port("x", "input")], "outputs": [port("y", "output")]})
        })
        .collect();
    from_json(&minimal_document(json!({"functions": functions}))).unwrap()
}

#[test]
#[ignore = "release timing; run with --ignored --nocapture"]
fn validate_cost_by_call_chain_depth() {
    let registry = registry_with_provider(key());
    for depth in [200usize, 1_000, 4_000] {
        let doc = call_chain(depth);
        let report = validate(&doc, &registry);
        assert!(report.diagnostics.is_empty());
        assert_eq!(report.resolved.len(), 3 * depth - 1);
        let time = best(&doc, |copy| {
            std::hint::black_box(validate(copy, &registry));
        });
        println!("depth={depth:>5} validate: {time:.3?}");
    }
}
