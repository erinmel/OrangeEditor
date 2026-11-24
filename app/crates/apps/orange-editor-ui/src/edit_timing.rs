//! Where one edit's time goes on a large graph (docs/10, "Performance"). Run with
//! `ORANGE_EDIT_TIMING_PAIRS=2500 cargo test --release -p orange-editor-ui edit_timing -- --nocapture`.
use super::*;
use std::time::{Duration, Instant};

/// A graph of `pairs` literal → absolute chains: 2 × `pairs` nodes, `pairs` wires.
fn large(host: &mut Host, pairs: usize) -> String {
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    for index in 0..pairs {
        #[expect(clippy::cast_precision_loss, reason = "a test grid")]
        let at = [(index % 50) as f32 * 300.0, (index / 50) as f32 * 120.0];
        let number = host.add_node(&mut draft, "core/whole-number", at).unwrap();
        let absolute = host
            .add_node(
                &mut draft,
                "org.orange.math/absolute",
                [at[0] + 150.0, at[1]],
            )
            .unwrap();
        host.connect(&mut draft, (number, "result"), (absolute, "value"))
            .unwrap();
    }
    draft.text().unwrap()
}

/// Runs `work` and records how long it took under `label`.
macro_rules! time {
    ($label:expr, $runs:expr, $work:expr) => {{
        let started = Instant::now();
        let value = $work;
        $runs.push(($label.to_owned(), started.elapsed()));
        value
    }};
}

#[test]
fn edit_timing() {
    let (mut host, _events) = Host::new(orange_host::HostOptions::default()).unwrap();
    let pairs = std::env::var("ORANGE_EDIT_TIMING_PAIRS").map_or(64, |value| {
        let pairs = value.parse().unwrap();
        assert!(pairs > 0);
        pairs
    });
    let text = large(&mut host, pairs);
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, text);
    let target = editor.graph().unwrap().nodes[0].id;
    for round in 0..4 {
        let mut runs: Vec<(String, Duration)> = Vec::new();
        let value = (round + 2).to_string();
        let total = Instant::now();
        time!("keep layout", runs, editor.keep_layout());
        {
            let document = editor.document.as_mut().unwrap();
            let result = time!("host edit", runs, {
                host.set_input(&mut document.draft, (target, "value"), Some(&value))
            });
            assert!(result.unwrap());
        }
        let document = editor.document.as_mut().unwrap();
        document.names = time!("names", runs, {
            Names::of_document(&host, document.draft.document()).without_ids()
        });
        let (before, failure_routes, port_details) = time!("draft view", runs, {
            let orange_host::AuthoringView {
                graph,
                failure_routes,
                port_details,
            } = host
                .draft_authoring_view(&document.draft)
                .unwrap_or_default();
            (document.replace_graph(graph), failure_routes, port_details)
        });
        let touched = time!("touched", runs, {
            touched_by_edit(&before, &document.graph)
        });
        document.declared = time!("declarations", runs, { host.declarations(&document.draft) });
        let graph = document.graph.clone();
        time!("canvas update", runs, {
            editor.canvas.set_failure_routes(failure_routes);
            editor.canvas.set_port_details(port_details);
            editor.canvas.update(&graph);
        });
        let document = editor.document.as_ref().unwrap();
        let enums = time!("enums", runs, {
            canvas::enums_of(&host, &document.draft, &document.graph, &document.declared)
        });
        editor.canvas.enums = enums;
        time!("check", runs, editor.check(&host));
        time!("previews", runs, {
            editor.preview_scope(&mut host, touched.as_ref())
        });
        let total = total.elapsed();
        let line: Vec<String> = runs
            .iter()
            .map(|(label, took)| format!("{label} {:.2}", took.as_secs_f64() * 1000.0))
            .collect();
        eprintln!(
            "round {round}: total {:.2} ms | {}",
            total.as_secs_f64() * 1000.0,
            line.join(" | ")
        );
    }
}

/// An edit previews only what it touched and what follows it; every other preview
/// stays as it was.
#[test]
fn an_edit_previews_what_it_touched_and_keeps_the_rest() {
    let (mut host, _events) = Host::new(orange_host::HostOptions::default()).unwrap();
    let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
    let chain = |host: &Host, draft: &mut Draft, value: &str, y: f32| {
        let number = host.add_node(draft, "core/whole-number", [0.0, y]).unwrap();
        host.set_input(draft, (number, "value"), Some(value))
            .unwrap();
        let absolute = host
            .add_node(draft, "org.orange.math/absolute", [200.0, y])
            .unwrap();
        host.connect(draft, (number, "result"), (absolute, "value"))
            .unwrap();
        (number, absolute)
    };
    let (first, first_absolute) = chain(&host, &mut draft, "5", 0.0);
    let (_, second_absolute) = chain(&host, &mut draft, "-3", 200.0);
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, draft.text().unwrap());
    let shown = |editor: &Editor, node| {
        let graph = editor.graph().unwrap();
        editor
            .canvas
            .port_sample(graph, node, "result", false)
            .map(|(shown, _)| shown.short.clone())
    };
    assert_eq!(shown(&editor, first_absolute).as_deref(), Some("5"));
    assert_eq!(shown(&editor, second_absolute).as_deref(), Some("3"));
    assert!(editor.set_input(&mut host, (first, "value"), Some("-9")));
    assert_eq!(shown(&editor, first_absolute).as_deref(), Some("9"));
    assert_eq!(shown(&editor, second_absolute).as_deref(), Some("3"));
}
