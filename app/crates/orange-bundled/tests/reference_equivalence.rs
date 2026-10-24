//! Independent graph demand evaluation versus compiled constant exports (E2a).
//! The oracle shares the model's literal codec and metadata, not executable arithmetic.
#[path = "support/reference.rs"]
mod reference;

use orange_bundled::{MathPackage, Operation, math_definitions};
use orange_compiler::{CompileOptions, EvaluationOptions, compile_pure};
use orange_model::catalog::{Ports, RegistryBuilder, RegistrySnapshot};
use orange_model::document::NodeKind;
use orange_model::validation::HostOrigin;
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use proptest::prelude::*;
use reference::{Outcome, Reference};
use serde_json::{Value as Json, json};
use std::sync::Arc;

fn id(prefix: u8, n: usize) -> String {
    format!("{prefix:08x}-0000-4000-8000-{n:012x}")
}
fn catalog() -> (MathPackage, Arc<RegistrySnapshot>) {
    let digest = format!("sha256:{}", "a".repeat(64));
    let package = MathPackage::new(digest.clone()).unwrap();
    let registry = package
        .register(RegistryBuilder::core(
            HostOrigin::new("0.0.0", digest).unwrap(),
        ))
        .unwrap()
        .build()
        .unwrap();
    (package, registry)
}
fn literal(value: &Value) -> Json {
    match value {
        Value::I32(v) => json!({"type":"core/i32@1","value":v}),
        Value::I64(v) => json!({"type":"core/i64@1","value":v.to_string()}),
        Value::U32(v) => json!({"type":"core/u32@1","value":v}),
        Value::U64(v) => json!({"type":"core/u64@1","value":v.to_string()}),
        Value::Bool(v) => json!({"type":"core/bool@1","value":v}),
        _ => panic!("type outside oracle fixture vocabulary"),
    }
}
fn math_node(n: usize, op: Operation, a: &Value, b: &Value, mode: &str) -> Json {
    json!({"id":id(1,n),"definition_id":op.definition_id(),"definition_version":1,"config_version":1,
        "config":{"int_overflow":mode},"signature":op.definition_id(),
        "provider":{"package":"org.orange.math","version":"0.0.0","id":"native-v1"},
        "inputs":{"a":literal(a),"b":literal(b)}})
}
fn edge(n: usize, source: usize, op: Operation, target: usize, input: &str) -> Json {
    json!({"id":id(3,n),"from":{"node":id(1,source),"port":op.output_port()},
        "to":{"node":id(1,target),"port":input}})
}
fn export(n: usize, source: usize, port: &str) -> Json {
    json!({"id":format!("out-{n}"),"node":id(1,source),"port":port})
}
fn document(
    package: &MathPackage,
    ty: &str,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
) -> GraphDocument {
    let mut signatures = serde_json::Map::new();
    for definition in math_definitions().unwrap() {
        let args = if definition.type_params.is_empty() {
            Default::default()
        } else {
            [("T".into(), ty.parse().unwrap())].into()
        };
        let Ports::Declared(ports) = definition.ports else {
            panic!()
        };
        signatures.insert(
            definition.definition.id.to_string(),
            serde_json::to_value(orange_model::signature::project(
                NodeKind::Pure,
                ports,
                &args,
            ))
            .unwrap(),
        );
    }
    let key = package.provider_key();
    // Export/lock authoring commands do not exist yet: use the actual versioned JSON
    // boundary to assemble product documents, not an alternative behavioral test DSL.
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":id(2,1),"revision":0,
        "package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "signatures":signatures,"nodes":nodes,"edges":edges,"exports":exports}).to_string()).unwrap()
}
fn check(
    doc: &GraphDocument,
    package: &MathPackage,
    snapshot: &Arc<RegistrySnapshot>,
) -> Vec<Outcome> {
    let before = (to_json(doc).unwrap(), doc.state_id(), doc.revision());
    let analysis = analyze(doc, snapshot);
    assert!(
        analysis.is_ready_for_lowering(doc, snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let mut reference = Reference::new(doc, snapshot, &analysis);
    let expected = reference.exports();
    assert!(reference.executions.values().all(|count| *count == 1));
    let plan = compile_pure(
        doc,
        snapshot.clone(),
        &analysis,
        package,
        CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(plan.exports().len(), expected.len());
    let evaluation = plan
        .evaluate(doc, snapshot, EvaluationOptions::default())
        .unwrap();
    for ((export, source), outcome) in evaluation.exports().zip(doc.exports()).zip(&expected) {
        let export = export.unwrap();
        assert_eq!(export.id, source.id);
        match (export.outcome, outcome) {
            (Ok(actual), Ok(value)) => assert_eq!(actual, value),
            (Err(actual), Err(fault)) => {
                assert_eq!(actual.source, fault.source);
                assert_eq!(actual.fault.code, fault.code);
            }
            (actual, expected) => {
                panic!("compiled {actual:?} disagrees with independent {expected:?}")
            }
        }
    }
    assert_eq!(
        (to_json(doc).unwrap(), doc.state_id(), doc.revision()),
        before
    );
    expected
}

#[test]
fn diamond_has_known_answer_and_reference_memoizes_shared_producer() {
    let (package, snapshot) = catalog();
    let doc = document(
        &package,
        "core/i64@1",
        vec![
            math_node(
                4,
                Operation::Add,
                &Value::I64(999),
                &Value::I64(999),
                "checked",
            ),
            math_node(
                3,
                Operation::Multiply,
                &Value::I64(999),
                &Value::I64(3),
                "checked",
            ),
            math_node(
                2,
                Operation::Multiply,
                &Value::I64(999),
                &Value::I64(2),
                "checked",
            ),
            math_node(1, Operation::Add, &Value::I64(3), &Value::I64(4), "checked"),
            math_node(
                5,
                Operation::Divide,
                &Value::I64(1),
                &Value::I64(0),
                "checked",
            ),
        ],
        vec![
            edge(1, 1, Operation::Add, 2, "a"),
            edge(2, 1, Operation::Add, 3, "a"),
            edge(3, 2, Operation::Multiply, 4, "a"),
            edge(4, 3, Operation::Multiply, 4, "b"),
        ],
        vec![export(1, 4, "sum")],
    );
    assert_eq!(check(&doc, &package, &snapshot), [Ok(Value::I64(35))]);
    let analysis = analyze(&doc, &snapshot);
    let mut reference = Reference::new(&doc, &snapshot, &analysis);
    assert_eq!(reference.exports(), [Ok(Value::I64(35))]);
    assert_eq!(reference.executions.len(), 4);
    assert_eq!(reference.exports(), [Ok(Value::I64(35))]);
    assert!(reference.executions.values().all(|count| *count == 1));
}

#[test]
fn first_fault_is_declared_input_a_even_with_higher_uuid_and_reversed_storage() {
    let (package, snapshot) = catalog();
    let doc = document(
        &package,
        "core/i64@1",
        vec![
            math_node(3, Operation::Add, &Value::I64(0), &Value::I64(0), "checked"),
            math_node(
                1,
                Operation::Divide,
                &Value::I64(1),
                &Value::I64(0),
                "checked",
            ),
            math_node(
                9,
                Operation::Add,
                &Value::I64(i64::MAX),
                &Value::I64(1),
                "checked",
            ),
        ],
        vec![
            edge(1, 1, Operation::Divide, 3, "b"),
            edge(2, 9, Operation::Add, 3, "a"),
        ],
        vec![export(1, 3, "sum")],
    );
    let result = check(&doc, &package, &snapshot);
    assert_eq!(result[0].as_ref().unwrap_err().code, "core/NumericOverflow");
    assert_eq!(
        result[0].as_ref().unwrap_err().source,
        id(1, 9).parse().unwrap()
    );
}

#[test]
fn signed_division_overflow_and_unsigned_underflow_have_independent_known_answers() {
    let (package, snapshot) = catalog();
    for (mode, signed, unsigned) in [
        ("checked", None, None),
        (
            "wrapping",
            Some(Value::I64(i64::MIN)),
            Some(Value::U64(u64::MAX)),
        ),
        (
            "saturating",
            Some(Value::I64(i64::MAX)),
            Some(Value::U64(0)),
        ),
    ] {
        for (ty, op, a, b, expected) in [
            (
                "core/i64@1",
                Operation::Divide,
                Value::I64(i64::MIN),
                Value::I64(-1),
                signed,
            ),
            (
                "core/u64@1",
                Operation::Subtract,
                Value::U64(0),
                Value::U64(1),
                unsigned,
            ),
        ] {
            let doc = document(
                &package,
                ty,
                vec![math_node(1, op, &a, &b, mode)],
                vec![],
                vec![export(1, 1, op.output_port())],
            );
            let outcome = check(&doc, &package, &snapshot).remove(0);
            if let Some(value) = expected {
                assert_eq!(outcome, Ok(value));
            } else {
                assert_eq!(outcome.unwrap_err().code, "core/NumericOverflow");
            }
        }
    }
}

#[test]
fn boolean_and_integer_converter_results_have_independent_known_answers() {
    let (package, snapshot) = catalog();
    for input in [true, false] {
        let mut node = math_node(
            1,
            Operation::Not,
            &Value::Bool(input),
            &Value::Bool(input),
            "checked",
        );
        node["config"] = json!({});
        node["inputs"] = json!({"value":literal(&Value::Bool(input))});
        let doc = document(
            &package,
            "core/i64@1",
            vec![node],
            vec![],
            vec![export(1, 1, "result")],
        );
        assert_eq!(check(&doc, &package, &snapshot), [Ok(Value::Bool(!input))]);
    }
    for (converter, input, from, to, expected) in [
        (
            "core/i32-to-i64",
            Value::I32(i32::MIN),
            "core/i32@1",
            "core/i64@1",
            Some(Value::I64(i64::from(i32::MIN))),
        ),
        (
            "core/i64-to-i32",
            Value::I64(i64::from(i32::MAX)),
            "core/i64@1",
            "core/i32@1",
            Some(Value::I32(i32::MAX)),
        ),
        (
            "core/i64-to-i32",
            Value::I64(i64::from(i32::MAX) + 1),
            "core/i64@1",
            "core/i32@1",
            None,
        ),
    ] {
        let base = document(&package, "core/i64@1", vec![], vec![], vec![]);
        let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
        raw["nodes"] = json!([{"id":id(1,1),"definition_id":"core/convert","definition_version":1,"config_version":1,
            "signature":"convert","config":{"converter":converter,"contract_version":1,"options":{}},"inputs":{"value":literal(&input)}}]);
        raw["signatures"]["convert"] = json!({"kind":"pure","ports":[
            {"port_id":"value","direction":"input","kind":"data","type":from},
            {"port_id":"result","direction":"output","kind":"data","type":to,"source":"computed"}]});
        raw["exports"] = json!([export(1, 1, "result")]);
        let outcome = check(&from_json(&raw.to_string()).unwrap(), &package, &snapshot).remove(0);
        if let Some(value) = expected {
            assert_eq!(outcome, Ok(value));
        } else {
            assert_eq!(outcome.unwrap_err().code, "core/OutOfRange");
        }
    }
}

type Recipe = (u8, u64, u64, u8, u8);
fn dag(
    package: &MathPackage,
    kind: u8,
    mode: u8,
    recipes: &[Recipe],
    reverse: bool,
) -> GraphDocument {
    let ty = ["core/i32@1", "core/i64@1", "core/u32@1", "core/u64@1"][usize::from(kind)];
    let mode = ["checked", "wrapping", "saturating"][usize::from(mode)];
    let value = |bits| match kind {
        0 => Value::I32(bits as i32),
        1 => Value::I64(bits as i64),
        2 => Value::U32(bits as u32),
        3 => Value::U64(bits),
        _ => unreachable!(),
    };
    let operations: Vec<_> = recipes
        .iter()
        .map(|r| Operation::ALL[usize::from(r.0 % 4)])
        .collect();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for (i, recipe) in recipes.iter().enumerate() {
        // Earlier dependency indices intentionally have larger UUIDs.
        let n = recipes.len() - i;
        nodes.push(math_node(
            n,
            operations[i],
            &value(recipe.1),
            &value(recipe.2),
            mode,
        ));
        for (token, input) in [(recipe.3, "a"), (recipe.4, "b")] {
            if i > 0 && token < 192 {
                let source = usize::from(token) % i;
                edges.push(edge(
                    edges.len() + 1,
                    recipes.len() - source,
                    operations[source],
                    n,
                    input,
                ));
            }
        }
    }
    if reverse {
        nodes.reverse();
        edges.reverse();
    }
    let exports = [recipes.len() - 1, 0, recipes.len() / 2]
        .into_iter()
        .enumerate()
        .map(|(i, source)| export(i, recipes.len() - source, operations[source].output_port()))
        .collect();
    document(package, ty, nodes, edges, exports)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn bounded_integer_dags_match_independent_demand_evaluation(
        kind in 0_u8..4, mode in 0_u8..3,
        recipes in prop::collection::vec((any::<u8>(),any::<u64>(),any::<u64>(),any::<u8>(),any::<u8>()),1..25),
        reverse in any::<bool>(),
    ) {
        let (package,snapshot) = catalog();
        let doc = dag(&package,kind,mode,&recipes,reverse);
        let expected = check(&doc,&package,&snapshot);
        let saved = to_json(&doc).unwrap();
        let reloaded = from_json(&saved).unwrap();
        prop_assert_eq!(check(&reloaded,&package,&snapshot),expected);
    }
}
