//! 00c.4a: read-only inference over the generic component an edit touches (11 §5.5).

mod common;

use common::generic::{add, add_definition, catalog, data_edge, document, only, source};
use common::{minimal_document, node_id};
use orange_model::DocIndex;
use orange_model::catalog::ProviderKey;
use orange_model::id::NodeId;
use orange_model::inference::{
    ComponentInference, EquationEvidence, InferenceIssue, TemplateCatalog, UnificationError,
    infer_components,
};
use orange_model::types::TypeRegistry;
use orange_model::{GraphDocument, from_json};
use proptest::prelude::*;
use serde_json::json;

fn id(n: u8) -> NodeId {
    node_id(n).parse().unwrap()
}

fn infer(
    document: &GraphDocument,
    touched: &[u8],
    catalog: &TemplateCatalog,
) -> Vec<ComponentInference> {
    infer_components(
        document,
        &DocIndex::build(document),
        touched.iter().map(|&n| id(n)),
        catalog,
        &TypeRegistry::core(),
    )
}

fn resolved(component: &ComponentInference, n: u8) -> Option<&str> {
    component
        .type_args
        .get(&id(n))?
        .get("T")
        .map(|ty| ty.as_str())
}

fn codes(component: &ComponentInference) -> Vec<&'static str> {
    component.issues.iter().map(InferenceIssue::code).collect()
}

#[test]
fn typed_source_resolves_a_generic_chain_from_any_touched_node() {
    let doc = document(
        vec![source(1), add(2, json!({})), add(3, json!({}))],
        vec![
            data_edge(1, (1, "value"), (2, "a")),
            data_edge(2, (2, "sum"), (3, "a")),
        ],
    );
    for touched in [&[2][..], &[3], &[1]] {
        let components = infer(&doc, touched, &catalog(None));
        assert_eq!(components.len(), 1, "touched {touched:?}");
        let component = &components[0];
        assert_eq!(component.nodes, [id(2), id(3)].into());
        assert_eq!(resolved(component, 2), Some("core/i64@1"));
        assert_eq!(resolved(component, 3), Some("core/i64@1"));
        assert!(component.issues.is_empty(), "{:?}", component.issues);
    }
}

#[test]
fn stale_inferred_arguments_are_never_reused_but_pins_are() {
    // After a disconnect, a persisted `type_args` cache is not evidence.
    let stale = document(
        vec![add(2, json!({"type_args": {"T": "core/i64@1"}}))],
        vec![],
    );
    let component = &infer(&stale, &[2], &catalog(None))[0];
    assert_eq!(resolved(component, 2), None);
    assert_eq!(codes(component), ["E-TYPE-AMBIGUOUS"]);

    let pinned = document(
        vec![add(
            2,
            json!({"type_args": {"T": "core/i64@1"}, "type_pins": {"T": "core/f64@1"}}),
        )],
        vec![],
    );
    let component = &infer(&pinned, &[2], &catalog(None))[0];
    assert_eq!(resolved(component, 2), Some("core/f64@1"));
    assert!(component.issues.is_empty());
}

#[test]
fn active_literals_seed_inference_and_shadowed_literals_do_not() {
    let literal = json!({"inputs": {"b": {"type": "core/i32@1", "value": 3}}});
    let doc = document(vec![add(2, literal.clone())], vec![]);
    assert_eq!(
        resolved(&infer(&doc, &[2], &catalog(None))[0], 2),
        Some("core/i32@1")
    );

    // An edge into `b` shadows its literal (11 §4.4): no conflict with the i64 source.
    let doc = document(
        vec![source(1), add(2, literal)],
        vec![data_edge(1, (1, "value"), (2, "b"))],
    );
    let component = &infer(&doc, &[2], &catalog(None))[0];
    assert_eq!(resolved(component, 2), Some("core/i64@1"));
    assert!(component.issues.is_empty());
}

#[test]
fn conflicting_pins_report_a_mismatch_with_pin_evidence_and_no_bindings() {
    let doc = document(
        vec![
            add(2, json!({"type_pins": {"T": "core/i64@1"}})),
            add(3, json!({"type_pins": {"T": "core/f64@1"}})),
        ],
        vec![data_edge(1, (2, "sum"), (3, "a"))],
    );
    let component = &infer(&doc, &[2], &catalog(None))[0];
    assert!(component.type_args.is_empty());
    assert_eq!(codes(component), ["E-TYPE-MISMATCH"]);
    let InferenceIssue::Unification(UnificationError::TypeMismatch { context, .. }) =
        &component.issues[0]
    else {
        panic!("{:?}", component.issues);
    };
    assert!(matches!(
        context.evidence,
        Some(EquationEvidence::Pin { .. })
    ));
}

#[test]
fn a_pin_nested_past_the_parser_budget_is_a_complexity_issue_not_a_mismatch() {
    let deep = format!(
        "{}core/i64@1{}",
        "core/list@1<".repeat(orange_model::type_term::MAX_NESTING + 1),
        ">".repeat(orange_model::type_term::MAX_NESTING + 1)
    );
    let doc = document(vec![add(2, json!({"type_pins": {"T": deep}}))], vec![]);
    assert_eq!(
        codes(&infer(&doc, &[2], &catalog(None))[0]),
        ["E-TYPE-COMPLEXITY"]
    );
}

#[test]
fn constraints_and_specializations_are_checked_after_solving() {
    let text = document(
        vec![add(2, json!({"type_pins": {"T": "core/string@1"}}))],
        vec![],
    );
    let component = &infer(&text, &[2], &catalog(None))[0];
    assert_eq!(codes(component), ["E-CONSTRAINT-UNSATISFIED"]);
    assert_eq!(resolved(component, 2), None);

    let float = document(
        vec![add(2, json!({"type_pins": {"T": "core/f64@1"}}))],
        vec![],
    );
    let only_integers = catalog(Some(vec!["core/i64@1", "core/i32@1"]));
    let component = &infer(&float, &[2], &only_integers)[0];
    assert_eq!(codes(component), ["E-NO-SPECIALIZATION"]);
    assert!(
        infer(&float, &[2], &catalog(Some(vec!["core/f64@1"])))[0]
            .issues
            .is_empty()
    );
}

#[test]
fn specializations_belong_to_the_exact_provider_of_a_node() {
    // Two packages implement `add`; only `org.ints` limits it, and only to integers.
    let digest = |byte: &str| format!("sha256:{}", byte.repeat(32));
    let doc: GraphDocument = from_json(&minimal_document(json!({
        "package_lock": [
            {"id": "org.ints", "version": "1.0.0", "content_digest": digest("aa")},
            {"id": "org.any", "version": "1.0.0", "content_digest": digest("bb")},
        ],
        "nodes": [
            add(2, json!({"type_pins": {"T": "core/f64@1"},
                "provider": {"package": "org.ints", "version": "1.0.0", "id": "add"}})),
            add(3, json!({"type_pins": {"T": "core/f64@1"},
                "provider": {"package": "org.any", "version": "1.0.0", "id": "add"}})),
        ],
    })))
    .unwrap();
    let ints = ProviderKey {
        package: "org.ints".to_owned(),
        version: "1.0.0".to_owned(),
        content_digest: digest("aa"),
        id: "add".to_owned(),
    };
    let mut templates = catalog(None);
    templates.insert_specializations(add_definition(), Some(ints.clone()), only(&["core/i64@1"]));
    assert_eq!(
        codes(&infer(&doc, &[2], &templates)[0]),
        ["E-NO-SPECIALIZATION"]
    );
    assert!(infer(&doc, &[3], &templates)[0].issues.is_empty());

    // The same package version under another digest is a different provider.
    let mut other = catalog(None);
    let rebuilt = ProviderKey {
        content_digest: digest("cc"),
        ..ints
    };
    other.insert_specializations(add_definition(), Some(rebuilt), only(&["core/i64@1"]));
    assert!(infer(&doc, &[2], &other)[0].issues.is_empty());
}

#[test]
fn both_sides_of_a_disconnect_are_separate_components_and_others_are_untouched() {
    // 2 and 3 were connected; the edge is gone. 4 → 5 is an unrelated component.
    let doc = document(
        vec![
            add(2, json!({"type_pins": {"T": "core/i64@1"}})),
            add(3, json!({})),
            add(4, json!({"type_pins": {"T": "core/f32@1"}})),
            add(5, json!({})),
        ],
        vec![data_edge(1, (4, "sum"), (5, "a"))],
    );
    let components = infer(&doc, &[3, 2], &catalog(None));
    let nodes: Vec<_> = components.iter().map(|c| c.nodes.clone()).collect();
    assert_eq!(nodes, [[id(2)].into(), [id(3)].into()]);
    assert_eq!(resolved(&components[0], 2), Some("core/i64@1"));
    assert_eq!(codes(&components[1]), ["E-TYPE-AMBIGUOUS"]);
}

proptest! {
    /// Shuffled node and edge insertion orders yield identical inference results.
    #[test]
    fn insertion_order_does_not_change_the_result(
        node_order in Just((0..6).collect::<Vec<usize>>()).prop_shuffle(),
        edge_order in Just((0..5).collect::<Vec<usize>>()).prop_shuffle(),
    ) {
        // source 1 → add 2 → add 3 → add 4, and add 5 → add 6 pinned to u32.
        let nodes = vec![
            source(1), add(2, json!({})), add(3, json!({})), add(4, json!({})),
            add(5, json!({})), add(6, json!({"type_pins": {"T": "core/u32@1"}})),
        ];
        let edges = vec![
            data_edge(1, (1, "value"), (2, "a")), data_edge(2, (2, "sum"), (3, "a")),
            data_edge(3, (3, "sum"), (4, "b")), data_edge(4, (5, "sum"), (6, "a")),
            data_edge(5, (2, "sum"), (4, "a")),
        ];
        let reference = infer(&document(nodes.clone(), edges.clone()), &[4, 5], &catalog(None));
        let shuffled = document(
            node_order.iter().map(|&i| nodes[i].clone()).collect(),
            edge_order.iter().map(|&i| edges[i].clone()).collect(),
        );
        prop_assert_eq!(infer(&shuffled, &[5, 4], &catalog(None)), reference);
    }
}
