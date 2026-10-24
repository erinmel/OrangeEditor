use std::sync::Arc;

use orange_bundled::{KernelError, MathPackage, Operation, ResolvedKernel};
use orange_model::Value;
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot, TypeArgs};
use orange_model::document::DefinitionRef;
use orange_model::validation::HostOrigin;
use proptest::prelude::*;
use serde_json::{Value as Json, json};

fn catalog() -> (MathPackage, Arc<RegistrySnapshot>) {
    let digest = format!("sha256:{}", "a".repeat(64));
    let package = MathPackage::new(digest.clone()).unwrap();
    let snapshot = package
        .register(RegistryBuilder::core(
            HostOrigin::new("0.0.0", digest).unwrap(),
        ))
        .unwrap()
        .build()
        .unwrap();
    (package, snapshot)
}

fn kernel(operation: Operation, ty: &str, config: Json) -> ResolvedKernel {
    let (package, snapshot) = catalog();
    let definition = DefinitionRef {
        id: operation.definition_id().parse().unwrap(),
        version: 1,
    };
    let args: TypeArgs = if operation == Operation::Not {
        Default::default()
    } else {
        [("T".into(), ty.parse().unwrap())].into()
    };
    package
        .resolve(
            snapshot,
            package.provider_key(),
            &definition,
            1,
            &args,
            config.as_object().unwrap(),
        )
        .unwrap()
}

#[test]
fn known_answers_cover_all_scalar_specializations_and_operations() {
    macro_rules! integers {
        ($variant:ident, $ty:literal) => {
            for (operation, expected) in [
                (Operation::Add, 9),
                (Operation::Subtract, 5),
                (Operation::Multiply, 14),
                (Operation::Divide, 3),
            ] {
                assert_eq!(
                    kernel(operation, $ty, json!({}))
                        .evaluate(&[Value::$variant(7), Value::$variant(2)]),
                    Ok(Value::$variant(expected))
                );
            }
        };
    }
    integers!(I32, "core/i32@1");
    integers!(I64, "core/i64@1");
    integers!(U32, "core/u32@1");
    integers!(U64, "core/u64@1");
    for (operation, expected) in [
        (Operation::Add, 9.0),
        (Operation::Subtract, 5.0),
        (Operation::Multiply, 14.0),
        (Operation::Divide, 3.5),
    ] {
        assert_eq!(
            kernel(operation, "core/f32@1", json!({}))
                .evaluate(&[Value::F32(7.0), Value::F32(2.0)]),
            Ok(Value::F32(expected as f32))
        );
        assert_eq!(
            kernel(operation, "core/f64@1", json!({}))
                .evaluate(&[Value::F64(7.0), Value::F64(2.0)]),
            Ok(Value::F64(expected))
        );
    }
    let not = kernel(Operation::Not, "", json!({}));
    assert_eq!(not.evaluate(&[Value::Bool(true)]), Ok(Value::Bool(false)));
    assert_eq!(not.evaluate(&[Value::Bool(false)]), Ok(Value::Bool(true)));
    assert_eq!(
        not.evaluate(&[Value::I64(1)]),
        Err(KernelError::InputContract)
    );
    assert_eq!(not.evaluate(&[]), Err(KernelError::InputContract));
}

#[test]
fn integer_modes_cover_overflow_and_underflow_at_every_width() {
    macro_rules! limits {
        ($variant:ident, $primitive:ident, $ty:literal) => {{
            let checked = kernel(Operation::Add, $ty, json!({}));
            let wrapping = kernel(Operation::Add, $ty, json!({"int_overflow":"wrapping"}));
            let saturating = kernel(Operation::Add, $ty, json!({"int_overflow":"saturating"}));
            let inputs = [Value::$variant($primitive::MAX), Value::$variant(1)];
            assert_eq!(checked.evaluate(&inputs), Err(KernelError::NumericOverflow));
            assert_eq!(wrapping.evaluate(&inputs), Ok(Value::$variant($primitive::MIN)));
            assert_eq!(saturating.evaluate(&inputs), Ok(Value::$variant($primitive::MAX)));
            let inputs = [Value::$variant($primitive::MAX), Value::$variant(2)];
            assert_eq!(kernel(Operation::Multiply, $ty, json!({})).evaluate(&inputs), Err(KernelError::NumericOverflow));
            let wrapped = (($primitive::MAX as u128) * 2) as $primitive;
            assert_eq!(kernel(Operation::Multiply, $ty, json!({"int_overflow":"wrapping"})).evaluate(&inputs), Ok(Value::$variant(wrapped)));
            assert_eq!(kernel(Operation::Multiply, $ty, json!({"int_overflow":"saturating"})).evaluate(&inputs), Ok(Value::$variant($primitive::MAX)));
            let inputs = [Value::$variant($primitive::MIN), Value::$variant(1)];
            assert_eq!(kernel(Operation::Subtract, $ty, json!({})).evaluate(&inputs), Err(KernelError::NumericOverflow));
            assert_eq!(kernel(Operation::Subtract, $ty, json!({"int_overflow":"wrapping"})).evaluate(&inputs), Ok(Value::$variant($primitive::MAX)));
            assert_eq!(kernel(Operation::Subtract, $ty, json!({"int_overflow":"saturating"})).evaluate(&inputs), Ok(Value::$variant($primitive::MIN)));
            for mode in ["checked", "wrapping", "saturating"] {
                assert_eq!(kernel(Operation::Divide, $ty, json!({"int_overflow":mode})).evaluate(&[Value::$variant(1), Value::$variant(0)]), Err(KernelError::DivideByZero));
            }
        }};
    }
    limits!(I32, i32, "core/i32@1");
    limits!(I64, i64, "core/i64@1");
    limits!(U32, u32, "core/u32@1");
    limits!(U64, u64, "core/u64@1");
}

#[test]
fn signed_division_handles_the_minimum_and_truncates_toward_zero() {
    macro_rules! signed {
        ($variant:ident, $primitive:ident, $ty:literal) => {
            let inputs = [Value::$variant($primitive::MIN), Value::$variant(-1)];
            assert_eq!(kernel(Operation::Divide, $ty, json!({})).evaluate(&inputs), Err(KernelError::NumericOverflow));
            assert_eq!(kernel(Operation::Divide, $ty, json!({"int_overflow":"wrapping"})).evaluate(&inputs), Ok(Value::$variant($primitive::MIN)));
            assert_eq!(kernel(Operation::Divide, $ty, json!({"int_overflow":"saturating"})).evaluate(&inputs), Ok(Value::$variant($primitive::MAX)));
            assert_eq!(kernel(Operation::Divide, $ty, json!({})).evaluate(&[Value::$variant(-7), Value::$variant(2)]), Ok(Value::$variant(-3)));
        };
    }
    signed!(I32, i32, "core/i32@1");
    signed!(I64, i64, "core/i64@1");
}

#[test]
fn finite_and_ieee_policies_preserve_precision_signed_zero_and_error_order() {
    macro_rules! floats {
        ($variant:ident, $primitive:ident, $ty:literal) => {{
            let finite = kernel(Operation::Divide, $ty, json!({}));
            assert_eq!(finite.evaluate(&[Value::$variant(1.0), Value::$variant(-0.0)]), Err(KernelError::DivideByZero));
            assert_eq!(finite.evaluate(&[Value::$variant($primitive::NAN), Value::$variant(0.0)]), Err(KernelError::NonFinite));
            let multiply = kernel(Operation::Multiply, $ty, json!({}));
            assert_eq!(multiply.evaluate(&[Value::$variant($primitive::MAX), Value::$variant(2.0)]), Err(KernelError::NonFinite));
            assert_eq!(multiply.evaluate(&[Value::$variant(-0.0), Value::$variant(2.0)]), Ok(Value::$variant(-0.0)));
            let ieee = kernel(Operation::Divide, $ty, json!({"float_policy":"ieee"}));
            assert_eq!(ieee.evaluate(&[Value::$variant(1.0), Value::$variant(-0.0)]), Ok(Value::$variant($primitive::NEG_INFINITY)));
            let Ok(Value::$variant(nan)) = ieee.evaluate(&[Value::$variant(0.0), Value::$variant(0.0)]) else { panic!("expected IEEE NaN"); };
            assert!(nan.is_nan());
            let Ok(Value::$variant(nan)) = ieee.evaluate(&[Value::$variant($primitive::NAN), Value::$variant(1.0)]) else { panic!("expected IEEE NaN"); };
            assert!(nan.is_nan());
        }};
    }
    floats!(F32, f32, "core/f32@1");
    floats!(F64, f64, "core/f64@1");
    assert_eq!(
        kernel(Operation::Add, "core/f32@1", json!({}))
            .evaluate(&[Value::F32(16_777_216.0), Value::F32(1.0)]),
        Ok(Value::F32(16_777_216.0))
    );
    assert_eq!(
        kernel(Operation::Add, "core/f64@1", json!({}))
            .evaluate(&[Value::F64(16_777_216.0), Value::F64(1.0)]),
        Ok(Value::F64(16_777_217.0))
    );
}

#[test]
fn wrong_arity_or_value_types_do_not_coerce_or_panic() {
    let add = kernel(Operation::Add, "core/i64@1", json!({}));
    for inputs in [
        vec![],
        vec![Value::I64(1)],
        vec![Value::I64(1); 3],
        vec![Value::I64(1), Value::U64(2)],
        vec![Value::Bool(true), Value::Bool(false)],
    ] {
        assert_eq!(add.evaluate(&inputs), Err(KernelError::InputContract));
    }
    for error in [
        KernelError::InputContract,
        KernelError::NumericOverflow,
        KernelError::DivideByZero,
        KernelError::NonFinite,
    ] {
        assert!(error.code().starts_with("core/"));
        assert!(error.message_key().starts_with("core/"));
        assert!(!error.to_string().contains("private"));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn checked_i64_arithmetic_matches_wider_integer_oracle(a: i64, b: i64) {
        for (operation, wide) in [(Operation::Add, i128::from(a) + i128::from(b)),
            (Operation::Subtract, i128::from(a) - i128::from(b)),
            (Operation::Multiply, i128::from(a) * i128::from(b))] {
            let expected = i64::try_from(wide).map(Value::I64).map_err(|_| KernelError::NumericOverflow);
            prop_assert_eq!(kernel(operation, "core/i64@1", json!({})).evaluate(&[Value::I64(a), Value::I64(b)]), expected);
        }
        let expected = if b == 0 { Err(KernelError::DivideByZero) } else {
            i64::try_from(i128::from(a) / i128::from(b)).map(Value::I64).map_err(|_| KernelError::NumericOverflow)
        };
        prop_assert_eq!(kernel(Operation::Divide, "core/i64@1", json!({})).evaluate(&[Value::I64(a), Value::I64(b)]), expected);
    }
}
