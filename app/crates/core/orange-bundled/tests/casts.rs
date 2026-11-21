//! L02d: every ordered numeric pair has an exact Core converter contract.
use std::collections::BTreeMap;

use orange_bundled::{BindingError, ConversionError, ResolvedConversion, resolve_core_conversion};
use orange_compiler::{CompileOptions, FoldedValue, compile_pure};
use orange_model::Value;
use orange_model::conversion::{ConversionClass, ConverterConfig, ConverterKey, ConverterProvider};
use proptest::prelude::*;
use serde_json::json;

use crate::data_graph::{catalog, id};

const NUMERIC: [&str; 6] = ["i32", "i64", "u32", "u64", "f32", "f64"];
const ROUNDINGS: [&str; 4] = ["truncate", "floor", "ceiling", "nearest-even"];

fn key(from: &str, to: &str) -> ConverterKey {
    ConverterKey {
        id: format!("core/{from}-to-{to}").parse().unwrap(),
        contract_version: 1,
    }
}
fn options(from: &str, to: &str, rounding: &str) -> BTreeMap<String, String> {
    let float = |ty: &str| ty.starts_with('f');
    let mut options = BTreeMap::new();
    match (float(from), float(to)) {
        (true, false) => {
            options.insert("rounding".into(), rounding.into());
        }
        (true, true) if from == "f64" => {
            options.insert("rounding".into(), "nearest-even".into());
            options.insert("overflow".into(), "reject".into());
        }
        (false, true) if !matches!((from, to), ("i32" | "u32", "f64")) => {
            options.insert("rounding".into(), "nearest-even".into());
        }
        _ => {}
    }
    options
}
fn bind(from: &str, to: &str, rounding: &str) -> ResolvedConversion {
    let (_, snapshot) = catalog();
    resolve_core_conversion(
        snapshot,
        &ConverterConfig {
            entry: key(from, to),
            options: options(from, to, rounding),
        },
        &ConverterProvider::Core,
    )
    .unwrap()
}
fn integer(ty: &str, value: i128) -> Value {
    match ty {
        "i32" => Value::I32(value as i32),
        "i64" => Value::I64(value as i64),
        "u32" => Value::U32(value as u32),
        _ => Value::U64(value as u64),
    }
}
fn range(ty: &str) -> (i128, i128) {
    match ty {
        "i32" => (i32::MIN.into(), i32::MAX.into()),
        "i64" => (i64::MIN.into(), i64::MAX.into()),
        "u32" => (0, u32::MAX.into()),
        _ => (0, u64::MAX.into()),
    }
}
fn lost(loss: &Option<Value>) -> bool {
    let Some(Value::Json(text)) = loss else {
        panic!("lossy entries always report loss")
    };
    serde_json::from_str::<serde_json::Value>(text.as_str()).unwrap()["lost"]
        .as_bool()
        .unwrap()
}

#[test]
fn registry_holds_one_core_entry_per_ordered_numeric_pair_with_exact_classes() {
    let (_, snapshot) = catalog();
    let registry = snapshot.converters();
    let mut seen = 0;
    for from in NUMERIC {
        for to in NUMERIC.into_iter().filter(|to| *to != from) {
            let entry = registry.entry(&key(from, to)).unwrap();
            assert_eq!(entry.from.as_str(), format!("core/{from}@1"));
            assert_eq!(entry.to.as_str(), format!("core/{to}@1"));
            let expected = match (from, to) {
                ("i32", "i64")
                | ("u32", "i64" | "u64")
                | ("i32" | "u32", "f64")
                | ("f32", "f64") => ConversionClass::LosslessTotal,
                ("f64", "f32") => ConversionClass::Lossy,
                (_, "f32" | "f64") if !from.starts_with('f') => ConversionClass::Lossy,
                _ => ConversionClass::CheckedPartial,
            };
            assert_eq!(entry.class, expected, "{from}->{to}");
            let policies: Vec<_> = entry.policies.iter().map(|p| p.name.as_str()).collect();
            let names: Vec<_> = options(from, to, "truncate").into_keys().collect();
            assert_eq!(policies, names, "{from}->{to}");
            if from.starts_with('f') && !to.starts_with('f') {
                let rounding = &entry.policies[0];
                let mut choices = ROUNDINGS.map(String::from).to_vec();
                choices.sort();
                assert_eq!(rounding.choices, choices);
                assert_eq!(rounding.default.as_deref(), Some("truncate"));
                let errors: Vec<_> = entry.errors.iter().map(|e| e.as_str()).collect();
                assert_eq!(errors, ["core/NonFinite", "core/OutOfRange"]);
            }
            assert!(
                registry
                    .providers(&key(from, to))
                    .unwrap()
                    .any(|provider| provider == &ConverterProvider::Core)
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 30);
}

#[test]
fn integer_pairs_are_exact_or_checked_at_every_boundary() {
    let probes: Vec<i128> = [
        i128::from(i64::MIN),
        i128::from(i32::MIN) - 1,
        i128::from(i32::MIN),
        -1,
        0,
        1,
        i128::from(i32::MAX),
        i128::from(i32::MAX) + 1,
        i128::from(u32::MAX),
        i128::from(u32::MAX) + 1,
        i128::from(i64::MAX),
        i128::from(i64::MAX) + 1,
        i128::from(u64::MAX),
    ]
    .into();
    for from in ["i32", "i64", "u32", "u64"] {
        for to in ["i32", "i64", "u32", "u64"]
            .into_iter()
            .filter(|to| *to != from)
        {
            let conversion = bind(from, to, "");
            for &value in probes
                .iter()
                .filter(|v| (range(from).0..=range(from).1).contains(*v))
            {
                let output = conversion.evaluate(&integer(from, value));
                if (range(to).0..=range(to).1).contains(&value) {
                    let output = output.unwrap();
                    assert_eq!(output.result, integer(to, value), "{from}->{to} {value}");
                    assert!(output.loss.is_none());
                } else {
                    assert_eq!(
                        output,
                        Err(ConversionError::OutOfRange),
                        "{from}->{to} {value}"
                    );
                }
            }
            assert_eq!(
                conversion.evaluate(&Value::F64(1.0)),
                Err(ConversionError::InputContract)
            );
        }
    }
}

#[test]
fn integer_to_float_rounds_ties_to_even_and_reports_loss_exactly() {
    let two53 = 1i128 << 53;
    for (from, value, expected, was_lost) in [
        ("i64", two53, two53 as f64, false),
        ("i64", two53 + 1, two53 as f64, true),
        ("i64", two53 + 3, (two53 + 4) as f64, true),
        (
            "i64",
            i128::from(i64::MIN),
            -9_223_372_036_854_775_808.0,
            false,
        ),
        (
            "u64",
            i128::from(u64::MAX),
            18_446_744_073_709_551_616.0,
            true,
        ),
        ("u64", 0, 0.0, false),
    ] {
        let output = bind(from, "f64", "")
            .evaluate(&integer(from, value))
            .unwrap();
        assert_eq!(output.result, Value::F64(expected), "{from} {value}");
        assert_eq!(lost(&output.loss), was_lost, "{from} {value}");
    }
    for (from, value, expected, was_lost) in [
        ("i32", 16_777_216, 16_777_216.0f32, false),
        ("i32", 16_777_217, 16_777_216.0, true),
        ("i32", 16_777_219, 16_777_220.0, true),
        ("u32", i128::from(u32::MAX), 4_294_967_296.0, true),
        ("i64", -3, -3.0, false),
        (
            "u64",
            i128::from(u64::MAX),
            18_446_744_073_709_551_616.0,
            true,
        ),
    ] {
        let output = bind(from, "f32", "")
            .evaluate(&integer(from, value))
            .unwrap();
        assert_eq!(output.result, Value::F32(expected), "{from} {value}");
        assert_eq!(lost(&output.loss), was_lost, "{from} {value}");
    }
    for (from, value) in [("i32", i128::from(i32::MIN)), ("u32", i128::from(u32::MAX))] {
        let output = bind(from, "f64", "")
            .evaluate(&integer(from, value))
            .unwrap();
        assert_eq!(output.result, Value::F64(value as f64));
        assert!(output.loss.is_none());
    }
}

#[test]
fn float_to_integer_applies_the_saved_rounding_then_checks_range() {
    for (value, answers) in [
        (-2.5, [-2, -3, -2, -2]),
        (-1.5, [-1, -2, -1, -2]),
        (2.5, [2, 2, 3, 2]),
        (0.5, [0, 0, 1, 0]),
        (-0.0, [0, 0, 0, 0]),
        (7.0, [7, 7, 7, 7]),
    ] {
        for (rounding, answer) in ROUNDINGS.into_iter().zip(answers) {
            for from in ["f32", "f64"] {
                let input = if from == "f32" {
                    Value::F32(value as f32)
                } else {
                    Value::F64(value)
                };
                let output = bind(from, "i64", rounding).evaluate(&input).unwrap();
                assert_eq!(
                    output.result,
                    Value::I64(answer),
                    "{from} {rounding} {value}"
                );
                assert!(output.loss.is_none());
            }
        }
    }
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for to in ["i32", "i64", "u32", "u64"] {
            assert_eq!(
                bind("f64", to, "truncate").evaluate(&Value::F64(bad)),
                Err(ConversionError::NonFinite)
            );
        }
    }
    let two63 = 9_223_372_036_854_775_808.0f64;
    let below_two64 = f64::from_bits(18_446_744_073_709_551_616.0f64.to_bits() - 1);
    for (to, value, rounding, expected) in [
        (
            "i64",
            -two63,
            "truncate",
            Ok(integer("i64", i128::from(i64::MIN))),
        ),
        ("i64", two63, "truncate", Err(ConversionError::OutOfRange)),
        (
            "u64",
            below_two64,
            "truncate",
            Ok(integer("u64", below_two64 as i128)),
        ),
        (
            "u64",
            18_446_744_073_709_551_616.0,
            "floor",
            Err(ConversionError::OutOfRange),
        ),
        ("u64", -0.5, "truncate", Ok(Value::U64(0))),
        ("u64", -0.5, "floor", Err(ConversionError::OutOfRange)),
        ("u32", -0.4, "nearest-even", Ok(Value::U32(0))),
        ("i32", 2_147_483_647.5, "floor", Ok(Value::I32(i32::MAX))),
        (
            "i32",
            2_147_483_647.5,
            "ceiling",
            Err(ConversionError::OutOfRange),
        ),
        (
            "i32",
            -2_147_483_648.9,
            "truncate",
            Ok(Value::I32(i32::MIN)),
        ),
        (
            "i32",
            -2_147_483_648.9,
            "floor",
            Err(ConversionError::OutOfRange),
        ),
        ("u32", 4_294_967_295.9, "truncate", Ok(Value::U32(u32::MAX))),
        ("i64", 1.0e300, "truncate", Err(ConversionError::OutOfRange)),
    ] {
        assert_eq!(
            bind("f64", to, rounding)
                .evaluate(&Value::F64(value))
                .map(|output| output.result),
            expected,
            "{to} {rounding} {value}"
        );
    }
    assert_eq!(
        bind("f32", "u32", "truncate")
            .evaluate(&Value::F32(4_294_967_296.0))
            .map(|o| o.result),
        Err(ConversionError::OutOfRange)
    );
    assert_eq!(ConversionError::NonFinite.code(), "core/NonFinite");
    assert_eq!(
        ConversionError::NonFinite.message_key(),
        "core/conversion-non-finite"
    );
}

#[test]
fn bindings_require_complete_policies_for_each_pair() {
    let (_, snapshot) = catalog();
    let check = |from: &str, to: &str, options: &[(&str, &str)]| {
        resolve_core_conversion(
            snapshot.clone(),
            &ConverterConfig {
                entry: key(from, to),
                options: options
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
            &ConverterProvider::Core,
        )
        .map(|_| ())
    };
    let invalid = Err(BindingError::InvalidConfig);
    assert_eq!(check("f64", "i64", &[]), invalid);
    assert_eq!(check("f64", "i64", &[("rounding", "away")]), invalid);
    assert_eq!(
        check(
            "f64",
            "i64",
            &[("rounding", "floor"), ("overflow", "reject")]
        ),
        invalid
    );
    assert_eq!(check("i64", "f64", &[]), invalid);
    assert_eq!(check("i64", "f64", &[("rounding", "truncate")]), invalid);
    assert_eq!(
        check("i32", "f64", &[("rounding", "nearest-even")]),
        invalid
    );
    assert_eq!(check("u64", "i32", &[("rounding", "truncate")]), invalid);
    assert_eq!(check("f64", "i64", &[("rounding", "floor")]), Ok(()));
    assert_eq!(check("u64", "f32", &[("rounding", "nearest-even")]), Ok(()));
    assert_eq!(check("u64", "u32", &[]), Ok(()));
    // Identity and non-numeric endpoints have no entry.
    assert_eq!(check("i64", "i64", &[]), Err(BindingError::Unavailable));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn i64_to_f64_matches_native_rounding_and_loss(value in any::<i64>()) {
        let output = bind("i64", "f64", "").evaluate(&Value::I64(value)).unwrap();
        prop_assert_eq!(&output.result, &Value::F64(value as f64));
        prop_assert_eq!(lost(&output.loss), (value as f64) as i128 != i128::from(value));
    }
    #[test]
    fn f64_to_i64_truncation_inverts_exact_widening(value in -(1i64 << 53)..(1i64 << 53), fraction in 0.0f64..1.0) {
        let float = value as f64 + if value >= 0 { fraction } else { -fraction } * 0.5;
        let output = bind("f64", "i64", "truncate").evaluate(&Value::F64(float)).unwrap();
        prop_assert_eq!(output.result, Value::I64(float.trunc() as i64));
    }
    #[test]
    fn integer_round_trips_through_a_wider_type(value in any::<u32>()) {
        let wide = bind("u32", "i64", "").evaluate(&Value::U32(value)).unwrap().result;
        prop_assert_eq!(bind("i64", "u32", "").evaluate(&wide).unwrap().result, Value::U32(value));
    }
}

#[test]
fn compiled_converters_fold_results_and_keep_conversion_fault_provenance() {
    let (package, snapshot) = catalog();
    for (from, to, input, rounding, expected) in [
        (
            "f64",
            "i32",
            json!({"type":"core/f64@1","bits":format!("{:016x}", (-2.5f64).to_bits())}),
            "floor",
            Ok(Value::I32(-3)),
        ),
        (
            "u64",
            "i32",
            json!({"type":"core/u64@1","value":"4294967296"}),
            "",
            Err("core/OutOfRange"),
        ),
        (
            "f64",
            "u32",
            json!({"type":"core/f64@1","bits":format!("{:016x}", f64::NAN.to_bits())}),
            "truncate",
            Err("core/NonFinite"),
        ),
    ] {
        let config = ConverterConfig {
            entry: key(from, to),
            options: options(from, to, rounding),
        };
        let doc = orange_model::from_json(&json!({
            "format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000003","revision":0,
            "nodes":[{"id":id(1),"definition_id":"core/convert","definition_version":1,"config_version":1,
                "config":config.to_config(),"signature":"cast","inputs":{"value":input}}],
            "signatures":{"cast":{"kind":"pure","ports":[
                {"port_id":"value","direction":"input","kind":"data","type":format!("core/{from}@1")},
                {"port_id":"result","direction":"output","kind":"data","type":format!("core/{to}@1"),"source":"computed"}]}},
            "exports":[{"id":"answer","node":id(1),"port":"result"}]
        }).to_string()).unwrap();
        let analysis = orange_model::analyze(&doc, &snapshot);
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
        let folded = &plan.constants()[plan.exports()[0].slot.index()];
        match (folded, expected) {
            (FoldedValue::Value(value), Ok(expected)) => assert_eq!(value, &expected),
            (FoldedValue::Fault(fault), Err(code)) => {
                assert_eq!(fault.fault.code, code);
                assert_eq!(fault.source, id(1).parse().unwrap());
            }
            (other, expected) => panic!("{from}->{to}: {other:?} vs {expected:?}"),
        }
    }
}
