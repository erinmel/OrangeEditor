//! The package lifecycle of a function made in the editor (tools/release/README.md,
//! "Package lifecycle"): extract a function, export it, import it into another graph,
//! use and reopen it, remove the package, and install the exact archive again.
use orange_host::{Grants, Host, HostOptions, Publish, Value};
use orange_model::validation::DiagnosticCode;

const POWER: &str = include_str!("../../../apps/orange-cli/graphs/power-with-count.json");

#[test]
fn an_exported_function_survives_removal_and_comes_back_when_reinstalled() {
    // Make the function: Base and its multiplication become one pure node.
    let (author, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = author.draft(POWER).unwrap();
    let view = author.draft_view(&draft).unwrap();
    let chosen: Vec<_> = view
        .nodes
        .iter()
        .filter(|node| matches!(node.name.as_str(), "Base" | "Multiply by base"))
        .map(|node| node.id)
        .collect();
    assert_eq!(chosen.len(), 2);
    let extraction = author.extraction(&draft, &chosen).unwrap();
    assert_eq!(extraction.kind, "pure");
    let call = author
        .extract(&mut draft, &chosen, "Scale by base", [0.0, 0.0])
        .unwrap();
    let source = draft.text().unwrap();
    assert!(author.check(&source).unwrap().is_empty());
    assert!(
        author
            .draft_view(&draft)
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.id == call)
    );
    let archive = author
        .export_function_package(
            &source,
            "Scale by base",
            &Publish {
                package: "org.example.scaling",
                version: "1.0.0",
                node: "scale-by-base",
                label: None,
            },
        )
        .unwrap();

    // Import it into a different graph and use it.
    let (mut consumer, _events) = Host::new(HostOptions::default()).unwrap();
    let key = consumer.install(&archive, Grants::default()).unwrap();
    let mut draft = consumer
        .draft(&orange_host::new_document().unwrap())
        .unwrap();
    let node = consumer
        .add_node(&mut draft, "org.example.scaling/scale-by-base", [0.0, 0.0])
        .unwrap();
    assert!(
        consumer
            .set_input(&mut draft, (node, "b"), Some("7"))
            .unwrap()
    );
    let saved = draft.text().unwrap();
    let wanted = [(node, "product".parse().unwrap())];
    let computed = |host: &mut Host, text: &str| {
        host.preview(text, &wanted)
            .unwrap()
            .remove(0)
            .value
            .unwrap()
    };
    assert_eq!(computed(&mut consumer, &saved), Value::I64(21));
    // Reopened, it is the same graph with the same result.
    let reopened = consumer.draft(&saved).unwrap().text().unwrap();
    assert!(consumer.check(&reopened).unwrap().is_empty());
    assert_eq!(computed(&mut consumer, &reopened), Value::I64(21));

    // Removed, the saved node stays on the canvas and says its provider is gone.
    consumer.uninstall(&key).unwrap();
    let view = consumer.view(&saved).unwrap();
    assert!(view.nodes.iter().any(|shown| shown.id == node));
    let problems = consumer.check(&saved).unwrap();
    assert!(
        problems
            .iter()
            .any(|problem| problem.subject.node == Some(node)
                && problem.code == DiagnosticCode::ProviderUnavailable),
        "{problems:?}"
    );

    // The exact archive installed again restores it.
    let _ = consumer.install(&archive, Grants::default()).unwrap();
    assert!(consumer.check(&saved).unwrap().is_empty());
    assert_eq!(computed(&mut consumer, &saved), Value::I64(21));
}
