use super::*;
use crate::document::literal::Literal;
use crate::id::TypeExpr;
use crate::type_term::{MAX_NESTING, NamedTerm, TypeTerm};
use crate::value::Value;
use crate::value::literal_codec::{
    EnumSchema, FieldSchema, RecordSchema, VariantSchema, decode, encode,
};
use serde_json::json;

fn term(text: &str) -> TypeTerm {
    TypeTerm::parse(&text.parse::<TypeExpr>().unwrap()).unwrap()
}

fn expr(text: &str) -> TypeExpr {
    text.parse().unwrap()
}

#[test]
fn resolves_nested_core_types_and_interns_them_per_registry() {
    let expression = term("core/map@1<core/string@1,core/list@1<core/i64@1>>");
    let mut builder = TypeRegistryBuilder::core();

    let first = builder.resolve(&expression).unwrap();
    let second = builder.resolve(&expression).unwrap();
    let registry = builder.build().unwrap();

    assert_eq!(first, second);
    assert_eq!(registry.expression(&first).unwrap(), &expression);
}

#[test]
fn keeps_numeric_widths_and_float_precisions_distinct() {
    let mut registry = TypeRegistryBuilder::core();
    for (left, right) in [
        ("core/i32@1", "core/i64@1"),
        ("core/u32@1", "core/u64@1"),
        ("core/f32@1", "core/f64@1"),
    ] {
        assert_ne!(
            registry.resolve(&term(left)).unwrap(),
            registry.resolve(&term(right)).unwrap()
        );
    }
}

#[test]
fn rejects_unknown_types_and_wrong_constructor_arities() {
    let registry = TypeRegistry::core();
    assert!(matches!(
        registry.validate(&term("core/decimal@1")),
        Err(TypeResolveError::UnknownType { .. })
    ));
    assert!(matches!(
        registry.validate(&term("core/i64@2")),
        Err(TypeResolveError::UnknownType { major: 2, .. })
    ));
    assert!(matches!(
        registry.validate(&term("core/list@1<core/i64@1,core/i32@1>")),
        Err(TypeResolveError::WrongArity {
            expected: 1,
            found: 2,
            ..
        })
    ));
    assert!(matches!(
        registry.validate(&term("core/map@1<core/i64@1>")),
        Err(TypeResolveError::WrongArity {
            expected: 2,
            found: 1,
            ..
        })
    ));
}

#[test]
fn rejects_unresolved_parameters_and_foreign_registry_keys() {
    let mut first_builder = TypeRegistryBuilder::core();
    let mut second_builder = TypeRegistryBuilder::core();
    let key = first_builder.resolve(&term("core/i64@1")).unwrap();

    assert!(matches!(
        first_builder.resolve(&term("core/list@1<$T>")),
        Err(TypeResolveError::UnresolvedParameter { name }) if name == "T"
    ));
    let other_key = second_builder.resolve(&term("core/i64@1")).unwrap();
    let first_registry = first_builder.build().unwrap();
    let second_registry = second_builder.build().unwrap();
    assert_eq!(
        first_registry.expression(&key).unwrap(),
        &term("core/i64@1")
    );
    assert_ne!(other_key, key);
    assert!(matches!(
        second_registry.expression(&key),
        Err(TypeResolveError::ForeignKey)
    ));
    assert_eq!(
        second_registry.expression(&other_key).unwrap(),
        &term("core/i64@1")
    );
}

#[test]
fn bounds_programmatically_constructed_terms() {
    let mut deep = TypeTerm::Named(NamedTerm {
        namespace: "core".to_owned(),
        name: "i64".to_owned(),
        major: 1,
        args: Vec::new(),
    });
    for _ in 0..=MAX_NESTING {
        deep = TypeTerm::Named(NamedTerm {
            namespace: "core".to_owned(),
            name: "list".to_owned(),
            major: 1,
            args: vec![deep],
        });
    }
    let registry = TypeRegistry::core();

    assert!(matches!(
        registry.validate(&deep),
        Err(TypeResolveError::TooDeep { max: MAX_NESTING })
    ));
}

#[test]
fn registers_nominal_schemas_and_reuses_them_for_literal_codec() {
    let mut builder = TypeRegistryBuilder::core();
    builder
        .register_record(
            "org.example",
            &expr("org.example/point@1"),
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
        .register_enum(
            "org.example",
            &expr("org.example/state@2"),
            EnumSchema {
                variants: vec![VariantSchema {
                    variant_id: "ready".to_owned(),
                    payload: Some(term("core/string@1")),
                }],
            },
        )
        .unwrap();
    let registry = builder.build().unwrap();

    let point_ty = expr("org.example/point@1");
    let point = decode(
        &Literal::value(point_ty.clone(), json!({"x": "7"})),
        &registry,
    )
    .unwrap();
    assert_eq!(
        encode(&point_ty, &point, &registry).unwrap(),
        Literal::value(point_ty, json!({"x": "7"}))
    );
    let enum_ty = expr("org.example/state@2");
    let state = decode(
        &Literal::value(
            enum_ty.clone(),
            json!({"variant": "ready", "payload": "ok"}),
        ),
        &registry,
    )
    .unwrap();
    assert_eq!(
        encode(&enum_ty, &state, &registry).unwrap(),
        Literal::value(enum_ty, json!({"variant": "ready", "payload": "ok"}))
    );
    assert!(matches!(point, Value::Record(_)));
}

#[test]
fn validates_nominal_identity_owner_and_kind_read_only() {
    let mut builder = TypeRegistryBuilder::core();
    builder
        .register_record(
            "org.example",
            &expr("org.example/point@1"),
            RecordSchema { fields: vec![] },
        )
        .unwrap();
    builder
        .register_record(
            "org.other",
            &expr("org.other/point@1"),
            RecordSchema { fields: vec![] },
        )
        .unwrap();
    builder
        .register_handle_kind("org.example", &expr("org.example/session@1"))
        .unwrap();
    let point = builder.resolve(&term("org.example/point@1")).unwrap();
    let other = builder.resolve(&term("org.other/point@1")).unwrap();
    let session = builder.resolve(&term("org.example/session@1")).unwrap();
    let handle = builder
        .resolve(&term("core/handle@1<org.example/session@1>"))
        .unwrap();
    let registry = builder.build().unwrap();
    assert_ne!(point, other);
    assert_eq!(
        registry.owner(&point).unwrap(),
        &TypeOwner::Package("org.example".into())
    );
    assert_eq!(registry.owner(&handle).unwrap(), &TypeOwner::Core);
    assert_eq!(registry.kind(&session).unwrap(), TypeKind::HandleKind);
    assert_eq!(registry.kind(&point).unwrap(), TypeKind::Record);
    let uninterned = term("core/list@1<org.example/point@1>");
    assert_eq!(registry.owner_of(&uninterned).unwrap(), &TypeOwner::Core);
    assert_eq!(registry.kind_of(&uninterned).unwrap(), TypeKind::Core);
    assert_eq!(
        registry.owner_of(&term("org.example/point@1")).unwrap(),
        &TypeOwner::Package("org.example".into())
    );
    assert!(
        registry
            .validate(&term("core/handle@1<core/i64@1>"))
            .is_err()
    );
    assert!(
        registry
            .validate(&term("core/handle@1<core/list@1<org.example/session@1>>"))
            .is_err()
    );
}

#[test]
fn rejects_invalid_package_registration_and_unresolved_schema_types() {
    let mut builder = TypeRegistryBuilder::core();
    assert!(matches!(
        builder.register_record(
            "org.example",
            &expr("org.other/point@1"),
            RecordSchema { fields: vec![] }
        ),
        Err(TypeRegistrationError::OwnerNamespaceMismatch { .. })
    ));
    assert!(matches!(
        builder.register_handle_kind("org.example", &expr("core/fake-handle@1")),
        Err(TypeRegistrationError::ReservedCoreNamespace { .. })
    ));

    builder
        .register_record(
            "org.example",
            &expr("org.example/broken@1"),
            RecordSchema {
                fields: vec![FieldSchema {
                    field_id: "missing".to_owned(),
                    ty: term("org.unknown/missing@1"),
                    optional: false,
                }],
            },
        )
        .unwrap();
    assert!(matches!(
        builder.build(),
        Err(TypeRegistrationError::InvalidSchemaType { .. })
    ));
}

#[test]
fn rejects_blank_or_duplicate_schema_member_ids_and_duplicate_types() {
    let mut blank = TypeRegistryBuilder::core();
    blank
        .register_record(
            "org.example",
            &expr("org.example/blank@1"),
            RecordSchema {
                fields: vec![FieldSchema {
                    field_id: "  ".to_owned(),
                    ty: term("core/i64@1"),
                    optional: false,
                }],
            },
        )
        .unwrap();
    assert!(matches!(
        blank.build(),
        Err(TypeRegistrationError::BlankMemberId { .. })
    ));

    let mut duplicate = TypeRegistryBuilder::core();
    let ty = expr("org.example/duplicate@1");
    duplicate
        .register_record(
            "org.example",
            &ty,
            RecordSchema {
                fields: vec![
                    FieldSchema {
                        field_id: "x".to_owned(),
                        ty: term("core/i64@1"),
                        optional: false,
                    },
                    FieldSchema {
                        field_id: "x".to_owned(),
                        ty: term("core/i32@1"),
                        optional: false,
                    },
                ],
            },
        )
        .unwrap();
    assert!(matches!(
        duplicate.build(),
        Err(TypeRegistrationError::DuplicateMemberId { .. })
    ));

    let mut duplicate_type = TypeRegistryBuilder::core();
    duplicate_type
        .register_record("org.example", &ty, RecordSchema { fields: vec![] })
        .unwrap();
    assert!(matches!(
        duplicate_type.register_enum("org.example", &ty, EnumSchema { variants: vec![] }),
        Err(TypeRegistrationError::DuplicateType { .. })
    ));
}
