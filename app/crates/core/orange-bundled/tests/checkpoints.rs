//! Portable persistent-value bootstrap at public core boundaries.
use crate::data_graph::{catalog, edge, graph, id};
use orange_compiler::{CompileOptions, ControlPlan, compile_control};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::literal::Literal;
use orange_model::id::VariableId;
use orange_model::value::{JsonText, ListRepr, MapRepr, literal_codec};
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use orange_runtime::{
    CheckpointError as Error, CheckpointLimits, Run, RunOptions, RunState, ValueCheckpoint,
    Workspace, WorkspaceLimits,
};
use serde_json::{Value as Json, json};
use std::sync::Arc;
fn vid(n: u32) -> VariableId {
    id(n).parse().unwrap()
}
fn variable(n: u32, scope: &str, ty: &str, init: Option<Json>, persistent: bool) -> Json {
    let mut v = json!({"id":id(n),"name":"secret-display-name","scope":scope,"type":ty,"persistent":persistent});
    if let Some(init) = init {
        v["init"] = json!({"type":ty,"value":init});
    }
    v
}
fn edit(doc: &GraphDocument, f: impl FnOnce(&mut Json)) -> GraphDocument {
    let mut raw: Json = serde_json::from_str(&to_json(doc).unwrap()).unwrap();
    f(&mut raw);
    from_json(&raw.to_string()).unwrap()
}
fn fixture(vars: Vec<Json>) -> (GraphDocument, Arc<RegistrySnapshot>) {
    let (p, r) = catalog();
    let base = graph(&p, vec![], vec![], vec![]);
    (edit(&base, |d| d["variables"] = json!(vars)), r)
}
fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}});
    if name == "on-start" {
        node["activation"] =
            json!({"mode":"queue","capacity":4,"overflow":"fault","on_fault":"continue","order":0});
    }
    node
}
fn run_fixture() -> (GraphDocument, Arc<ControlPlan>) {
    let (p, r) = catalog();
    let mut a = core(2, "set-variable");
    a["config"] = json!({"variable":id(100)});
    a["inputs"] = json!({"value":{"type":"core/u64@1","value":"9"}});
    let mut b = core(3, "set-variable");
    b["config"] = json!({"variable":id(101)});
    b["inputs"] = json!({"value":{"type":"core/u64@1","value":"8"}});
    let doc = edit(
        &graph(
            &p,
            vec![core(1, "on-start"), a, b],
            vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
            vec![],
        ),
        |d| {
            d["variables"] = json!([
                variable(100, "run", "core/u64@1", Some(json!("1")), true),
                variable(101, "run", "core/u64@1", Some(json!("2")), false)
            ])
        },
    );
    let plan = Arc::new(
        compile_control(
            &doc,
            r.clone(),
            &analyze(&doc, &r),
            &p,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    (doc, plan)
}
fn idle(run: &mut Run) {
    for _ in 0..10000 {
        if run.state() == RunState::Stopped || !run.pump(1).unwrap().ready {
            return;
        }
    }
    panic!("not idle");
}
fn stopped(doc: &GraphDocument, plan: &Arc<ControlPlan>) -> Run {
    let mut r = Run::start(
        plan.clone(),
        doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    idle(&mut r);
    r.stop().unwrap();
    idle(&mut r);
    r
}
fn roundtrip(c: &ValueCheckpoint) -> ValueCheckpoint {
    ValueCheckpoint::from_bytes(&c.to_bytes().unwrap(), CheckpointLimits::default()).unwrap()
}
#[test]
fn portable_workspace_roundtrip_preserves_native_values_and_float_bits() {
    let values = vec![
        ("core/u64@1", Value::U64(u64::MAX)),
        ("core/i64@1", Value::I64(i64::MIN)),
        ("core/f32@1", Value::F32(f32::from_bits(0x7fc01234))),
        ("core/f64@1", Value::F64(-0.0)),
        ("core/bool@1", Value::Bool(true)),
        ("core/string@1", Value::Str(Arc::new("héllo中".into()))),
        ("core/bytes@1", Value::Bytes(Arc::new(vec![0, 1, 255]))),
        ("core/duration@1", Value::Duration(-13)),
        ("core/timestamp@1", Value::Timestamp(123)),
        (
            "core/list@1<core/u64@1>",
            Value::List(Arc::new(ListRepr::U64(vec![2, 1].into()))),
        ),
        (
            "core/map@1<core/i64@1,core/i64@1>",
            Value::Map(Arc::new(
                MapRepr::new(vec![
                    (Value::I64(2), Value::I64(3)),
                    (Value::I64(1), Value::I64(4)),
                ])
                .unwrap(),
            )),
        ),
    ];
    let (doc, r) = fixture(
        values
            .iter()
            .enumerate()
            .map(|(i, (ty, _))| variable(i as u32 + 1, "workspace", ty, None, true))
            .collect(),
    );
    let mut w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    for (i, (_, value)) in values.iter().enumerate() {
        w.set(vid(i as u32 + 1), value).unwrap();
    }
    let ids: Vec<_> = (1..=values.len() as u32).map(vid).collect();
    let c = roundtrip(
        &w.checkpoint(&doc, &ids, CheckpointLimits::default())
            .unwrap(),
    );
    let restored = Workspace::from_checkpoint(&doc, r, WorkspaceLimits::default(), &c).unwrap();
    assert_ne!(w.snapshot().id(), restored.snapshot().id());
    assert_eq!(restored.snapshot().version(), 0);
    for (i, (_, v)) in values.iter().enumerate() {
        assert_eq!(restored.snapshot().get(vid(i as u32 + 1)).unwrap(), Some(v));
    }
}
#[test]
fn uninitialized_option_none_some_none_and_json_null_remain_distinct() {
    let types = [
        "core/option@1<core/u64@1>",
        "core/option@1<core/u64@1>",
        "core/option@1<core/option@1<core/u64@1>>",
        "core/json@1",
    ];
    let (doc, r) = fixture(
        types
            .iter()
            .enumerate()
            .map(|(i, ty)| variable(i as u32 + 1, "workspace", ty, None, true))
            .collect(),
    );
    let mut w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    w.set(vid(2), &Value::None).unwrap();
    w.set(vid(3), &Value::Some(Arc::new(Value::None))).unwrap();
    let null = Value::Json(Arc::new(JsonText::new("null".into()).unwrap()));
    w.set(vid(4), &null).unwrap();
    let c = roundtrip(
        &w.checkpoint(
            &doc,
            &[vid(1), vid(2), vid(3), vid(4)],
            CheckpointLimits::default(),
        )
        .unwrap(),
    );
    let restored = Workspace::from_checkpoint(&doc, r, WorkspaceLimits::default(), &c).unwrap();
    let s = restored.snapshot();
    assert_eq!(s.get(vid(1)).unwrap(), None);
    assert_eq!(s.get(vid(2)).unwrap(), Some(&Value::None));
    assert_eq!(
        s.get(vid(3)).unwrap(),
        Some(&Value::Some(Arc::new(Value::None)))
    );
    assert_eq!(s.get(vid(4)).unwrap(), Some(&null));
}
#[test]
fn only_explicit_persistent_selection_carries_and_other_cells_reset() {
    let (doc, r) = fixture(vec![
        variable(1, "workspace", "core/u64@1", Some(json!("1")), true),
        variable(2, "workspace", "core/u64@1", Some(json!("2")), true),
        variable(3, "workspace", "core/u64@1", Some(json!("3")), false),
    ]);
    let mut w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    for n in 1..=3 {
        w.set(vid(n), &Value::U64(90 + n as u64)).unwrap();
    }
    let c = w
        .checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
        .unwrap();
    let restored =
        Workspace::from_checkpoint(&doc, r, WorkspaceLimits::default(), &roundtrip(&c)).unwrap();
    assert_eq!(
        restored.snapshot().get(vid(1)).unwrap(),
        Some(&Value::U64(91))
    );
    assert_eq!(
        restored.snapshot().get(vid(2)).unwrap(),
        Some(&Value::U64(2))
    );
    assert_eq!(
        restored.snapshot().get(vid(3)).unwrap(),
        Some(&Value::U64(3))
    );
    assert_eq!(
        w.checkpoint(&doc, &[vid(3)], CheckpointLimits::default())
            .unwrap_err(),
        Error::Selection
    );
    assert_eq!(
        w.checkpoint(&doc, &[vid(1), vid(1)], CheckpointLimits::default())
            .unwrap_err(),
        Error::Selection
    );
}
#[test]
fn stopped_run_restores_values_before_on_start_without_reviving_old_clock_or_work() {
    let (doc, plan) = run_fixture();
    let r = stopped(&doc, &plan);
    let c = roundtrip(
        &r.checkpoint(&doc, &[vid(100)], CheckpointLimits::default())
            .unwrap(),
    );
    let mut restored = Run::start_from_checkpoint(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        88,
        RunOptions::default(),
        &c,
    )
    .unwrap();
    assert_ne!(r.id(), restored.id());
    assert_eq!(restored.now(), 0);
    assert!(restored.trace().is_empty());
    assert_eq!(restored.variable(vid(100)), Some(&Value::U64(9)));
    assert_eq!(restored.variable(vid(101)), Some(&Value::U64(2)));
    assert_eq!(restored.ownership().effects, 0);
    assert_eq!(restored.ownership().resources, 0);
    idle(&mut restored);
    assert_eq!(restored.variable(vid(101)), Some(&Value::U64(8)));
}
#[test]
fn running_and_disposed_runs_cannot_checkpoint() {
    let (doc, plan) = run_fixture();
    let mut r = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    assert_eq!(
        r.checkpoint(&doc, &[vid(100)], CheckpointLimits::default())
            .unwrap_err(),
        Error::Boundary
    );
    idle(&mut r);
    r.stop().unwrap();
    idle(&mut r);
    r.dispose().unwrap();
    assert_eq!(
        r.checkpoint(&doc, &[], CheckpointLimits::default())
            .unwrap_err(),
        Error::Boundary
    );
}
#[test]
fn malformed_truncated_duplicate_unknown_version_and_byte_limits_fail_without_editing_source() {
    let (doc, r) = fixture(vec![variable(
        1,
        "workspace",
        "core/u64@1",
        Some(json!("1")),
        true,
    )]);
    let w = Workspace::new(&doc, r, WorkspaceLimits::default()).unwrap();
    let bytes = w
        .checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
        .unwrap()
        .to_bytes()
        .unwrap();
    let saved = bytes.clone();
    assert_eq!(
        ValueCheckpoint::from_bytes(
            &bytes,
            CheckpointLimits {
                max_bytes: 1,
                ..CheckpointLimits::default()
            }
        )
        .unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        ValueCheckpoint::from_bytes(
            &bytes,
            CheckpointLimits {
                max_variables: 0,
                ..CheckpointLimits::default()
            }
        )
        .unwrap_err(),
        Error::Limit
    );
    assert!(
        ValueCheckpoint::from_bytes(&bytes[..bytes.len() - 1], CheckpointLimits::default())
            .is_err()
    );
    let raw: Json = serde_json::from_slice(&bytes).unwrap();
    for change in 0..5 {
        let mut raw = raw.clone();
        match change {
            0 => raw["format_version"] = json!(2),
            1 => raw["extra"] = json!("secret"),
            2 => {
                let v = raw["values"][0].clone();
                raw["values"].as_array_mut().unwrap().push(v);
            }
            3 => raw["compatibility"] = json!("invalid"),
            _ => {
                raw["values"][0].as_object_mut().unwrap().remove("value");
            }
        };
        assert!(
            ValueCheckpoint::from_bytes(
                &serde_json::to_vec(&raw).unwrap(),
                CheckpointLimits::default()
            )
            .is_err()
        );
    }
    assert_eq!(bytes, saved);
    assert_eq!(w.snapshot().get(vid(1)).unwrap(), Some(&Value::U64(1)));
}
#[test]
fn incompatible_owner_type_scope_and_payload_refuse_before_new_actor_publication() {
    let (doc, r) = fixture(vec![variable(
        1,
        "workspace",
        "core/u64@1",
        Some(json!("1")),
        true,
    )]);
    let w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    let c = roundtrip(
        &w.checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
            .unwrap(),
    );
    let bytes = c.to_bytes().unwrap();
    let before = w.meter().retention();
    for change in 0..4 {
        let changed = edit(&doc, |d| match change {
            0 => d["graph_id"] = json!(id(999)),
            1 => {
                d["variables"][0]["type"] = json!("core/i64@1");
            }
            2 => d["variables"][0]["persistent"] = json!(false),
            _ => d["variables"][0]["scope"] = json!("run"),
        });
        assert!(
            Workspace::from_checkpoint(&changed, r.clone(), WorkspaceLimits::default(), &c)
                .is_err()
        );
    }
    let mut raw: Json = serde_json::from_slice(&bytes).unwrap();
    raw["values"][0]["value"]["data"] = json!(true);
    let bad = ValueCheckpoint::from_bytes(
        &serde_json::to_vec(&raw).unwrap(),
        CheckpointLimits::default(),
    )
    .unwrap();
    assert!(Workspace::from_checkpoint(&doc, r.clone(), WorkspaceLimits::default(), &bad).is_err());
    assert_eq!(w.meter().retention(), before);
    assert_eq!(c.to_bytes().unwrap(), bytes);
}
#[test]
fn renamed_labels_and_changed_initializers_do_not_change_checkpoint_identity() {
    let (doc, r) = fixture(vec![variable(
        1,
        "workspace",
        "core/u64@1",
        Some(json!("1")),
        true,
    )]);
    let mut w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    w.set(vid(1), &Value::U64(77)).unwrap();
    let c = roundtrip(
        &w.checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
            .unwrap(),
    );
    let changed = edit(&doc, |d| {
        d["variables"][0]["name"] = json!("renamed");
        d["variables"][0]["init"]["value"] = json!("999");
    });
    let new = Workspace::from_checkpoint(&changed, r, WorkspaceLimits::default(), &c).unwrap();
    assert_eq!(new.snapshot().get(vid(1)).unwrap(), Some(&Value::U64(77)));
}
#[test]
fn nominal_record_and_enum_codecs_roundtrip_and_changed_schema_is_rejected() {
    let (p, r) = crate::record_declarations::catalog();
    let base = graph(&p, vec![], vec![], vec![]);
    let doc = edit(&base, |d| {
        d["variables"] = json!([
            variable(1, "workspace", "org.example/point@1", None, true),
            variable(2, "workspace", "org.example/shape@1", None, true)
        ])
    });
    let mut w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    for (n, ty, payload) in [
        (1, "org.example/point@1", json!({"x":"4"})),
        (2, "org.example/shape@1", json!({"variant":"empty"})),
    ] {
        let value = literal_codec::decode(
            &Literal::value(ty.parse().unwrap(), payload),
            r.types().as_ref(),
        )
        .unwrap();
        w.set(vid(n), &value).unwrap();
    }
    let c = roundtrip(
        &w.checkpoint(&doc, &[vid(1), vid(2)], CheckpointLimits::default())
            .unwrap(),
    );
    let new = Workspace::from_checkpoint(&doc, r, WorkspaceLimits::default(), &c).unwrap();
    assert_eq!(
        new.snapshot().get(vid(1)).unwrap(),
        w.snapshot().get(vid(1)).unwrap()
    );
    assert_eq!(
        new.snapshot().get(vid(2)).unwrap(),
        w.snapshot().get(vid(2)).unwrap()
    );
    let point = w
        .checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
        .unwrap();
    let mut types = orange_model::types::TypeRegistryBuilder::core();
    types
        .register_record(
            "org.example",
            &"org.example/point@1".parse().unwrap(),
            literal_codec::RecordSchema {
                fields: vec![
                    literal_codec::FieldSchema {
                        field_id: "x".into(),
                        ty: orange_model::type_term::TypeTerm::parse(
                            &"core/i64@1".parse().unwrap(),
                        )
                        .unwrap(),
                        optional: false,
                    },
                    literal_codec::FieldSchema {
                        field_id: "label".into(),
                        ty: orange_model::type_term::TypeTerm::parse(
                            &"core/string@1".parse().unwrap(),
                        )
                        .unwrap(),
                        optional: false,
                    },
                ],
            },
        )
        .unwrap();
    for constraint in [
        orange_model::types::Constraint::Eq,
        orange_model::types::Constraint::Hash,
    ] {
        types
            .derive_constraint(
                "org.example",
                &"org.example/point@1".parse().unwrap(),
                constraint,
            )
            .unwrap();
    }
    let changed = orange_model::catalog::RegistryBuilder::from_snapshot(w.snapshot().registry())
        .types(Arc::new(types.build().unwrap()))
        .build()
        .unwrap();
    assert_eq!(
        Workspace::from_checkpoint(&doc, changed, WorkspaceLimits::default(), &point).unwrap_err(),
        Error::Incompatible
    );
}
#[test]
fn unsupported_resource_stream_result_and_error_types_are_never_restored() {
    let (doc, r) = fixture(vec![variable(1, "workspace", "core/u64@1", None, true)]);
    let w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    let c = w
        .checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
        .unwrap();
    for ty in [
        "core/handle@1<core/u64@1>",
        "core/stream@1<core/u64@1>",
        "core/result@1<core/u64@1>",
        "core/error@1",
    ] {
        let changed = edit(&doc, |d| d["variables"][0]["type"] = json!(ty));
        assert_eq!(
            Workspace::from_checkpoint(&changed, r.clone(), WorkspaceLimits::default(), &c)
                .unwrap_err(),
            Error::UnsupportedValue
        );
    }
}
#[test]
fn changed_package_codec_and_unavailable_provider_identity_are_refused() {
    let (p, r) = catalog();
    let base = graph(
        &p,
        vec![crate::data_graph::math(9, "add", 1, 2)],
        vec![],
        vec![],
    );
    let doc = edit(&base, |d| {
        d["variables"] = json!([variable(
            1,
            "workspace",
            "core/u64@1",
            Some(json!("4")),
            true
        )]);
        d["type_lock"] = json!([{"type":"core/u64@1","codec":"orange.value.v1"}]);
    });
    let w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    let c = w
        .checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
        .unwrap();
    for mode in 0..3 {
        let changed = edit(&doc, |d| match mode {
            0 => {
                d["package_lock"][0]["content_digest"] = json!(format!("sha256:{}", "d".repeat(64)))
            }
            1 => d["type_lock"][0]["codec"] = json!("unavailable.codec.v2"),
            _ => d["nodes"][0]["provider"]["id"] = json!("unavailable"),
        });
        assert_eq!(
            Workspace::from_checkpoint(&changed, r.clone(), WorkspaceLimits::default(), &c)
                .unwrap_err(),
            Error::Incompatible
        );
    }
}
#[test]
fn artifact_and_restore_value_state_metadata_quotas_are_atomic() {
    let (doc, r) = fixture(vec![variable(1, "workspace", "core/string@1", None, true)]);
    let mut w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    w.set(vid(1), &Value::Str(Arc::new("x".repeat(30))))
        .unwrap();
    for limits in [
        CheckpointLimits {
            max_value_bytes: 1,
            ..CheckpointLimits::default()
        },
        CheckpointLimits {
            max_state_bytes: 1,
            ..CheckpointLimits::default()
        },
        CheckpointLimits {
            max_identity_items: 0,
            ..CheckpointLimits::default()
        },
        CheckpointLimits {
            max_bytes: 1,
            ..CheckpointLimits::default()
        },
    ] {
        let before = w.meter().retention();
        assert_eq!(
            w.checkpoint(&doc, &[vid(1)], limits).unwrap_err(),
            Error::Limit
        );
        assert_eq!(w.meter().retention(), before);
    }
    let c = w
        .checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
        .unwrap();
    let bytes = c.to_bytes().unwrap();
    for limits in [
        CheckpointLimits {
            max_value_bytes: 1,
            ..CheckpointLimits::default()
        },
        CheckpointLimits {
            max_state_bytes: 1,
            ..CheckpointLimits::default()
        },
    ] {
        let read = ValueCheckpoint::from_bytes(&bytes, limits).unwrap();
        assert_eq!(
            Workspace::from_checkpoint(&doc, r.clone(), WorkspaceLimits::default(), &read)
                .unwrap_err(),
            Error::Limit
        );
    }
    let before = w.meter().retention();
    assert_eq!(
        Workspace::from_checkpoint(
            &doc,
            r,
            WorkspaceLimits {
                max_state_bytes: 1,
                ..WorkspaceLimits::default()
            },
            &c
        )
        .unwrap_err(),
        Error::Limit
    );
    assert_eq!(w.meter().retention(), before);
}
#[test]
fn restoring_small_selected_values_does_not_admit_discarded_large_initializers() {
    let (doc, r) = fixture(vec![variable(1, "workspace", "core/string@1", None, true)]);
    let mut w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    w.set(vid(1), &Value::Str(Arc::new("ok".into()))).unwrap();
    let c = w
        .checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
        .unwrap();
    let changed = edit(&doc, |d| {
        d["variables"][0]["init"] = json!({"type":"core/string@1","value":"large".repeat(1000)})
    });
    let restored = Workspace::from_checkpoint(
        &changed,
        r,
        WorkspaceLimits {
            max_state_bytes: 32,
            max_value_bytes: 32,
            ..WorkspaceLimits::default()
        },
        &c,
    )
    .unwrap();
    assert_eq!(
        restored.snapshot().get(vid(1)).unwrap(),
        Some(&Value::Str(Arc::new("ok".into())))
    );
}
#[test]
fn duplicate_wire_keys_are_rejected_but_opaque_json_text_is_preserved_verbatim() {
    let (doc, r) = fixture(vec![variable(1, "workspace", "core/json@1", None, true)]);
    let mut w = Workspace::new(&doc, r.clone(), WorkspaceLimits::default()).unwrap();
    let value = Value::Json(Arc::new(
        JsonText::new(" { \"a\":1, \"a\":2 } ".into()).unwrap(),
    ));
    w.set(vid(1), &value).unwrap();
    let c = roundtrip(
        &w.checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
            .unwrap(),
    );
    let restored = Workspace::from_checkpoint(&doc, r, WorkspaceLimits::default(), &c).unwrap();
    assert_eq!(restored.snapshot().get(vid(1)).unwrap(), Some(&value));
    let bytes = c.to_bytes().unwrap();
    let source = String::from_utf8(bytes).unwrap().replace(
        "\"format_version\":1",
        "\"format_version\":1,\"format_version\":1",
    );
    assert_eq!(
        ValueCheckpoint::from_bytes(source.as_bytes(), CheckpointLimits::default()).unwrap_err(),
        Error::Invalid
    );
}
#[test]
fn portable_identity_matches_an_independent_dotnet_sha256_length_framed_vector() {
    let (doc, r) = fixture(vec![variable(
        1,
        "workspace",
        "core/u64@1",
        Some(json!("1")),
        true,
    )]);
    let w = Workspace::new(&doc, r, WorkspaceLimits::default()).unwrap();
    let checkpoint = w
        .checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
        .unwrap();
    let raw: Json = serde_json::from_slice(&checkpoint.to_bytes().unwrap()).unwrap();
    // Independently computed with .NET SHA256 over documented LE-u64 length frames.
    assert_eq!(
        raw["compatibility"],
        "666040c65d852ec6410c6cbbc1822a677b83f5fec305e3a7476da5ef48f3912e"
    );
    assert_eq!(raw["format"], "orange.checkpoint");
    assert_eq!(raw["format_version"], 1);
}
#[test]
fn checkpoint_and_refusal_debug_omit_value_and_display_payloads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<ValueCheckpoint>();
    let (doc, r) = fixture(vec![variable(
        1,
        "workspace",
        "core/string@1",
        Some(json!("secret-value-not-debug")),
        true,
    )]);
    let w = Workspace::new(&doc, r, WorkspaceLimits::default()).unwrap();
    let checkpoint = w
        .checkpoint(&doc, &[vid(1)], CheckpointLimits::default())
        .unwrap();
    for text in [
        format!("{checkpoint:?}"),
        format!("{:?}", Error::Incompatible),
        Error::Incompatible.to_string(),
    ] {
        assert!(!text.contains("secret-value-not-debug"));
        assert!(!text.contains("secret-display-name"));
    }
}
#[test]
fn run_restore_replaces_selected_initializers_before_combined_state_admission() {
    let (p, r) = catalog();
    let mut get = core(3, "get-variable");
    get["config"] = json!({"variable":id(100)});
    let mut set = core(2, "set-variable");
    set["config"] = json!({"variable":id(100)});
    set["inputs"] = json!({"value":{"type":"core/string@1","value":"ok"}});
    let mut sink = core(4, "set-variable");
    sink["config"] = json!({"variable":id(101)});
    let doc = edit(
        &graph(
            &p,
            vec![core(1, "on-start"), set, get, sink],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "then", 4, "in"),
                edge(3, 3, "value", 4, "value"),
            ],
            vec![],
        ),
        |d| {
            d["variables"] = json!([
                variable(100, "run", "core/string@1", None, true),
                variable(101, "run", "core/string@1", None, false)
            ])
        },
    );
    let plan = Arc::new(
        compile_control(
            &doc,
            r.clone(),
            &analyze(&doc, &r),
            &p,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let old = stopped(&doc, &plan);
    let c = old
        .checkpoint(&doc, &[vid(100)], CheckpointLimits::default())
        .unwrap();
    let changed = edit(&doc, |d| {
        d["variables"][0]["init"] = json!({"type":"core/string@1","value":"large".repeat(1000)})
    });
    let new_plan = Arc::new(
        compile_control(
            &changed,
            r.clone(),
            &analyze(&changed, &r),
            &p,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 34;
    let restored = Run::start_from_checkpoint(new_plan, &changed, &r, 1, options, &c).unwrap();
    assert_eq!(
        restored.variable(vid(100)),
        Some(&Value::Str(Arc::new("ok".into())))
    );
    assert!(restored.trace().is_empty());
}
#[test]
fn ic2_shared_values_events_backpressure_and_checkpoint_bootstrap_keep_live_ownership_separate() {
    use orange_runtime::{
        CloseReason, PullStream, PushAdmission, StreamBufferOptions, StreamDemand, StreamError,
        StreamOverflow, StreamRead, StreamStep, WorkspaceBus, WorkspaceBusLimits, WorkspaceQueue,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Producer(Arc<AtomicUsize>);
    impl PullStream for Producer {
        fn next(&mut self, _: StreamDemand) -> Result<StreamStep, orange_compiler::KernelFault> {
            Ok(StreamStep::End)
        }
        fn close(self: Box<Self>, _: CloseReason) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    const EVENT: &str = "40000000-0000-4000-8000-000000000001";
    let (package, registry) = catalog();
    let shared = variable(100, "workspace", "core/u64@1", Some(json!("5")), true);
    let mut update = core(2, "update-shared");
    update["config"] = json!({"variable":id(100),"operation":"add"});
    update["inputs"] = json!({"value":{"type":"core/u64@1","value":"1"}});
    let mut emit = core(3, "emit");
    emit["config"] = json!({"event":EVENT});
    emit["inputs"] = json!({"payload":{"type":"core/u64@1","value":"1"}});
    let events = json!([{"id":EVENT,"name":"ready","payload":"core/u64@1","scope":"workspace"}]);
    let source = edit(
        &graph(
            &package,
            vec![core(1, "on-start"), update, emit],
            vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
            vec![],
        ),
        |d| {
            d["variables"] = json!([shared.clone()]);
            d["events"] = events.clone();
        },
    );
    let mut entry = core(1, "on-event");
    entry["config"] = json!({"event":EVENT});
    entry["activation"] =
        json!({"mode":"queue","capacity":4,"overflow":"fault","on_fault":"continue","order":0});
    let mut set = core(2, "set-variable");
    set["config"] = json!({"variable":id(101)});
    let mut get = core(3, "get-variable");
    get["config"] = json!({"variable":id(100)});
    let target = edit(
        &graph(
            &package,
            vec![entry, set, get],
            vec![edge(1, 1, "then", 2, "in"), edge(2, 3, "value", 2, "value")],
            vec![],
        ),
        |d| {
            d["variables"] = json!([shared, variable(101, "run", "core/u64@1", None, true)]);
            d["events"] = events;
        },
    );
    let compile = |doc: &GraphDocument| {
        Arc::new(
            compile_control(
                doc,
                registry.clone(),
                &analyze(doc, &registry),
                &package,
                CompileOptions::default(),
            )
            .unwrap(),
        )
    };
    let source_plan = compile(&source);
    let target_plan = compile(&target);
    let mut workspace =
        Workspace::new(&source, registry.clone(), WorkspaceLimits::default()).unwrap();
    let mut bus = WorkspaceBus::new(
        &source,
        registry.clone(),
        &workspace.snapshot(),
        WorkspaceBusLimits::default(),
    )
    .unwrap();
    let mut a = Run::start_in_workspace(
        source_plan,
        &source,
        &registry,
        1,
        RunOptions::default(),
        &workspace.snapshot(),
    )
    .unwrap();
    let mut b = Run::start_in_workspace(
        target_plan,
        &target,
        &registry,
        2,
        RunOptions::default(),
        &workspace.snapshot(),
    )
    .unwrap();
    for run in [&mut a, &mut b] {
        run.attach_workspace_bus(&mut bus, WorkspaceQueue::default())
            .unwrap();
    }
    let push = a
        .open_push_stream(
            &"core/u64@1".parse().unwrap(),
            StreamBufferOptions {
                capacity: 1,
                max_bytes: 16,
                overflow: StreamOverflow::Backpressure,
            },
        )
        .unwrap();
    let mut consumer = a.claim_stream(push.stream).unwrap();
    assert_eq!(
        a.push_stream(&push.source, 1, &Value::U64(7)),
        Ok(PushAdmission::Accepted)
    );
    assert_eq!(
        a.push_stream(&push.source, 2, &Value::U64(8)),
        Err(StreamError::Full)
    );
    assert!(matches!(
        a.next_stream(&mut consumer).unwrap(),
        StreamRead::Item(Value::U64(7))
    ));
    assert_eq!(
        a.push_stream(&push.source, 2, &Value::U64(8)),
        Ok(PushAdmission::Accepted)
    );
    let closed = Arc::new(AtomicUsize::new(0));
    a.open_stream(
        &"core/u64@1".parse().unwrap(),
        Box::new(Producer(closed.clone())),
    )
    .unwrap();
    while a.pump_with_workspace_bus(&mut bus, 1).unwrap().ready {}
    let receipt = workspace
        .apply_shared(a.take_shared().unwrap().unwrap())
        .unwrap();
    a.complete_shared(receipt).unwrap();
    while a.pump_with_workspace_bus(&mut bus, 1).unwrap().ready {}
    b.publish_workspace(&workspace.snapshot()).unwrap();
    let delivery = b.take_workspace_delivery(&mut bus).unwrap().unwrap();
    b.ingest_workspace(delivery).unwrap();
    while b.pump_with_workspace_bus(&mut bus, 1).unwrap().ready {}
    assert_eq!(b.variable(vid(101)), Some(&Value::U64(6)));
    a.stop().unwrap();
    idle(&mut a);
    b.stop().unwrap();
    idle(&mut b);
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    let workspace_c = roundtrip(
        &workspace
            .checkpoint(&source, &[vid(100)], CheckpointLimits::default())
            .unwrap(),
    );
    let run_c = roundtrip(
        &b.checkpoint(&target, &[vid(101)], CheckpointLimits::default())
            .unwrap(),
    );
    let (fresh_package, fresh_registry) = catalog();
    assert_ne!(fresh_registry.generation(), registry.generation());
    let mut restored_workspace = Workspace::from_checkpoint(
        &source,
        fresh_registry.clone(),
        WorkspaceLimits::default(),
        &workspace_c,
    )
    .unwrap();
    let fresh_plan = Arc::new(
        compile_control(
            &target,
            fresh_registry.clone(),
            &analyze(&target, &fresh_registry),
            &fresh_package,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let mut restored = Run::start_in_workspace_from_checkpoint(
        fresh_plan,
        &target,
        &fresh_registry,
        3,
        RunOptions::default(),
        &restored_workspace.snapshot(),
        &run_c,
    )
    .unwrap();
    assert_eq!(restored.variable(vid(101)), Some(&Value::U64(6)));
    assert_eq!(restored.ownership().resources, 0);
    assert_eq!(restored.ownership().effects, 0);
    assert!(restored.take_shared().unwrap().is_none());
    assert!(restored.take_workspace_delivery(&mut bus).is_err());
    restored_workspace.set(vid(100), &Value::U64(20)).unwrap();
    assert_eq!(
        workspace.snapshot().get(vid(100)).unwrap(),
        Some(&Value::U64(6))
    );
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    restored.stop().unwrap();
    idle(&mut restored);
}
#[test]
fn capture_decode_and_restore_start_never_invoke_an_effect_callback() {
    use orange_compiler::{
        ActionKernel, BindingRequest, EffectBinding, EffectResolver, KernelFault, ResolveError,
        compile_control_with_effects,
    };
    use orange_model::catalog::{
        NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Touch(Arc<AtomicUsize>);
    impl ActionKernel for Touch {
        fn execute(&self, _: &[Value], _: &mut [Value]) -> Result<(), KernelFault> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    struct Adapter(Arc<AtomicUsize>);
    impl EffectResolver for Adapter {
        fn resolve_effect(&self, r: &BindingRequest<'_>) -> Result<EffectBinding, ResolveError> {
            Ok(EffectBinding {
                route: 1,
                context: Arc::new(r.provider().unwrap().clone()),
                inputs: Box::new([]),
                outputs: Box::new([]),
                action: Some(Arc::new(Touch(self.0.clone()))),
            })
        }
    }
    let (p, base) = catalog();
    let definition=NodeDefinition::from_json(&json!({"definition_id":"acme.checkpoint/touch","definition_version":1,"config_version":1,"kind":"action","suspends":"never","determinism":"external","ports":[]}).to_string()).unwrap();
    let Ports::Declared(ports) = &definition.ports else {
        panic!()
    };
    let signature =
        orange_model::signature::project(definition.kind, ports.clone(), &Default::default());
    let key = ProviderKey {
        package: "acme.checkpoint".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "c".repeat(64)),
        id: "native".into(),
    };
    let provider = ProviderDescriptor {
        key: key.clone(),
        implements: [(definition.definition.clone(), 1)].into(),
        specializations: Default::default(),
    };
    let registry = RegistryBuilder::from_snapshot(&base)
        .definition(definition)
        .provider(provider)
        .build()
        .unwrap();
    let touch = json!({"id":id(2),"definition_id":"acme.checkpoint/touch","definition_version":1,"config_version":1,"config":{},"signature":"touch","provider":{"package":key.package,"version":key.version,"id":key.id}});
    let mut set = core(3, "set-variable");
    set["config"] = json!({"variable":id(100)});
    set["inputs"] = json!({"value":{"type":"core/u64@1","value":"9"}});
    let doc = edit(
        &graph(
            &p,
            vec![core(1, "on-start"), touch, set],
            vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
            vec![],
        ),
        |d| {
            d["package_lock"].as_array_mut().unwrap().push(
                json!({"id":key.package,"version":key.version,"content_digest":key.content_digest}),
            );
            d["signatures"]["touch"] = serde_json::to_value(signature).unwrap();
            d["variables"] = json!([variable(100, "run", "core/u64@1", None, true)]);
        },
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter = Adapter(calls.clone());
    let plan = Arc::new(
        compile_control_with_effects(
            &doc,
            registry.clone(),
            &analyze(&doc, &registry),
            &p,
            &adapter,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let old = stopped(&doc, &plan);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let checkpoint = roundtrip(
        &old.checkpoint(&doc, &[vid(100)], CheckpointLimits::default())
            .unwrap(),
    );
    let mut restored =
        Run::start_from_checkpoint(plan, &doc, &registry, 1, RunOptions::default(), &checkpoint)
            .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(restored.variable(vid(100)), Some(&Value::U64(9)));
    assert!(restored.trace().is_empty());
    idle(&mut restored);
    assert_eq!(calls.load(Ordering::Relaxed), 2); // fresh On Start, only after explicit pump
}
#[test]
fn initial_watch_uses_restored_value_before_fresh_on_start_changes_it() {
    let (source, source_plan) = run_fixture();
    let old = stopped(&source, &source_plan);
    let c = old
        .checkpoint(&source, &[vid(100)], CheckpointLimits::default())
        .unwrap();
    let (package, registry) = catalog();
    let mut set = core(2, "set-variable");
    set["config"] = json!({"variable":id(100)});
    set["inputs"] = json!({"value":{"type":"core/u64@1","value":"1"}});
    let mut get = core(3, "get-variable");
    get["config"] = json!({"variable":id(100)});
    let mut watch = core(4, "on-watch");
    watch["config"] = json!({"mode":"changed"});
    watch["type_pins"] = json!({"T":"core/u64@1"});
    watch["activation"] =
        json!({"mode":"queue","capacity":4,"overflow":"fault","on_fault":"continue","order":0});
    let mut sink = core(5, "set-variable");
    sink["config"] = json!({"variable":id(101)});
    let doc = edit(
        &graph(
            &package,
            vec![core(1, "on-start"), set, get, watch, sink],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 3, "value", 4, "watch"),
                edge(3, 4, "then", 5, "in"),
                edge(4, 4, "value", 5, "value"),
            ],
            vec![],
        ),
        |d| {
            d["variables"] = json!([
                variable(100, "run", "core/u64@1", Some(json!("1")), true),
                variable(101, "run", "core/u64@1", Some(json!("2")), false)
            ])
        },
    );
    let plan = Arc::new(
        compile_control(
            &doc,
            registry.clone(),
            &analyze(&doc, &registry),
            &package,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let mut run =
        Run::start_from_checkpoint(plan, &doc, &registry, 3, RunOptions::default(), &c).unwrap();
    assert_eq!(run.variable(vid(100)), Some(&Value::U64(9)));
    idle(&mut run);
    assert_eq!(run.variable(vid(101)), Some(&Value::U64(1)));
}
