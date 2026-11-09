//! L03b `org.orange.text` known answers, Unicode units, bounds, limits and folds.
use std::sync::Arc;

use orange_bundled::{
    BindingError, KernelError, MAX_TEXT_BYTES, MathPackage, ResolvedText, TextOperation as Op,
    TextUnit, text_definitions,
};
use orange_compiler::{CompileOptions, FoldedValue, compile_pure};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::DefinitionRef;
use orange_model::value::ListRepr;
use orange_model::{GraphDocument, Value, analyze, from_json};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

use crate::data_graph::{catalog, edge, id};

fn bind_in(
    package: &MathPackage,
    snapshot: Arc<RegistrySnapshot>,
    op: Op,
    config: Json,
) -> Result<ResolvedText, BindingError> {
    package.resolve_text(
        snapshot,
        package.text_provider_key(),
        &DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        },
        1,
        &serde_json::from_value(config).unwrap(),
    )
}
fn bind(op: Op, config: Json) -> ResolvedText {
    let (package, snapshot) = catalog();
    bind_in(&package, snapshot, op, config).unwrap()
}
fn s(value: &str) -> Value {
    Value::Str(Arc::new(value.into()))
}
fn list(items: &[&str]) -> Value {
    let items: Vec<Arc<String>> = items.iter().map(|i| Arc::new(i.to_string())).collect();
    Value::List(Arc::new(ListRepr::Str(items.into())))
}
fn eval(op: Op, inputs: &[Value]) -> Result<Value, KernelError> {
    bind(op, json!({})).evaluate(inputs)
}
fn unit(op: Op, unit: &str, inputs: &[Value]) -> Result<Value, KernelError> {
    bind(op, json!({ "unit": unit })).evaluate(inputs)
}

#[test]
fn all_sixteen_operations_have_known_answers() {
    for (op, inputs, expected) in [
        (
            Op::ByteLength,
            vec![s("h\u{e9}llo \u{1f34a}")],
            Value::U64(11),
        ),
        (
            Op::ScalarLength,
            vec![s("h\u{e9}llo \u{1f34a}")],
            Value::U64(7),
        ),
        (Op::Concat, vec![s("or"), s("ange")], s("orange")),
        (Op::Contains, vec![s("orange"), s("ran")], Value::Bool(true)),
        (
            Op::StartsWith,
            vec![s("orange"), s("or")],
            Value::Bool(true),
        ),
        (Op::EndsWith, vec![s("orange"), s("or")], Value::Bool(false)),
        (Op::Find, vec![s("h\u{e9}llo"), s("l")], Value::U64(2)),
        (
            Op::Substring,
            vec![s("h\u{e9}llo"), Value::U64(1), Value::U64(2)],
            s("\u{e9}l"),
        ),
        (Op::Replace, vec![s("a-b-c"), s("-"), s("--")], s("a--b--c")),
        (Op::Trim, vec![s("\u{3000} x \u{a0}\n")], s("x")),
        (Op::TrimStart, vec![s("  x  ")], s("x  ")),
        (Op::TrimEnd, vec![s("  x  ")], s("  x")),
        (Op::ToUppercase, vec![s("stra\u{df}e")], s("STRASSE")),
        (
            Op::ToLowercase,
            vec![s("\u{391}\u{3a3}")],
            s("\u{3b1}\u{3c2}"),
        ),
        (Op::Split, vec![s("a,,b"), s(",")], list(&["a", "", "b"])),
        (Op::Join, vec![list(&["a", "", "b"]), s(", ")], s("a, , b")),
    ] {
        assert_eq!(eval(op, &inputs), Ok(expected), "{op:?}");
    }
    assert_eq!(text_definitions().unwrap().len(), Op::ALL.len());
}

#[test]
fn find_and_substring_index_scalars_or_bytes_explicitly() {
    let word = s("h\u{e9}llo");
    assert_eq!(
        unit(Op::Find, "byte", &[word.clone(), s("l")]),
        Ok(Value::U64(3))
    );
    assert_eq!(
        unit(Op::Find, "scalar", &[word.clone(), s("l")]),
        Ok(Value::U64(2))
    );
    assert_eq!(eval(Op::Find, &[word.clone(), s("z")]), Ok(Value::None));
    assert_eq!(eval(Op::Find, &[word.clone(), s("")]), Ok(Value::U64(0)));
    // Combining marks are separate scalars: graphemes are not a unit here.
    assert_eq!(eval(Op::ScalarLength, &[s("e\u{301}")]), Ok(Value::U64(2)));
    let sub = |unit_name: &str, start: u64, length: u64| {
        unit(
            Op::Substring,
            unit_name,
            &[word.clone(), Value::U64(start), Value::U64(length)],
        )
    };
    assert_eq!(sub("scalar", 5, 0), Ok(s("")));
    assert_eq!(sub("scalar", 0, 5), Ok(s("h\u{e9}llo")));
    assert_eq!(sub("scalar", 5, 1), Err(KernelError::OutOfRange));
    assert_eq!(sub("scalar", 6, 0), Err(KernelError::OutOfRange));
    assert_eq!(sub("scalar", u64::MAX, 1), Err(KernelError::OutOfRange));
    assert_eq!(sub("byte", 1, 2), Ok(s("\u{e9}")));
    assert_eq!(sub("byte", 1, 1), Err(KernelError::Domain));
    assert_eq!(sub("byte", 2, 1), Err(KernelError::Domain));
    assert_eq!(sub("byte", 0, 7), Err(KernelError::OutOfRange));
    assert_eq!(sub("byte", 6, 0), Ok(s("")));
}

#[test]
fn empty_patterns_unicode_rules_and_list_contracts_are_explicit() {
    assert_eq!(
        eval(Op::Replace, &[s("abc"), s(""), s("x")]),
        Err(KernelError::Domain)
    );
    assert_eq!(
        eval(Op::Split, &[s("abc"), s("")]),
        Err(KernelError::Domain)
    );
    assert_eq!(eval(Op::Split, &[s(""), s(",")]), Ok(list(&[""])));
    assert_eq!(
        eval(Op::Contains, &[s("abc"), s("")]),
        Ok(Value::Bool(true))
    );
    // Case mapping is locale-independent: no Turkish dotless i.
    assert_eq!(eval(Op::ToLowercase, &[s("I")]), Ok(s("i")));
    assert_eq!(eval(Op::ToUppercase, &[s("i")]), Ok(s("I")));
    // Generic list storage with string items joins like packed storage.
    let values = Value::List(Arc::new(ListRepr::Values(
        vec![s("x"), s("y")].into_boxed_slice(),
    )));
    assert_eq!(eval(Op::Join, &[values, s("+")]), Ok(s("x+y")));
    let numbers = Value::List(Arc::new(ListRepr::I64(vec![1, 2].into_boxed_slice())));
    assert_eq!(
        eval(Op::Join, &[numbers, s(",")]),
        Err(KernelError::InputContract)
    );
    assert_eq!(eval(Op::Join, &[list(&[]), s(",")]), Ok(s("")));
    for op in Op::ALL {
        let kernel = bind(op, json!({}));
        assert_eq!(
            kernel.evaluate(&[]),
            Err(KernelError::InputContract),
            "{op:?}"
        );
        let wrong = vec![Value::I64(1); op.inputs().len()];
        assert_eq!(
            kernel.evaluate(&wrong),
            Err(KernelError::InputContract),
            "{op:?}"
        );
    }
}

#[test]
fn results_beyond_the_text_limit_fail_before_they_are_built() {
    let half = s(&"a".repeat(MAX_TEXT_BYTES / 2 + 1));
    assert_eq!(
        eval(Op::Concat, &[half.clone(), half.clone()]),
        Err(KernelError::LimitExceeded)
    );
    let mega = s(&"a".repeat(1 << 20));
    assert_eq!(
        eval(Op::Replace, &[mega.clone(), s("a"), s(&"b".repeat(17))]),
        Err(KernelError::LimitExceeded)
    );
    assert!(eval(Op::Replace, &[mega, s("a"), s(&"b".repeat(16))]).is_ok());
    assert_eq!(
        eval(
            Op::Join,
            &[
                list(&[&"a".repeat(MAX_TEXT_BYTES / 2 + 1) as &str; 2]),
                s("")
            ]
        ),
        Err(KernelError::LimitExceeded)
    );
    // U+0390 uppercases to three scalars (six bytes) from two bytes.
    let expanding = s(&"\u{390}".repeat(MAX_TEXT_BYTES / 4));
    assert_eq!(
        eval(Op::ToUppercase, &[expanding]),
        Err(KernelError::LimitExceeded)
    );
    assert_eq!(KernelError::LimitExceeded.code(), "core/LimitExceeded");
    assert_eq!(
        KernelError::LimitExceeded.message_key(),
        "core/kernel-limit-exceeded"
    );
}

#[test]
fn bindings_check_the_text_provider_versions_and_unit_config() {
    let (package, snapshot) = catalog();
    for op in Op::ALL {
        assert_eq!(bind(op, json!({})).unit(), TextUnit::Scalar);
        let configs = if op.has_unit() {
            vec![
                json!({"unit":"grapheme"}),
                json!({"unit":1}),
                json!({"other":true}),
            ]
        } else {
            vec![json!({"unit":"scalar"}), json!({"other":true})]
        };
        for config in configs {
            assert_eq!(
                bind_in(&package, snapshot.clone(), op, config).unwrap_err(),
                BindingError::InvalidConfig,
                "{op:?}"
            );
        }
        let definition = DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        };
        // The math provider of the same build never stands in for the text provider.
        assert_eq!(
            package
                .resolve_text(
                    snapshot.clone(),
                    package.provider_key(),
                    &definition,
                    1,
                    &Default::default()
                )
                .unwrap_err(),
            BindingError::Unavailable
        );
        for (version, config_version) in [(2, 1), (1, 2)] {
            assert_eq!(
                package
                    .resolve_text(
                        snapshot.clone(),
                        package.text_provider_key(),
                        &DefinitionRef {
                            version,
                            ..definition.clone()
                        },
                        config_version,
                        &Default::default()
                    )
                    .unwrap_err(),
                BindingError::Unavailable
            );
        }
        assert!(snapshot.provides(&definition, 1, package.text_provider_key()));
        assert!(!snapshot.provides(&definition, 1, package.provider_key()));
    }
    assert_eq!(package.text_provider_key().package, "org.orange.text");
    assert_eq!(
        package.text_provider_key().content_digest,
        package.provider_key().content_digest
    );
    let pinned = bind_in(&package, snapshot.clone(), Op::Trim, json!({})).unwrap();
    assert!(Arc::ptr_eq(pinned.snapshot(), &snapshot));
    assert_eq!(pinned.provider(), package.text_provider_key());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn split_then_join_restores_the_text(text in "[a-c,\u{e9}\u{1f34a}]{0,40}") {
        let parts = eval(Op::Split, &[s(&text), s(",")]).unwrap();
        prop_assert_eq!(eval(Op::Join, &[parts, s(",")]), Ok(s(&text)));
    }
    #[test]
    fn scalar_substrings_partition_the_text(text in ".{0,24}", cut in 0usize..25) {
        let count = text.chars().count();
        let cut = cut.min(count) as u64;
        let left = eval(Op::Substring, &[s(&text), Value::U64(0), Value::U64(cut)]).unwrap();
        let right = eval(Op::Substring, &[s(&text), Value::U64(cut), Value::U64(count as u64 - cut)]).unwrap();
        prop_assert_eq!(eval(Op::Concat, &[left, right]), Ok(s(&text)));
    }
    #[test]
    fn found_offsets_point_at_the_pattern(text in "[ab\u{e9}]{0,30}", pattern in "[ab\u{e9}]{1,3}") {
        if let Ok(Value::U64(at)) = unit(Op::Find, "byte", &[s(&text), s(&pattern)]) {
            prop_assert!(text[at as usize..].starts_with(&pattern));
        } else {
            prop_assert!(!text.contains(&pattern));
        }
    }
}

fn provider() -> Json {
    json!({"package":"org.orange.text","version":"0.0.0","id":"native-v1"})
}
fn literal(value: &str) -> Json {
    json!({"type":"core/string@1","value":value})
}
fn number(value: u64) -> Json {
    json!({"type":"core/u64@1","value":value.to_string()})
}
fn node(n: u32, op: Op, config: Json, inputs: Json) -> Json {
    json!({"id":id(n),"definition_id":op.definition_id(),"definition_version":1,"config_version":1,
        "config":config,"provider":provider(),"inputs":inputs,"signature":format!("s{n}")})
}
fn graph(
    package: &MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
) -> GraphDocument {
    let definitions = text_definitions().unwrap();
    let mut signatures = serde_json::Map::new();
    for node in &nodes {
        let definition = definitions
            .iter()
            .find(|d| Some(d.definition.id.as_str()) == node["definition_id"].as_str())
            .unwrap();
        let orange_model::catalog::Ports::Declared(ports) = definition.ports.clone() else {
            unreachable!()
        };
        signatures.insert(
            node["signature"].as_str().unwrap().into(),
            serde_json::to_value(orange_model::signature::project(
                definition.kind,
                ports,
                &Default::default(),
            ))
            .unwrap(),
        );
    }
    let key = package.text_provider_key();
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000005","revision":0,
        "package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "signatures":signatures,"nodes":nodes,"edges":edges,"exports":exports}).to_string()).unwrap()
}

#[test]
fn compiled_text_chains_fold_and_keep_fault_provenance() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            node(
                1,
                Op::Concat,
                json!({}),
                json!({"a":literal("H\u{e9}"),"b":literal("llo")}),
            ),
            node(2, Op::ToUppercase, json!({}), json!({})),
            node(
                3,
                Op::Substring,
                json!({"unit":"scalar"}),
                json!({"start":number(1),"length":number(3)}),
            ),
            node(4, Op::ByteLength, json!({}), json!({})),
            node(
                5,
                Op::Substring,
                json!({"unit":"byte"}),
                json!({"start":number(2),"length":number(1)}),
            ),
        ],
        vec![
            edge(1, 1, "result", 2, "value"),
            edge(2, 2, "result", 3, "text"),
            edge(3, 2, "result", 4, "value"),
            edge(4, 2, "result", 5, "text"),
        ],
        vec![
            json!({"id":"sub","node":id(3),"port":"result"}),
            json!({"id":"bytes","node":id(4),"port":"result"}),
            json!({"id":"fault","node":id(5),"port":"result"}),
        ],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let plan = compile_pure(
        &doc,
        snapshot.clone(),
        &analysis,
        &package,
        CompileOptions::default(),
    )
    .unwrap();
    let constant = |n: usize| plan.constants()[plan.exports()[n].slot.index()].clone();
    assert_eq!(constant(0), FoldedValue::Value(s("\u{c9}LL")));
    assert_eq!(constant(1), FoldedValue::Value(Value::U64(6)));
    let FoldedValue::Fault(fault) = constant(2) else {
        panic!("byte substring inside a scalar must fault")
    };
    assert_eq!(fault.fault.code, "core/Domain");
    assert_eq!(fault.source, id(5).parse().unwrap());
}
