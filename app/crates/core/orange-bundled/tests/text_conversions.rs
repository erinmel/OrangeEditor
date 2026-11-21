//! L03a: invariant text parse/display and UTF-8 encode/decode Core converters.
use std::collections::BTreeMap;
use std::sync::Arc;

use orange_bundled::{BindingError, ConversionError, ResolvedConversion, resolve_core_conversion};
use orange_compiler::{CompileOptions, FoldedValue, compile_pure};
use orange_model::Value;
use orange_model::conversion::{ConversionClass, ConverterConfig, ConverterKey, ConverterProvider};
use proptest::prelude::*;
use serde_json::json;

use crate::data_graph::{catalog, id};

const TEXTUAL: [&str; 7] = ["bool", "i32", "i64", "u32", "u64", "f32", "f64"];

fn key(id: &str) -> ConverterKey {
    ConverterKey {
        id: id.parse().unwrap(),
        contract_version: 1,
    }
}
fn options(id: &str) -> BTreeMap<String, String> {
    match id {
        "core/string-to-f32" | "core/string-to-f64" => {
            [("rounding".into(), "nearest-even".into())].into()
        }
        "core/bytes-to-string-lossy" => [("invalid".into(), "replace".into())].into(),
        _ => BTreeMap::new(),
    }
}
fn bind(id: &str) -> ResolvedConversion {
    let (_, snapshot) = catalog();
    resolve_core_conversion(
        snapshot,
        &ConverterConfig {
            entry: key(id),
            options: options(id),
        },
        &ConverterProvider::Core,
    )
    .unwrap()
}
fn text(value: &str) -> Value {
    Value::Str(Arc::new(value.into()))
}
fn display(ty: &str, value: Value) -> String {
    match bind(&format!("core/{ty}-to-string"))
        .evaluate(&value)
        .unwrap()
        .result
    {
        Value::Str(text) => text.as_str().to_owned(),
        other => panic!("{other:?}"),
    }
}
fn parse(ty: &str, value: &str) -> Result<Value, ConversionError> {
    bind(&format!("core/string-to-{ty}"))
        .evaluate(&text(value))
        .map(|output| output.result)
}
fn lost(loss: &Option<Value>) -> bool {
    let Some(Value::Json(json)) = loss else {
        panic!("lossy entries always report loss")
    };
    serde_json::from_str::<serde_json::Value>(json.as_str()).unwrap()["lost"]
        .as_bool()
        .unwrap()
}

#[test]
fn registry_has_text_entries_with_exact_classes_and_errors() {
    let (_, snapshot) = catalog();
    let registry = snapshot.converters();
    for ty in TEXTUAL {
        let float = ty.starts_with('f');
        let shown = registry
            .entry(&key(&format!("core/{ty}-to-string")))
            .unwrap();
        assert_eq!(
            shown.class,
            if float {
                ConversionClass::Lossy
            } else {
                ConversionClass::LosslessTotal
            }
        );
        let parsed = registry
            .entry(&key(&format!("core/string-to-{ty}")))
            .unwrap();
        assert_eq!(parsed.class, ConversionClass::CheckedPartial);
        let errors: Vec<_> = parsed.errors.iter().map(|e| e.as_str()).collect();
        if ty == "bool" {
            assert_eq!(errors, ["core/Parse"]);
        } else {
            assert_eq!(errors, ["core/OutOfRange", "core/Parse"]);
        }
    }
    for (id, class) in [
        ("core/string-to-bytes", ConversionClass::LosslessTotal),
        ("core/bytes-to-string", ConversionClass::CheckedPartial),
        ("core/bytes-to-string-lossy", ConversionClass::Lossy),
    ] {
        assert_eq!(registry.entry(&key(id)).unwrap().class, class);
    }
}

#[test]
fn display_is_invariant_decimal_with_shortest_round_trip_floats() {
    assert_eq!(display("bool", Value::Bool(true)), "true");
    assert_eq!(display("i32", Value::I32(i32::MIN)), "-2147483648");
    assert_eq!(display("i64", Value::I64(i64::MAX)), "9223372036854775807");
    assert_eq!(display("u64", Value::U64(u64::MAX)), "18446744073709551615");
    assert_eq!(display("u32", Value::U32(0)), "0");
    for (value, expected) in [
        (0.0, "0"),
        (-0.0, "-0"),
        (1.0, "1"),
        (-1.5, "-1.5"),
        (0.1, "0.1"),
        (123.456, "123.456"),
        (1e20, "100000000000000000000"),
        (1e21, "1e+21"),
        (1.2345678901234568e20, "123456789012345680000"),
        (0.000001, "0.000001"),
        (1e-7, "1e-7"),
        (-1.5e-7, "-1.5e-7"),
        (5e-324, "5e-324"),
        (f64::MAX, "1.7976931348623157e+308"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (f64::NAN, "NaN"),
    ] {
        assert_eq!(display("f64", Value::F64(value)), expected, "{value:e}");
    }
    for (value, expected) in [
        (0.1f32, "0.1"),
        (f32::MAX, "3.4028235e+38"),
        (16_777_216.0, "16777216"),
        (-0.0, "-0"),
    ] {
        assert_eq!(display("f32", Value::F32(value)), expected);
    }
    // Only a NaN that is not the canonical positive quiet NaN loses information.
    let canonical = bind("core/f64-to-string")
        .evaluate(&Value::F64(f64::NAN))
        .unwrap();
    assert!(!lost(&canonical.loss));
    let payload = bind("core/f64-to-string")
        .evaluate(&Value::F64(f64::from_bits(0x7ff8_0000_0000_0123)))
        .unwrap();
    assert!(lost(&payload.loss));
    assert!(!lost(
        &bind("core/f32-to-string")
            .evaluate(&Value::F32(-1.5))
            .unwrap()
            .loss
    ));
    assert!(
        bind("core/i64-to-string")
            .evaluate(&Value::I64(1))
            .unwrap()
            .loss
            .is_none()
    );
}

#[test]
fn integer_and_bool_parsing_accepts_only_the_invariant_grammar() {
    assert_eq!(parse("bool", "true"), Ok(Value::Bool(true)));
    assert_eq!(parse("bool", "false"), Ok(Value::Bool(false)));
    for bad in ["True", "1", "", " true", "yes"] {
        assert_eq!(parse("bool", bad), Err(ConversionError::Parse), "{bad:?}");
    }
    assert_eq!(parse("i64", "+7"), Ok(Value::I64(7)));
    assert_eq!(parse("i64", "-007"), Ok(Value::I64(-7)));
    assert_eq!(parse("u32", "-0"), Ok(Value::U32(0)));
    assert_eq!(parse("i32", "-2147483648"), Ok(Value::I32(i32::MIN)));
    assert_eq!(
        parse("u64", "18446744073709551615"),
        Ok(Value::U64(u64::MAX))
    );
    for bad in [
        "",
        "-",
        "+",
        " 1",
        "1 ",
        "1_000",
        "1,000",
        "0x10",
        "1.0",
        "1e3",
        "\u{0661}",
        "--1",
        "99999999999999999999999999999999999999999x",
    ] {
        for ty in ["i32", "i64", "u32", "u64"] {
            assert_eq!(parse(ty, bad), Err(ConversionError::Parse), "{ty} {bad:?}");
        }
    }
    for (ty, value) in [
        ("i32", "2147483648"),
        ("i32", "-2147483649"),
        ("u32", "4294967296"),
        ("u64", "-1"),
        ("i64", "9223372036854775808"),
        ("u64", "18446744073709551616"),
        ("i64", "999999999999999999999999999999999999999999999999"),
    ] {
        assert_eq!(
            parse(ty, value),
            Err(ConversionError::OutOfRange),
            "{ty} {value}"
        );
    }
}

#[test]
fn float_parsing_is_correctly_rounded_in_the_target_precision() {
    for (value, expected) in [
        ("1.5", 1.5),
        (".5", 0.5),
        ("5.", 5.0),
        ("+1e3", 1000.0),
        ("1E+3", 1000.0),
        ("2.5e-3", 0.0025),
        ("1e-400", 0.0),
        ("Infinity", f64::INFINITY),
        ("-Infinity", f64::NEG_INFINITY),
        ("0.1", 0.1),
    ] {
        assert_eq!(parse("f64", value), Ok(Value::F64(expected)), "{value}");
    }
    assert_eq!(parse("f64", "-0"), Ok(Value::F64(-0.0)));
    let Ok(Value::F64(nan)) = parse("f64", "NaN") else {
        panic!()
    };
    assert!(nan.is_nan());
    for bad in [
        "", ".", "e1", "1e", "1e+", "inf", "infinity", "nan", "+NaN", " 1", "1 ", "1,5", "0x1p3",
        "1.5f", "--1",
    ] {
        assert_eq!(parse("f64", bad), Err(ConversionError::Parse), "{bad:?}");
        assert_eq!(parse("f32", bad), Err(ConversionError::Parse), "{bad:?}");
    }
    assert_eq!(parse("f64", "1e400"), Err(ConversionError::OutOfRange));
    assert_eq!(parse("f32", "1e39"), Err(ConversionError::OutOfRange));
    // Just above the halfway point between 1 and the next binary32 value: a binary64
    // intermediate would land exactly on the tie and round down to 1.
    assert_eq!(
        parse("f32", "1.000000059604644775390626"),
        Ok(Value::F32(f32::from_bits(1.0f32.to_bits() + 1)))
    );
}

#[test]
fn utf8_encode_decode_rejects_or_replaces_invalid_sequences() {
    let encoded = bind("core/string-to-bytes")
        .evaluate(&text("h\u{e9}llo \u{1f34a}"))
        .unwrap();
    let Value::Bytes(bytes) = &encoded.result else {
        panic!()
    };
    assert_eq!(bytes.as_slice(), "h\u{e9}llo \u{1f34a}".as_bytes());
    assert!(encoded.loss.is_none());
    let strict = bind("core/bytes-to-string");
    let lossy = bind("core/bytes-to-string-lossy");
    assert_eq!(
        strict.evaluate(&encoded.result).unwrap().result,
        text("h\u{e9}llo \u{1f34a}")
    );
    assert!(!lost(&lossy.evaluate(&encoded.result).unwrap().loss));
    for invalid in [
        vec![0xff],
        vec![b'a', 0xc3],
        vec![0xed, 0xa0, 0x80],
        vec![0xc0, 0xaf],
        vec![0xf4, 0x90, 0x80, 0x80],
    ] {
        let input = Value::Bytes(Arc::new(invalid.clone()));
        assert_eq!(strict.evaluate(&input), Err(ConversionError::InvalidUtf8));
        let output = lossy.evaluate(&input).unwrap();
        let Value::Str(replaced) = &output.result else {
            panic!()
        };
        assert!(replaced.contains('\u{fffd}'), "{invalid:?}");
        assert!(lost(&output.loss));
    }
    assert_eq!(
        strict.evaluate(&text("not bytes")),
        Err(ConversionError::InputContract)
    );
    for error in [ConversionError::Parse, ConversionError::InvalidUtf8] {
        assert!(error.code().starts_with("core/"));
        assert!(error.message_key().starts_with("core/conversion-"));
    }
}

#[test]
fn text_bindings_require_their_fixed_options() {
    let (_, snapshot) = catalog();
    let check = |id: &str, options: &[(&str, &str)]| {
        resolve_core_conversion(
            snapshot.clone(),
            &ConverterConfig {
                entry: key(id),
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
    assert_eq!(check("core/string-to-f64", &[]), invalid);
    assert_eq!(
        check("core/string-to-f64", &[("rounding", "truncate")]),
        invalid
    );
    assert_eq!(check("core/bytes-to-string-lossy", &[]), invalid);
    assert_eq!(
        check("core/bool-to-string", &[("rounding", "nearest-even")]),
        invalid
    );
    assert_eq!(
        check("core/bytes-to-string", &[("invalid", "replace")]),
        invalid
    );
    assert_eq!(check("core/string-to-i64", &[]), Ok(()));
    assert_eq!(
        check("core/string-to-duration", &[]),
        Err(BindingError::Unavailable)
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn f64_display_parses_back_to_identical_bits(bits in any::<u64>()) {
        let value = f64::from_bits(bits);
        prop_assume!(!value.is_nan());
        let shown = display("f64", Value::F64(value));
        prop_assert_eq!(parse("f64", &shown), Ok(Value::F64(value)), "{}", shown);
    }
    #[test]
    fn f32_display_parses_back_to_identical_bits(bits in any::<u32>()) {
        let value = f32::from_bits(bits);
        prop_assume!(!value.is_nan());
        let shown = display("f32", Value::F32(value));
        prop_assert_eq!(parse("f32", &shown), Ok(Value::F32(value)), "{}", shown);
    }
    #[test]
    fn integer_display_parses_back(value in any::<i64>(), unsigned in any::<u64>()) {
        prop_assert_eq!(parse("i64", &display("i64", Value::I64(value))), Ok(Value::I64(value)));
        prop_assert_eq!(parse("u64", &display("u64", Value::U64(unsigned))), Ok(Value::U64(unsigned)));
    }
    #[test]
    fn strict_utf8_decoding_inverts_encoding(value in ".{0,64}") {
        let bytes = bind("core/string-to-bytes").evaluate(&text(&value)).unwrap().result;
        prop_assert_eq!(bind("core/bytes-to-string").evaluate(&bytes).unwrap().result, text(&value));
    }
}

#[test]
fn compiled_text_converters_fold_and_keep_parse_fault_provenance() {
    let (package, snapshot) = catalog();
    for (from, to, id_text, input, expected) in [
        (
            "string",
            "i64",
            "core/string-to-i64",
            json!({"type":"core/string@1","value":"-42"}),
            Ok(Value::I64(-42)),
        ),
        (
            "string",
            "u32",
            "core/string-to-u32",
            json!({"type":"core/string@1","value":"4x"}),
            Err("core/Parse"),
        ),
        (
            "i64",
            "string",
            "core/i64-to-string",
            json!({"type":"core/i64@1","value":"-9"}),
            Ok(text("-9")),
        ),
    ] {
        let config = ConverterConfig {
            entry: key(id_text),
            options: options(id_text),
        };
        let doc = orange_model::from_json(&json!({
            "format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000004","revision":0,
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
        match (&plan.constants()[plan.exports()[0].slot.index()], expected) {
            (FoldedValue::Value(value), Ok(expected)) => assert_eq!(value, &expected),
            (FoldedValue::Fault(fault), Err(code)) => {
                assert_eq!(fault.fault.code, code);
                assert_eq!(fault.source, id(1).parse().unwrap());
            }
            (other, expected) => panic!("{id_text}: {other:?} vs {expected:?}"),
        }
    }
}
