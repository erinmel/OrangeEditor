//! L04a `org.orange.collections`: generic list/map/option known answers, option boxing,
//! shape checks, limits, binding and compiled folds.
use std::sync::Arc;

use orange_bundled::{
    BindingError, CollectionOperation as Op, KernelError, MAX_COLLECTION_BYTES, MathPackage,
    ResolvedCollection, collection_definitions,
};
use orange_compiler::{CompileOptions, FoldedValue, compile_pure};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::DefinitionRef;
use orange_model::value::{ListRepr, MapRepr};
use orange_model::{GraphDocument, Value, analyze, from_json};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

use crate::data_graph::{catalog, edge, id};

const LIST_I64: &str = "core/list@1<core/i64@1>";
const MAP_STR_I64: &str = "core/map@1<core/string@1,core/i64@1>";

fn bind_in(
    package: &MathPackage,
    snapshot: Arc<RegistrySnapshot>,
    op: Op,
    container: &str,
) -> Result<ResolvedCollection, BindingError> {
    package.resolve_collection(
        snapshot,
        package.collections_provider_key(),
        &DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        },
        1,
        &container.parse().unwrap(),
        &Default::default(),
    )
}
fn bind(op: Op, container: &str) -> ResolvedCollection {
    let (package, snapshot) = catalog();
    bind_in(&package, snapshot, op, container).unwrap()
}
fn ints(items: &[i64]) -> Value {
    Value::List(Arc::new(ListRepr::I64(items.to_vec().into_boxed_slice())))
}
fn s(text: &str) -> Value {
    Value::Str(Arc::new(text.into()))
}
fn map(entries: &[(&str, i64)]) -> Value {
    let entries: Vec<_> = entries
        .iter()
        .map(|(k, v)| (s(k), Value::I64(*v)))
        .collect();
    Value::Map(Arc::new(MapRepr::new(entries).unwrap()))
}
fn strings(items: &[&str]) -> Value {
    let items: Vec<Arc<String>> = items.iter().map(|i| Arc::new(i.to_string())).collect();
    Value::List(Arc::new(ListRepr::Str(items.into())))
}

#[test]
fn list_operations_have_known_answers() {
    let list = ints(&[3, 1, 4, 1, 5]);
    let eval = |op: Op, rest: &[Value]| {
        let mut inputs = vec![list.clone()];
        inputs.extend_from_slice(rest);
        bind(op, LIST_I64).evaluate(&inputs)
    };
    assert_eq!(eval(Op::ListLength, &[]), Ok(Value::U64(5)));
    assert_eq!(eval(Op::ListGet, &[Value::U64(2)]), Ok(Value::I64(4)));
    assert_eq!(eval(Op::ListGet, &[Value::U64(5)]), Ok(Value::None));
    assert_eq!(eval(Op::ListGet, &[Value::U64(u64::MAX)]), Ok(Value::None));
    assert_eq!(
        eval(Op::ListSlice, &[Value::U64(1), Value::U64(3)]),
        Ok(ints(&[1, 4, 1]))
    );
    assert_eq!(
        eval(Op::ListSlice, &[Value::U64(5), Value::U64(0)]),
        Ok(ints(&[]))
    );
    assert_eq!(
        eval(Op::ListSlice, &[Value::U64(4), Value::U64(2)]),
        Err(KernelError::OutOfRange)
    );
    assert_eq!(
        eval(Op::ListSlice, &[Value::U64(u64::MAX), Value::U64(1)]),
        Err(KernelError::OutOfRange)
    );
    assert_eq!(
        eval(Op::ListConcat, &[ints(&[9])]),
        Ok(ints(&[3, 1, 4, 1, 5, 9]))
    );
    assert_eq!(
        eval(Op::ListAppend, &[Value::I64(2)]),
        Ok(ints(&[3, 1, 4, 1, 5, 2]))
    );
    assert_eq!(eval(Op::ListReverse, &[]), Ok(ints(&[5, 1, 4, 1, 3])));
    assert_eq!(
        eval(Op::ListContains, &[Value::I64(4)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        eval(Op::ListContains, &[Value::I64(7)]),
        Ok(Value::Bool(false))
    );
    assert_eq!(eval(Op::ListIndexOf, &[Value::I64(1)]), Ok(Value::U64(1)));
    assert_eq!(eval(Op::ListIndexOf, &[Value::I64(7)]), Ok(Value::None));
    // Generic and packed storage of the same list are equal values.
    let generic = Value::List(Arc::new(ListRepr::Values(
        vec![Value::I64(3), Value::I64(1)].into_boxed_slice(),
    )));
    assert_eq!(
        bind(Op::ListReverse, LIST_I64).evaluate(&[generic]),
        Ok(ints(&[1, 3]))
    );
}

#[test]
fn map_operations_keep_insertion_order_and_replace_in_place() {
    let m = map(&[("a", 1), ("b", 2)]);
    let eval = |op: Op, rest: &[Value]| {
        let mut inputs = vec![m.clone()];
        inputs.extend_from_slice(rest);
        bind(op, MAP_STR_I64).evaluate(&inputs)
    };
    assert_eq!(eval(Op::MapLength, &[]), Ok(Value::U64(2)));
    assert_eq!(eval(Op::MapGet, &[s("b")]), Ok(Value::I64(2)));
    assert_eq!(eval(Op::MapGet, &[s("z")]), Ok(Value::None));
    assert_eq!(eval(Op::MapContainsKey, &[s("a")]), Ok(Value::Bool(true)));
    let Ok(Value::Map(updated)) = eval(Op::MapInsert, &[s("a"), Value::I64(9)]) else {
        panic!()
    };
    assert_eq!(
        updated.entries(),
        &[(s("a"), Value::I64(9)), (s("b"), Value::I64(2))]
    );
    let Ok(Value::Map(grown)) = eval(Op::MapInsert, &[s("c"), Value::I64(3)]) else {
        panic!()
    };
    assert_eq!(grown.entries().last(), Some(&(s("c"), Value::I64(3))));
    assert_eq!(eval(Op::MapRemove, &[s("a")]), Ok(map(&[("b", 2)])));
    assert_eq!(eval(Op::MapRemove, &[s("z")]), Ok(m.clone()));
    assert_eq!(eval(Op::MapKeys, &[]), Ok(strings(&["a", "b"])));
    assert_eq!(eval(Op::MapValues, &[]), Ok(ints(&[1, 2])));
    assert_eq!(
        bind(Op::MapKeys, MAP_STR_I64).evaluate(&[Value::Map(Arc::new(MapRepr::default()))]),
        Ok(strings(&[]))
    );
}

#[test]
fn optional_results_box_only_when_the_element_can_be_none() {
    let nested = "core/list@1<core/option@1<core/i64@1>>";
    let items = Value::List(Arc::new(ListRepr::Values(
        vec![Value::None, Value::I64(5)].into_boxed_slice(),
    )));
    let get = bind(Op::ListGet, nested);
    // A present None element must stay distinguishable from a missing index.
    assert_eq!(
        get.evaluate(&[items.clone(), Value::U64(0)]),
        Ok(Value::Some(Arc::new(Value::None)))
    );
    assert_eq!(
        get.evaluate(&[items.clone(), Value::U64(1)]),
        Ok(Value::Some(Arc::new(Value::I64(5))))
    );
    assert_eq!(get.evaluate(&[items, Value::U64(2)]), Ok(Value::None));
    let some = bind(Op::OptionIsSome, "core/option@1<core/i64@1>");
    assert_eq!(some.evaluate(&[Value::I64(0)]), Ok(Value::Bool(true)));
    assert_eq!(some.evaluate(&[Value::None]), Ok(Value::Bool(false)));
    let deep = bind(Op::OptionIsSome, "core/option@1<core/option@1<core/i64@1>>");
    assert_eq!(
        deep.evaluate(&[Value::Some(Arc::new(Value::None))]),
        Ok(Value::Bool(true))
    );
}

#[test]
fn inputs_are_checked_against_the_bound_shape_and_growth_is_limited() {
    let kernel = bind(Op::ListAppend, LIST_I64);
    assert_eq!(
        kernel.evaluate(&[ints(&[1]), Value::U64(1)]),
        Err(KernelError::InputContract)
    );
    assert_eq!(
        kernel.evaluate(&[strings(&["x"]), Value::I64(1)]),
        Err(KernelError::InputContract)
    );
    assert_eq!(
        kernel.evaluate(&[ints(&[1])]),
        Err(KernelError::InputContract)
    );
    assert_eq!(
        bind(Op::MapGet, MAP_STR_I64).evaluate(&[map(&[("a", 1)]), Value::I64(1)]),
        Err(KernelError::InputContract)
    );
    let big = strings(&[&"x".repeat(MAX_COLLECTION_BYTES / 2)]);
    let concat = bind(Op::ListConcat, "core/list@1<core/string@1>");
    assert_eq!(
        concat.evaluate(&[big.clone(), big.clone()]),
        Err(KernelError::LimitExceeded)
    );
    let over = strings(&[&"x".repeat(MAX_COLLECTION_BYTES + 1)]);
    assert_eq!(
        bind(Op::ListLength, "core/list@1<core/string@1>").evaluate(&[over]),
        Err(KernelError::LimitExceeded)
    );
}

#[test]
fn binding_checks_provider_container_shape_and_eq_capability() {
    let (package, snapshot) = catalog();
    for op in Op::ALL {
        let container = match op {
            Op::OptionIsSome => "core/option@1<core/i64@1>",
            op if op.definition_id().contains("/map-") => MAP_STR_I64,
            _ => LIST_I64,
        };
        assert!(
            bind_in(&package, snapshot.clone(), op, container).is_ok(),
            "{op:?}"
        );
        for wrong in ["core/i64@1", "core/string@1"] {
            assert_eq!(
                bind_in(&package, snapshot.clone(), op, wrong).unwrap_err(),
                BindingError::UnsupportedType
            );
        }
        let definition = DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        };
        assert_eq!(
            package
                .resolve_collection(
                    snapshot.clone(),
                    package.provider_key(),
                    &definition,
                    1,
                    &container.parse().unwrap(),
                    &Default::default()
                )
                .unwrap_err(),
            BindingError::Unavailable
        );
        assert_eq!(
            package
                .resolve_collection(
                    snapshot.clone(),
                    package.collections_provider_key(),
                    &definition,
                    1,
                    &container.parse().unwrap(),
                    &serde_json::from_value(json!({"unit":"x"})).unwrap()
                )
                .unwrap_err(),
            BindingError::InvalidConfig
        );
    }
    // Floats have no Core eq, so membership tests refuse them; plain access does not.
    for op in [Op::ListContains, Op::ListIndexOf] {
        assert_eq!(
            bind_in(&package, snapshot.clone(), op, "core/list@1<core/f64@1>").unwrap_err(),
            BindingError::UnsupportedType
        );
    }
    assert!(
        bind_in(
            &package,
            snapshot.clone(),
            Op::ListGet,
            "core/list@1<core/f64@1>"
        )
        .is_ok()
    );
    assert_eq!(
        bind_in(&package, snapshot.clone(), Op::MapGet, LIST_I64).unwrap_err(),
        BindingError::UnsupportedType
    );
    assert_eq!(
        package.collections_provider_key().package,
        "org.orange.collections"
    );
    assert_eq!(collection_definitions().unwrap().len(), Op::ALL.len());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn slices_partition_and_reverse_is_an_involution(items in proptest::collection::vec(any::<i64>(), 0..40), cut in 0usize..41) {
        let list = ints(&items);
        let cut = cut.min(items.len()) as u64;
        let slice = bind(Op::ListSlice, LIST_I64);
        let left = slice.evaluate(&[list.clone(), Value::U64(0), Value::U64(cut)]).unwrap();
        let right = slice.evaluate(&[list.clone(), Value::U64(cut), Value::U64(items.len() as u64 - cut)]).unwrap();
        prop_assert_eq!(bind(Op::ListConcat, LIST_I64).evaluate(&[left, right]).unwrap(), list.clone());
        let reverse = bind(Op::ListReverse, LIST_I64);
        let twice = reverse.evaluate(&[reverse.evaluate(std::slice::from_ref(&list)).unwrap()]).unwrap();
        prop_assert_eq!(twice, list);
    }
    #[test]
    fn index_of_agrees_with_get_and_contains(items in proptest::collection::vec(0i64..6, 0..20), needle in 0i64..6) {
        let list = ints(&items);
        let found = bind(Op::ListIndexOf, LIST_I64).evaluate(&[list.clone(), Value::I64(needle)]).unwrap();
        let contains = bind(Op::ListContains, LIST_I64).evaluate(&[list.clone(), Value::I64(needle)]).unwrap();
        prop_assert_eq!(contains, Value::Bool(items.contains(&needle)));
        match found {
            Value::U64(at) => {
                prop_assert_eq!(bind(Op::ListGet, LIST_I64).evaluate(&[list, Value::U64(at)]).unwrap(), Value::I64(needle));
                prop_assert!(!items[..at as usize].contains(&needle));
            }
            other => {
                prop_assert_eq!(other, Value::None);
                prop_assert!(!items.contains(&needle));
            }
        }
    }
    #[test]
    fn insert_then_get_returns_the_value_and_remove_forgets_it(keys in proptest::collection::btree_set("[a-d]{1,2}", 0..8), key in "[a-d]{1,2}", value in any::<i64>()) {
        let entries: Vec<(&str, i64)> = keys.iter().map(|k| (k.as_str(), 0)).collect();
        let m = map(&entries);
        let inserted = bind(Op::MapInsert, MAP_STR_I64).evaluate(&[m, s(&key), Value::I64(value)]).unwrap();
        prop_assert_eq!(bind(Op::MapGet, MAP_STR_I64).evaluate(&[inserted.clone(), s(&key)]).unwrap(), Value::I64(value));
        let removed = bind(Op::MapRemove, MAP_STR_I64).evaluate(&[inserted, s(&key)]).unwrap();
        prop_assert_eq!(bind(Op::MapContainsKey, MAP_STR_I64).evaluate(&[removed, s(&key)]).unwrap(), Value::Bool(false));
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
fn node(n: u32, op: Op, pins: Json, inputs: Json) -> Json {
    json!({"id":id(n),"definition_id":op.definition_id(),"definition_version":1,"config_version":1,
        "config":{},"type_pins":pins,"provider":{"package":"org.orange.collections","version":"0.0.0","id":"native-v1"},
        "inputs":inputs,"signature":format!("s{n}")})
}
fn graph(
    package: &MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
) -> GraphDocument {
    let definitions = collection_definitions().unwrap();
    let mut signatures = serde_json::Map::new();
    for node in &nodes {
        let definition = definitions
            .iter()
            .find(|d| Some(d.definition.id.as_str()) == node["definition_id"].as_str())
            .unwrap();
        let orange_model::catalog::Ports::Declared(ports) = definition.ports.clone() else {
            unreachable!()
        };
        let args = node["type_pins"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().parse().unwrap()))
            .collect();
        signatures.insert(
            node["signature"].as_str().unwrap().into(),
            serde_json::to_value(orange_model::signature::project(
                definition.kind,
                ports,
                &args,
            ))
            .unwrap(),
        );
    }
    let key = package.collections_provider_key();
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000006","revision":0,
        "package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "signatures":signatures,"nodes":nodes,"edges":edges,"exports":exports}).to_string()).unwrap()
}

#[test]
fn compiled_generic_collection_chains_fold_and_keep_fault_provenance() {
    let (package, snapshot) = catalog();
    let t = json!({"T":"core/i64@1"});
    let kv = json!({"K":"core/string@1","V":"core/i64@1"});
    let doc = graph(
        &package,
        vec![
            node(
                1,
                Op::ListAppend,
                t.clone(),
                json!({"items":literal(LIST_I64, &ints(&[1, 2])),"item":literal("core/i64@1", &Value::I64(3))}),
            ),
            node(2, Op::ListReverse, t.clone(), json!({})),
            node(
                3,
                Op::ListGet,
                t.clone(),
                json!({"index":literal("core/u64@1", &Value::U64(0))}),
            ),
            node(
                4,
                Op::MapInsert,
                kv.clone(),
                json!({"map":literal(MAP_STR_I64, &map(&[("a", 1)])),"key":literal("core/string@1", &s("b")),"value":literal("core/i64@1", &Value::I64(2))}),
            ),
            node(5, Op::MapKeys, kv, json!({})),
            node(
                6,
                Op::ListSlice,
                t,
                json!({"start":literal("core/u64@1", &Value::U64(2)),"length":literal("core/u64@1", &Value::U64(5))}),
            ),
        ],
        vec![
            edge(1, 1, "result", 2, "items"),
            edge(2, 2, "result", 3, "items"),
            edge(3, 4, "result", 5, "map"),
            edge(4, 2, "result", 6, "items"),
        ],
        vec![
            json!({"id":"first","node":id(3),"port":"result"}),
            json!({"id":"keys","node":id(5),"port":"result"}),
            json!({"id":"fault","node":id(6),"port":"result"}),
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
    assert_eq!(constant(0), FoldedValue::Value(Value::I64(3)));
    assert_eq!(constant(1), FoldedValue::Value(strings(&["a", "b"])));
    let FoldedValue::Fault(fault) = constant(2) else {
        panic!("an out-of-range slice must fault")
    };
    assert_eq!(fault.fault.code, "core/OutOfRange");
    assert_eq!(fault.source, id(6).parse().unwrap());
}
