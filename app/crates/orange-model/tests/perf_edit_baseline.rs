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
use orange_model::validation::{
    ConnectionValidation, validate, validate_availability, validate_exports, validate_nodes,
    validate_regions,
};
use orange_model::{DocCommand, GraphDocument, from_json};
use serde_json::{Value as Json, json};

const DIGEST: &str = "sha256:abababababababababababababababababababababababababababababababab";
const RUNS: u32 = 5;

#[test]
#[ignore = "release timing evidence, not NFR acceptance"]
fn data_cycles_cost_by_port_width() {
    use orange_model::catalog::NodeContract;
    use orange_model::document::{DefinitionRef, NodeKind, OutputSource, PortDescriptor};
    use orange_model::validation::{validate_connections, validate_data_cycles, validate_inputs};
    let definition = DefinitionRef {
        id: "org.test/wide".parse().unwrap(),
        version: 1,
    };
    let ports = vec![
        PortDescriptor::data_in("items", "core/i64@1".parse().unwrap()).variadic(0, None),
        PortDescriptor::data_out(
            "results",
            "core/i64@1".parse().unwrap(),
            OutputSource::Computed,
        )
        .variadic(0, None),
    ];
    let mut facts = common::registry::Facts::new(common::generic::origin());
    facts.definition(
        definition.clone(),
        NodeContract {
            kind: NodeKind::Pure,
            suspends: None,
            type_params: vec![],
            ports: ports.clone(),
        },
    );
    let mut provider = key();
    provider.id = "wide".into();
    facts.provider(definition, 1, provider);
    let registry = facts.snapshot();
    let snapshot = orange_model::signature::project(NodeKind::Pure, ports, &Default::default());
    for width in [64usize, 256, 512] {
        let mut node = json!({"id":node_id(0),"definition_id":"org.test/wide","definition_version":1,
            "config_version":1,"config":{},"signature":"wide",
            "provider":{"package":"org.test","version":"1.0.0","id":"wide"},
            "variadic":{"items":{"order":(0..width).collect::<Vec<_>>(),"next":width},
                "results":{"order":(0..width).collect::<Vec<_>>(),"next":width}}});
        node["inputs"] = Json::Object(
            (0..width)
                .map(|i| {
                    (
                        format!("items#{i}"),
                        json!({"type":"core/i64@1","value":"0"}),
                    )
                })
                .collect(),
        );
        let doc = from_json(&minimal_document(json!({
            "package_lock":[{"id":"org.test","version":"1.0.0","content_digest":DIGEST}],
            "signatures":{"wide":snapshot},"nodes":[node]
        })))
        .unwrap();
        let nodes = validate_nodes(&doc, &registry);
        assert!(nodes.is_valid(), "{:?}", nodes.diagnostics);
        let connections = validate_connections(&doc, &nodes.resolved, &registry);
        assert!(connections.is_valid());
        assert!(validate_inputs(&doc, &nodes.resolved, &registry).is_empty());
        assert!(validate_data_cycles(&doc, &nodes.resolved, &connections, &registry).is_valid());
        let mut timings = Vec::new();
        for _ in 0..RUNS {
            let start = Instant::now();
            std::hint::black_box(validate_data_cycles(
                &doc,
                &nodes.resolved,
                &connections,
                &registry,
            ));
            timings.push(start.elapsed());
        }
        timings.sort_unstable();
        println!(
            "width={width} data cycles: median={:.3?} max={:.3?}",
            timings[timings.len() / 2],
            timings.last().unwrap()
        );
    }
}

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
        "type_args": {"T": "core/i64@1"}, "signature": "s-add-i64",
        "inputs": {"a": {"type": "core/i64@1", "value": "0"},
            "b": {"type": "core/i64@1", "value": "0"}},
        "provider": {"package": "org.test", "version": "1.0.0", "id": "add"}})
}

fn add_nodes(chain_length: usize, extra: usize) -> Vec<Json> {
    (0..chain_length + extra)
        .map(|n| {
            let mut node = add_node(n);
            if n > 0 && n < chain_length {
                node["inputs"].as_object_mut().unwrap().remove("a");
            }
            node
        })
        .collect()
}

fn add_signatures() -> Json {
    json!({"s-add-i64": {"kind": "pure", "ports": [
        {"port_id": "a", "direction": "input", "kind": "data", "type": "core/i64@1"},
        {"port_id": "b", "direction": "input", "kind": "data", "type": "core/i64@1"},
        {"port_id": "sum", "direction": "output", "kind": "data", "type": "core/i64@1", "source": "computed"}
    ]}})
}

fn edge_json(n: usize, from: usize, to: usize, port: &str) -> Json {
    json!({"id": format!("20000000-0000-4000-8000-{n:012}"),
        "from": {"node": node_id(from), "port": "sum"}, "to": {"node": node_id(to), "port": port}})
}

/// A chain of `n` `add` nodes plus two isolated nodes `n` and `n + 1`.
fn chain(n: usize) -> GraphDocument {
    let nodes = add_nodes(n, 2);
    let edges: Vec<_> = (1..n).map(|i| edge_json(i, i - 1, i, "a")).collect();
    from_json(&minimal_document(json!({
        "package_lock": [{"id": "org.test", "version": "1.0.0", "content_digest": DIGEST}],
        "nodes": nodes,
        "edges": edges,
        "signatures": add_signatures(),
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
        let report = validate(&doc, &registry);
        assert!(report.is_valid(), "{:?}", report.diagnostics);
        assert_eq!(report.resolved.len(), n + 2);
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

#[test]
#[ignore = "release timing; run with --ignored --nocapture"]
fn export_cone_cost_by_document_size() {
    let registry = registry_with_provider(key());
    for n in [1_000usize, 10_000] {
        let doc = from_json(&minimal_document(json!({
            "package_lock": [{"id": "org.test", "version": "1.0.0", "content_digest": DIGEST}],
            "nodes": add_nodes(n, 0),
            "signatures": add_signatures(),
            "edges": (1..n).map(|i| edge_json(i, i - 1, i, "a")).collect::<Vec<_>>(),
            "exports": [{"id": "sum", "node": node_id(n - 1), "port": "sum"}]
        })))
        .unwrap();
        let resolved = validate_nodes(&doc, &registry).resolved;
        assert_eq!(resolved.len(), n);
        assert!(validate_exports(&doc, &resolved, &registry).is_empty());
        let time = best(&doc, |copy| {
            std::hint::black_box(validate_exports(copy, &resolved, &registry));
        });
        println!("n={n:>6} validate_exports (pure chain): {time:.3?}");
    }
}

#[test]
#[ignore = "release timing; run with --ignored --nocapture"]
fn availability_cost_by_action_chain_size() {
    let registry = registry_with_provider(key());
    let variable = "40000000-0000-4000-8000-000000000001";
    let event = "40000000-0000-4000-8000-000000000002";
    for n in [200usize, 1_000, 2_000] {
        let mut nodes = vec![json!({"id":node_id(0),"definition_id":"core/on-event",
            "definition_version":1,"config_version":1,"config":{"event":event},
            "activation":{"mode":"queue","capacity":8,"overflow":"drop-oldest","on_fault":"continue","order":0}})];
        let mut edges = Vec::new();
        for i in 1..=n {
            nodes.push(json!({"id":node_id(i),"definition_id":"core/update-variable",
                "definition_version":1,"config_version":1,"config":{"variable":variable,"operation":"add"}}));
            for (offset, from_port, to_port) in [
                (0, "then", "in"),
                (1, if i == 1 { "payload" } else { "new" }, "value"),
            ] {
                edges.push(json!({"id":format!("20000000-0000-4000-8000-{:012}", 2 * i + offset),
                    "from":{"node":node_id(i - 1),"port":from_port},"to":{"node":node_id(i),"port":to_port}}));
            }
        }
        let doc = from_json(&minimal_document(json!({"nodes":nodes,"edges":edges,
            "variables":[{"id":variable,"name":"v","type":"core/i64@1","scope":"run"}],
            "events":[{"id":event,"name":"e","payload":"core/i64@1","scope":"run"}]
        })))
        .unwrap();
        let resolved = validate_nodes(&doc, &registry).resolved;
        assert_eq!(resolved.len(), n + 1);
        let connections = ConnectionValidation::default();
        let regions = validate_regions(&doc, &resolved, &connections, &registry);
        assert!(regions.is_valid());
        assert!(
            validate_availability(&doc, &resolved, &connections, &regions.index, &registry)
                .is_valid()
        );
        let time = best(&doc, |copy| {
            std::hint::black_box(validate_availability(
                copy,
                &resolved,
                &connections,
                &regions.index,
                &registry,
            ));
        });
        println!("n={n:>6} validate_availability (action/data chain): {time:.3?}");
    }
}
