use super::*;
use crate::document::literal::Literal;
use crate::id::TypeExpr;
use crate::type_term::TypeTerm;
use crate::value::literal_codec::{
    EnumSchema, FieldSchema, RecordSchema, SchemaSet, VariantSchema, decode, encode,
};
use serde_json::json;

fn term(text: &str) -> TypeTerm {
    TypeTerm::parse(&text.parse::<TypeExpr>().unwrap()).unwrap()
}

fn expr(text: &str) -> TypeExpr {
    text.parse().unwrap()
}

fn has(registry: &TypeRegistry, text: &str, constraint: Constraint) -> bool {
    registry
        .supports_term(&term(text), CapabilityRequest::Constraint(constraint))
        .unwrap()
        .is_some()
}

#[test]
fn core_constraints_propagate_through_aggregates_and_keep_numeric_operations_exact() {
    let registry = TypeRegistry::core();
    assert!(has(&registry, "core/i32@1", Constraint::Num));
    assert!(has(&registry, "core/i64@1", Constraint::Int));
    assert!(has(&registry, "core/f32@1", Constraint::Float));
    assert!(!has(&registry, "core/f32@1", Constraint::Eq));
    assert!(!has(&registry, "core/f64@1", Constraint::Hash));
    assert!(!has(&registry, "core/duration@1", Constraint::Hash));
    assert!(!has(&registry, "core/list@1<core/f64@1>", Constraint::Eq));
    assert!(has(
        &registry,
        "core/map@1<core/string@1,core/i32@1>",
        Constraint::Eq
    ));
    assert!(has(&registry, "core/f32@1", Constraint::Display));
    assert!(!has(
        &registry,
        "core/list@1<core/stream@1<core/i32@1>>",
        Constraint::Display
    ));
    assert!(has(
        &registry,
        "core/list@1<core/stream@1<core/i32@1>>",
        Constraint::Default
    ));
    assert!(has(&registry, "core/error@1", Constraint::Serialize));
    assert!(has(
        &registry,
        "core/result@1<core/i64@1>",
        Constraint::Serialize
    ));
    assert!(!has(
        &registry,
        "core/list@1<core/stream@1<core/i32@1>>",
        Constraint::Serialize
    ));

    for operation in [NumericOperation::Add, NumericOperation::Subtract] {
        assert!(
            registry
                .supports_term(
                    &term("core/duration@1"),
                    CapabilityRequest::NumericOperation(operation)
                )
                .unwrap()
                .is_some()
        );
    }
    for operation in [NumericOperation::Multiply, NumericOperation::Divide] {
        assert!(
            registry
                .supports_term(
                    &term("core/duration@1"),
                    CapabilityRequest::NumericOperation(operation)
                )
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn package_structural_derives_require_explicit_declaration_and_supported_members() {
    let mut builder = TypeRegistryBuilder::core();
    let point = expr("org.example/point@1");
    builder
        .register_record(
            "org.example",
            &point,
            RecordSchema {
                fields: vec![FieldSchema {
                    field_id: "x".to_owned(),
                    ty: term("core/i64@1"),
                    optional: false,
                }],
            },
        )
        .unwrap();
    builder
        .derive_constraint("org.example", &point, Constraint::Eq)
        .unwrap();
    builder
        .derive_constraint("org.example", &point, Constraint::Hash)
        .unwrap();
    let state = expr("org.example/state@1");
    builder
        .register_enum(
            "org.example",
            &state,
            EnumSchema {
                variants: vec![
                    VariantSchema {
                        variant_id: "ready".to_owned(),
                        payload: None,
                    },
                    VariantSchema {
                        variant_id: "count".to_owned(),
                        payload: Some(term("core/i64@1")),
                    },
                ],
            },
        )
        .unwrap();
    builder
        .derive_constraint("org.example", &state, Constraint::Hash)
        .unwrap();
    let registry = builder.build().unwrap();

    assert!(has(&registry, "org.example/point@1", Constraint::Eq));
    assert!(has(&registry, "org.example/point@1", Constraint::Hash));
    assert!(has(&registry, "org.example/state@1", Constraint::Hash));
    assert!(!has(&registry, "org.example/point@1", Constraint::Ord));
}

#[test]
fn derives_with_unsupported_members_fail_publication_but_type_without_derive_is_valid() {
    let mut unsupported = TypeRegistryBuilder::core();
    let float_record = expr("org.example/float-record@1");
    unsupported
        .register_record(
            "org.example",
            &float_record,
            RecordSchema {
                fields: vec![FieldSchema {
                    field_id: "value".to_owned(),
                    ty: term("core/f32@1"),
                    optional: false,
                }],
            },
        )
        .unwrap();
    unsupported
        .derive_constraint("org.example", &float_record, Constraint::Eq)
        .unwrap();
    assert!(matches!(
        unsupported.build(),
        Err(TypeRegistrationError::InvalidDerivedConstraint { .. })
    ));

    let mut type_only = TypeRegistryBuilder::core();
    type_only
        .register_record(
            "org.example",
            &float_record,
            RecordSchema {
                fields: vec![FieldSchema {
                    field_id: "value".to_owned(),
                    ty: term("core/f32@1"),
                    optional: false,
                }],
            },
        )
        .unwrap();
    assert!(type_only.build().is_ok());
}

#[test]
fn recursive_derived_schema_reports_bounded_publication_error_but_type_is_valid() {
    let mut builder = TypeRegistryBuilder::core();
    let node = expr("org.example/node@1");
    builder
        .register_record(
            "org.example",
            &node,
            RecordSchema {
                fields: vec![FieldSchema {
                    field_id: "next".to_owned(),
                    ty: term("core/option@1<org.example/node@1>"),
                    optional: true,
                }],
            },
        )
        .unwrap();
    builder
        .derive_constraint("org.example", &node, Constraint::Eq)
        .unwrap();
    assert!(matches!(
        builder.build(),
        Err(TypeRegistrationError::RecursiveDerivation { .. })
    ));

    let mut type_only = TypeRegistryBuilder::core();
    type_only
        .register_record(
            "org.example",
            &node,
            RecordSchema {
                fields: vec![FieldSchema {
                    field_id: "next".to_owned(),
                    ty: term("core/option@1<org.example/node@1>"),
                    optional: true,
                }],
            },
        )
        .unwrap();
    assert!(type_only.build().is_ok());
}

#[test]
fn long_acyclic_derive_chain_hits_proof_budget() {
    let mut builder = TypeRegistryBuilder::core();
    let count = MAX_NESTING + 2;
    let names: Vec<_> = (0..count)
        .map(|index| format!("org.example/node{index}@1"))
        .collect();
    for (index, name) in names.iter().enumerate() {
        let field_type = names
            .get(index + 1)
            .map_or_else(|| "core/i64@1".to_owned(), Clone::clone);
        let ty = expr(name);
        builder
            .register_record(
                "org.example",
                &ty,
                RecordSchema {
                    fields: vec![FieldSchema {
                        field_id: "next".to_owned(),
                        ty: term(&field_type),
                        optional: false,
                    }],
                },
            )
            .unwrap();
        builder
            .derive_constraint("org.example", &ty, Constraint::Eq)
            .unwrap();
    }
    assert!(matches!(
        builder.build(),
        Err(TypeRegistrationError::DerivationTooDeep { max: MAX_NESTING })
    ));
}

#[test]
fn unsupported_package_derives_fail_explicitly() {
    let mut builder = TypeRegistryBuilder::core();
    builder
        .register_record(
            "org.example",
            &expr("org.example/record@1"),
            RecordSchema { fields: vec![] },
        )
        .unwrap();
    assert!(matches!(
        builder.derive_constraint(
            "org.example",
            &expr("org.example/record@1"),
            Constraint::Default
        ),
        Err(TypeRegistrationError::UnsupportedDerivation { .. })
    ));
}

#[test]
fn registry_codec_checks_language_map_key_constraints_and_legacy_codec_path_is_explicit() {
    let registry = TypeRegistry::core();
    assert!(matches!(
        registry.validate(&term("core/map@1<core/list@1<core/i64@1>,core/string@1>")),
        Err(TypeResolveError::InvalidMapKey { .. })
    ));
    let valid_type = expr("core/map@1<core/i64@1,core/string@1>");
    let valid_literal = Literal::value(valid_type.clone(), json!([["1", "one"]]));
    let value = decode(&valid_literal, &registry).unwrap();
    assert_eq!(
        encode(&valid_type, &value, &registry).unwrap(),
        valid_literal
    );

    let ineligible = Literal::value(expr("core/map@1<core/duration@1,core/string@1>"), json!([]));
    assert!(matches!(
        decode(&ineligible, &registry),
        Err(crate::error::LiteralError::IneligibleMapKey { .. })
    ));
    let float = Literal::value(expr("core/map@1<core/f32@1,core/string@1>"), json!([]));
    assert!(matches!(
        decode(&float, &registry),
        Err(crate::error::LiteralError::FloatMapKey { .. })
    ));

    // The standalone codec-only API retains its historical float exclusion. Full language
    // hash/eq eligibility is available through TypeRegistry's Schemas implementation.
    assert!(decode(&ineligible, &SchemaSet::default()).is_ok());

    let mut builder = TypeRegistryBuilder::core();
    builder
        .resolve(&term("core/map@1<core/list@1<core/i64@1>,core/string@1>"))
        .unwrap();
    assert!(matches!(
        builder.build(),
        Err(TypeRegistrationError::InvalidResolvedType { .. })
    ));

    let mut schema_builder = TypeRegistryBuilder::core();
    schema_builder
        .register_record(
            "org.example",
            &expr("org.example/bad-map-field@1"),
            RecordSchema {
                fields: vec![FieldSchema {
                    field_id: "values".to_owned(),
                    ty: term("core/map@1<core/list@1<core/i64@1>,core/string@1>"),
                    optional: false,
                }],
            },
        )
        .unwrap();
    assert!(matches!(
        schema_builder.build(),
        Err(TypeRegistrationError::InvalidSchemaType {
            error: TypeResolveError::InvalidMapKey { .. },
            ..
        })
    ));
}
