//! L02a known answers, widened arithmetic identities and real compiled demands.
use std::sync::Arc;

use orange_bundled::{
    BindingError, IntegerOperation as Op, KernelError, MathPackage, ResolvedInteger,
    integer_definitions,
};
use orange_compiler::{CompileOptions, FoldedValue, compile_control, compile_pure};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::DefinitionRef;
use orange_model::{GraphDocument, Value, analyze, from_json};
use orange_runtime::{Run, RunOptions};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

use crate::data_graph::{catalog, edge, id, provider};

fn bind_in(
    package: &MathPackage,
    snapshot: Arc<RegistrySnapshot>,
    op: Op,
    ty: &str,
    config: Json,
) -> Result<ResolvedInteger, BindingError> {
    package.resolve_integer(
        snapshot,
        package.provider_key(),
        &DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        },
        1,
        &ty.parse().unwrap(),
        &serde_json::from_value(config).unwrap(),
    )
}
fn bind(op: Op, ty: &str, config: Json) -> ResolvedInteger {
    let (package, snapshot) = catalog();
    bind_in(&package, snapshot, op, ty, config).unwrap()
}

fn answer(op: Op) -> (Vec<i64>, i64) {
    match op {
        Op::Remainder => (vec![17, 5], 2),
        Op::Modulo => (vec![17, 5], 2),
        Op::Negate => (vec![0], 0),
        Op::Absolute => (vec![7], 7),
        Op::Minimum => (vec![17, 5], 5),
        Op::Maximum => (vec![17, 5], 17),
        Op::Clamp => (vec![17, 5, 10], 10),
        Op::BitAnd => (vec![0b1100, 0b1010], 0b1000),
        Op::BitOr => (vec![0b1100, 0b1010], 0b1110),
        Op::BitXor => (vec![0b1100, 0b1010], 0b0110),
        Op::BitNot => (vec![0], -1),
        Op::ShiftLeft => (vec![3, 2], 12),
        Op::ShiftRight => (vec![17, 2], 4),
    }
}

#[test]
fn all_thirteen_operations_have_known_answers_on_all_four_widths() {
    macro_rules! check {
        ($variant:ident, $primitive:ty, $ty:literal) => {
            for op in Op::ALL {
                let (values, expected) = answer(op);
                let inputs: Vec<_> = values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        if op.input_ports()[i] == "count" {
                            Value::U32(*v as u32)
                        } else {
                            Value::$variant(*v as $primitive)
                        }
                    })
                    .collect();
                assert_eq!(
                    bind(op, $ty, json!({})).evaluate(&inputs),
                    Ok(Value::$variant(expected as $primitive)),
                    "{op:?} {}",
                    $ty
                );
            }
        };
    }
    check!(I32, i32, "core/i32@1");
    check!(I64, i64, "core/i64@1");
    check!(U32, u32, "core/u32@1");
    check!(U64, u64, "core/u64@1");
}

#[test]
fn negative_remainder_and_modulo_and_min_divisor_are_mathematical() {
    for (a, b, rem, modulo) in [
        (-17, 5, -2, 3),
        (-17, -5, -2, 3),
        (17, -5, 2, 2),
        (i64::MIN, -1, 0, 0),
        (-1, i64::MIN, -1, i64::MAX),
        (i64::MIN, i64::MIN, 0, 0),
    ] {
        for (op, expected) in [(Op::Remainder, rem), (Op::Modulo, modulo)] {
            assert_eq!(
                bind(op, "core/i64@1", json!({})).evaluate(&[Value::I64(a), Value::I64(b)]),
                Ok(Value::I64(expected))
            );
        }
    }
    for op in [Op::Remainder, Op::Modulo] {
        assert_eq!(
            bind(op, "core/i32@1", json!({})).evaluate(&[Value::I32(i32::MIN), Value::I32(-1)]),
            Ok(Value::I32(0))
        );
        for ty in ["core/i32@1", "core/i64@1", "core/u32@1", "core/u64@1"] {
            let zero = match ty {
                "core/i32@1" => Value::I32(0),
                "core/i64@1" => Value::I64(0),
                "core/u32@1" => Value::U32(0),
                _ => Value::U64(0),
            };
            assert_eq!(
                bind(op, ty, json!({})).evaluate(&[zero.clone(), zero]),
                Err(KernelError::DivideByZero)
            );
        }
    }
}

#[test]
fn negate_and_absolute_overflow_policies_cover_signed_min_and_unsigned_negation() {
    macro_rules! signed {
        ($variant:ident, $primitive:ty, $ty:literal) => {
            for op in [Op::Negate, Op::Absolute] {
                for (mode, expected) in [("checked", Err(KernelError::NumericOverflow)),
                    ("wrapping", Ok(Value::$variant(<$primitive>::MIN))), ("saturating", Ok(Value::$variant(<$primitive>::MAX)))] {
                    assert_eq!(bind(op, $ty, json!({"int_overflow":mode})).evaluate(&[Value::$variant(<$primitive>::MIN)]), expected);
                }
                assert_eq!(bind(op, $ty, json!({})).evaluate(&[Value::$variant(-7)]), Ok(Value::$variant(7)));
            }
        };
    }
    signed!(I32, i32, "core/i32@1");
    signed!(I64, i64, "core/i64@1");
    macro_rules! unsigned {
        ($variant:ident, $primitive:ty, $ty:literal) => {
            for (mode, expected) in [("checked", Err(KernelError::NumericOverflow)), ("wrapping", Ok(Value::$variant(<$primitive>::MAX))), ("saturating", Ok(Value::$variant(0)))] {
                assert_eq!(bind(Op::Negate, $ty, json!({"int_overflow":mode})).evaluate(&[Value::$variant(1)]), expected);
                assert_eq!(bind(Op::Absolute, $ty, json!({"int_overflow":mode})).evaluate(&[Value::$variant(<$primitive>::MAX)]), Ok(Value::$variant(<$primitive>::MAX)));
            }
        };
    }
    unsigned!(U32, u32, "core/u32@1");
    unsigned!(U64, u64, "core/u64@1");
}

#[test]
fn checked_shift_counts_and_twos_complement_bits_are_explicit() {
    macro_rules! check {
        ($variant:ident, $primitive:ty, $ty:literal) => {
            for op in [Op::ShiftLeft, Op::ShiftRight] {
                let checked = bind(op, $ty, json!({}));
                let wrapping = bind(op, $ty, json!({"int_overflow":"wrapping"}));
                for count in [<$primitive>::BITS, <$primitive>::BITS+1, u32::MAX] {
                    assert_eq!(checked.evaluate(&[Value::$variant(1), Value::U32(count)]), Err(KernelError::OutOfRange));
                    assert_eq!(wrapping.evaluate(&[Value::$variant(1), Value::U32(count)]), checked.evaluate(&[Value::$variant(1), Value::U32(count % <$primitive>::BITS)]));
                }
                assert!(checked.evaluate(&[Value::$variant(1), Value::U32(<$primitive>::BITS-1)]).is_ok());
            }
            // Count validation is separate from loss of high value bits.
            assert_eq!(bind(Op::ShiftLeft, $ty, json!({})).evaluate(&[Value::$variant(<$primitive>::MAX), Value::U32(1)]), Ok(Value::$variant(<$primitive>::MAX << 1)));
        };
    }
    check!(I32, i32, "core/i32@1");
    check!(I64, i64, "core/i64@1");
    check!(U32, u32, "core/u32@1");
    check!(U64, u64, "core/u64@1");
    assert_eq!(
        bind(Op::ShiftRight, "core/i64@1", json!({})).evaluate(&[Value::I64(-3), Value::U32(1)]),
        Ok(Value::I64(-2))
    );
    assert_eq!(
        bind(Op::ShiftRight, "core/u64@1", json!({}))
            .evaluate(&[Value::U64(u64::MAX), Value::U32(63)]),
        Ok(Value::U64(1))
    );
}

#[test]
fn invalid_clamp_and_input_contracts_are_safe_errors() {
    assert_eq!(
        bind(Op::Clamp, "core/u64@1", json!({})).evaluate(&[
            Value::U64(0),
            Value::U64(2),
            Value::U64(1)
        ]),
        Err(KernelError::Domain)
    );
    assert_eq!(
        bind(Op::Clamp, "core/u64@1", json!({})).evaluate(&[
            Value::U64(9),
            Value::U64(2),
            Value::U64(2)
        ]),
        Ok(Value::U64(2))
    );
    for op in Op::ALL {
        let kernel = bind(op, "core/i64@1", json!({}));
        for inputs in [
            vec![],
            vec![Value::I64(1); 4],
            vec![Value::Bool(true); op.input_ports().len()],
            vec![Value::U64(1); op.input_ports().len()],
        ] {
            assert_eq!(
                kernel.evaluate(&inputs),
                Err(KernelError::InputContract),
                "{op:?}"
            );
        }
    }
    assert_eq!(
        bind(Op::ShiftLeft, "core/i64@1", json!({})).evaluate(&[Value::I64(1), Value::I64(1)]),
        Err(KernelError::InputContract)
    );
    for error in [KernelError::Domain, KernelError::OutOfRange] {
        assert!(error.message_key().starts_with("core/kernel-"));
        assert!(!error.message().contains("123456789"));
        assert!(error.code().starts_with("core/"));
    }
}

#[test]
fn binding_checks_exact_provider_versions_specializations_and_operation_config() {
    let (package, snapshot) = catalog();
    for op in Op::ALL {
        for ty in ["core/f64@1", "core/bool@1", "core/duration@1"] {
            assert_eq!(
                bind_in(&package, snapshot.clone(), op, ty, json!({})).unwrap_err(),
                BindingError::UnsupportedType
            );
        }
        for config in [
            json!({"unknown":true}),
            json!({"float_policy":"ieee"}),
            json!({"int_overflow":7}),
        ] {
            assert_eq!(
                bind_in(&package, snapshot.clone(), op, "core/i64@1", config).unwrap_err(),
                BindingError::InvalidConfig
            );
        }
        let definition = DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        };
        let ty = "core/i64@1".parse().unwrap();
        let mut other = package.provider_key().clone();
        other.id = "another-provider".into();
        assert_eq!(
            package
                .resolve_integer(
                    snapshot.clone(),
                    &other,
                    &definition,
                    1,
                    &ty,
                    &Default::default()
                )
                .unwrap_err(),
            BindingError::Unavailable
        );
        for (version, config_version) in [(2, 1), (1, 2)] {
            let definition = DefinitionRef {
                version,
                ..definition.clone()
            };
            assert_eq!(
                package
                    .resolve_integer(
                        snapshot.clone(),
                        package.provider_key(),
                        &definition,
                        config_version,
                        &ty,
                        &Default::default()
                    )
                    .unwrap_err(),
                BindingError::Unavailable
            );
        }
        if !matches!(
            op,
            Op::Negate | Op::Absolute | Op::ShiftLeft | Op::ShiftRight
        ) {
            assert_eq!(
                bind_in(
                    &package,
                    snapshot.clone(),
                    op,
                    "core/i64@1",
                    json!({"int_overflow":"checked"})
                )
                .unwrap_err(),
                BindingError::InvalidConfig
            );
        }
    }
    for op in [Op::ShiftLeft, Op::ShiftRight] {
        assert_eq!(
            bind_in(
                &package,
                snapshot.clone(),
                op,
                "core/i64@1",
                json!({"int_overflow":"saturating"})
            )
            .unwrap_err(),
            BindingError::InvalidConfig
        );
    }
    let pinned = bind_in(
        &package,
        snapshot.clone(),
        Op::Modulo,
        "core/i64@1",
        json!({}),
    )
    .unwrap();
    let new_package = MathPackage::new(format!("sha256:{}", "b".repeat(64))).unwrap();
    let other = new_package
        .register(orange_model::catalog::RegistryBuilder::core(
            orange_model::validation::HostOrigin::new(
                "0.0.0",
                format!("sha256:{}", "b".repeat(64)),
            )
            .unwrap(),
        ))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        bind_in(&package, other, Op::Modulo, "core/i64@1", json!({})).unwrap_err(),
        BindingError::Unavailable
    );
    assert!(Arc::ptr_eq(pinned.snapshot(), &snapshot));
    assert_eq!(pinned.provider(), package.provider_key());
    assert_eq!(
        pinned.evaluate(&[Value::I64(-17), Value::I64(5)]),
        Ok(Value::I64(3))
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn signed_remainders_follow_quotient_identity(a in any::<i64>(), b in any::<i64>().prop_filter("nonzero", |b|*b!=0)) {
        let (package,snapshot)=catalog();
        let (wide_a,wide_b)=(i128::from(a),i128::from(b));
        let remainder=wide_a-(wide_a/wide_b)*wide_b;
        let modulo=if remainder<0 {remainder+wide_b.abs()} else {remainder};
        for (op,expected) in [(Op::Remainder,remainder),(Op::Modulo,modulo)] {
            let kernel=bind_in(&package,snapshot.clone(),op,"core/i64@1",json!({})).unwrap();
            prop_assert_eq!(kernel.evaluate(&[Value::I64(a),Value::I64(b)]),Ok(Value::I64(expected as i64)));
        }
    }
    #[test]
    fn unsigned_remainders_follow_u128_quotient_identity(a in any::<u64>(), b in 1..=u64::MAX) {
        let expected=u128::from(a)-(u128::from(a)/u128::from(b))*u128::from(b);
        for op in [Op::Remainder,Op::Modulo] {
            prop_assert_eq!(bind(op,"core/u64@1",json!({})).evaluate(&[Value::U64(a),Value::U64(b)]),Ok(Value::U64(expected as u64)));
        }
    }
    #[test]
    fn fixed_width_bit_algebra_and_unsigned_shift_match_widened_product(a in any::<u64>(), b in any::<u64>(), count in 0u32..64) {
        let xor=bind(Op::BitXor,"core/u64@1",json!({}));
        let mixed=xor.evaluate(&[Value::U64(a),Value::U64(b)]).unwrap();
        prop_assert_eq!(xor.evaluate(&[mixed,Value::U64(b)]),Ok(Value::U64(a)));
        let not=bind(Op::BitNot,"core/u64@1",json!({}));
        let and=bind(Op::BitAnd,"core/u64@1",json!({})).evaluate(&[Value::U64(a),Value::U64(b)]).unwrap();
        let left=not.evaluate(&[and]).unwrap();
        let right=bind(Op::BitOr,"core/u64@1",json!({})).evaluate(&[not.evaluate(&[Value::U64(a)]).unwrap(),not.evaluate(&[Value::U64(b)]).unwrap()]).unwrap();
        prop_assert_eq!(left,right);
        let expected=(u128::from(a)*(1u128<<count)) & u128::from(u64::MAX);
        prop_assert_eq!(bind(Op::ShiftLeft,"core/u64@1",json!({})).evaluate(&[Value::U64(a),Value::U32(count)]),Ok(Value::U64(expected as u64)));
    }
}

fn literal(ty: &str, value: i64) -> Json {
    if matches!(ty, "core/i32@1" | "core/u32@1") {
        json!({"type":ty,"value":value})
    } else {
        json!({"type":ty,"value":value.to_string()})
    }
}
fn node(n: u32, op: Op, ty: &str, inputs: Json) -> Json {
    json!({"id":id(n),"definition_id":op.definition_id(),"definition_version":1,"config_version":1,"config":{},"type_pins":{"T":ty},"provider":provider(),"inputs":inputs})
}
fn graph(
    package: &MathPackage,
    mut nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
    variables: Json,
) -> GraphDocument {
    let mut signatures = serde_json::Map::new();
    let definitions = integer_definitions().unwrap();
    for node in &mut nodes {
        let Some(definition) = definitions
            .iter()
            .find(|d| Some(d.definition.id.as_str()) == node["definition_id"].as_str())
        else {
            continue;
        };
        let orange_model::catalog::Ports::Declared(ports) = definition.ports.clone() else {
            unreachable!()
        };
        let args = [(
            "T".into(),
            node["type_pins"]["T"].as_str().unwrap().parse().unwrap(),
        )]
        .into();
        let signature_id = format!("s{}", node["id"].as_str().unwrap());
        signatures.insert(
            signature_id.clone(),
            serde_json::to_value(orange_model::signature::project(
                definition.kind,
                ports,
                &args,
            ))
            .unwrap(),
        );
        node["signature"] = json!(signature_id);
    }
    let key = package.provider_key();
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001","revision":0,
        "package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],"signatures":signatures,"nodes":nodes,"edges":edges,"exports":exports,"variables":variables}).to_string()).unwrap()
}

#[test]
fn actual_definitions_fold_constants_and_keep_original_fault_sources() {
    let (package, snapshot) = catalog();
    let mut nodes = vec![];
    let mut exports = vec![];
    let mut expected = vec![];
    assert_eq!(integer_definitions().unwrap().len(), Op::ALL.len());
    for (i, op) in Op::ALL.into_iter().enumerate() {
        let n = i as u32 + 1;
        let (values, answer) = answer(op);
        let inputs: serde_json::Map<_, _> = op
            .input_ports()
            .iter()
            .zip(values)
            .map(|(&port, value)| {
                (
                    port.into(),
                    literal(
                        if port == "count" {
                            "core/u32@1"
                        } else {
                            "core/i64@1"
                        },
                        value,
                    ),
                )
            })
            .collect();
        nodes.push(node(n, op, "core/i64@1", Json::Object(inputs)));
        exports.push(json!({"id":format!("out{n}"),"node":id(n),"port":"result"}));
        expected.push(answer);
    }
    let doc = graph(&package, nodes, vec![], exports, json!([]));
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
    for (export, answer) in plan.exports().iter().zip(expected) {
        assert_eq!(
            plan.constants()[export.slot.index()],
            FoldedValue::Value(Value::I64(answer))
        );
    }
    for (op, inputs, code) in [
        (
            Op::Modulo,
            json!({"a":literal("core/i64@1",1),"b":literal("core/i64@1",0)}),
            "core/DivideByZero",
        ),
        (
            Op::Clamp,
            json!({"value":literal("core/i64@1",1),"min":literal("core/i64@1",2),"max":literal("core/i64@1",0)}),
            "core/Domain",
        ),
        (
            Op::ShiftLeft,
            json!({"value":literal("core/i64@1",1),"count":literal("core/u32@1",64)}),
            "core/OutOfRange",
        ),
    ] {
        let doc = graph(
            &package,
            vec![node(1, op, "core/i64@1", inputs)],
            vec![],
            vec![json!({"id":"fault","node":id(1),"port":"result"})],
            json!([]),
        );
        let analysis = analyze(&doc, &snapshot);
        let plan = compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap();
        let FoldedValue::Fault(fault) = &plan.constants()[plan.exports()[0].slot.index()] else {
            panic!("fault was folded away")
        };
        assert_eq!(fault.source, id(1).parse().unwrap());
        assert_eq!(fault.fault.code, code);
        assert_eq!(plan.diagnostics().len(), 1);
    }
}

#[test]
fn real_state_demands_are_stable_across_pump_slices() {
    let (package, snapshot) = catalog();
    let core = |n: u32, name: &str, config: Json| json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":config});
    let mut start = core(1, "on-start", json!({}));
    start["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
    let nodes = vec![
        start,
        core(2, "get-variable", json!({"variable":id(100)})),
        node(
            3,
            Op::Modulo,
            "core/i64@1",
            json!({"b":literal("core/i64@1",5)}),
        ),
        node(
            4,
            Op::ShiftLeft,
            "core/i64@1",
            json!({"count":literal("core/u32@1",2)}),
        ),
        node(
            5,
            Op::Clamp,
            "core/i64@1",
            json!({"min":literal("core/i64@1",0),"max":literal("core/i64@1",7)}),
        ),
        core(6, "set-variable", json!({"variable":id(101)})),
    ];
    let variables = json!([
        {"id":id(100),"name":"source","type":"core/i64@1","scope":"run","init":literal("core/i64@1",-17)},
        {"id":id(101),"name":"sink","type":"core/i64@1","scope":"run","init":literal("core/i64@1",0)}]);
    let doc = graph(
        &package,
        nodes,
        vec![
            edge(1, 1, "then", 6, "in"),
            edge(2, 2, "value", 3, "a"),
            edge(3, 3, "result", 4, "value"),
            edge(4, 4, "result", 5, "value"),
            edge(5, 5, "result", 6, "value"),
        ],
        vec![],
        variables,
    );
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
    for budget in [1, 2, 64] {
        let mut run = Run::start(plan.clone(), &doc, &snapshot, 1, RunOptions::default()).unwrap();
        let mut idle = false;
        for _ in 0..1000 {
            if !run.pump(budget).unwrap().ready {
                idle = true;
                break;
            }
        }
        assert!(idle);
        assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::I64(7)));
        run.stop().unwrap();
        run.dispose().unwrap();
        assert_eq!(run.retention().reserved_bytes, 0);
    }
}
