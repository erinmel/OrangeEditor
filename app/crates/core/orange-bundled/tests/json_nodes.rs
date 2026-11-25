//! L04c `org.orange.json` reads and the string/json converters: kinds, navigation with
//! exact number text, duplicate keys, typed extraction and compiled folds.
use std::collections::BTreeMap;
use std::sync::Arc;

use orange_bundled::{
    BindingError, ConversionError, JsonOperation as Op, KernelError, MathPackage, ResolvedJson,
    json_definitions, resolve_core_conversion,
};
use orange_compiler::{CompileOptions, FoldedValue, compile_pure};
use orange_model::catalog::RegistrySnapshot;
use orange_model::conversion::{ConverterConfig, ConverterKey, ConverterProvider};
use orange_model::document::DefinitionRef;
use orange_model::value::{JsonText, ListRepr, MAX_JSON_TEXT_BYTES, MapRepr};
use orange_model::{GraphDocument, Value, analyze, from_json};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

use crate::data_graph::{catalog, edge, id};

fn bind_in(
    package: &MathPackage,
    snapshot: Arc<RegistrySnapshot>,
    op: Op,
) -> Result<ResolvedJson, BindingError> {
    package.resolve_json(
        snapshot,
        package.json_provider_key(),
        &DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        },
        1,
        &Default::default(),
    )
}
fn j(text: &str) -> Value {
    Value::Json(Arc::new(JsonText::try_from(text).unwrap()))
}
fn s(text: &str) -> Value {
    Value::Str(Arc::new(text.into()))
}
fn eval(op: Op, inputs: &[Value]) -> Result<Value, KernelError> {
    let (package, snapshot) = catalog();
    bind_in(&package, snapshot, op).unwrap().evaluate(inputs)
}
fn text_of(value: Result<Value, KernelError>) -> String {
    match value {
        Ok(Value::Json(text)) => text.as_str().to_owned(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn json_builders_preserve_order_number_text_and_explicit_null() {
    let number = "1.0000000000000000000002e-10";
    let fields = MapRepr::new(vec![(s("quoted\""), j(number)), (s("z"), j("null"))]).unwrap();
    let object = eval(Op::BuildObject, &[Value::Map(Arc::new(fields))]).unwrap();
    assert_eq!(
        text_of(Ok(object.clone())),
        format!(r#"{{"quoted\"":{number},"z":null}}"#)
    );
    let items = ListRepr::Values(vec![object, j("true"), j("null")].into_boxed_slice());
    assert_eq!(
        text_of(eval(Op::BuildArray, &[Value::List(Arc::new(items))])),
        format!(r#"[{{"quoted\"":{number},"z":null}},true,null]"#)
    );
    assert_eq!(text_of(eval(Op::Null, &[])), "null");
    assert_eq!(
        text_of(eval(Op::FromU64, &[Value::U64(u64::MAX)])),
        u64::MAX.to_string()
    );
    assert_eq!(text_of(eval(Op::FromString, &[s("a\n\"")])), r#""a\n\"""#);
    assert_eq!(
        eval(Op::FromF64, &[Value::F64(f64::NAN)]),
        Err(KernelError::Domain)
    );
}

#[test]
fn json_field_edits_replace_duplicates_and_enforce_output_budget() {
    let object = j(r#"{"name":1,"keep":0.000000000000000000002,"\u006eame":3}"#);
    let updated = eval(Op::SetField, &[object, s("name"), j("null")]).unwrap();
    assert_eq!(
        text_of(Ok(updated.clone())),
        r#"{"keep":0.000000000000000000002,"name":null}"#
    );
    assert_eq!(eval(Op::Get, &[updated.clone(), s("name")]), Ok(j("null")));
    let removed = eval(Op::RemoveField, &[updated, s("name")]).unwrap();
    assert_eq!(eval(Op::Get, &[removed, s("name")]), Ok(Value::None));
    assert_eq!(
        eval(Op::RemoveField, &[j("[]"), s("name")]),
        Err(KernelError::Domain)
    );
    assert_eq!(
        eval(Op::FromString, &[s(&"\n".repeat(MAX_JSON_TEXT_BYTES / 2))]),
        Err(KernelError::OutOfRange)
    );
}

#[test]
fn kinds_and_typed_reads_never_conflate_null_missing_and_wrong_kind() {
    for (text, kind) in [
        ("null", "null"),
        ("true", "boolean"),
        (" -1.5e3 ", "number"),
        ("\"x\"", "string"),
        ("[1]", "array"),
        ("{}", "object"),
    ] {
        assert_eq!(eval(Op::Kind, &[j(text)]), Ok(s(kind)), "{text}");
        assert_eq!(
            eval(Op::IsNull, &[j(text)]),
            Ok(Value::Bool(kind == "null"))
        );
    }
    assert_eq!(eval(Op::AsBool, &[j("false")]), Ok(Value::Bool(false)));
    assert_eq!(eval(Op::AsBool, &[j("null")]), Ok(Value::None));
    assert_eq!(eval(Op::AsString, &[j(r#""a\"é""#)]), Ok(s("a\"\u{e9}")));
    assert_eq!(eval(Op::AsString, &[j("1")]), Ok(Value::None));
    assert_eq!(
        eval(Op::AsI64, &[j("-9223372036854775808")]),
        Ok(Value::I64(i64::MIN))
    );
    assert_eq!(
        eval(Op::AsI64, &[j("9223372036854775808")]),
        Err(KernelError::OutOfRange)
    );
    assert_eq!(
        eval(Op::AsU64, &[j("18446744073709551615")]),
        Ok(Value::U64(u64::MAX))
    );
    assert_eq!(eval(Op::AsU64, &[j("-1")]), Err(KernelError::OutOfRange));
    for not_integer in ["1.0", "1e2", "-0.5"] {
        assert_eq!(
            eval(Op::AsI64, &[j(not_integer)]),
            Err(KernelError::Domain),
            "{not_integer}"
        );
    }
    assert_eq!(eval(Op::AsI64, &[j("\"1\"")]), Ok(Value::None));
    assert_eq!(eval(Op::AsF64, &[j("0.1")]), Ok(Value::F64(0.1)));
    assert_eq!(eval(Op::AsF64, &[j("1e400")]), Err(KernelError::OutOfRange));
    assert_eq!(eval(Op::AsF64, &[j("[]")]), Ok(Value::None));
}

#[test]
fn navigation_keeps_exact_text_and_resolves_duplicate_keys_last() {
    let doc =
        j(r#"{ "a": 12345678901234567890.000, "b": [1, {"c": null}, "x,]}"], "ab": 2, "a": -0 }"#);
    // The last duplicate wins, and number text is returned exactly as written.
    assert_eq!(text_of(eval(Op::Get, &[doc.clone(), s("a")])), "-0");
    assert_eq!(text_of(eval(Op::Get, &[doc.clone(), s("ab")])), "2");
    assert_eq!(eval(Op::Get, &[doc.clone(), s("zz")]), Ok(Value::None));
    let list = eval(Op::Get, &[doc.clone(), s("b")]).unwrap();
    assert_eq!(text_of(Ok(list.clone())), r#"[1, {"c": null}, "x,]}"]"#);
    assert_eq!(
        eval(Op::Length, std::slice::from_ref(&list)),
        Ok(Value::U64(3))
    );
    assert_eq!(
        text_of(eval(Op::At, &[list.clone(), Value::U64(2)])),
        r#""x,]}""#
    );
    let inner = eval(Op::At, &[list.clone(), Value::U64(1)]).unwrap();
    // A present JSON null is a value, not a missing member.
    assert_eq!(text_of(eval(Op::Get, &[inner, s("c")])), "null");
    assert_eq!(eval(Op::At, &[list, Value::U64(3)]), Ok(Value::None));
    assert_eq!(
        eval(Op::Length, std::slice::from_ref(&doc)),
        Ok(Value::U64(3))
    );
    let exact = j(r#"{"n": 12345678901234567890.000}"#);
    assert_eq!(
        text_of(eval(Op::Get, &[exact, s("n")])),
        "12345678901234567890.000"
    );
    assert_eq!(eval(Op::Length, &[j("[]")]), Ok(Value::U64(0)));
    assert_eq!(eval(Op::Length, &[j("{ }")]), Ok(Value::U64(0)));
    assert_eq!(eval(Op::Get, &[j("[1]"), s("a")]), Err(KernelError::Domain));
    assert_eq!(
        eval(Op::At, &[j("{}"), Value::U64(0)]),
        Err(KernelError::Domain)
    );
    assert_eq!(eval(Op::Length, &[j("\"abc\"")]), Err(KernelError::Domain));
    assert_eq!(
        eval(Op::Get, &[s("{}"), s("a")]),
        Err(KernelError::InputContract)
    );
}

#[test]
fn paths_read_nested_items_and_quoted_fields_without_losing_exact_text() {
    let doc = j(r#"{"items":[{"levels":{"peak.db":-0.000,"label":null,"say\"hi":false}}]}"#);
    for (path, expected) in [
        (r#"items[0].levels["peak.db"]"#, "-0.000"),
        ("items[0].levels.label", "null"),
        (r#"items[0].levels["say\"hi"]"#, "false"),
    ] {
        assert_eq!(
            text_of(eval(Op::GetPath, &[doc.clone(), s(path)])),
            expected
        );
    }
    for path in ["items[1].levels", "items.levels", "items[0].levels.absent"] {
        assert_eq!(eval(Op::GetPath, &[doc.clone(), s(path)]), Ok(Value::None));
    }
    assert_eq!(text_of(eval(Op::GetPath, &[j("[true]"), s("[0]")])), "true");
}

#[test]
fn paths_validate_the_whole_selector_even_when_a_segment_is_missing() {
    for path in [
        "",
        "missing..name",
        "missing[*]",
        "missing[",
        "missing[-1]",
        "missing[\"\\",
    ] {
        assert_eq!(
            eval(Op::GetPath, &[j("{}"), s(path)]),
            Err(KernelError::Domain),
            "{path}"
        );
    }
    assert_eq!(
        eval(Op::GetPath, &[j("{}"), s(&"a.".repeat(2049))]),
        Err(KernelError::OutOfRange)
    );
}

/// Read nodes go from JSON to a typed value in one step: the value at the path, or the
/// fallback when it is missing, of another kind, not whole or out of range.
#[test]
fn reads_give_the_typed_value_or_the_fallback() {
    let doc = j(
        r#"{"user":{"name":"Ada \"A\"","age":36,"score":9.5,"admin":true,"none":null},
        "items":[{"id":7}],"big":99999999999999999999,"whole":2.0}"#,
    );
    let read = |op: Op, path: &str, fallback: Value| eval(op, &[doc.clone(), s(path), fallback]);

    assert_eq!(read(Op::ReadText, "user.name", s("-")), Ok(s("Ada \"A\"")));
    assert_eq!(
        read(Op::ReadInt, "user.age", Value::I64(0)),
        Ok(Value::I64(36))
    );
    assert_eq!(
        read(Op::ReadInt, "items[0].id", Value::I64(0)),
        Ok(Value::I64(7))
    );
    assert_eq!(
        read(Op::ReadNumber, "user.score", Value::F64(0.0)),
        Ok(Value::F64(9.5))
    );
    assert_eq!(
        read(Op::ReadNumber, "user.age", Value::F64(0.0)),
        Ok(Value::F64(36.0))
    );
    assert_eq!(
        read(Op::ReadBool, "user.admin", Value::Bool(false)),
        Ok(Value::Bool(true))
    );

    // Missing, null, another kind, not whole, or too big: the fallback.
    assert_eq!(read(Op::ReadText, "user.missing", s("-")), Ok(s("-")));
    assert_eq!(read(Op::ReadText, "user.none", s("-")), Ok(s("-")));
    assert_eq!(read(Op::ReadText, "user.age", s("-")), Ok(s("-")));
    assert_eq!(
        read(Op::ReadInt, "user.name", Value::I64(-1)),
        Ok(Value::I64(-1))
    );
    assert_eq!(
        read(Op::ReadInt, "whole", Value::I64(-1)),
        Ok(Value::I64(-1))
    );
    assert_eq!(read(Op::ReadInt, "big", Value::I64(-1)), Ok(Value::I64(-1)));
    assert_eq!(
        read(Op::ReadBool, "items[5]", Value::Bool(false)),
        Ok(Value::Bool(false))
    );

    // An empty path reads the whole value.
    assert_eq!(
        eval(Op::ReadInt, &[j("42"), s(" "), Value::I64(0)]),
        Ok(Value::I64(42))
    );
    // A broken path is a mistake in the graph, not missing data.
    assert_eq!(
        read(Op::ReadText, "user..name", s("-")),
        Err(KernelError::Domain)
    );
    // The fallback must have the node's type.
    assert_eq!(
        read(Op::ReadInt, "user.age", s("0")),
        Err(KernelError::InputContract)
    );
}

#[test]
fn string_json_converters_validate_and_keep_text() {
    let (_, snapshot) = catalog();
    let bind = |entry: &str| {
        resolve_core_conversion(
            snapshot.clone(),
            &ConverterConfig {
                entry: ConverterKey {
                    id: entry.parse().unwrap(),
                    contract_version: 1,
                },
                options: BTreeMap::new(),
            },
            &ConverterProvider::Core,
        )
        .unwrap()
    };
    let parse = bind("core/string-to-json");
    assert_eq!(
        parse.evaluate(&s(" [1, 2.50] ")).unwrap().result,
        j(" [1, 2.50] ")
    );
    assert_eq!(parse.evaluate(&s("[1,")), Err(ConversionError::Parse));
    assert_eq!(parse.evaluate(&s("")), Err(ConversionError::Parse));
    let deep = "[".repeat(200) + &"]".repeat(200);
    assert_eq!(
        parse.evaluate(&s(&deep)),
        Err(ConversionError::LimitExceeded)
    );
    let text = bind("core/json-to-string");
    let output = text.evaluate(&j("{\"a\":1.50}")).unwrap();
    assert_eq!(output.result, s("{\"a\":1.50}"));
    assert!(output.loss.is_none());
    assert_eq!(ConversionError::LimitExceeded.code(), "core/LimitExceeded");
}

#[test]
fn bindings_check_the_json_provider_and_empty_config() {
    let (package, snapshot) = catalog();
    for op in Op::ALL {
        assert!(bind_in(&package, snapshot.clone(), op).is_ok());
        let definition = DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        };
        assert_eq!(
            package
                .resolve_json(
                    snapshot.clone(),
                    package.provider_key(),
                    &definition,
                    1,
                    &Default::default()
                )
                .unwrap_err(),
            BindingError::Unavailable
        );
        assert_eq!(
            package
                .resolve_json(
                    snapshot.clone(),
                    package.json_provider_key(),
                    &definition,
                    1,
                    &serde_json::from_value(json!({"path":"a"})).unwrap()
                )
                .unwrap_err(),
            BindingError::InvalidConfig
        );
    }
    assert_eq!(json_definitions().unwrap().len(), Op::ALL.len());
    assert_eq!(package.json_provider_key().package, "org.orange.json");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn array_navigation_matches_serde(items in proptest::collection::vec(prop_oneof![
        any::<i64>().prop_map(|v| json!(v)),
        "[a-z\",\\\\]{0,6}".prop_map(|v| json!(v)),
        Just(json!(null)),
        Just(json!({"k":[1,{"z":"]"}]})),
    ], 0..8)) {
        let text = serde_json::to_string(&items).unwrap();
        let array = j(&text);
        prop_assert_eq!(eval(Op::Length, std::slice::from_ref(&array)), Ok(Value::U64(items.len() as u64)));
        for (index, item) in items.iter().enumerate() {
            let got = text_of(eval(Op::At, &[array.clone(), Value::U64(index as u64)]));
            prop_assert_eq!(serde_json::from_str::<Json>(&got).unwrap(), item.clone());
        }
    }
}

fn literal(ty: &str, value: &Value) -> Json {
    let (_, snapshot) = catalog();
    serde_json::to_value(
        orange_model::value::literal_codec::encode(
            &ty.parse().unwrap(),
            value,
            snapshot.types().as_ref(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn node(n: u32, op: Op, inputs: Json) -> Json {
    json!({"id":id(n),"definition_id":op.definition_id(),"definition_version":1,"config_version":1,
        "config":{},"provider":{"package":"org.orange.json","version":"0.0.0","id":"native-v1"},
        "inputs":inputs,"signature":format!("s{n}")})
}
fn graph(
    package: &MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
) -> GraphDocument {
    let definitions = json_definitions().unwrap();
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
    let key = package.json_provider_key();
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000008","revision":0,
        "package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "signatures":signatures,"nodes":nodes,"edges":edges,"exports":exports}).to_string()).unwrap()
}

#[test]
fn compiled_json_reads_fold_and_keep_fault_provenance() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            node(
                1,
                Op::Get,
                json!({"value":literal("core/json@1", &j(r#"{"user":{"age":42}}"#)),"key":literal("core/string@1", &s("user"))}),
            ),
            node(
                2,
                Op::Get,
                json!({"key":literal("core/string@1", &s("age"))}),
            ),
            node(
                3,
                Op::Kind,
                json!({"value":literal("core/json@1", &j("[true]"))}),
            ),
            node(
                4,
                Op::AsI64,
                json!({"value":literal("core/json@1", &j("4.5"))}),
            ),
        ],
        vec![edge(1, 1, "result", 2, "value")],
        vec![
            json!({"id":"age","node":id(2),"port":"result"}),
            json!({"id":"kind","node":id(3),"port":"result"}),
            json!({"id":"fault","node":id(4),"port":"result"}),
        ],
    );
    let analysis = analyze(&doc, &snapshot);
    // `get` returns option<json>; feeding it into another `get` is a type mismatch.
    assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
    let doc = graph(
        &package,
        vec![
            node(
                3,
                Op::Kind,
                json!({"value":literal("core/json@1", &j("[true]"))}),
            ),
            node(
                4,
                Op::AsI64,
                json!({"value":literal("core/json@1", &j("4.5"))}),
            ),
            node(
                5,
                Op::Length,
                json!({"value":literal("core/json@1", &j(r#"{"a":1,"a":2,"b":3}"#))}),
            ),
        ],
        vec![],
        vec![
            json!({"id":"kind","node":id(3),"port":"result"}),
            json!({"id":"fault","node":id(4),"port":"result"}),
            json!({"id":"length","node":id(5),"port":"result"}),
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
    assert_eq!(constant(0), FoldedValue::Value(s("array")));
    let FoldedValue::Fault(fault) = constant(1) else {
        panic!("a fractional number read as i64 must fault")
    };
    assert_eq!(fault.fault.code, "core/Domain");
    assert_eq!(fault.source, id(4).parse().unwrap());
    assert_eq!(constant(2), FoldedValue::Value(Value::U64(2)));
}
