//! Checked zone lowering and, as runtime support lands, source-level execution regressions.
#[path = "support/data_graph.rs"]
mod support;
use orange_compiler::{
    CompileError, CompileLimits, CompileOptions, DataOperation, compile_data, compile_pure,
};
use orange_model::{analyze, from_json, to_json};
use serde_json::{Value as Json, json};
use std::sync::atomic::Ordering;
use support::*;

fn zone(n: u32, family: &str, end: bool, label: &str, pins: Json, inputs: Json) -> Json {
    json!({"id": id(n), "definition_id":format!("core/{family}-{}", if end { "end" } else { "begin" }),
        "definition_version":1, "config_version":1, "config":{"zone":label}, "type_pins":pins, "inputs":inputs})
}
fn repeat(n: u32, label: &str, count: i64, initial: u64) -> Json {
    zone(
        n,
        "repeat",
        false,
        label,
        json!({"A":"core/u64@1"}),
        json!({
        "count":{"type":"core/i64@1","value":count.to_string()},
        "initial":{"type":"core/u64@1","value":initial.to_string()}}),
    )
}

#[test]
fn million_iteration_repeat_lowers_once_with_original_sources_and_no_body_folding() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 1_000_000, 7),
            math(2, "add", 0, 1),
            zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![edge(1, 1, "carried", 2, "a"), edge(2, 2, "sum", 3, "next")],
        vec![export(3, "result")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot, &resolver);
    assert_eq!(plan.zones().len(), 1);
    assert_eq!(plan.nodes().len(), 3);
    let parameters = plan.zones()[0].parameters().unwrap();
    assert!(matches!(
        plan.nodes()[parameters.index()].operation(),
        DataOperation::Iteration(_)
    ));
    assert_eq!(plan.nodes()[parameters.index()].inputs().len(), 0);
    let body = plan
        .nodes()
        .iter()
        .find(|n| n.node().to_string() == id(2))
        .unwrap();
    assert_eq!(body.zone(), plan.nodes()[parameters.index()].zone());
    let end = plan.nodes().last().unwrap();
    assert_eq!(end.zone(), None);
    assert!(matches!(
        end.operation(),
        DataOperation::Zone { prefix: 2, .. }
    ));
    assert_eq!(end.inputs().len(), 3);
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
}

#[test]
fn map_fold_and_filter_lower_their_declared_strict_ports_without_binding_intrinsics() {
    for family in ["map", "fold", "filter"] {
        let (package, snapshot) = catalog();
        let pins = if family == "map" {
            json!({"T":"core/u64@1"})
        } else if family == "fold" {
            json!({"T":"core/u64@1","A":"core/u64@1"})
        } else {
            json!({"T":"core/u64@1"})
        };
        let mut inputs = json!({"items":{"type":"core/list@1<core/u64@1>","value":["3","1","9"]}});
        if family == "fold" {
            inputs["initial"] = json!({"type":"core/u64@1","value":"0"});
        }
        let end_pins = if family == "map" {
            json!({"U":"core/u64@1"})
        } else if family == "fold" {
            json!({"A":"core/u64@1"})
        } else {
            json!({"T":"core/u64@1"})
        };
        let end_inputs = if family == "filter" {
            json!({"keep":{"type":"core/bool@1","value":false}})
        } else {
            json!({})
        };
        let doc = graph(
            &package,
            vec![
                zone(1, family, false, "z", pins, inputs),
                zone(2, family, true, "z", end_pins, end_inputs),
            ],
            vec![edge(
                1,
                1,
                "element",
                2,
                if family == "fold" { "next" } else { "item" },
            )],
            vec![export(2, "result")],
        );
        let resolver = Counting::new(package);
        let plan = compile(&doc, snapshot, &resolver);
        assert_eq!(plan.zones().len(), 1);
        assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
        let end = plan.nodes().last().unwrap();
        let prefix = if family == "fold" { 2 } else { 1 };
        assert!(matches!(end.operation(),DataOperation::Zone{prefix:p,..} if *p==prefix));
        assert_eq!(
            end.inputs()[prefix].source.port.to_string(),
            if family == "filter" {
                "keep"
            } else if family == "fold" {
                "next"
            } else {
                "item"
            }
        );
    }
}

#[test]
fn pure_borrowed_result_api_refuses_zones_without_binding_callbacks() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 100, 7),
            zone(2, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![edge(1, 1, "carried", 2, "next")],
        vec![export(2, "result")],
    );
    let resolver = Counting::new(package);
    let analysis = analyze(&doc, &snapshot);
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot,
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::UnsupportedNode { .. })
    ));
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
}

#[test]
fn generic_calls_with_constant_arguments_keep_separate_zone_programs() {
    let (package, snapshot) = catalog();
    let call = |n, ty, value: Json| {
        json!({"id":id(n),"definition_id":"core/call","definition_version":1,
        "config_version":1,"config":{"function":id(100)},"type_pins":{"T":ty},
        "inputs":{"value":{"type":ty,"value":value}}})
    };
    let base = graph(
        &package,
        vec![
            call(1, "core/u64@1", json!("7")),
            call(2, "core/string@1", json!("hello")),
        ],
        vec![],
        vec![export(1, "result"), export(2, "result")],
    );
    let mut saved: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    let marker = |n, name| json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}});
    saved["functions"] = json!([{"id":id(100),"name":"repeat_identity","kind":"pure","type_params":[{"name":"T"}],
        "inputs":[{"port_id":"value","direction":"input","kind":"data","type":"$T"}],
        "outputs":[{"port_id":"result","direction":"output","kind":"data","type":"$T"}],
        "nodes":[marker(101,"function-entry"),zone(102,"repeat",false,"z",json!({}),json!({"count":{"type":"core/i64@1","value":"1000000"}})),
            zone(103,"repeat",true,"z",json!({}),json!({})),marker(104,"function-outputs")],
        "edges":[edge(101,101,"value",102,"initial"),edge(102,102,"carried",103,"next"),edge(103,103,"result",104,"result")]}]);
    let doc = from_json(&saved.to_string()).unwrap();
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot, &resolver);
    assert_eq!(plan.zones().len(), 2);
    assert_ne!(
        plan.zones()[0].call_instance(),
        plan.zones()[1].call_instance()
    );
    assert_eq!(plan.zones()[0].begin(), plan.zones()[1].begin());
    assert!(
        plan.nodes()
            .iter()
            .filter(|n| matches!(n.operation(), DataOperation::Call { .. }))
            .count()
            == 2
    );
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
}

#[test]
fn dense_zone_metadata_is_admitted_before_any_provider_binding() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 9, 7),
            math(2, "add", 0, 1),
            zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![edge(1, 1, "carried", 2, "a"), edge(2, 2, "sum", 3, "next")],
        vec![export(3, "result")],
    );
    let analysis = analyze(&doc, &snapshot);
    let resolver = Counting::new(package);
    let options = CompileOptions {
        limits: CompileLimits {
            max_constant_bytes: 128,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        compile_data(&doc, snapshot, &analysis, &resolver, options),
        Err(CompileError::ResourceLimit)
    ));
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
}

#[test]
fn entropy_capture_keeps_the_outer_domain_while_body_is_iteration_owned() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 20, 0),
            random(2),
            math(3, "add", 0, 0),
            zone(4, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![
            edge(1, 1, "carried", 3, "a"),
            edge(2, 2, "value", 3, "b"),
            edge(3, 3, "sum", 4, "next"),
        ],
        vec![export(4, "result")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot, &resolver);
    let capture = plan
        .nodes()
        .iter()
        .find(|n| matches!(n.operation(), DataOperation::Entropy(_)))
        .unwrap();
    assert_eq!(capture.zone(), None);
    let body = plan
        .nodes()
        .iter()
        .find(|n| n.node().to_string() == id(3))
        .unwrap();
    assert!(body.zone().is_some());
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
}

#[test]
fn closed_zone_parameters_and_body_values_cannot_be_exported_directly() {
    let (package, snapshot) = catalog();
    for target in [export(1, "carried"), export(2, "sum")] {
        let doc = graph(
            &package,
            vec![
                repeat(1, "z", 3, 0),
                math(2, "add", 0, 1),
                zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
            ],
            vec![edge(1, 1, "carried", 2, "a"), edge(2, 2, "sum", 3, "next")],
            vec![target],
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis
                .report()
                .diagnostics
                .iter()
                .any(|d| d.code == orange_model::validation::DiagnosticCode::ZoneStructure)
        );
        assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
    }
}

#[test]
fn filter_cannot_change_element_type_and_repeat_cannot_change_carry_type() {
    let (package, snapshot) = catalog();
    for family in ["filter", "repeat", "fold"] {
        let pins = if family == "repeat" {
            json!({"A":"core/u64@1"})
        } else if family == "fold" {
            json!({"T":"core/u64@1","A":"core/u64@1"})
        } else {
            json!({"T":"core/u64@1"})
        };
        let mut inputs = if family == "repeat" {
            json!({"count":{"type":"core/i64@1","value":"1"}})
        } else {
            json!({"items":{"type":"core/list@1<core/u64@1>","value":[]}})
        };
        if family != "filter" {
            inputs["initial"] = literal(7);
        }
        let end_pins = if family == "filter" {
            json!({"T":"core/bool@1"})
        } else {
            json!({"A":"core/bool@1"})
        };
        let end_inputs = if family == "filter" {
            json!({"keep":{"type":"core/bool@1","value":true},"item":{"type":"core/bool@1","value":false}})
        } else {
            json!({"next":{"type":"core/bool@1","value":false}})
        };
        let doc = graph(
            &package,
            vec![
                zone(1, family, false, "z", pins, inputs),
                zone(2, family, true, "z", end_pins, end_inputs),
            ],
            vec![],
            vec![export(2, "result")],
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis
                .report()
                .diagnostics
                .iter()
                .any(|d| d.code == orange_model::validation::DiagnosticCode::TypeMismatch),
            "{:?}",
            analysis.report().diagnostics
        );
        assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
    }
}

#[test]
fn nested_zone_layout_resolves_parents_independently_of_uuid_order() {
    let (package, snapshot) = catalog();
    let mut inner = repeat(2, "inner", 2, 0);
    inner["inputs"].as_object_mut().unwrap().remove("initial");
    let doc = graph(
        &package,
        vec![
            repeat(1, "outer", 3, 0),
            inner,
            math(3, "add", 0, 1),
            zone(
                4,
                "repeat",
                true,
                "inner",
                json!({"A":"core/u64@1"}),
                json!({}),
            ),
            zone(
                5,
                "repeat",
                true,
                "outer",
                json!({"A":"core/u64@1"}),
                json!({}),
            ),
        ],
        vec![
            edge(1, 1, "carried", 2, "initial"),
            edge(2, 2, "carried", 3, "a"),
            edge(3, 3, "sum", 4, "next"),
            edge(4, 4, "result", 5, "next"),
        ],
        vec![export(5, "result")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot, &resolver);
    let inner = plan
        .zones()
        .iter()
        .find(|z| z.end().to_string() == id(4))
        .unwrap();
    let parent = inner.parent().unwrap();
    assert_eq!(plan.zones()[parent.index()].end().to_string(), id(5));
    let inner_end = plan
        .nodes()
        .iter()
        .find(|n| n.node().to_string() == id(4))
        .unwrap();
    assert_eq!(inner_end.zone(), Some(parent));
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
}
