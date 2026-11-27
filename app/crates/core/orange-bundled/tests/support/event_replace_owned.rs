//! Self-emission must restore checked-out strands and retained function frames first.
use super::event_replace::{quiesce, release, replacement};
use super::*;

fn set(n: u32, variable: u32, value: u64) -> Json {
    let mut node = variable_node(n, "set-variable", variable);
    node["inputs"] = json!({"value":{"type":"core/u64@1","value":value.to_string()}});
    node
}

#[test]
fn self_emit_from_fork_and_function_children_preserves_old_cleanup_context() {
    for in_function in [false, true] {
        let mut switch = core(11, "switch");
        switch["type_pins"] = json!({"K":"core/u64@1"});
        switch["config"] = json!({"cases":{"0":1}});
        switch["variadic"] = json!({"case":{"order":[0],"next":1}});
        let mut fork = core(303, "fork");
        fork["config"] = json!({"join":"all"});
        fork["variadic"] = json!({"branch":{"order":[0,1],"next":2}});
        let body_nodes = vec![
            set(302, 200, 1),
            fork,
            wait_node(304, 100),
            wait_node(305, 5),
            emit(306, 2),
            set(307, 102, 99),
            variable_node(308, "get-variable", 200),
            set(309, 101, 0),
        ];
        let body_edges = vec![
            edge(301, 302, "then", 303, "in"),
            edge(302, 303, "branch#0", 304, "in"),
            edge(303, 303, "branch#1", 305, "in"),
            edge(304, 305, "then", 306, "in"),
            edge(305, 306, "then", 307, "in"),
            edge(306, 304, "cancelled", 309, "in"),
            edge(307, 308, "value", 309, "value"),
        ];
        let (package, registry) = catalog();
        let mut nodes = vec![
            core(1, "on-start"),
            emit(2, 1),
            replacement(10, 8, "fault"),
            switch,
            set(12, 100, 2),
        ];
        let mut edges = vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 10, "then", 11, "in"),
            edge(3, 10, "payload", 11, "key"),
            edge(4, 11, "default", 12, "in"),
        ];
        let mut functions = vec![];
        if in_function {
            let mut call = core(13, "call");
            call["config"] = json!({"function":id(300)});
            nodes.extend([call, set(14, 102, 88)]);
            edges.extend([
                edge(5, 11, "case#0", 13, "in"),
                edge(6, 13, "then", 14, "in"),
            ]);
            let mut body_nodes = body_nodes;
            body_nodes.extend([core(301, "function-entry"), core(310, "return")]);
            let mut body_edges = body_edges;
            body_edges.extend([
                edge(308, 301, "then", 302, "in"),
                edge(309, 303, "completed", 310, "in"),
            ]);
            functions.push(json!({"id":id(300),"name":"replace-owned","kind":"async","inputs":[],"outputs":[],"nodes":body_nodes,"edges":body_edges}));
        } else {
            nodes.extend(body_nodes);
            edges.extend(body_edges);
            edges.push(edge(5, 11, "case#0", 302, "in"));
        }
        let source = graph(&package, nodes, edges, vec![]);
        let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
        saved["functions"] = json!(functions);
        saved["events"] = json!([{"id":EVENT,"name":"pulse","payload":"core/u64@1","scope":"run"}]);
        saved["variables"] = json!([
            variable(100, "run", Some(0)),
            variable(101, "run", Some(0)),
            variable(102, "run", Some(0)),
            variable(200, "local", Some(0))
        ]);
        let doc = from_json(&saved.to_string()).unwrap();
        let analysis = analyze(&doc, &registry);
        let plan = Arc::new(
            compile_control(
                &doc,
                registry,
                &analysis,
                &package,
                CompileOptions::default(),
            )
            .unwrap(),
        );
        let mut run = run(&doc, plan);
        quiesce(&mut run);
        assert_eq!(run.event_stats().delivered, 1);
        run.advance_to(5).unwrap();
        quiesce(&mut run);
        assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
        assert_eq!(
            run.variable(id(101).parse().unwrap()),
            Some(&Value::U64(1)),
            "original local frame must survive cleanup"
        );
        assert_eq!(
            run.variable(id(102).parse().unwrap()),
            Some(&Value::U64(0)),
            "self-emitter/caller continuation must be cancelled"
        );
        assert_eq!(run.event_stats().delivered, 2);
        assert_eq!(run.event_stats().live_payload_bytes, 0);
        assert_eq!(run.cleanup_truncated(), 0);
        let cleanup = run
            .trace()
            .iter()
            .position(|e| e.node.to_string() == id(309) && e.outcome == ControlOutcome::Then)
            .unwrap();
        let latest = run
            .trace()
            .iter()
            .enumerate()
            .filter(|(_, e)| e.node.to_string() == id(10) && e.outcome == ControlOutcome::Then)
            .nth(1)
            .unwrap()
            .0;
        assert!(
            cleanup < latest,
            "latest must wait for old descendants and function frames"
        );
        release(run);
    }
}
