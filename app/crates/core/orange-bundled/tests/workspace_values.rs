//! Typed serialized host workspace state, before graph bridge/bus integration.
use crate::data_graph::{catalog, graph, id};
use orange_compiler::UpdateOperation;
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::id::VariableId;
use orange_model::scalar::{FloatPolicy, Operation, Overflow, Scalar};
use orange_model::value::{ListRepr, MapRepr};
use orange_model::{GraphDocument, Value, from_json, to_json};
use orange_runtime::{Workspace, WorkspaceError, WorkspaceLimits};
use serde_json::{Value as Json, json};
use std::sync::Arc;
fn variable(n: u32, ty: &str, init: Option<Json>) -> Json {
    let mut v = json!({"id":id(n),"name":format!("v{n}"),"type":ty,"scope":"workspace"});
    if let Some(value) = init {
        v["init"] = json!({"type":ty,"value":value});
    }
    v
}
fn fixture(variables: Vec<Json>) -> (GraphDocument, Arc<RegistrySnapshot>) {
    let (package, snapshot) = catalog();
    let doc = graph(&package, vec![], vec![], vec![]);
    (edit(&doc, |w| w["variables"] = json!(variables)), snapshot)
}
fn edit(doc: &GraphDocument, f: impl FnOnce(&mut Json)) -> GraphDocument {
    let mut w: Json = serde_json::from_str(&to_json(doc).unwrap()).unwrap();
    f(&mut w);
    from_json(&w.to_string()).unwrap()
}
fn vid(n: u32) -> VariableId {
    id(n).parse().unwrap()
}
fn integer(overflow: Overflow) -> UpdateOperation {
    UpdateOperation::Numeric {
        operation: Operation::Add,
        scalar: Scalar::U64,
        overflow,
        float_policy: FloatPolicy::Finite,
    }
}
fn map(items: &[(i64, i64)]) -> Value {
    Value::Map(Arc::new(
        MapRepr::new(
            items
                .iter()
                .map(|(a, b)| (Value::I64(*a), Value::I64(*b)))
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    ))
}
fn text(s: &str) -> Value {
    Value::Str(Arc::new(s.into()))
}

#[test]
fn snapshots_are_immutable_and_commits_publish_monotonic_read_your_write_versions() {
    let (doc, registry) = fixture(vec![
        variable(1, "core/u64@1", Some(json!("3"))),
        variable(2, "core/u64@1", None),
    ]);
    let mut workspace = Workspace::new(&doc, registry.clone(), WorkspaceLimits::default()).unwrap();
    let old = workspace.snapshot();
    assert_eq!(old.version(), 0);
    assert_eq!(old.get(vid(1)).unwrap(), Some(&Value::U64(3)));
    assert_eq!(old.get(vid(2)).unwrap(), None);
    let commit = workspace.set(vid(1), &Value::U64(4)).unwrap();
    assert_eq!(commit.old(), Some(&Value::U64(3)));
    assert_eq!(commit.new_value(), Some(&Value::U64(4)));
    assert!(commit.written());
    assert_eq!(commit.changed(), Some(true));
    assert_eq!(commit.snapshot().version(), 1);
    assert_eq!(
        workspace.snapshot().get(vid(1)).unwrap(),
        Some(&Value::U64(4))
    );
    assert_eq!(old.get(vid(1)).unwrap(), Some(&Value::U64(3)));
    let init = workspace.set(vid(2), &Value::U64(9)).unwrap();
    assert_eq!(init.old(), None);
    assert_eq!(init.changed(), Some(true));
    assert_eq!(init.snapshot().version(), 2);
    assert_eq!(
        init.snapshot().registry().generation(),
        registry.generation()
    );
    let same = workspace.set(vid(2), &Value::U64(9)).unwrap();
    assert_eq!(same.changed(), Some(false));
    assert_eq!(same.snapshot().version(), 3);
}
#[test]
fn numeric_updates_share_checked_wrapping_and_saturating_semantics() {
    let (doc, registry) = fixture(vec![variable(
        1,
        "core/u64@1",
        Some(json!(u64::MAX.to_string())),
    )]);
    let mut workspace = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    let before = workspace.meter().retention();
    let error = workspace
        .update(vid(1), integer(Overflow::Checked), &[Value::U64(1)])
        .unwrap_err();
    assert!(matches!(error,WorkspaceError::Failed(f) if f.code=="core/NumericOverflow"));
    assert_eq!(workspace.snapshot().version(), 0);
    assert_eq!(workspace.meter().retention(), before);
    let saturated = workspace
        .update(vid(1), integer(Overflow::Saturating), &[Value::U64(1)])
        .unwrap();
    assert_eq!(saturated.new_value(), Some(&Value::U64(u64::MAX)));
    assert_eq!(saturated.changed(), Some(false));
    assert_eq!(saturated.snapshot().version(), 1);
    let wrapped = workspace
        .update(vid(1), integer(Overflow::Wrapping), &[Value::U64(1)])
        .unwrap();
    assert_eq!(wrapped.new_value(), Some(&Value::U64(0)));
    assert_eq!(wrapped.changed(), Some(true));
    assert_eq!(wrapped.snapshot().version(), 2);
}
#[test]
fn compare_and_set_mismatch_keeps_version_and_success_commits_even_an_equal_value() {
    let (doc, registry) = fixture(vec![variable(1, "core/u64@1", Some(json!("5")))]);
    let mut workspace = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    let before = workspace.meter().retention();
    let mismatch = workspace
        .update(
            vid(1),
            UpdateOperation::CompareAndSet,
            &[Value::U64(4), Value::U64(8)],
        )
        .unwrap();
    assert!(!mismatch.written());
    assert_eq!(mismatch.exchanged(), Some(false));
    assert_eq!(mismatch.changed(), Some(false));
    assert_eq!(mismatch.old(), mismatch.new_value());
    assert_eq!(mismatch.snapshot().version(), 0);
    assert_eq!(workspace.meter().retention(), before);
    let same = workspace
        .update(
            vid(1),
            UpdateOperation::CompareAndSet,
            &[Value::U64(5), Value::U64(5)],
        )
        .unwrap();
    assert!(same.written());
    assert_eq!(same.exchanged(), Some(true));
    assert_eq!(same.changed(), Some(false));
    assert_eq!(same.snapshot().version(), 1);
    let next = workspace
        .update(
            vid(1),
            UpdateOperation::CompareAndSet,
            &[Value::U64(5), Value::U64(8)],
        )
        .unwrap();
    assert_eq!(next.exchanged(), Some(true));
    assert_eq!(next.new_value(), Some(&Value::U64(8)));
    assert_eq!(next.snapshot().version(), 2);
}
#[test]
fn compare_and_set_uses_language_map_equality_instead_of_insertion_order() {
    let (doc, registry) = fixture(vec![variable(1, "core/map@1<core/i64@1,core/i64@1>", None)]);
    let mut workspace = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    workspace.set(vid(1), &map(&[(1, 2), (3, 4)])).unwrap();
    let commit = workspace
        .update(
            vid(1),
            UpdateOperation::CompareAndSet,
            &[map(&[(3, 4), (1, 2)]), map(&[(5, 6)])],
        )
        .unwrap();
    assert_eq!(commit.exchanged(), Some(true));
    assert_eq!(commit.new_value(), Some(&map(&[(5, 6)])));
}
#[test]
fn bool_bitwise_min_max_and_append_are_serialized_through_the_same_core_operations() {
    let (doc, registry) = fixture(vec![
        variable(1, "core/bool@1", Some(json!(true))),
        variable(2, "core/u64@1", Some(json!("6"))),
        variable(3, "core/string@1", Some(json!("ab"))),
        variable(4, "core/list@1<core/u64@1>", None),
    ]);
    let mut workspace = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    assert_eq!(
        workspace
            .update(vid(1), UpdateOperation::Xor, &[Value::Bool(true)])
            .unwrap()
            .new_value(),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        workspace
            .update(vid(2), UpdateOperation::And, &[Value::U64(3)])
            .unwrap()
            .new_value(),
        Some(&Value::U64(2))
    );
    assert_eq!(
        workspace
            .update(vid(2), UpdateOperation::Max, &[Value::U64(9)])
            .unwrap()
            .new_value(),
        Some(&Value::U64(9))
    );
    assert_eq!(
        workspace
            .update(vid(2), UpdateOperation::Min, &[Value::U64(4)])
            .unwrap()
            .new_value(),
        Some(&Value::U64(4))
    );
    assert_eq!(
        workspace
            .update(vid(3), UpdateOperation::Append, &[text("cd")])
            .unwrap()
            .new_value(),
        Some(&text("abcd"))
    );
    let list = |values: &[u64]| Value::List(Arc::new(ListRepr::U64(values.to_vec().into())));
    workspace.set(vid(4), &list(&[1, 2])).unwrap();
    assert_eq!(
        workspace
            .update(vid(4), UpdateOperation::Append, &[list(&[3])])
            .unwrap()
            .new_value(),
        Some(&list(&[1, 2, 3]))
    );
}
#[test]
fn uninitialized_unknown_bad_type_and_operand_count_refusals_are_atomic() {
    let (doc, registry) = fixture(vec![variable(1, "core/u64@1", None)]);
    let mut workspace = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    let before = workspace.meter().retention();
    assert_eq!(
        workspace
            .update(vid(1), integer(Overflow::Checked), &[Value::U64(1)])
            .unwrap_err(),
        WorkspaceError::Uninitialized
    );
    assert_eq!(
        workspace.set(vid(2), &Value::U64(1)).unwrap_err(),
        WorkspaceError::UnknownVariable
    );
    assert_eq!(
        workspace.set(vid(1), &Value::Bool(true)).unwrap_err(),
        WorkspaceError::Contract
    );
    assert_eq!(
        workspace
            .update(vid(1), integer(Overflow::Checked), &[])
            .unwrap_err(),
        WorkspaceError::Contract
    );
    assert_eq!(
        workspace
            .update(vid(1), UpdateOperation::CompareAndSet, &[Value::U64(1)])
            .unwrap_err(),
        WorkspaceError::Contract
    );
    assert_eq!(workspace.snapshot().version(), 0);
    assert_eq!(workspace.snapshot().get(vid(1)).unwrap(), None);
    assert_eq!(workspace.meter().retention(), before);
}
#[test]
fn raw_float_does_not_gain_eq_or_compare_and_set_from_rust_value_identity() {
    let (doc, registry) = fixture(vec![variable(1, "core/f64@1", None)]);
    let mut workspace = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    workspace.set(vid(1), &Value::F64(1.0)).unwrap();
    let same = workspace.set(vid(1), &Value::F64(1.0)).unwrap();
    assert_eq!(same.changed(), None);
    assert_eq!(
        workspace
            .update(
                vid(1),
                UpdateOperation::CompareAndSet,
                &[Value::F64(1.0), Value::F64(2.0)]
            )
            .unwrap_err(),
        WorkspaceError::Contract
    );
    assert_eq!(
        workspace.snapshot().get(vid(1)).unwrap(),
        Some(&Value::F64(1.0))
    );
}
#[test]
fn value_state_and_retained_limits_refuse_without_publishing_a_partial_generation() {
    let (doc, registry) = fixture(vec![variable(1, "core/string@1", Some(json!("abcd")))]);
    let mut limits = WorkspaceLimits {
        max_value_bytes: 24,
        max_state_bytes: 24,
        ..Default::default()
    };
    let mut workspace = Workspace::new(&doc, registry.clone(), limits).unwrap();
    let old = workspace.snapshot();
    let before = workspace.meter().retention();
    assert_eq!(
        workspace.set(vid(1), &text("123456789")).unwrap_err(),
        WorkspaceError::Limit
    );
    assert!(
        workspace
            .update(vid(1), UpdateOperation::Append, &[text("12345")])
            .is_err()
    );
    assert_eq!(workspace.snapshot().version(), 0);
    assert_eq!(
        workspace.snapshot().get(vid(1)).unwrap(),
        old.get(vid(1)).unwrap()
    );
    assert_eq!(workspace.meter().retention(), before);
    let base = Workspace::new(&doc, registry.clone(), WorkspaceLimits::default())
        .unwrap()
        .meter()
        .retention()
        .reserved_bytes;
    limits = WorkspaceLimits {
        max_retained_bytes: base,
        ..Default::default()
    };
    let mut tight = Workspace::new(&doc, registry, limits).unwrap();
    let before = tight.meter().retention();
    assert_eq!(
        tight.set(vid(1), &text("efgh")).unwrap_err(),
        WorkspaceError::Limit
    );
    assert_eq!(tight.snapshot().version(), 0);
    assert_eq!(tight.meter().retention(), before);
}
#[test]
fn old_generations_and_receipts_remain_charged_until_their_last_reader_drops() {
    let (doc, registry) = fixture(vec![variable(1, "core/u64@1", Some(json!("1")))]);
    let mut workspace = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    let meter = workspace.meter();
    let base = meter.retention().reserved_bytes;
    let old = workspace.snapshot();
    let commit = workspace.set(vid(1), &Value::U64(2)).unwrap();
    let current = workspace.snapshot();
    assert!(meter.retention().reserved_bytes > base);
    drop(old);
    assert!(meter.retention().reserved_bytes > base);
    drop(commit);
    assert_eq!(meter.retention().reserved_bytes, base);
    drop(workspace);
    assert_eq!(current.get(vid(1)).unwrap(), Some(&Value::U64(2)));
    assert!(meter.retention().reserved_bytes > 0);
    drop(current);
    assert_eq!(meter.retention().reserved_bytes, 0);
}
#[test]
fn snapshots_pin_schema_and_compatible_documents_do_not_reset_shared_values() {
    let (doc, registry) = fixture(vec![variable(1, "core/u64@1", Some(json!("1")))]);
    let mut workspace = Workspace::new(&doc, registry.clone(), WorkspaceLimits::default()).unwrap();
    workspace.set(vid(1), &Value::U64(9)).unwrap();
    let changed = edit(&doc, |w| {
        w["variables"][0]["name"] = json!("renamed");
        w["variables"][0]["init"]["value"] = json!("123");
    });
    let new_registry = RegistryBuilder::from_snapshot(&registry).build().unwrap();
    workspace
        .snapshot()
        .check_schema(&changed, &new_registry)
        .unwrap();
    assert_eq!(
        workspace.snapshot().get(vid(1)).unwrap(),
        Some(&Value::U64(9))
    );
    assert_eq!(
        workspace.snapshot().registry().generation(),
        registry.generation()
    );
    let wrong = edit(&doc, |w| {
        w["variables"][0]["type"] = json!("core/bool@1");
        w["variables"][0]["init"] = json!({"type":"core/bool@1","value":true});
    });
    assert_eq!(
        workspace.snapshot().check_schema(&wrong, &new_registry),
        Err(WorkspaceError::SchemaMismatch)
    );
}
#[test]
fn only_workspace_declarations_are_admitted_and_live_identities_do_not_alias() {
    let mut local = variable(2, "core/u64@1", Some(json!("8")));
    local["scope"] = json!("run");
    let (doc, registry) = fixture(vec![variable(1, "core/u64@1", None), local]);
    let a = Workspace::new(&doc, registry.clone(), WorkspaceLimits::default()).unwrap();
    let b = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    assert_ne!(a.snapshot().id(), b.snapshot().id());
    assert_eq!(
        a.snapshot().get(vid(2)),
        Err(WorkspaceError::UnknownVariable)
    );
    assert_eq!(
        a.snapshot().variable_type(vid(1)).unwrap().as_str(),
        "core/u64@1"
    );
}
#[test]
fn constructor_refuses_schema_capacity_oversized_initializers_and_resource_types() {
    let (doc, registry) = fixture(vec![variable(
        1,
        "core/string@1",
        Some(json!("secret-not-in-error")),
    )]);
    for limits in [
        WorkspaceLimits {
            max_variables: 0,
            ..Default::default()
        },
        WorkspaceLimits {
            max_schema_bytes: 0,
            ..Default::default()
        },
        WorkspaceLimits {
            max_value_bytes: 16,
            ..Default::default()
        },
        WorkspaceLimits {
            max_state_bytes: 16,
            ..Default::default()
        },
        WorkspaceLimits {
            max_retained_bytes: 0,
            ..Default::default()
        },
    ] {
        assert_eq!(
            Workspace::new(&doc, registry.clone(), limits).unwrap_err(),
            WorkspaceError::Limit
        );
    }
    for ty in ["core/stream@1<core/u64@1>", "core/handle@1<core/u64@1>"] {
        let bad = edit(&doc, |w| {
            w["variables"][0]["type"] = json!(ty);
            w["variables"][0].as_object_mut().unwrap().remove("init");
        });
        assert_eq!(
            Workspace::new(&bad, registry.clone(), WorkspaceLimits::default()).unwrap_err(),
            WorkspaceError::Contract
        );
    }
}
#[test]
fn metadata_debug_and_errors_omit_values_and_all_surfaces_are_send_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Workspace>();
    send_sync::<orange_runtime::WorkspaceSnapshot>();
    send_sync::<orange_runtime::WorkspaceCommit>();
    let secret = "secret-not-in-workspace-debug";
    let (doc, registry) = fixture(vec![variable(1, "core/string@1", Some(json!(secret)))]);
    let mut workspace = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    let commit = workspace.set(vid(1), &text(secret)).unwrap();
    for debug in [
        format!("{workspace:?}"),
        format!("{:?}", workspace.snapshot()),
        format!("{commit:?}"),
        format!("{:?}", workspace.meter()),
    ] {
        assert!(!debug.contains(secret));
    }
    let error = workspace.set(vid(1), &Value::U64(7)).unwrap_err();
    assert!(!error.to_string().contains(secret));
    assert_eq!(error.message_key(), "core/workspace-contract");
}

#[test]
fn nominal_records_keep_pinned_fields_eq_and_schema_compatibility() {
    use orange_model::type_term::TypeTerm;
    use orange_model::types::{Constraint, TypeRegistryBuilder};
    use orange_model::value::literal_codec::{self, FieldSchema, RecordSchema};
    const POINT: &str = "org.example/point@1";
    let (package, registry) = crate::record_declarations::catalog();
    let base = graph(&package, vec![], vec![], vec![]);
    let doc = edit(&base, |w| {
        w["variables"] = json!([variable(1, POINT, Some(json!({"x":"3"})))])
    });
    let decode = |value: Json| {
        let literal = serde_json::from_value(json!({"type":POINT,"value":value})).unwrap();
        literal_codec::decode(&literal, registry.types().as_ref()).unwrap()
    };
    let mut workspace = Workspace::new(&doc, registry.clone(), WorkspaceLimits::default()).unwrap();
    let expected = decode(json!({"x":"3"}));
    let new = decode(json!({"x":"9","label":"ok"}));
    let commit = workspace
        .update(
            vid(1),
            UpdateOperation::CompareAndSet,
            &[expected, new.clone()],
        )
        .unwrap();
    assert_eq!(commit.exchanged(), Some(true));
    assert_eq!(commit.new_value(), Some(&new));
    let mut types = TypeRegistryBuilder::core();
    let term = |s: &str| TypeTerm::parse(&s.parse().unwrap()).unwrap();
    types
        .register_record(
            "org.example",
            &POINT.parse().unwrap(),
            RecordSchema {
                fields: vec![
                    FieldSchema {
                        field_id: "x".into(),
                        ty: term("core/i64@1"),
                        optional: false,
                    },
                    FieldSchema {
                        field_id: "label".into(),
                        ty: term("core/string@1"),
                        optional: false,
                    },
                ],
            },
        )
        .unwrap();
    types
        .derive_constraint("org.example", &POINT.parse().unwrap(), Constraint::Eq)
        .unwrap();
    types
        .derive_constraint("org.example", &POINT.parse().unwrap(), Constraint::Hash)
        .unwrap();
    let changed = RegistryBuilder::from_snapshot(&registry)
        .types(Arc::new(types.build().unwrap()))
        .build()
        .unwrap();
    assert_eq!(
        workspace.snapshot().check_schema(&doc, &changed),
        Err(WorkspaceError::SchemaMismatch)
    );
    assert_eq!(workspace.snapshot().get(vid(1)).unwrap(), Some(&new));
}

#[test]
fn uninitialized_none_and_some_none_keep_distinct_typed_values() {
    let (doc, registry) = fixture(vec![
        variable(1, "core/option@1<core/u64@1>", None),
        variable(2, "core/option@1<core/option@1<core/u64@1>>", None),
    ]);
    let mut workspace = Workspace::new(&doc, registry, WorkspaceLimits::default()).unwrap();
    assert_eq!(workspace.snapshot().get(vid(1)).unwrap(), None);
    let first = workspace.set(vid(1), &Value::None).unwrap();
    assert_eq!(first.new_value(), Some(&Value::None));
    let exchanged = workspace
        .update(
            vid(1),
            UpdateOperation::CompareAndSet,
            &[Value::None, Value::U64(7)],
        )
        .unwrap();
    assert_eq!(exchanged.exchanged(), Some(true));
    assert_eq!(exchanged.new_value(), Some(&Value::U64(7)));
    workspace.set(vid(2), &Value::None).unwrap();
    let old = workspace.snapshot();
    let nested = Value::Some(Arc::new(Value::None));
    workspace.set(vid(2), &nested).unwrap();
    assert_eq!(old.get(vid(2)).unwrap(), Some(&Value::None));
    assert_eq!(workspace.snapshot().get(vid(2)).unwrap(), Some(&nested));
}

#[test]
fn foreign_domain_metadata_is_preserved_and_not_reinterpreted_as_control_workspace_state() {
    let (doc, registry) = fixture(vec![variable(1, "core/u64@1", Some(json!("1")))]);
    let workspace = Workspace::new(&doc, registry.clone(), WorkspaceLimits::default()).unwrap();
    let foreign = edit(&doc, |w| {
        w["domain"] = json!({"id":"acme/foreign","opaque":"preserved"})
    });
    let before = to_json(&foreign).unwrap();
    assert_eq!(
        Workspace::new(&foreign, registry.clone(), WorkspaceLimits::default()).unwrap_err(),
        WorkspaceError::DomainUnsupported
    );
    assert_eq!(
        workspace.snapshot().check_schema(&foreign, &registry),
        Err(WorkspaceError::DomainUnsupported)
    );
    assert_eq!(to_json(&foreign).unwrap(), before);
}
