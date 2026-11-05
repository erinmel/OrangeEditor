//! Known answers for the bundled `eq`/`ord` comparison kernels (11 §5.4), the shapes
//! they refuse, and their lowering through data and control plans.

use std::cmp::Ordering;
use std::sync::Arc;

use orange_bundled::{
    BindingError, Comparison, KernelError, MathPackage, RANDOM_U64_DEFINITION, ResolvedComparison,
    comparison_definitions, random_u64_definition,
};
use orange_compiler::{CompileError, CompileOptions, FoldedValue, compile_control, compile_pure};
use orange_model::catalog::{Ports, RegistryBuilder, RegistrySnapshot};
use orange_model::document::{DefinitionRef, ProviderBinding};
use orange_model::type_term::TypeTerm;
use orange_model::types::{
    Constraint, ConstraintId, ConstraintImplementationRef, TypeRegistryBuilder,
};
use orange_model::validation::{DiagnosticCode, HostOrigin};
use orange_model::value::literal_codec::{EnumSchema, FieldSchema, RecordSchema, VariantSchema};
use orange_model::value::{ListRepr, MapRepr, RecordRepr};
use orange_model::{GraphDocument, Value, analyze, from_json};
use serde_json::{Value as Json, json};

const LIGHT: &str = "org.example/light@1";
const PALETTE: &str = "org.example/palette@1";
const OPAQUE: &str = "org.example/opaque@1";
const WRAPPER: &str = "org.example/wrapper@1";
const POINT: &str = "org.example/point@1";
const SEGMENT: &str = "org.example/segment@1";

fn term(ty: &str) -> TypeTerm {
    TypeTerm::parse(&ty.parse().unwrap()).unwrap()
}

/// The math package plus package types:
/// - `light` (red, amber(u32), green) deriving `eq` and `hash`, and `palette` (empty,
///   lights(list<light>), named(map<string, light>)) deriving `eq`;
/// - `opaque` (one, two) with a package-implemented `eq`, and `wrapper(opaque)` deriving it;
/// - records `point {x, y: i64}` and `segment {from, to: point}` deriving `eq`.
fn catalog() -> (MathPackage, Arc<RegistrySnapshot>) {
    let mut types = TypeRegistryBuilder::core();
    let variant = |id: &str, payload: Option<&str>| VariantSchema {
        variant_id: id.into(),
        payload: payload.map(term),
    };
    let enums = [
        (
            LIGHT,
            vec![
                variant("red", None),
                variant("amber", Some("core/u32@1")),
                variant("green", None),
            ],
        ),
        (
            PALETTE,
            vec![
                variant("empty", None),
                variant("lights", Some("core/list@1<org.example/light@1>")),
                variant(
                    "named",
                    Some("core/map@1<core/string@1,org.example/light@1>"),
                ),
            ],
        ),
        (OPAQUE, vec![variant("one", None), variant("two", None)]),
        (WRAPPER, vec![variant("wrap", Some(OPAQUE))]),
    ];
    for (ty, variants) in enums {
        types
            .register_enum("org.example", &ty.parse().unwrap(), EnumSchema { variants })
            .unwrap();
    }
    let field = |id: &str, ty: &str| FieldSchema {
        field_id: id.into(),
        ty: term(ty),
        optional: false,
    };
    let records = [
        (
            POINT,
            vec![field("x", "core/i64@1"), field("y", "core/i64@1")],
        ),
        (SEGMENT, vec![field("from", POINT), field("to", POINT)]),
    ];
    for (ty, fields) in records {
        types
            .register_record("org.example", &ty.parse().unwrap(), RecordSchema { fields })
            .unwrap();
    }
    types
        .register_constraint_implementation(
            "org.example",
            &term(OPAQUE),
            ConstraintId::Core(Constraint::Eq),
            ConstraintImplementationRef {
                provider: ProviderBinding {
                    package: "org.example".into(),
                    version: "1.0.0".into(),
                    id: "types".into(),
                    extensions: Default::default(),
                },
                content_digest: format!("sha256:{}", "b".repeat(64)),
                implementation_id: "opaque-eq".into(),
            },
        )
        .unwrap();
    for ty in [LIGHT, PALETTE, WRAPPER, POINT, SEGMENT] {
        types
            .derive_constraint("org.example", &ty.parse().unwrap(), Constraint::Eq)
            .unwrap();
    }
    types
        .derive_constraint("org.example", &LIGHT.parse().unwrap(), Constraint::Hash)
        .unwrap();
    let digest = format!("sha256:{}", "a".repeat(64));
    let package = MathPackage::new(digest.clone()).unwrap();
    let builder = RegistryBuilder::core(HostOrigin::new("0.0.0", digest).unwrap())
        .types(Arc::new(types.build().unwrap()));
    let snapshot = package.register(builder).unwrap().build().unwrap();
    (package, snapshot)
}

fn bind(comparison: Comparison, ty: &str) -> Result<ResolvedComparison, BindingError> {
    let (package, snapshot) = catalog();
    package.resolve_comparison(
        snapshot,
        package.provider_key(),
        &DefinitionRef {
            id: comparison.definition_id().parse().unwrap(),
            version: 1,
        },
        1,
        &ty.parse().unwrap(),
        &Default::default(),
    )
}

fn eval(comparison: Comparison, ty: &str, a: &Value, b: &Value) -> Result<bool, KernelError> {
    bind(comparison, ty).unwrap().evaluate(a, b)
}

/// Every comparison agrees with the expected total order of `a` and `b`.
fn assert_order(ty: &str, a: Value, b: Value, order: Ordering) {
    for comparison in Comparison::ALL {
        let expected = match comparison {
            Comparison::Equal => order.is_eq(),
            Comparison::NotEqual => order.is_ne(),
            Comparison::Less => order.is_lt(),
            Comparison::LessOrEqual => order.is_le(),
            Comparison::Greater => order.is_gt(),
            Comparison::GreaterOrEqual => order.is_ge(),
        };
        assert_eq!(
            eval(comparison, ty, &a, &b),
            Ok(expected),
            "{comparison:?} {ty} {a:?} {b:?}"
        );
    }
}

/// Equal/NotEqual give `equal`; the ordering comparisons refuse the type.
fn assert_equality(ty: &str, a: Value, b: Value, equal: bool) {
    assert_eq!(
        eval(Comparison::Equal, ty, &a, &b),
        Ok(equal),
        "{ty} {a:?} {b:?}"
    );
    assert_eq!(eval(Comparison::NotEqual, ty, &a, &b), Ok(!equal));
    assert_eq!(eval(Comparison::Equal, ty, &b, &a), Ok(equal), "symmetry");
    for comparison in [Comparison::Less, Comparison::GreaterOrEqual] {
        assert_eq!(
            bind(comparison, ty).unwrap_err(),
            BindingError::UnsupportedType
        );
    }
}

fn s(text: &str) -> Value {
    Value::Str(Arc::new(text.into()))
}
fn list(values: Vec<Value>) -> Value {
    Value::List(Arc::new(ListRepr::Values(values.into())))
}
fn map(entries: Vec<(Value, Value)>) -> Value {
    Value::Map(Arc::new(MapRepr::new(entries).unwrap()))
}
fn variant(tag: u32, payload: Option<Value>) -> Value {
    Value::Enum {
        tag,
        payload: payload.map(Arc::new),
    }
}
fn amber(n: u32) -> Value {
    variant(1, Some(Value::U32(n)))
}

#[test]
fn core_total_orders_follow_their_scalar_rules() {
    use Ordering::*;
    let cases = [
        (
            "core/i32@1",
            Value::I32(i32::MIN),
            Value::I32(i32::MAX),
            Less,
        ),
        ("core/i32@1", Value::I32(5), Value::I32(-5), Greater),
        (
            "core/i64@1",
            Value::I64(i64::MIN),
            Value::I64(i64::MAX),
            Less,
        ),
        ("core/i64@1", Value::I64(-1), Value::I64(-1), Equal),
        ("core/u32@1", Value::U32(u32::MAX), Value::U32(0), Greater),
        // Unsigned order never reinterprets the high bit as a sign.
        (
            "core/u64@1",
            Value::U64(u64::MAX),
            Value::U64(1 << 63),
            Greater,
        ),
        ("core/u64@1", Value::U64(0), Value::U64(u64::MAX), Less),
        ("core/bool@1", Value::Bool(false), Value::Bool(true), Less),
        ("core/bool@1", Value::Bool(true), Value::Bool(true), Equal),
        (
            "core/duration@1",
            Value::Duration(-1),
            Value::Duration(0),
            Less,
        ),
        (
            "core/timestamp@1",
            Value::Timestamp(i64::MIN),
            Value::Timestamp(0),
            Less,
        ),
        ("core/string@1", s(""), s("a"), Less),
        ("core/string@1", s("ab"), s("abc"), Less),
        ("core/string@1", s("Z"), s("a"), Less),
        ("core/string@1", s("z"), s("\u{e9}"), Less),
        // Code point order, not UTF-16 order (which puts U+10000 before U+FFFF).
        ("core/string@1", s("\u{ffff}"), s("\u{10000}"), Less),
        // No normalization: precomposed é is not decomposed e + U+0301.
        ("core/string@1", s("\u{e9}"), s("e\u{301}"), Greater),
        ("core/string@1", s("\u{1f34a}"), s("\u{1f34a}"), Equal),
    ];
    for (ty, a, b, order) in cases {
        assert_order(ty, a.clone(), b.clone(), order);
        assert_order(ty, b, a, order.reverse());
    }
}

#[test]
fn core_equality_covers_bytes_lists_maps_and_nested_options() {
    let bytes = |b: &[u8]| Value::Bytes(Arc::new(b.to_vec()));
    assert_equality("core/bytes@1", bytes(&[1, 2]), bytes(&[1, 2]), true);
    assert_equality("core/bytes@1", bytes(&[]), bytes(&[0]), false);

    let ints = "core/list@1<core/i64@1>";
    let packed = Value::List(Arc::new(ListRepr::I64(Box::new([1, 2]))));
    assert_equality(
        ints,
        packed.clone(),
        list(vec![Value::I64(1), Value::I64(2)]),
        true,
    );
    assert_equality(ints, packed.clone(), list(vec![Value::I64(1)]), false);
    assert_equality(
        ints,
        packed,
        list(vec![Value::I64(1), Value::I64(3)]),
        false,
    );

    // Map equality ignores insertion order, for small and indexed large maps.
    let ty = "core/map@1<core/string@1,core/i64@1>";
    for n in [3_i64, 40] {
        let entries: Vec<_> = (0..n).map(|i| (s(&i.to_string()), Value::I64(i))).collect();
        let mut reversed = entries.clone();
        reversed.reverse();
        assert_equality(ty, map(entries.clone()), map(reversed.clone()), true);
        let mut changed = reversed.clone();
        changed[0].1 = Value::I64(-1);
        assert_equality(ty, map(entries.clone()), map(changed), false);
        let mut renamed = reversed;
        renamed[0].0 = s("other");
        assert_equality(ty, map(entries.clone()), map(renamed), false);
        assert_equality(ty, map(entries[1..].to_vec()), map(entries), false);
    }

    // option<option<i64>>: present values box, so Some(None) differs from None.
    let ty = "core/option@1<core/option@1<core/i64@1>>";
    let some = |v: Value| Value::Some(Arc::new(v));
    assert_equality(ty, Value::None, some(Value::None), false);
    assert_equality(ty, some(Value::None), some(Value::None), true);
    assert_equality(ty, some(Value::I64(3)), some(Value::I64(3)), true);
    assert_equality(ty, some(Value::I64(3)), some(Value::None), false);
    assert_equality(ty, Value::None, Value::None, true);
}

#[test]
fn derived_enum_equality_compares_tags_then_payloads() {
    let red = variant(0, None);
    let green = variant(2, None);
    assert_equality(LIGHT, red.clone(), red.clone(), true);
    assert_equality(LIGHT, amber(7), amber(7), true);
    assert_equality(LIGHT, amber(7), amber(9), false);
    assert_equality(LIGHT, red.clone(), green.clone(), false);
    assert_equality(LIGHT, red.clone(), amber(0), false);

    // Nested derived enums inside lists and order-independent maps.
    let lights = |items: Vec<Value>| variant(1, Some(list(items)));
    assert_equality(
        PALETTE,
        lights(vec![red.clone(), amber(1)]),
        lights(vec![red.clone(), amber(1)]),
        true,
    );
    assert_equality(
        PALETTE,
        lights(vec![red.clone(), amber(1)]),
        lights(vec![red.clone(), amber(2)]),
        false,
    );
    assert_equality(PALETTE, lights(vec![]), variant(0, None), false);
    let named = |entries: Vec<(&str, Value)>| {
        variant(
            2,
            Some(map(entries.into_iter().map(|(k, v)| (s(k), v)).collect())),
        )
    };
    assert_equality(
        PALETTE,
        named(vec![("go", green.clone()), ("stop", red.clone())]),
        named(vec![("stop", red.clone()), ("go", green.clone())]),
        true,
    );
    assert_equality(
        PALETTE,
        named(vec![("go", green), ("stop", red.clone())]),
        named(vec![("stop", red.clone()), ("go", amber(3))]),
        false,
    );
    // A list of derived enums is itself comparable.
    let ty = "core/list@1<org.example/light@1>";
    assert_equality(ty, list(vec![amber(4)]), list(vec![amber(4)]), true);

    // Derived `eq` + `hash` enums key maps; equality still ignores insertion order and
    // tells keys apart by payload.
    let ty = "core/map@1<org.example/light@1,core/i64@1>";
    let keyed = |second: Value| map(vec![(red.clone(), Value::I64(0)), (second, Value::I64(1))]);
    assert_equality(
        ty,
        keyed(amber(1)),
        map(vec![
            (amber(1), Value::I64(1)),
            (red.clone(), Value::I64(0)),
        ]),
        true,
    );
    assert_equality(ty, keyed(amber(1)), keyed(amber(2)), false);
    // `palette` derives no `hash`, and `opaque`'s `eq` is package code.
    for ty in [
        "core/map@1<org.example/palette@1,core/i64@1>",
        "core/map@1<org.example/opaque@1,core/i64@1>",
    ] {
        assert_eq!(
            bind(Comparison::Equal, ty).unwrap_err(),
            BindingError::UnsupportedType,
            "{ty}"
        );
    }
}

#[test]
fn shapes_without_host_semantics_are_refused_at_binding() {
    for (comparison, ty) in [
        // Floats have no Core eq/ord; IEEE `==` and total-order nodes are separate.
        (Comparison::Equal, "core/f64@1"),
        (Comparison::Less, "core/f32@1"),
        (Comparison::Equal, "core/option@1<core/f64@1>"),
        // Core eq is declared for these, but no executable equality is defined yet.
        (Comparison::Equal, "core/json@1"),
        (Comparison::Equal, "core/error@1"),
        (Comparison::Equal, "core/result@1<core/i64@1>"),
        (Comparison::Equal, "core/list@1<core/json@1>"),
        // Package code owns these equalities, also when nested in a derive.
        (Comparison::Equal, OPAQUE),
        (Comparison::NotEqual, WRAPPER),
        // No enum order is ever derived.
        (Comparison::Less, LIGHT),
        // Records have no executable shape.
        (Comparison::Equal, POINT),
        (Comparison::Equal, SEGMENT),
        // Ord is scalar only.
        (Comparison::Less, "core/bytes@1"),
        (Comparison::Greater, "core/list@1<core/i64@1>"),
    ] {
        assert_eq!(
            bind(comparison, ty).unwrap_err(),
            BindingError::UnsupportedType,
            "{comparison:?} {ty}"
        );
    }
    let (package, snapshot) = catalog();
    let equal = DefinitionRef {
        id: Comparison::Equal.definition_id().parse().unwrap(),
        version: 1,
    };
    let ty = "core/i64@1".parse().unwrap();
    let config = json!({"float_policy":"ieee"});
    assert_eq!(
        package
            .resolve_comparison(
                snapshot.clone(),
                package.provider_key(),
                &equal,
                1,
                &ty,
                config.as_object().unwrap(),
            )
            .unwrap_err(),
        BindingError::InvalidConfig
    );
    assert_eq!(
        package
            .resolve_comparison(
                snapshot,
                package.provider_key(),
                &equal,
                2,
                &ty,
                &Default::default(),
            )
            .unwrap_err(),
        BindingError::Unavailable
    );
}

#[test]
fn values_outside_the_bound_shape_are_contract_errors() {
    let contract = Err(KernelError::InputContract);
    assert_eq!(
        eval(
            Comparison::Equal,
            "core/i64@1",
            &Value::I32(1),
            &Value::I32(1)
        ),
        contract
    );
    assert_eq!(
        eval(
            Comparison::Less,
            "core/i64@1",
            &Value::I64(1),
            &Value::U64(1)
        ),
        contract
    );
    // An unknown tag, a missing payload or a payload of a payload-free variant.
    for bad in [
        variant(9, None),
        variant(1, None),
        variant(0, Some(Value::U32(1))),
    ] {
        assert_eq!(
            eval(Comparison::Equal, LIGHT, &bad, &variant(2, None)),
            contract
        );
    }
    let ty = "core/map@1<core/string@1,core/i64@1>";
    let wrong_key = map(vec![(Value::I64(1), Value::I64(1))]);
    assert_eq!(
        eval(Comparison::Equal, ty, &wrong_key, &wrong_key),
        contract
    );
}

/// Equal and NotEqual refuse `bad` in either position, even where a comparison could
/// stop early against `other`.
fn assert_refused_both_ways(ty: &str, bad: &Value, other: &Value) {
    for comparison in [Comparison::Equal, Comparison::NotEqual] {
        for (a, b) in [(bad, other), (other, bad)] {
            assert_eq!(
                eval(comparison, ty, a, b),
                Err(KernelError::InputContract),
                "{comparison:?} {ty} {a:?} {b:?}"
            );
        }
    }
}

#[test]
fn equality_validates_both_values_before_any_shortcut() {
    let strings = "core/list@1<core/string@1>";
    // Different lengths and an earlier unequal element both hide a bad element.
    assert_refused_both_ways(strings, &list(vec![s("a"), Value::I64(1)]), &list(vec![]));
    assert_refused_both_ways(
        strings,
        &list(vec![s("a"), Value::I64(1)]),
        &list(vec![s("b"), s("c")]),
    );
    assert_refused_both_ways(strings, &Value::I64(1), &list(vec![]));

    // None against a present value that is itself wrongly typed.
    let option = "core/option@1<core/string@1>";
    assert_refused_both_ways(option, &Value::Some(Arc::new(Value::I64(1))), &Value::None);
    let nested = "core/option@1<core/option@1<core/i64@1>>";
    assert_refused_both_ways(nested, &Value::Some(Arc::new(s("x"))), &Value::None);

    // Maps of different sizes, absent keys or unequal earlier values hide bad entries.
    let ty = "core/map@1<core/string@1,core/i64@1>";
    let bad_value = map(vec![(s("a"), s("x"))]);
    let bad_key = map(vec![(Value::I64(1), Value::I64(1))]);
    let valid = map(vec![(s("a"), Value::I64(1)), (s("b"), Value::I64(2))]);
    for bad in [&bad_value, &bad_key] {
        assert_refused_both_ways(ty, bad, &valid);
        assert_refused_both_ways(ty, bad, &map(vec![]));
        assert_refused_both_ways(ty, bad, &map(vec![(s("z"), Value::I64(1))]));
    }
    assert_refused_both_ways(
        ty,
        &map(vec![(s("a"), Value::I64(5)), (s("q"), s("x"))]),
        &map(vec![(s("a"), Value::I64(6)), (s("r"), Value::I64(1))]),
    );

    // Enums with different tags, and a bad payload inside a differing variant.
    let bad_payload = variant(1, Some(Value::I64(1)));
    assert_refused_both_ways(LIGHT, &bad_payload, &variant(0, None));
    assert_refused_both_ways(LIGHT, &bad_payload, &amber(1));
    let lights = variant(1, Some(list(vec![variant(0, None), variant(9, None)])));
    assert_refused_both_ways(PALETTE, &lights, &variant(0, None));
}

#[test]
fn ordering_refuses_wrongly_typed_inputs_in_either_position() {
    for comparison in [Comparison::Less, Comparison::GreaterOrEqual] {
        for (a, b) in [
            (Value::I64(1), Value::U64(1)),
            (Value::U64(1), Value::I64(1)),
            (Value::I64(1), Value::None),
        ] {
            assert_eq!(
                eval(comparison, "core/i64@1", &a, &b),
                Err(KernelError::InputContract)
            );
        }
    }
}

// Graph-level helpers.

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn edge(n: u32, source: u32, output: &str, target: u32, input: &str) -> Json {
    json!({"id":format!("30000000-0000-4000-8000-{n:012x}"),"from":{"node":id(source),"port":output},
        "to":{"node":id(target),"port":input}})
}
fn provider() -> Json {
    json!({"package":"org.orange.math","version":"0.0.0","id":"native-v1"})
}
fn literal(snapshot: &RegistrySnapshot, ty: &str, value: &Value) -> Json {
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

/// A comparison node over `ty` with optional literal inputs; its signature is `s{n}`.
struct Compare<'a> {
    n: u32,
    comparison: Comparison,
    ty: &'a str,
    a: Option<Value>,
    b: Option<Value>,
}

fn graph(
    package: &MathPackage,
    snapshot: &RegistrySnapshot,
    mut nodes: Vec<Json>,
    compares: &[Compare<'_>],
    edges: Vec<Json>,
    exports: Vec<Json>,
) -> GraphDocument {
    let mut signatures = serde_json::Map::new();
    let definitions = comparison_definitions().unwrap();
    for compare in compares {
        let definition = definitions
            .iter()
            .find(|d| d.definition.id.as_str() == compare.comparison.definition_id())
            .unwrap();
        let Ports::Declared(ports) = definition.ports.clone() else {
            unreachable!()
        };
        let args = [("T".into(), compare.ty.parse().unwrap())].into();
        signatures.insert(
            format!("s{}", compare.n),
            serde_json::to_value(orange_model::signature::project(
                definition.kind,
                ports,
                &args,
            ))
            .unwrap(),
        );
        let mut inputs = serde_json::Map::new();
        for (port, value) in [("a", &compare.a), ("b", &compare.b)] {
            if let Some(value) = value {
                inputs.insert(port.into(), literal(snapshot, compare.ty, value));
            }
        }
        nodes.push(
            json!({"id":id(compare.n),"definition_id":compare.comparison.definition_id(),
            "definition_version":1,"config_version":1,"config":{},"type_pins":{"T":compare.ty},
            "signature":format!("s{}", compare.n),"provider":provider(),"inputs":inputs}),
        );
    }
    let random = random_u64_definition().unwrap();
    let Ports::Declared(ports) = random.ports else {
        unreachable!()
    };
    signatures.insert(
        "random".into(),
        serde_json::to_value(orange_model::signature::project(
            random.kind,
            ports,
            &Default::default(),
        ))
        .unwrap(),
    );
    let key = package.provider_key();
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,"package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "signatures":signatures,"nodes":nodes,"edges":edges,"exports":exports}).to_string()).unwrap()
}

fn export(n: u32) -> Json {
    json!({"id":format!("out-{n}"),"node":id(n),"port":"result"})
}

fn compile_exports(compares: &[Compare<'_>]) -> Result<Vec<FoldedValue>, CompileError> {
    let (package, snapshot) = catalog();
    let exports = compares.iter().map(|c| export(c.n)).collect();
    let doc = graph(&package, &snapshot, vec![], compares, vec![], exports);
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let plan = compile_pure(
        &doc,
        snapshot,
        &analysis,
        &package,
        CompileOptions::default(),
    )?;
    Ok(plan
        .exports()
        .iter()
        .map(|export| plan.constants()[export.slot.index()].clone())
        .collect())
}

#[test]
fn literal_comparisons_fold_through_the_pure_compiler() {
    let compare = |n, comparison, ty, a, b| Compare {
        n,
        comparison,
        ty,
        a: Some(a),
        b: Some(b),
    };
    let folded = compile_exports(&[
        compare(
            1,
            Comparison::Less,
            "core/i64@1",
            Value::I64(-3),
            Value::I64(2),
        ),
        compare(
            2,
            Comparison::GreaterOrEqual,
            "core/string@1",
            s("a"),
            s("b"),
        ),
        compare(3, Comparison::Equal, LIGHT, amber(5), amber(5)),
        compare(4, Comparison::NotEqual, LIGHT, amber(5), variant(0, None)),
        compare(
            5,
            Comparison::Equal,
            "core/map@1<core/string@1,core/i64@1>",
            map(vec![(s("x"), Value::I64(1)), (s("y"), Value::I64(2))]),
            map(vec![(s("y"), Value::I64(2)), (s("x"), Value::I64(1))]),
        ),
    ])
    .unwrap();
    assert_eq!(
        folded,
        [true, false, true, true, true].map(|b| FoldedValue::Value(Value::Bool(b)))
    );
}

#[test]
fn constraint_proofs_without_a_host_kernel_fail_compilation_with_a_typed_reason() {
    let (package, snapshot) = catalog();
    // Validation accepts these (Core eq / package-implemented eq), compilation refuses.
    for (ty, value) in [
        ("core/list@1<core/json@1>", list(vec![])),
        (OPAQUE, variant(0, None)),
    ] {
        let compare = Compare {
            n: 1,
            comparison: Comparison::Equal,
            ty,
            a: Some(value.clone()),
            b: Some(value),
        };
        let doc = graph(
            &package,
            &snapshot,
            vec![],
            &[compare],
            vec![],
            vec![export(1)],
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis.is_ready_for_lowering(&doc, &snapshot),
            "{ty}: {:?}",
            analysis.report().diagnostics
        );
        assert_eq!(
            compile_pure(
                &doc,
                snapshot.clone(),
                &analysis,
                &package,
                CompileOptions::default()
            )
            .unwrap_err(),
            CompileError::KernelUnsupportedType {
                node: id(1).parse().unwrap()
            },
            "{ty}"
        );
    }
    // A nested record satisfies derived eq, but records have no executable port shape.
    let point = |x, y| {
        Value::Record(Arc::new(RecordRepr {
            ty: POINT.parse().unwrap(),
            fields: Box::new([Some(Value::I64(x)), Some(Value::I64(y))]),
        }))
    };
    let segment = Value::Record(Arc::new(RecordRepr {
        ty: SEGMENT.parse().unwrap(),
        fields: Box::new([Some(point(0, 0)), Some(point(1, 2))]),
    }));
    let compare = Compare {
        n: 1,
        comparison: Comparison::Equal,
        ty: SEGMENT,
        a: Some(segment.clone()),
        b: Some(segment),
    };
    let doc = graph(
        &package,
        &snapshot,
        vec![],
        &[compare],
        vec![],
        vec![export(1)],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot,
            &analysis,
            &package,
            CompileOptions::default()
        ),
        Err(CompileError::UnsupportedPort { .. })
    ));
}

#[test]
fn float_comparisons_including_nan_are_rejected_by_validation() {
    let (package, snapshot) = catalog();
    for (comparison, ty, value) in [
        (Comparison::Equal, "core/f64@1", Value::F64(f64::NAN)),
        (Comparison::Less, "core/f64@1", Value::F64(f64::NAN)),
        (Comparison::Equal, "core/f32@1", Value::F32(f32::NAN)),
        (
            Comparison::Equal,
            "core/option@1<core/f64@1>",
            Value::F64(f64::NAN),
        ),
    ] {
        let compare = Compare {
            n: 1,
            comparison,
            ty,
            a: Some(value.clone()),
            b: Some(value),
        };
        let doc = graph(
            &package,
            &snapshot,
            vec![],
            &[compare],
            vec![],
            vec![export(1)],
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(!analysis.is_ready_for_lowering(&doc, &snapshot), "{ty}");
        assert!(
            analysis
                .report()
                .diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::ConstraintUnsatisfied),
            "{ty}: {:?}",
            analysis.report().diagnostics
        );
    }
}

#[test]
fn runtime_comparisons_drive_branches_through_a_run() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    let (package, snapshot) = catalog();
    let core = |n: u32, name: &str| {
        let mut node = json!({"id":id(n),"definition_id":format!("core/{name}"),
            "definition_version":1,"config_version":1,"config":{}});
        if name == "on-start" {
            node["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest",
                "on_fault":"continue","order":0});
        }
        node
    };
    let random = json!({"id":id(9),"definition_id":RANDOM_U64_DEFINITION,"definition_version":1,
        "config_version":1,"config":{},"signature":"random","provider":provider()});
    let u64 = "core/u64@1";
    // on-start -> b2 -> b3 -> b4 -> b5; each branch's condition is one comparison of the
    // same seeded sample (n9), so every outcome is known for any seed.
    let compares = [
        Compare {
            n: 12,
            comparison: Comparison::GreaterOrEqual,
            ty: u64,
            a: None,
            b: Some(Value::U64(0)),
        },
        Compare {
            n: 13,
            comparison: Comparison::Less,
            ty: u64,
            a: None,
            b: Some(Value::U64(0)),
        },
        // The sample equals itself within one epoch.
        Compare {
            n: 14,
            comparison: Comparison::Equal,
            ty: u64,
            a: None,
            b: None,
        },
        Compare {
            n: 15,
            comparison: Comparison::NotEqual,
            ty: LIGHT,
            a: Some(amber(1)),
            b: Some(amber(2)),
        },
    ];
    let mut nodes = vec![core(1, "on-start"), random];
    let mut edges = vec![edge(1, 1, "then", 2, "in")];
    for n in 2..=5 {
        nodes.push(core(n, "branch"));
        edges.push(edge(20 + n, 10 + n, "result", n, "condition"));
        if n < 5 {
            edges.push(edge(30 + n, n, "true", n + 1, "in"));
            edges.push(edge(40 + n, n, "false", n + 1, "in"));
        }
    }
    edges.push(edge(50, 9, "value", 12, "a"));
    edges.push(edge(51, 9, "value", 13, "a"));
    edges.push(edge(52, 9, "value", 14, "a"));
    edges.push(edge(53, 9, "value", 14, "b"));
    let doc = graph(&package, &snapshot, nodes, &compares, edges, vec![]);
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let plan = Arc::new(
        compile_control(
            &doc,
            snapshot.clone(),
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    for seed in [1, 42, u64::MAX] {
        let mut run =
            Run::start(plan.clone(), &doc, &snapshot, seed, RunOptions::default()).unwrap();
        while run.pump(64).unwrap().ready {}
        let outcomes: Vec<_> = (2..=5)
            .map(|n| {
                run.trace()
                    .iter()
                    .find(|e| e.node.to_string() == id(n))
                    .map(|e| e.outcome)
            })
            .collect();
        let (t, f) = (Some(ControlOutcome::True), Some(ControlOutcome::False));
        assert_eq!(outcomes, [t, f, t, t], "seed {seed}");
        run.stop().unwrap();
        run.dispose().unwrap();
    }
}

/// `depth` list wrappers around one i64 leaf: the leaf sits at nesting depth `depth`.
fn nested(depth: usize, leaf: i64) -> (String, Value) {
    let mut ty = "core/i64@1".to_owned();
    let mut value = Value::I64(leaf);
    for _ in 0..depth {
        ty = format!("core/list@1<{ty}>");
        value = list(vec![value]);
    }
    (ty, value)
}

#[test]
fn nesting_at_the_limit_compares_and_one_past_is_refused() {
    use orange_model::type_term::MAX_NESTING;
    let (ty, a) = nested(MAX_NESTING, 7);
    let (_, same) = nested(MAX_NESTING, 7);
    let (_, other) = nested(MAX_NESTING, 8);
    assert_equality(&ty, a.clone(), same, true);
    assert_equality(&ty, a, other, false);
    // Ordering is scalar only, at any depth.
    assert_eq!(
        bind(Comparison::Less, &ty).unwrap_err(),
        BindingError::UnsupportedType
    );
    // A value deeper than its bound shape is a contract error, not a deeper walk.
    let (_, deeper) = nested(MAX_NESTING + 1, 7);
    assert_eq!(
        eval(Comparison::Equal, &ty, &deeper, &deeper),
        Err(KernelError::InputContract)
    );

    let (ty, _) = nested(MAX_NESTING + 1, 7);
    for comparison in [Comparison::Equal, Comparison::NotEqual, Comparison::Less] {
        assert_eq!(
            bind(comparison, &ty).unwrap_err(),
            BindingError::UnsupportedType,
            "{comparison:?}"
        );
    }
}
