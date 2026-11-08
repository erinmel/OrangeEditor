//! L02b/L02c float known answers, policy precedence, IEEE edges, predicates and real
//! compiled demands.
use std::sync::Arc;

use orange_bundled::{
    BindingError, FloatOperation as Op, KernelError, MathPackage, ResolvedFloat, float_definitions,
};
use orange_compiler::{CompileOptions, FoldedValue, compile_control, compile_pure};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::DefinitionRef;
use orange_model::{GraphDocument, Value, analyze, from_json};
use orange_runtime::{Run, RunOptions};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

use crate::data_graph::{catalog, edge, id, provider};

const F32: &str = "core/f32@1";
const F64: &str = "core/f64@1";

fn bind_in(
    package: &MathPackage,
    snapshot: Arc<RegistrySnapshot>,
    op: Op,
    ty: &str,
    config: Json,
) -> Result<ResolvedFloat, BindingError> {
    package.resolve_float(
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
fn bind(op: Op, ty: &str, config: Json) -> ResolvedFloat {
    let (package, snapshot) = catalog();
    bind_in(&package, snapshot, op, ty, config).unwrap()
}
fn finite(op: Op) -> ResolvedFloat {
    bind(op, F64, json!({}))
}
fn ieee(op: Op) -> ResolvedFloat {
    bind(op, F64, json!({"float_policy":"ieee"}))
}
fn f64s(values: &[f64]) -> Vec<Value> {
    values.iter().map(|v| Value::F64(*v)).collect()
}
fn bits(result: Result<Value, KernelError>) -> u64 {
    match result {
        Ok(Value::F64(v)) => v.to_bits(),
        other => panic!("{other:?}"),
    }
}

/// Answers exact in binary32 and binary64 on every conforming platform library.
/// Predicates encode true as 1.0; `expected` maps them to `core/bool@1`.
fn answer(op: Op) -> (Vec<f64>, f64) {
    match op {
        Op::SquareRoot => (vec![16.0], 4.0),
        Op::Exponential => (vec![0.0], 1.0),
        Op::NaturalLog => (vec![1.0], 0.0),
        Op::Log2 => (vec![8.0], 3.0),
        Op::Log10 => (vec![1.0], 0.0),
        Op::Power => (vec![2.0, 10.0], 1024.0),
        Op::Floor => (vec![-2.5], -3.0),
        Op::Ceiling => (vec![-2.5], -2.0),
        Op::Truncate => (vec![-2.5], -2.0),
        Op::RoundTiesEven => (vec![2.5], 2.0),
        Op::Negate => (vec![1.5], -1.5),
        Op::Absolute => (vec![-1.5], 1.5),
        Op::Minimum => (vec![1.0, 2.0], 1.0),
        Op::Maximum => (vec![1.0, 2.0], 2.0),
        Op::Clamp => (vec![5.0, 0.0, 3.0], 3.0),
        Op::Remainder => (vec![7.5, 2.0], 1.5),
        Op::Sine => (vec![0.0], 0.0),
        Op::Cosine => (vec![0.0], 1.0),
        Op::Tangent => (vec![0.0], 0.0),
        Op::Arcsine => (vec![0.0], 0.0),
        Op::Arccosine => (vec![1.0], 0.0),
        Op::Arctangent => (vec![0.0], 0.0),
        Op::Arctangent2 => (vec![0.0, 1.0], 0.0),
        Op::Equal => (vec![1.0, 1.0], 1.0),
        Op::NotEqual => (vec![1.0, 2.0], 1.0),
        Op::Less => (vec![1.0, 2.0], 1.0),
        Op::LessOrEqual => (vec![2.0, 2.0], 1.0),
        Op::Greater => (vec![1.0, 2.0], 0.0),
        Op::GreaterOrEqual => (vec![1.0, 2.0], 0.0),
        Op::TotalOrderLess => (vec![-0.0, 0.0], 1.0),
        Op::IsNan => (vec![1.0], 0.0),
        Op::IsFinite => (vec![1.0], 1.0),
        Op::IsInfinite => (vec![1.0], 0.0),
    }
}
fn expected(op: Op, answer: f64, narrow: bool) -> Value {
    if op.is_predicate() {
        Value::Bool(answer != 0.0)
    } else if narrow {
        Value::F32(answer as f32)
    } else {
        Value::F64(answer)
    }
}
fn policies(op: Op) -> Vec<Json> {
    if op.is_predicate() {
        vec![json!({})]
    } else {
        vec![
            json!({"float_policy":"finite"}),
            json!({"float_policy":"ieee"}),
        ]
    }
}

#[test]
fn all_operations_have_known_answers_in_both_precisions_and_policies() {
    for op in Op::ALL {
        let (values, answer) = answer(op);
        for config in policies(op) {
            assert_eq!(
                bind(op, F64, config.clone()).evaluate(&f64s(&values)),
                Ok(expected(op, answer, false)),
                "{op:?}"
            );
            let narrow: Vec<_> = values.iter().map(|v| Value::F32(*v as f32)).collect();
            assert_eq!(
                bind(op, F32, config).evaluate(&narrow),
                Ok(expected(op, answer, true)),
                "{op:?}"
            );
        }
    }
}

#[test]
fn finite_policy_checks_inputs_then_poles_then_domain_then_result() {
    for op in Op::ALL.into_iter().filter(|op| !op.is_predicate()) {
        let arity = op.input_ports().len();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for slot in 0..arity {
                let mut values = answer(op).0;
                values[slot] = bad;
                assert_eq!(
                    finite(op).evaluate(&f64s(&values)),
                    Err(KernelError::NonFinite),
                    "{op:?}"
                );
            }
        }
    }
    use KernelError::{DivideByZero, Domain, NonFinite};
    for (op, values, expected) in [
        (Op::SquareRoot, vec![-1.0], Err(Domain)),
        (Op::SquareRoot, vec![-0.0], Ok(Value::F64(-0.0))),
        (Op::NaturalLog, vec![0.0], Err(DivideByZero)),
        (Op::Log2, vec![-0.0], Err(DivideByZero)),
        (Op::Log10, vec![-1.0], Err(Domain)),
        (Op::Power, vec![0.0, -1.0], Err(DivideByZero)),
        (Op::Power, vec![-0.0, -0.5], Err(DivideByZero)),
        (Op::Power, vec![-8.0, 0.5], Err(Domain)),
        (Op::Power, vec![-2.0, 3.0], Ok(Value::F64(-8.0))),
        (Op::Power, vec![10.0, 400.0], Err(NonFinite)),
        (Op::Exponential, vec![1000.0], Err(NonFinite)),
        (Op::Exponential, vec![-1000.0], Ok(Value::F64(0.0))),
        (Op::Remainder, vec![1.0, 0.0], Err(DivideByZero)),
        (Op::Remainder, vec![1.0, -0.0], Err(DivideByZero)),
        (Op::Clamp, vec![1.0, 2.0, 0.0], Err(Domain)),
        (Op::Arcsine, vec![1.5], Err(Domain)),
        (Op::Arccosine, vec![-1.0000001], Err(Domain)),
        (Op::Arcsine, vec![-0.0], Ok(Value::F64(-0.0))),
        (Op::Arctangent2, vec![0.0, 0.0], Ok(Value::F64(0.0))),
        (
            Op::Arctangent2,
            vec![0.0, -1.0],
            Ok(Value::F64(std::f64::consts::PI)),
        ),
        (
            Op::Arctangent2,
            vec![-0.0, -1.0],
            Ok(Value::F64(-std::f64::consts::PI)),
        ),
    ] {
        assert_eq!(finite(op).evaluate(&f64s(&values)), expected, "{op:?}");
    }
    // Non-finite inputs precede both a pole and a domain error.
    assert_eq!(
        finite(Op::Remainder).evaluate(&f64s(&[f64::NAN, 0.0])),
        Err(NonFinite)
    );
    // Binary32 overflows at its own range, not binary64's.
    assert_eq!(
        bind(Op::Exponential, F32, json!({})).evaluate(&[Value::F32(100.0)]),
        Err(NonFinite)
    );
}

#[test]
fn ieee_policy_returns_native_nan_infinity_and_signed_zero() {
    assert!(f64::from_bits(bits(ieee(Op::SquareRoot).evaluate(&f64s(&[-1.0])))).is_nan());
    assert_eq!(
        ieee(Op::NaturalLog).evaluate(&f64s(&[0.0])),
        Ok(Value::F64(f64::NEG_INFINITY))
    );
    assert_eq!(
        ieee(Op::Exponential).evaluate(&f64s(&[1000.0])),
        Ok(Value::F64(f64::INFINITY))
    );
    assert!(f64::from_bits(bits(ieee(Op::Remainder).evaluate(&f64s(&[1.0, 0.0])))).is_nan());
    assert_eq!(
        ieee(Op::Power).evaluate(&f64s(&[f64::NAN, 0.0])),
        Ok(Value::F64(1.0))
    );
    // Sign-bit operations act on NaN bits exactly.
    let nan = f64::from_bits(0x7ff8_0000_0000_0123);
    assert_eq!(
        bits(ieee(Op::Negate).evaluate(&[Value::F64(nan)])),
        0xfff8_0000_0000_0123
    );
    assert_eq!(
        bits(ieee(Op::Absolute).evaluate(&[Value::F64(-nan)])),
        0x7ff8_0000_0000_0123
    );
    for policy in [finite, ieee] {
        for (op, value, expected) in [
            (Op::Negate, 0.0, -0.0f64),
            (Op::Absolute, -0.0, 0.0),
            (Op::Floor, -0.0, -0.0),
            (Op::Ceiling, -0.5, -0.0),
            (Op::Truncate, -0.7, -0.0),
            (Op::RoundTiesEven, -0.4, -0.0),
            (Op::RoundTiesEven, -2.5, -2.0),
            (Op::RoundTiesEven, 3.5, 4.0),
        ] {
            assert_eq!(
                bits(policy(op).evaluate(&[Value::F64(value)])),
                expected.to_bits(),
                "{op:?} {value}"
            );
        }
        assert_eq!(
            bits(policy(Op::Remainder).evaluate(&f64s(&[-7.5, 2.0]))),
            (-1.5f64).to_bits()
        );
        assert_eq!(
            bits(policy(Op::Remainder).evaluate(&f64s(&[-4.0, 2.0]))),
            (-0.0f64).to_bits()
        );
    }
}

#[test]
fn minimum_maximum_and_clamp_follow_ieee_2019_nan_and_zero_order() {
    for policy in [finite, ieee] {
        for (a, b) in [(0.0, -0.0), (-0.0, 0.0)] {
            assert_eq!(
                bits(policy(Op::Minimum).evaluate(&f64s(&[a, b]))),
                (-0.0f64).to_bits()
            );
            assert_eq!(
                bits(policy(Op::Maximum).evaluate(&f64s(&[a, b]))),
                0.0f64.to_bits()
            );
        }
        // Signed-zero bounds still order -0 below +0 inside clamp.
        assert_eq!(
            bits(policy(Op::Clamp).evaluate(&f64s(&[-0.0, 0.0, 0.0]))),
            0.0f64.to_bits()
        );
        assert_eq!(
            bits(policy(Op::Clamp).evaluate(&f64s(&[0.0, -0.0, -0.0]))),
            (-0.0f64).to_bits()
        );
    }
    let first = f64::from_bits(0x7ff8_0000_0000_0001);
    let second = f64::from_bits(0x7ff8_0000_0000_0002);
    for op in [Op::Minimum, Op::Maximum] {
        assert_eq!(
            bits(ieee(op).evaluate(&f64s(&[1.0, second]))),
            second.to_bits()
        );
        assert_eq!(
            bits(ieee(op).evaluate(&f64s(&[first, second]))),
            first.to_bits()
        );
    }
    assert_eq!(
        bits(ieee(Op::Clamp).evaluate(&f64s(&[first, 0.0, 1.0]))),
        first.to_bits()
    );
    for bounds in [[f64::NAN, 1.0], [0.0, f64::NAN], [1.0, 0.0]] {
        assert_eq!(
            ieee(Op::Clamp).evaluate(&f64s(&[0.5, bounds[0], bounds[1]])),
            Err(KernelError::Domain)
        );
    }
    assert_eq!(
        ieee(Op::Clamp).evaluate(&f64s(&[f64::INFINITY, 0.0, f64::INFINITY])),
        Ok(Value::F64(f64::INFINITY))
    );
}

#[test]
fn input_contracts_bindings_and_provider_pins_are_exact() {
    for op in Op::ALL {
        let kernel = finite(op);
        let arity = op.input_ports().len();
        for inputs in [
            vec![],
            vec![Value::F64(1.0); 4],
            vec![Value::F32(1.0); arity],
            vec![Value::I64(1); arity],
        ] {
            assert_eq!(
                kernel.evaluate(&inputs),
                Err(KernelError::InputContract),
                "{op:?}"
            );
        }
    }
    let (package, snapshot) = catalog();
    for op in Op::ALL {
        for ty in ["core/i64@1", "core/u32@1", "core/bool@1"] {
            assert_eq!(
                bind_in(&package, snapshot.clone(), op, ty, json!({})).unwrap_err(),
                BindingError::UnsupportedType
            );
        }
        let mut invalid = vec![
            json!({"unknown":true}),
            json!({"int_overflow":"checked"}),
            json!({"float_policy":"strict"}),
            json!({"float_policy":7}),
        ];
        if op.is_predicate() {
            invalid.push(json!({"float_policy":"ieee"}));
        }
        for config in invalid {
            assert_eq!(
                bind_in(&package, snapshot.clone(), op, F64, config).unwrap_err(),
                BindingError::InvalidConfig
            );
        }
        let definition = DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        };
        let ty = F64.parse().unwrap();
        let mut other = package.provider_key().clone();
        other.id = "another-provider".into();
        assert_eq!(
            package
                .resolve_float(
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
                    .resolve_float(
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
    }
    // The integer contracts keep `T: int`; float-only names never bind as integers.
    assert_eq!(
        package
            .resolve_integer(
                snapshot.clone(),
                package.provider_key(),
                &DefinitionRef {
                    id: "org.orange.math/minimum".parse().unwrap(),
                    version: 1
                },
                1,
                &F64.parse().unwrap(),
                &Default::default()
            )
            .unwrap_err(),
        BindingError::UnsupportedType
    );
    let pinned = bind_in(&package, snapshot.clone(), Op::SquareRoot, F64, json!({})).unwrap();
    let other = MathPackage::new(format!("sha256:{}", "b".repeat(64)))
        .unwrap()
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
        bind_in(&package, other, Op::SquareRoot, F64, json!({})).unwrap_err(),
        BindingError::Unavailable
    );
    assert!(Arc::ptr_eq(pinned.snapshot(), &snapshot));
    assert_eq!(pinned.provider(), package.provider_key());
    assert_eq!(pinned.evaluate(&f64s(&[9.0])), Ok(Value::F64(3.0)));
}

#[test]
fn predicates_compare_and_classify_every_ieee_value_without_faults() {
    let values = [
        f64::NEG_INFINITY,
        -1.5,
        -0.0,
        0.0,
        f64::MIN_POSITIVE / 2.0,
        1.5,
        f64::INFINITY,
        f64::NAN,
        -f64::NAN,
    ];
    let truth = |op: Op, inputs: &[f64]| match finite(op).evaluate(&f64s(inputs)) {
        Ok(Value::Bool(truth)) => truth,
        other => panic!("{op:?} {other:?}"),
    };
    for &a in &values {
        assert_eq!(truth(Op::IsNan, &[a]), a.is_nan());
        assert_eq!(truth(Op::IsFinite, &[a]), a.is_finite());
        assert_eq!(truth(Op::IsInfinite, &[a]), a.is_infinite());
        for &b in &values {
            let unordered = a.is_nan() || b.is_nan();
            assert_eq!(truth(Op::Equal, &[a, b]), !unordered && a == b);
            assert_eq!(truth(Op::NotEqual, &[a, b]), unordered || a != b);
            for (op, holds) in [
                (Op::Less, a < b),
                (Op::LessOrEqual, a <= b),
                (Op::Greater, a > b),
                (Op::GreaterOrEqual, a >= b),
            ] {
                assert_eq!(truth(op, &[a, b]), !unordered && holds, "{op:?} {a} {b}");
            }
        }
    }
    // IEEE equality ignores the zero sign; total order separates it and orders NaNs
    // by sign at both ends.
    assert!(truth(Op::Equal, &[-0.0, 0.0]));
    assert!(truth(Op::TotalOrderLess, &[-0.0, 0.0]));
    assert!(!truth(Op::TotalOrderLess, &[0.0, -0.0]));
    assert!(truth(Op::TotalOrderLess, &[-f64::NAN, f64::NEG_INFINITY]));
    assert!(truth(Op::TotalOrderLess, &[f64::INFINITY, f64::NAN]));
    assert!(!truth(Op::TotalOrderLess, &[f64::NAN, f64::NAN]));
    let low = f64::from_bits(0x7ff8_0000_0000_0001);
    let high = f64::from_bits(0x7ff8_0000_0000_0002);
    assert!(truth(Op::TotalOrderLess, &[low, high]));
    assert_eq!(
        bind(Op::IsNan, F32, json!({})).evaluate(&[Value::F32(f32::NAN)]),
        Ok(Value::Bool(true))
    );
}

#[test]
fn boolean_xor_is_an_ordinary_eager_bool_definition() {
    let (package, snapshot) = catalog();
    let definition = DefinitionRef {
        id: orange_bundled::Operation::Xor
            .definition_id()
            .parse()
            .unwrap(),
        version: 1,
    };
    let xor = package
        .resolve(
            snapshot.clone(),
            package.provider_key(),
            &definition,
            1,
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
    for (a, b) in [(false, false), (false, true), (true, false), (true, true)] {
        assert_eq!(
            xor.evaluate(&[Value::Bool(a), Value::Bool(b)]),
            Ok(Value::Bool(a ^ b))
        );
    }
    for inputs in [
        vec![Value::Bool(true)],
        vec![Value::Bool(true), Value::I64(1)],
        vec![Value::I64(1), Value::I64(1)],
    ] {
        assert_eq!(xor.evaluate(&inputs), Err(KernelError::InputContract));
    }
    let args = [("T".to_string(), "core/i64@1".parse().unwrap())].into();
    assert_eq!(
        package
            .resolve(
                snapshot.clone(),
                package.provider_key(),
                &definition,
                1,
                &args,
                &Default::default()
            )
            .unwrap_err(),
        BindingError::UnsupportedType
    );
    assert_eq!(
        package
            .resolve(
                snapshot,
                package.provider_key(),
                &definition,
                1,
                &Default::default(),
                &serde_json::from_value(json!({"float_policy":"ieee"})).unwrap()
            )
            .unwrap_err(),
        BindingError::InvalidConfig
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn total_order_less_is_a_strict_total_order_on_bits(a in any::<u64>(), b in any::<u64>()) {
        let (a, b) = (f64::from_bits(a), f64::from_bits(b));
        let less = |x: f64, y: f64| finite(Op::TotalOrderLess).evaluate(&f64s(&[x, y])) == Ok(Value::Bool(true));
        prop_assert!(!less(a, a));
        prop_assert_eq!(less(a, b) || less(b, a), a.to_bits() != b.to_bits());
        prop_assert!(!(less(a, b) && less(b, a)));
    }
    #[test]
    fn inverse_trig_stays_in_principal_ranges(x in -1.0f64..=1.0, y in -1.0e6f64..1.0e6, z in -1.0e6f64..1.0e6) {
        let value = |op: Op, inputs: &[f64]| f64::from_bits(bits(finite(op).evaluate(&f64s(inputs))));
        let half = std::f64::consts::FRAC_PI_2;
        let pi = std::f64::consts::PI;
        prop_assert!(value(Op::Arcsine, &[x]).abs() <= half);
        prop_assert!((0.0..=pi).contains(&value(Op::Arccosine, &[x])));
        prop_assert!(value(Op::Arctangent, &[y]).abs() <= half);
        prop_assert!(value(Op::Arctangent2, &[y, z]).abs() <= pi);
        let (sine, cosine) = (value(Op::Sine, &[y]), value(Op::Cosine, &[y]));
        prop_assert!(sine.abs() <= 1.0 && cosine.abs() <= 1.0);
        prop_assert!((sine * sine + cosine * cosine - 1.0).abs() < 1.0e-12);
    }
    #[test]
    fn rounding_nodes_bracket_their_input(x in -1.0e15f64..1.0e15) {
        let floor = bits(finite(Op::Floor).evaluate(&[Value::F64(x)]));
        let ceil = bits(finite(Op::Ceiling).evaluate(&[Value::F64(x)]));
        let trunc = bits(finite(Op::Truncate).evaluate(&[Value::F64(x)]));
        let even = bits(finite(Op::RoundTiesEven).evaluate(&[Value::F64(x)]));
        let (floor, ceil, trunc, even) = (f64::from_bits(floor), f64::from_bits(ceil), f64::from_bits(trunc), f64::from_bits(even));
        prop_assert!(floor <= x && x < floor + 1.0 && floor.fract() == 0.0);
        prop_assert!(ceil >= x && x > ceil - 1.0 && ceil.fract() == 0.0);
        prop_assert!(trunc.abs() <= x.abs() && (x - trunc).abs() < 1.0 && trunc.fract() == 0.0);
        let distance = (even - x).abs();
        prop_assert!(distance <= 0.5 && even.fract() == 0.0);
        if distance == 0.5 {
            prop_assert_eq!(even % 2.0, 0.0);
        }
    }
    #[test]
    fn remainder_keeps_dividend_sign_and_divisor_bound(a in -1.0e12f64..1.0e12, b in prop_oneof![-1.0e6f64..-1.0e-6, 1.0e-6f64..1.0e6]) {
        let r = f64::from_bits(bits(finite(Op::Remainder).evaluate(&f64s(&[a, b]))));
        prop_assert!(r.abs() < b.abs());
        prop_assert!(r == 0.0 || r.is_sign_negative() == a.is_sign_negative());
    }
    #[test]
    fn square_root_of_exact_squares_is_exact(n in 0u32..(1 << 26)) {
        let square = f64::from(n) * f64::from(n);
        prop_assert_eq!(finite(Op::SquareRoot).evaluate(&[Value::F64(square)]), Ok(Value::F64(f64::from(n))));
    }
    #[test]
    fn minimum_and_maximum_partition_ordered_pairs(a in any::<f64>().prop_filter("ordered", |v| !v.is_nan()), b in any::<f64>().prop_filter("ordered", |v| !v.is_nan())) {
        let low = f64::from_bits(bits(ieee(Op::Minimum).evaluate(&f64s(&[a, b]))));
        let high = f64::from_bits(bits(ieee(Op::Maximum).evaluate(&f64s(&[a, b]))));
        prop_assert!(low.total_cmp(&high).is_le());
        let mut pair = [a.to_bits(), b.to_bits()];
        let mut got = [low.to_bits(), high.to_bits()];
        pair.sort_unstable();
        got.sort_unstable();
        prop_assert_eq!(pair, got);
    }
}

fn literal(value: f64) -> Json {
    json!({"type":F64,"bits":format!("{:016x}", value.to_bits())})
}
fn node(n: u32, op: Op, inputs: Json) -> Json {
    json!({"id":id(n),"definition_id":op.definition_id(),"definition_version":1,"config_version":1,"config":{},"type_pins":{"T":F64},"provider":provider(),"inputs":inputs})
}
fn graph(
    package: &MathPackage,
    mut nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
    variables: Json,
) -> GraphDocument {
    let mut signatures = serde_json::Map::new();
    let definitions = float_definitions().unwrap();
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
        let args = [("T".into(), F64.parse().unwrap())].into();
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
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000002","revision":0,
        "package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],"signatures":signatures,"nodes":nodes,"edges":edges,"exports":exports,"variables":variables}).to_string()).unwrap()
}

#[test]
fn actual_definitions_fold_constants_and_keep_original_fault_sources() {
    let (package, snapshot) = catalog();
    assert_eq!(float_definitions().unwrap().len(), Op::ALL.len());
    let mut nodes = vec![];
    let mut exports = vec![];
    let mut expected = vec![];
    for (i, op) in Op::ALL.into_iter().enumerate() {
        let n = i as u32 + 1;
        let (values, answer) = answer(op);
        let inputs: serde_json::Map<_, _> = op
            .input_ports()
            .iter()
            .zip(values)
            .map(|(&port, value)| (port.into(), literal(value)))
            .collect();
        nodes.push(node(n, op, Json::Object(inputs)));
        exports.push(json!({"id":format!("out{n}"),"node":id(n),"port":"result"}));
        expected.push(self::expected(op, answer, false));
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
            FoldedValue::Value(answer)
        );
    }
    for (op, inputs, code) in [
        (
            Op::NaturalLog,
            json!({"value":literal(0.0)}),
            "core/DivideByZero",
        ),
        (
            Op::SquareRoot,
            json!({"value":literal(-4.0)}),
            "core/Domain",
        ),
        (
            Op::Exponential,
            json!({"value":literal(1000.0)}),
            "core/NonFinite",
        ),
    ] {
        let doc = graph(
            &package,
            vec![node(1, op, inputs)],
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
        node(3, Op::Absolute, json!({})),
        node(4, Op::SquareRoot, json!({})),
        node(5, Op::Power, json!({"exponent":literal(3.0)})),
        node(
            6,
            Op::Clamp,
            json!({"min":literal(0.0),"max":literal(100.0)}),
        ),
        core(7, "set-variable", json!({"variable":id(101)})),
    ];
    let variables = json!([
        {"id":id(100),"name":"source","type":F64,"scope":"run","init":literal(-20.25)},
        {"id":id(101),"name":"sink","type":F64,"scope":"run","init":literal(0.0)}]);
    let doc = graph(
        &package,
        nodes,
        vec![
            edge(1, 1, "then", 7, "in"),
            edge(2, 2, "value", 3, "value"),
            edge(3, 3, "result", 4, "value"),
            edge(4, 4, "result", 5, "base"),
            edge(5, 5, "result", 6, "value"),
            edge(6, 6, "result", 7, "value"),
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
        // |-20.25| = 20.25, sqrt = 4.5, 4.5^3 = 91.125, clamped to [0, 100].
        assert_eq!(
            run.variable(id(101).parse().unwrap()),
            Some(&Value::F64(91.125))
        );
        run.stop().unwrap();
        run.dispose().unwrap();
        assert_eq!(run.retention().reserved_bytes, 0);
    }
}
