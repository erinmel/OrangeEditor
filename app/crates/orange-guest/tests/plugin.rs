//! K02b: the Rust probe guest (sdk/guests/probe, built into fixtures/probe.wasm) runs
//! through the `orange:expansion/plugin@1.0.0` world with grant-scoped imports and
//! `orange.value.v1` literal values.
use std::sync::Arc;

use orange_guest::{
    API_VERSION, Grants, GuestEngine, GuestError, GuestLimits, GuestNodeError, GuestPlugin,
    GuestTrap, MAX_LOG_ENTRIES,
};
use orange_model::Value;
use orange_model::id::TypeExpr;
use orange_model::types::{TypeRegistry, TypeRegistryBuilder};
use orange_model::value::{ListRepr, MapRepr};

/// The committed fixture, built by `node tools/build-guest-fixtures.mjs`. CI rebuilds the
/// guest from source and points `ORANGE_GUEST_PROBE` at the fresh component.
const PROBE: &[u8] = include_bytes!("fixtures/probe.wasm");

pub(crate) fn probe_bytes() -> Vec<u8> {
    match std::env::var_os("ORANGE_GUEST_PROBE") {
        Some(path) => std::fs::read(path).expect("ORANGE_GUEST_PROBE names a readable component"),
        None => PROBE.to_vec(),
    }
}
const LOG: Grants = Grants { log: true };

fn types() -> TypeRegistry {
    TypeRegistryBuilder::core().build().unwrap()
}
fn ty(text: &str) -> TypeExpr {
    text.parse().unwrap()
}
fn plugin(limits: GuestLimits, grants: Grants) -> Result<GuestPlugin, GuestError> {
    let engine = GuestEngine::new(limits).unwrap();
    GuestPlugin::new(&engine.compile(&probe_bytes()).unwrap(), grants)
}
fn i64s(values: &[i64]) -> Vec<(TypeExpr, Value)> {
    values
        .iter()
        .map(|v| (ty("core/i64@1"), Value::I64(*v)))
        .collect()
}

#[test]
fn typed_values_cross_the_boundary_and_node_errors_stay_data() {
    let types = types();
    let mut probe = plugin(GuestLimits::default(), LOG).unwrap();
    let out = [ty("core/i64@1")];
    assert_eq!(
        probe.evaluate(&types, "org.example.probe/add", &i64s(&[40, 2]), &out),
        Ok(Ok(vec![Value::I64(42)]))
    );
    assert_eq!(
        probe.evaluate(&types, "org.example.probe/add", &i64s(&[i64::MAX, 1]), &out),
        Ok(Err(GuestNodeError {
            code: "core/NumericOverflow".into(),
            message_key: "org.example.probe/failed".into()
        }))
    );
    assert_eq!(
        probe.evaluate(&types, "org.example.probe/api-major", &[], &out),
        Ok(Ok(vec![Value::I64(i64::from(API_VERSION.0))]))
    );
    // A guest-reported node error does not poison the instance.
    assert!(!probe.is_poisoned());
}

#[test]
fn nested_values_round_trip_exactly_through_the_literal_codec() {
    let types = types();
    let mut probe = plugin(GuestLimits::default(), LOG).unwrap();
    let nested = ty("core/list@1<core/map@1<core/string@1,core/f64@1>>");
    let map = MapRepr::new(vec![
        (
            Value::Str(Arc::new("nan".into())),
            Value::F64(f64::from_bits(0x7ff8_0000_0000_0123)),
        ),
        (Value::Str(Arc::new("neg-zero".into())), Value::F64(-0.0)),
    ])
    .unwrap();
    let value = Value::List(Arc::new(ListRepr::Values(
        vec![Value::Map(Arc::new(map))].into_boxed_slice(),
    )));
    let inputs = [
        (nested.clone(), value.clone()),
        (ty("core/option@1<core/i64@1>"), Value::None),
    ];
    let result = probe
        .evaluate(
            &types,
            "org.example.probe/echo-logged",
            &inputs,
            &[nested, ty("core/option@1<core/i64@1>")],
        )
        .unwrap()
        .unwrap();
    // Float bits, signed zero, map order and None survive the text boundary.
    assert_eq!(result, vec![value, Value::None]);
    assert_eq!(probe.take_log(), ["echo 2 inputs"]);
}

#[test]
fn imports_are_linked_only_when_granted_and_logging_is_bounded() {
    assert_eq!(
        plugin(GuestLimits::default(), Grants::default()).unwrap_err(),
        GuestError::Link
    );
    let types = types();
    let mut probe = plugin(GuestLimits::default(), LOG).unwrap();
    for _ in 0..MAX_LOG_ENTRIES + 6 {
        probe
            .evaluate(&types, "org.example.probe/echo-logged", &[], &[])
            .unwrap()
            .unwrap();
    }
    assert_eq!(probe.take_log().len(), MAX_LOG_ENTRIES);
    assert!(probe.take_log().is_empty());
}

#[test]
fn outputs_outside_the_contract_are_refused() {
    let types = types();
    let mut probe = plugin(GuestLimits::default(), LOG).unwrap();
    let out = [ty("core/i64@1")];
    assert_eq!(
        probe.evaluate(&types, "org.example.probe/wrong-type", &[], &out),
        Err(GuestError::OutputContract)
    );
    assert_eq!(
        probe.evaluate(&types, "org.example.probe/bad-error", &[], &out),
        Err(GuestError::OutputContract)
    );
    assert_eq!(
        probe.evaluate(&types, "org.example.probe/add", &i64s(&[1, 2]), &[]),
        Err(GuestError::OutputContract)
    );
    // A host value that does not have its declared type never reaches the guest.
    assert_eq!(
        probe.evaluate(
            &types,
            "org.example.probe/add",
            &[
                (ty("core/i64@1"), Value::Bool(true)),
                (ty("core/i64@1"), Value::I64(1))
            ],
            &out
        ),
        Err(GuestError::InputContract)
    );
    assert!(!probe.is_poisoned());
}

#[test]
fn panics_and_runaway_guests_fault_only_their_instance() {
    let types = types();
    let mut probe = plugin(GuestLimits::default(), LOG).unwrap();
    assert_eq!(
        probe.evaluate(&types, "org.example.probe/trap", &[], &[]),
        Err(GuestError::Trap(GuestTrap::Unreachable))
    );
    assert_eq!(
        probe.evaluate(
            &types,
            "org.example.probe/add",
            &i64s(&[1, 1]),
            &[ty("core/i64@1")]
        ),
        Err(GuestError::Poisoned)
    );
    let mut spinner = plugin(
        GuestLimits {
            fuel_per_call: 1_000_000,
            ..GuestLimits::default()
        },
        LOG,
    )
    .unwrap();
    assert_eq!(
        spinner.evaluate(&types, "org.example.probe/spin", &[], &[]),
        Err(GuestError::Trap(GuestTrap::OutOfFuel))
    );
    let mut fresh = plugin(GuestLimits::default(), LOG).unwrap();
    assert_eq!(
        fresh.evaluate(
            &types,
            "org.example.probe/add",
            &i64s(&[2, 3]),
            &[ty("core/i64@1")]
        ),
        Ok(Ok(vec![Value::I64(5)]))
    );
}
