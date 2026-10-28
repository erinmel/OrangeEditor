//! One nesting budget for editing, saving and loading: whatever an edit commits can be
//! saved and read back, and the deepest accepted edit reaches the budget exactly.

mod common;

use common::{body, content, empty_document, function, function_id, minimal_document, node};
use orange_model::document::nesting::MAX_NESTING;
use orange_model::edit::LayoutCommand;
use orange_model::{BodyRef, CodecError, DocCommand, GraphDocument, TxError, from_json, to_json};
use serde_json::{Value as Json, json};

/// `levels` nested objects around a scalar; 0 is the scalar itself.
fn nested(levels: usize) -> Json {
    (0..levels).fold(json!(1), |inner, _| json!({ "a": inner }))
}

fn nesting(value: &Json) -> usize {
    match value {
        Json::Array(items) => 1 + items.iter().map(nesting).max().unwrap_or(0),
        Json::Object(map) => 1 + map.values().map(nesting).max().unwrap_or(0),
        _ => 0,
    }
}

/// Grows `edit(levels)` until the budget rejects it, then checks both sides of the boundary.
fn check_boundary(start: GraphDocument, edit: impl Fn(usize) -> Vec<DocCommand>) {
    let mut levels = 0;
    loop {
        let mut document = start.clone();
        let before = content(&document);
        match document.apply(edit(levels)) {
            Ok(_) => {
                let saved = to_json(&document).unwrap();
                let reloaded = from_json(&saved).expect("an accepted edit reloads");
                assert_eq!(reloaded, document);
                levels += 1;
            }
            Err(error) => {
                assert_eq!(error, TxError::NestingLimit { max: MAX_NESTING });
                assert_eq!(
                    content(&document),
                    before,
                    "a rejected edit changes nothing"
                );
                break;
            }
        }
    }
    assert!(levels > 0, "some nesting must fit");
    let mut deepest = start;
    let _ = deepest.apply(edit(levels - 1)).unwrap();
    let saved: Json = serde_json::from_str(&to_json(&deepest).unwrap()).unwrap();
    assert_eq!(
        nesting(&saved),
        MAX_NESTING,
        "the deepest edit uses the whole budget"
    );
}

fn layout(path: Vec<String>, value: Json) -> Vec<DocCommand> {
    vec![DocCommand::Layout(LayoutCommand::Set {
        path,
        value: Some(value),
    })]
}

#[test]
fn layout_values_and_paths_share_the_budget() {
    check_boundary(empty_document(), |levels| {
        layout(vec!["a".into()], nested(levels))
    });
    check_boundary(empty_document(), |levels| {
        layout((0..=levels).map(|n| n.to_string()).collect(), json!(1))
    });
}

#[test]
fn node_config_shares_the_budget_in_root_and_function_bodies() {
    let deep_node = |levels| {
        let mut node = node(1);
        node.config.insert("deep".into(), nested(levels));
        node
    };
    check_boundary(empty_document(), |levels| {
        vec![DocCommand::AddNode {
            body: BodyRef::Root,
            node: deep_node(levels),
        }]
    });
    let mut with_function = empty_document();
    let _ = with_function
        .apply([DocCommand::UpsertFunction(function(body([], [])))])
        .unwrap();
    check_boundary(with_function, |levels| {
        vec![DocCommand::AddNode {
            body: BodyRef::Function(function_id()),
            node: deep_node(levels),
        }]
    });
}

#[test]
fn loading_applies_the_same_budget_on_both_construction_paths() {
    // Deeper than the budget but within the JSON parser's own recursion bound.
    let text = minimal_document(json!({ "editor": { "a": nested(MAX_NESTING) } }));
    assert!(matches!(
        from_json(&text),
        Err(CodecError::NestingLimit { max: MAX_NESTING })
    ));
    let error = serde_json::from_str::<GraphDocument>(&text).unwrap_err();
    assert!(
        error.to_string().contains("nests JSON deeper than 64"),
        "{error}"
    );

    let fits = minimal_document(json!({ "editor": { "a": nested(MAX_NESTING - 3) } }));
    assert!(from_json(&fits).is_ok());
    assert!(serde_json::from_str::<GraphDocument>(&fits).is_ok());
}
