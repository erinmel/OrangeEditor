use std::collections::BTreeMap;
use std::sync::Arc;

use orange_bundled::{
    BindingError, ConversionError, ConversionOutput, ResolvedConversion, resolve_core_conversion,
};
use orange_model::Value;
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::conversion::{
    ConverterConfig, ConverterKey, ConverterProvider, ConverterRegistryBuilder, PackageProvider,
};
use orange_model::validation::HostOrigin;
use proptest::prelude::*;

fn snapshot() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(HostOrigin::new("0.0.0", format!("sha256:{}", "a".repeat(64))).unwrap())
}

fn config(id: &str) -> ConverterConfig {
    ConverterConfig {
        entry: ConverterKey {
            id: id.parse().unwrap(),
            contract_version: 1,
        },
        options: if id == "core/f64-to-f32" {
            [
                ("rounding".into(), "nearest-even".into()),
                ("overflow".into(), "reject".into()),
            ]
            .into()
        } else {
            BTreeMap::new()
        },
    }
}

fn conversion(id: &str) -> ResolvedConversion {
    resolve_core_conversion(snapshot(), &config(id), &ConverterProvider::Core).unwrap()
}

fn lost(output: &ConversionOutput) -> bool {
    let Some(Value::Json(text)) = &output.loss else {
        panic!("lossy conversion must expose JSON loss metadata");
    };
    let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 1);
    value["lost"].as_bool().unwrap()
}

#[test]
fn integer_boundaries_widen_exactly_and_narrow_without_saturation() {
    let widen = conversion("core/i32-to-i64");
    let narrow = conversion("core/i64-to-i32");
    for input in [i32::MIN, -1, 0, 1, i32::MAX] {
        let output = widen.evaluate(&Value::I32(input)).unwrap();
        assert_eq!(output.result, Value::I64(i64::from(input)));
        assert!(output.loss.is_none());
        let output = narrow.evaluate(&output.result).unwrap();
        assert_eq!(output.result, Value::I32(input));
        assert!(output.loss.is_none());
    }
    for input in [
        i64::MIN,
        i64::from(i32::MIN) - 1,
        i64::from(i32::MAX) + 1,
        i64::MAX,
    ] {
        assert_eq!(
            narrow.evaluate(&Value::I64(input)),
            Err(ConversionError::OutOfRange)
        );
    }
    assert_eq!(
        widen.evaluate(&Value::U32(1)),
        Err(ConversionError::InputContract)
    );
    assert_eq!(
        narrow.evaluate(&Value::I32(1)),
        Err(ConversionError::InputContract)
    );
}

#[test]
fn floating_widening_preserves_zero_infinity_and_nan_payload_bits() {
    let widen = conversion("core/f32-to-f64");
    for (source, target) in [
        (0_u32, 0_u64),
        (0x8000_0000, 0x8000_0000_0000_0000),
        (0x7f80_0000, 0x7ff0_0000_0000_0000),
        (0xff80_0000, 0xfff0_0000_0000_0000),
        (0x7fc0_0001, 0x7ff8_0000_2000_0000),
        (0xff80_0001, 0xfff0_0000_2000_0000),
        (0x7f80_0001, 0x7ff0_0000_2000_0000),
    ] {
        let output = widen.evaluate(&Value::F32(f32::from_bits(source))).unwrap();
        assert_eq!(output.result, Value::F64(f64::from_bits(target)));
        assert!(output.loss.is_none());
    }
    assert_eq!(
        widen.evaluate(&Value::F64(1.0)),
        Err(ConversionError::InputContract)
    );
}

#[test]
fn narrowing_uses_ties_to_even_reports_underflow_and_preserves_signed_zero() {
    let narrow = conversion("core/f64-to-f32");
    let even_low_tie = 1.0_f64 + 2.0_f64.powi(-24);
    let odd_low_tie = 1.0_f64 + 3.0 * 2.0_f64.powi(-24);
    for (source, target, expected_loss) in [
        (1.0, 1.0_f32, false),
        (even_low_tie, 1.0, true),
        (odd_low_tie, f32::from_bits(1.0_f32.to_bits() + 2), true),
        (f64::MIN_POSITIVE, 0.0, true),
        (-f64::MIN_POSITIVE, -0.0, true),
        (0.0, 0.0, false),
        (-0.0, -0.0, false),
        (f64::INFINITY, f32::INFINITY, false),
        (f64::NEG_INFINITY, f32::NEG_INFINITY, false),
    ] {
        let output = narrow.evaluate(&Value::F64(source)).unwrap();
        assert_eq!(
            output.result,
            Value::F32(target),
            "source bits {:016x}",
            source.to_bits()
        );
        assert_eq!(lost(&output), expected_loss);
    }
    assert_eq!(
        narrow.evaluate(&Value::F32(1.0)),
        Err(ConversionError::InputContract)
    );
}

#[test]
fn narrowing_rejects_every_finite_value_outside_the_target_range() {
    let narrow = conversion("core/f64-to-f32");
    let max = f64::from(f32::MAX);
    let just_above = f64::from_bits(max.to_bits() + 1);
    // A raw cast would round this out-of-range value down; reject is stricter.
    assert_eq!(just_above as f32, f32::MAX);
    for value in [just_above, -just_above, f64::MAX, -f64::MAX] {
        assert_eq!(
            narrow.evaluate(&Value::F64(value)),
            Err(ConversionError::OutOfRange)
        );
    }
    let output = narrow.evaluate(&Value::F64(max)).unwrap();
    assert_eq!(output.result, Value::F32(f32::MAX));
    assert!(!lost(&output));
}

#[test]
fn narrowing_nan_payload_loss_is_explicit_and_never_turns_nan_into_infinity() {
    let narrow = conversion("core/f64-to-f32");
    for (source, target, expected_loss) in [
        (0x7ff8_0000_0000_0000_u64, 0x7fc0_0000_u32, false),
        (0x7ff8_0000_0000_0001, 0x7fc0_0000, true),
        (0x7ff0_0000_0000_0001, 0x7f80_0001, true),
        (0xfff0_0000_2000_0000, 0xff80_0001, false),
    ] {
        let output = narrow
            .evaluate(&Value::F64(f64::from_bits(source)))
            .unwrap();
        assert_eq!(output.result, Value::F32(f32::from_bits(target)));
        assert_eq!(lost(&output), expected_loss);
    }
    // Repeated calls share the already-validated immutable metadata allocation.
    let one = narrow.evaluate(&Value::F64(1.0)).unwrap();
    let two = narrow.evaluate(&Value::F64(2.0)).unwrap();
    let (Some(Value::Json(one)), Some(Value::Json(two))) = (one.loss, two.loss) else {
        panic!("expected loss flags");
    };
    assert!(Arc::ptr_eq(&one, &two));
}

#[test]
fn binding_requires_exact_core_identity_and_complete_saved_options() {
    let snapshot = snapshot();
    let mut incomplete = config("core/f64-to-f32");
    incomplete.options.clear();
    assert_eq!(
        resolve_core_conversion(Arc::clone(&snapshot), &incomplete, &ConverterProvider::Core)
            .unwrap_err(),
        BindingError::InvalidConfig
    );
    incomplete
        .options
        .insert("rounding".into(), "nearest-even".into());
    assert_eq!(
        resolve_core_conversion(Arc::clone(&snapshot), &incomplete, &ConverterProvider::Core)
            .unwrap_err(),
        BindingError::InvalidConfig
    );
    for (name, value) in [
        ("rounding", "truncate"),
        ("overflow", "saturating"),
        ("extra", "private-value"),
    ] {
        let mut invalid = config("core/f64-to-f32");
        invalid.options.insert(name.into(), value.into());
        assert_eq!(
            resolve_core_conversion(Arc::clone(&snapshot), &invalid, &ConverterProvider::Core)
                .unwrap_err(),
            BindingError::InvalidConfig
        );
    }
    let mut future = config("core/i32-to-i64");
    future.entry.contract_version = 2;
    assert_eq!(
        resolve_core_conversion(Arc::clone(&snapshot), &future, &ConverterProvider::Core)
            .unwrap_err(),
        BindingError::Unavailable
    );
    let unknown = config("core/unknown-conversion");
    assert_eq!(
        resolve_core_conversion(Arc::clone(&snapshot), &unknown, &ConverterProvider::Core)
            .unwrap_err(),
        BindingError::Unavailable
    );
    let mut extra = config("core/i32-to-i64");
    extra
        .options
        .insert("rounding".into(), "nearest-even".into());
    assert_eq!(
        resolve_core_conversion(Arc::clone(&snapshot), &extra, &ConverterProvider::Core)
            .unwrap_err(),
        BindingError::InvalidConfig
    );
    let missing = RegistryBuilder::core(snapshot.origin().clone())
        .converters(ConverterRegistryBuilder::new(Arc::clone(snapshot.types())).build())
        .build()
        .unwrap();
    assert_eq!(
        resolve_core_conversion(
            missing,
            &config("core/i32-to-i64"),
            &ConverterProvider::Core
        )
        .unwrap_err(),
        BindingError::Unavailable
    );
    let package = ConverterProvider::Package(PackageProvider {
        package: "org.example".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "b".repeat(64)),
        id: "selected".into(),
        contract_version: 1,
    });
    assert_eq!(
        resolve_core_conversion(snapshot, &config("core/i32-to-i64"), &package).unwrap_err(),
        BindingError::Unavailable
    );
    for error in [ConversionError::OutOfRange, ConversionError::InputContract] {
        assert!(error.code().starts_with("core/"));
        assert!(error.message_key().starts_with("core/"));
        assert!(!error.to_string().contains("private-value"));
    }
}

#[test]
fn a_saved_converter_graph_validates_its_result_and_loss_ports() {
    let snapshot = snapshot();
    let config = config("core/f64-to-f32");
    let id = "10000000-0000-4000-8000-000000000001";
    let doc = orange_model::from_json(&serde_json::json!({
        "format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001","revision":0,
        "nodes":[{"id":id,"definition_id":"core/convert","definition_version":1,"config_version":1,
            "config":config.to_config(),"signature":"narrow", "inputs":{"value":{"type":"core/f64@1","bits":"3ff0000010000000"}}}],
        "signatures":{"narrow":{"kind":"pure","ports":[
            {"port_id":"value","direction":"input","kind":"data","type":"core/f64@1"},
            {"port_id":"result","direction":"output","kind":"data","type":"core/f32@1","source":"computed"},
            {"port_id":"loss","direction":"output","kind":"data","type":"core/json@1","source":"computed"}]}},
        "exports":[{"id":"answer","node":id,"port":"result"}]
    }).to_string()).unwrap();
    let before = (
        orange_model::to_json(&doc).unwrap(),
        doc.state_id(),
        doc.revision(),
    );
    let analysis = orange_model::analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    assert!(!analysis.is_ready_for_compilation(&doc, &snapshot));
    let node = &doc.root().nodes[&id.parse::<orange_model::id::NodeId>().unwrap()];
    assert!(node.provider.is_none());
    let config = ConverterConfig::from_config(&node.config).unwrap();
    let bound =
        resolve_core_conversion(Arc::clone(&snapshot), &config, &ConverterProvider::Core).unwrap();
    let input = orange_model::value::literal_codec::decode(
        &node.inputs[&"value".parse().unwrap()],
        snapshot.types().as_ref(),
    )
    .unwrap();
    let result = bound.evaluate(&input).unwrap();
    assert_eq!(result.result, Value::F32(1.0));
    assert!(lost(&result));
    assert_eq!(
        (
            orange_model::to_json(&doc).unwrap(),
            doc.state_id(),
            doc.revision()
        ),
        before
    );
}

#[test]
fn core_bindings_pin_metadata_and_ignore_later_defaults_without_guest_fallback() {
    let old = snapshot();
    let config = config("core/f64-to-f32");
    let pinned =
        resolve_core_conversion(Arc::clone(&old), &config, &ConverterProvider::Core).unwrap();
    let mut converters =
        ConverterRegistryBuilder::with_core_conversions(Arc::clone(old.types())).unwrap();
    let alternative = PackageProvider {
        package: "org.example".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "b".repeat(64)),
        id: "selected-guest".into(),
        contract_version: 1,
    };
    converters
        .register_package_provider("org.example", &config.entry, alternative.clone())
        .unwrap();
    let alternative = ConverterProvider::Package(alternative);
    converters
        .set_new_binding_default(&config.entry, alternative.clone())
        .unwrap();
    let new = RegistryBuilder::core(
        HostOrigin::new("0.0.0", format!("sha256:{}", "b".repeat(64))).unwrap(),
    )
    .converters(converters.build())
    .build()
    .unwrap();
    assert_eq!(
        new.converters().new_binding_default(&config.entry),
        Some(&alternative)
    );
    let current =
        resolve_core_conversion(Arc::clone(&new), &config, &ConverterProvider::Core).unwrap();
    assert_eq!(
        resolve_core_conversion(new, &config, &alternative).unwrap_err(),
        BindingError::Unavailable
    );
    drop(old);
    assert_ne!(
        pinned.snapshot().generation(),
        current.snapshot().generation()
    );
    assert_eq!(pinned.entry(), &config.entry);
    assert_eq!(pinned.provider(), ConverterProvider::Core);
    let result = pinned.evaluate(&Value::F64(1.0)).unwrap();
    assert_eq!(result.result, Value::F32(1.0));
    assert!(!lost(&result));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn every_sampled_f32_bit_pattern_roundtrips_without_loss(bits: u32) {
        let wide = conversion("core/f32-to-f64").evaluate(&Value::F32(f32::from_bits(bits))).unwrap();
        let narrow = conversion("core/f64-to-f32").evaluate(&wide.result).unwrap();
        prop_assert_eq!(&narrow.result, &Value::F32(f32::from_bits(bits)));
        prop_assert!(!lost(&narrow));
    }
    #[test]
    fn sampled_i32_values_widen_and_narrow_exactly(value: i32) {
        let wide = conversion("core/i32-to-i64").evaluate(&Value::I32(value)).unwrap();
        let narrow = conversion("core/i64-to-i32").evaluate(&wide.result).unwrap();
        prop_assert_eq!(narrow.result, Value::I32(value));
        prop_assert!(narrow.loss.is_none());
    }
}
