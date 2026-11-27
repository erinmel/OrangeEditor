//! Local-only measurements, never a CI timing gate. Run with --release --locked.
use orange_editor_ui::{
    canvas::{Canvas, auto_layout},
    palette,
};
use orange_host::intrinsic;
use orange_host::{GraphView, Host, HostOptions, NodeView, PortView, WireView};
use std::{hint::black_box, time::Instant};

fn graph(count: u32) -> GraphView {
    let id = |n: u32| format!("10000000-0000-4000-8000-{n:012x}").parse().unwrap();
    let nodes = (0..count)
        .map(|n| {
            NodeView {
                id: id(n),
                name: format!("Node {n}"),
                kind: "Step".into(),
                definition: intrinsic::SEQUENCE.into(),
                detail: None,
                entry: false,
                ports: [true, false]
                    .into_iter()
                    .map(|input| PortView {
                        id: if input { "in" } else { "then" }.into(),
                        input,
                        exec: true,
                        ty: None,
                        value: None,
                        connected: true,
                        default: None,
                        label: None,
                    })
                    .collect(),
                position: None,
                settings: serde_json::json!({}),
                schema: None,
                references: Vec::new(),
                types: Vec::new(),
                variadic: Vec::new(),
                paths: Vec::new(),
                controls: Default::default(),
                missing: None,
            }
            .into()
        })
        .collect();
    // Reverse edge order is a valid document order and stresses repeated relaxation.
    let wires = (1..count)
        .rev()
        .map(|n| WireView {
            from: (id(n - 1), "then".into()),
            to: (id(n), "in".into()),
            exec: true,
        })
        .collect();
    GraphView { nodes, wires }
}

fn measure(label: &str, iterations: u32, mut work: impl FnMut()) {
    work();
    let start = Instant::now();
    for _ in 0..iterations {
        work();
    }
    println!(
        "{label}: {:.3} ms/op ({iterations} iterations)",
        start.elapsed().as_secs_f64() * 1000.0 / f64::from(iterations)
    );
}

fn main() {
    for count in [100, 1_000, 5_000] {
        let mut graph = graph(count);
        measure(&format!("layout/{count}"), 3, || {
            black_box(auto_layout(&graph));
        });
        for (n, node) in graph.nodes.iter_mut().enumerate() {
            std::sync::Arc::make_mut(node).position =
                Some([(n % 100) as f32 * 330.0, (n / 100) as f32 * 120.0]);
        }
        measure(&format!("saved-layout/{count}"), 3, || {
            black_box(auto_layout(&graph));
        });
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.reset_zoom();
        let context = egui::Context::default();
        measure(&format!("canvas/{count}"), 20, || {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 720.0),
                )),
                ..Default::default()
            };
            let output = context.run_ui(input, |ui| {
                black_box(canvas.show(ui, &graph, &[]));
            });
            black_box(output.shapes.len());
            output.drop_without_applying_deltas();
        });
    }
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    measure("palette/public-uncached", 100, || {
        black_box(palette(&host, ""));
    });
    for count in [100, 1_000, 5_000] {
        let view = graph(count);
        let variable = "20000000-0000-4000-8000-000000000001";
        let nodes: Vec<_> = view.nodes.iter().map(|node| serde_json::json!({
            "id": node.id.to_string(), "definition_id": intrinsic::SET_VARIABLE, "definition_version": 1,
            "config_version": 1, "config": {"variable": variable},
            "inputs": {"value": {"type": "core/i64@1", "value": "7"}}
        })).collect();
        let edges: Vec<_> = view
            .wires
            .iter()
            .enumerate()
            .map(|(n, wire)| {
                serde_json::json!({
                    "id": format!("30000000-0000-4000-8000-{n:012x}"),
                    "from": {"node": wire.from.0.to_string(), "port": "then"},
                    "to": {"node": wire.to.0.to_string(), "port": "in"}
                })
            })
            .collect();
        let text = serde_json::json!({"format":"orange.graph", "format_version":1,
            "graph_id":"40000000-0000-4000-8000-000000000001", "revision":1,
            "package_lock":[], "type_lock":[], "events":[], "connections":[],
            "signatures":{}, "functions":[], "exports":[], "editor":{},
            "variables":[{"id":variable, "name":"count", "type":"core/i64@1",
                "scope":"run", "persistent":false, "conflict":"reject"}],
            "nodes":nodes, "edges":edges})
        .to_string();
        measure(&format!("host-view/{count}"), 3, || {
            black_box(host.view(&text).unwrap());
        });
        // One value edit as the editor makes it, then its problems and view.
        let first: orange_host::NodeId = view.nodes[0].id;
        let mut value = 0u32;
        let mut next = || {
            value += 1;
            value.to_string()
        };
        // Text in and out: parse, edit, write, check from scratch, view.
        let mut saved = text.clone();
        measure(&format!("edit-cycle-text/{count}"), 3, || {
            let mut parsed = host.draft(&saved).unwrap();
            let typed = next();
            let _ = host
                .set_input(&mut parsed, (first, "value"), Some(&typed))
                .unwrap();
            saved = parsed.text().unwrap();
            black_box(host.check(&saved).unwrap());
            black_box(host.view(&saved).unwrap());
        });
        // A draft: edit in place, problems follow the edit, view.
        let mut draft = host.draft(&text).unwrap();
        black_box(host.problems(&mut draft).unwrap());
        measure(&format!("edit-cycle-draft/{count}"), 3, || {
            let typed = next();
            black_box(
                host.set_input(&mut draft, (first, "value"), Some(&typed))
                    .unwrap(),
            );
            black_box(host.problems(&mut draft).unwrap());
            black_box(host.draft_view(&draft).unwrap());
        });
    }
}
