//! L04b record/enum construction and projection: derived contracts, E-NOMINAL-CONFIG,
//! optional-field and payload boxing, checks and compiled folds.
use std::sync::Arc;

use orange_bundled::{
    BindingError, KernelError, MathPackage, NominalOperation as Op, ResolvedNominal,
};
use orange_compiler::{CompileOptions, FoldedValue, compile_pure};
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::document::DefinitionRef;
use orange_model::type_term::TypeTerm;
use orange_model::types::{Constraint, TypeRegistryBuilder};
use orange_model::validation::{DiagnosticCode, HostOrigin};
use orange_model::value::RecordRepr;
use orange_model::value::literal_codec::{EnumSchema, FieldSchema, RecordSchema, VariantSchema};
use orange_model::{GraphDocument, Value, analyze, from_json};
use serde_json::{Value as Json, json};

use crate::data_graph::{edge, id};

const POINT: &str = "org.example/point@1";
const HOLDER: &str = "org.example/holder@1";
const SHAPE: &str = "org.example/shape@1";
const SLOT: &str = "org.example/slot@1";

/// `point { x: i64, label?: string }`, `holder { maybe?: option<i64>, count: u32 }`,
/// `shape { dot(point), empty }` and `slot { full(option<i64>), none }`.
fn catalog() -> (MathPackage, Arc<RegistrySnapshot>) {
    let mut types = TypeRegistryBuilder::core();
    let term = |ty: &str| TypeTerm::parse(&ty.parse().unwrap()).unwrap();
    let field = |id: &str, ty: &str, optional| FieldSchema {
        field_id: id.into(),
        ty: term(ty),
        optional,
    };
    for (ty, fields) in [
        (
            POINT,
            vec![
                field("x", "core/i64@1", false),
                field("label", "core/string@1", true),
            ],
        ),
        (
            HOLDER,
            vec![
                field("maybe", "core/option@1<core/i64@1>", true),
                field("count", "core/u32@1", false),
            ],
        ),
    ] {
        types
            .register_record("org.example", &ty.parse().unwrap(), RecordSchema { fields })
            .unwrap();
    }
    let variant = |id: &str, payload: Option<&str>| VariantSchema {
        variant_id: id.into(),
        payload: payload.map(term),
    };
    for (ty, variants) in [
        (
            SHAPE,
            vec![variant("dot", Some(POINT)), variant("empty", None)],
        ),
        (
            SLOT,
            vec![
                variant("full", Some("core/option@1<core/i64@1>")),
                variant("none", None),
            ],
        ),
    ] {
        types
            .register_enum("org.example", &ty.parse().unwrap(), EnumSchema { variants })
            .unwrap();
    }
    for ty in [POINT, SHAPE] {
        types
            .derive_constraint("org.example", &ty.parse().unwrap(), Constraint::Serialize)
            .unwrap();
    }
    let digest = format!("sha256:{}", "a".repeat(64));
    let package = MathPackage::new(digest.clone()).unwrap();
    let builder = RegistryBuilder::core(HostOrigin::new("0.0.0", digest).unwrap())
        .types(Arc::new(types.build().unwrap()));
    let snapshot = package.register(builder).unwrap().build().unwrap();
    (package, snapshot)
}

fn bind_in(
    package: &MathPackage,
    snapshot: Arc<RegistrySnapshot>,
    op: Op,
    config: Json,
) -> Result<ResolvedNominal, BindingError> {
    package.resolve_nominal(
        snapshot,
        package.collections_provider_key(),
        &DefinitionRef {
            id: op.definition_id().parse().unwrap(),
            version: 1,
        },
        1,
        &serde_json::from_value(config).unwrap(),
    )
}
fn bind(op: Op, config: Json) -> ResolvedNominal {
    let (package, snapshot) = catalog();
    bind_in(&package, snapshot, op, config).unwrap()
}
fn s(text: &str) -> Value {
    Value::Str(Arc::new(text.into()))
}
fn record(ty: &str, fields: Vec<Option<Value>>) -> Value {
    Value::Record(Arc::new(RecordRepr {
        ty: ty.parse().unwrap(),
        fields: fields.into(),
    }))
}

#[test]
fn records_build_and_project_with_missing_and_present_none_fields() {
    let make = bind(Op::MakeRecord, json!({"type":POINT}));
    assert_eq!(make.input_ports(), ["x", "label"]);
    let bare = make.evaluate(&[Value::I64(1), Value::None]).unwrap();
    assert_eq!(bare, record(POINT, vec![Some(Value::I64(1)), None]));
    let labelled = make.evaluate(&[Value::I64(2), s("a")]).unwrap();
    assert_eq!(
        labelled,
        record(POINT, vec![Some(Value::I64(2)), Some(s("a"))])
    );
    let get = |field: &str, value: &Value| {
        bind(Op::GetField, json!({"type":POINT,"field":field}))
            .evaluate(std::slice::from_ref(value))
    };
    assert_eq!(get("x", &labelled), Ok(Value::I64(2)));
    assert_eq!(get("label", &labelled), Ok(s("a")));
    assert_eq!(get("label", &bare), Ok(Value::None));

    // `maybe?: option<i64>` keeps missing (None) apart from a present None (boxed Some).
    let holder = bind(Op::MakeRecord, json!({"type":HOLDER}));
    let present_none = holder
        .evaluate(&[Value::Some(Arc::new(Value::None)), Value::U32(1)])
        .unwrap();
    assert_eq!(
        present_none,
        record(HOLDER, vec![Some(Value::None), Some(Value::U32(1))])
    );
    let missing = holder.evaluate(&[Value::None, Value::U32(1)]).unwrap();
    assert_eq!(missing, record(HOLDER, vec![None, Some(Value::U32(1))]));
    let maybe = bind(Op::GetField, json!({"type":HOLDER,"field":"maybe"}));
    assert_eq!(
        maybe.evaluate(&[present_none]),
        Ok(Value::Some(Arc::new(Value::None)))
    );
    assert_eq!(maybe.evaluate(&[missing]), Ok(Value::None));
    assert_eq!(
        holder.evaluate(&[Value::Some(Arc::new(Value::I64(4))), Value::U32(1)]),
        Ok(record(
            HOLDER,
            vec![Some(Value::I64(4)), Some(Value::U32(1))]
        ))
    );
    // Values outside the bound shape are contract errors, not silently stored.
    assert_eq!(
        make.evaluate(&[s("not a number"), Value::None]),
        Err(KernelError::InputContract)
    );
    assert_eq!(
        make.evaluate(&[Value::I64(1)]),
        Err(KernelError::InputContract)
    );
}

#[test]
fn enums_build_and_project_payloads_by_variant() {
    let point = record(POINT, vec![Some(Value::I64(5)), None]);
    let dot = bind(Op::MakeEnum, json!({"type":SHAPE,"variant":"dot"}))
        .evaluate(std::slice::from_ref(&point))
        .unwrap();
    assert_eq!(
        dot,
        Value::Enum {
            tag: 0,
            payload: Some(Arc::new(point.clone()))
        }
    );
    let empty_node = bind(Op::MakeEnum, json!({"type":SHAPE,"variant":"empty"}));
    assert!(empty_node.input_ports().is_empty());
    let empty = empty_node.evaluate(&[]).unwrap();
    assert_eq!(
        empty,
        Value::Enum {
            tag: 1,
            payload: None
        }
    );
    let payload = bind(Op::EnumPayload, json!({"type":SHAPE,"variant":"dot"}));
    assert_eq!(payload.evaluate(std::slice::from_ref(&dot)), Ok(point));
    assert_eq!(payload.evaluate(&[empty]), Ok(Value::None));
    // An option payload that is itself None comes back boxed, never as a bare None.
    let full_none = bind(Op::MakeEnum, json!({"type":SLOT,"variant":"full"}))
        .evaluate(&[Value::None])
        .unwrap();
    assert_eq!(
        bind(Op::EnumPayload, json!({"type":SLOT,"variant":"full"})).evaluate(&[full_none]),
        Ok(Value::Some(Arc::new(Value::None)))
    );
    assert_eq!(
        bind(Op::MakeEnum, json!({"type":SHAPE,"variant":"dot"})).evaluate(&[Value::I64(1)]),
        Err(KernelError::InputContract)
    );
}

#[test]
fn bindings_and_contracts_refuse_unknown_types_members_and_fields() {
    let (package, snapshot) = catalog();
    for (op, config) in [
        (Op::MakeRecord, json!({"type":SHAPE})),
        (Op::MakeRecord, json!({"type":"org.example/missing@1"})),
        (Op::MakeRecord, json!({"type":POINT,"field":"x"})),
        (Op::GetField, json!({"type":POINT,"field":"y"})),
        (Op::GetField, json!({"type":POINT})),
        (Op::MakeEnum, json!({"type":POINT,"variant":"dot"})),
        (Op::MakeEnum, json!({"type":SHAPE,"variant":"square"})),
        (Op::EnumPayload, json!({"type":SHAPE,"variant":"empty"})),
        (Op::MakeRecord, json!({"type":"core/i64@1"})),
        (Op::MakeRecord, json!({})),
    ] {
        assert_eq!(
            bind_in(&package, snapshot.clone(), op, config.clone()).unwrap_err(),
            BindingError::InvalidConfig,
            "{op:?} {config}"
        );
        let mut placeholder = node(1, op, config.clone(), json!({}));
        placeholder["signature"] = json!("placeholder");
        let mut raw: Json = serde_json::from_str(
            &orange_model::to_json(&graph(&package, vec![placeholder], vec![], vec![])).unwrap(),
        )
        .unwrap();
        raw["signatures"] = json!({"placeholder":{"kind":"pure","ports":[]}});
        let doc = from_json(&raw.to_string()).unwrap();
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis
                .report()
                .diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::NominalConfig),
            "{op:?} {config}: {:?}",
            analysis.report().diagnostics
        );
    }
    let definition = DefinitionRef {
        id: Op::MakeRecord.definition_id().parse().unwrap(),
        version: 1,
    };
    assert_eq!(
        package
            .resolve_nominal(
                snapshot.clone(),
                package.provider_key(),
                &definition,
                1,
                &serde_json::from_value(json!({"type":POINT})).unwrap()
            )
            .unwrap_err(),
        BindingError::Unavailable
    );
    assert_eq!(DiagnosticCode::NominalConfig.as_str(), "E-NOMINAL-CONFIG");
}

fn literal(ty: &str, value: &Value) -> Json {
    let (_, snapshot) = catalog();
    serde_json::to_value(
        orange_model::value::literal_codec::encode(
            &ty.parse().unwrap(),
            value,
            snapshot.types().as_ref(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn node(n: u32, op: Op, config: Json, inputs: Json) -> Json {
    json!({"id":id(n),"definition_id":op.definition_id(),"definition_version":1,"config_version":1,
        "config":config,"provider":{"package":"org.orange.collections","version":"0.0.0","id":"native-v1"},
        "inputs":inputs})
}
fn graph(
    package: &MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
) -> GraphDocument {
    let key = package.collections_provider_key();
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000007","revision":0,
        "package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "nodes":nodes,"edges":edges,"exports":exports}).to_string()).unwrap()
}
/// Saves each node's derived contract as its signature, as an editor would.
fn with_signatures(doc: GraphDocument, snapshot: &RegistrySnapshot) -> GraphDocument {
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&doc).unwrap()).unwrap();
    let mut signatures = serde_json::Map::new();
    for node in raw["nodes"].as_array_mut().unwrap() {
        let instance = &doc.root().nodes[&node["id"]
            .as_str()
            .unwrap()
            .parse::<orange_model::id::NodeId>()
            .unwrap()];
        let contract = snapshot.contract(&doc, instance).unwrap().unwrap();
        let name = format!("s-{}", node["id"].as_str().unwrap());
        signatures.insert(
            name.clone(),
            serde_json::to_value(orange_model::signature::project(
                contract.kind,
                contract.ports.clone(),
                &Default::default(),
            ))
            .unwrap(),
        );
        node["signature"] = json!(name);
    }
    raw["signatures"] = Json::Object(signatures);
    from_json(&raw.to_string()).unwrap()
}

#[test]
fn compiled_record_and_enum_chains_fold() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            node(
                1,
                Op::MakeRecord,
                json!({"type":POINT}),
                json!({
                "x":literal("core/i64@1", &Value::I64(7)),
                "label":literal("core/option@1<core/string@1>", &s("seven"))}),
            ),
            node(
                2,
                Op::MakeEnum,
                json!({"type":SHAPE,"variant":"dot"}),
                json!({}),
            ),
            node(
                3,
                Op::EnumPayload,
                json!({"type":SHAPE,"variant":"dot"}),
                json!({}),
            ),
            node(
                4,
                Op::GetField,
                json!({"type":POINT,"field":"x"}),
                json!({}),
            ),
            node(
                5,
                Op::GetField,
                json!({"type":POINT,"field":"label"}),
                json!({}),
            ),
            node(
                6,
                Op::MakeEnum,
                json!({"type":SHAPE,"variant":"empty"}),
                json!({}),
            ),
        ],
        vec![
            edge(1, 1, "result", 2, "payload"),
            edge(2, 2, "result", 3, "value"),
            edge(3, 1, "result", 4, "record"),
            edge(4, 1, "result", 5, "record"),
        ],
        vec![
            json!({"id":"payload","node":id(3),"port":"result"}),
            json!({"id":"x","node":id(4),"port":"result"}),
            json!({"id":"label","node":id(5),"port":"result"}),
            json!({"id":"empty","node":id(6),"port":"result"}),
        ],
    );
    let doc = with_signatures(doc, &snapshot);
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
    let constant = |n: usize| plan.constants()[plan.exports()[n].slot.index()].clone();
    let point = record(POINT, vec![Some(Value::I64(7)), Some(s("seven"))]);
    assert_eq!(constant(0), FoldedValue::Value(point));
    assert_eq!(constant(1), FoldedValue::Value(Value::I64(7)));
    assert_eq!(constant(2), FoldedValue::Value(s("seven")));
    assert_eq!(
        constant(3),
        FoldedValue::Value(Value::Enum {
            tag: 1,
            payload: None
        })
    );
}

#[test]
fn set_field_replaces_one_field_and_can_clear_or_keep_present_none() {
    let point = record(POINT, vec![Some(Value::I64(1)), Some(s("a"))]);
    let set_x = bind(Op::SetField, json!({"type":POINT,"field":"x"}));
    assert_eq!(set_x.input_ports(), ["record", "value"]);
    assert_eq!(
        set_x.evaluate(&[point.clone(), Value::I64(9)]),
        Ok(record(POINT, vec![Some(Value::I64(9)), Some(s("a"))]))
    );
    let set_label = bind(Op::SetField, json!({"type":POINT,"field":"label"}));
    assert_eq!(
        set_label.evaluate(&[point.clone(), Value::None]),
        Ok(record(POINT, vec![Some(Value::I64(1)), None]))
    );
    let holder = record(HOLDER, vec![None, Some(Value::U32(1))]);
    let set_maybe = bind(Op::SetField, json!({"type":HOLDER,"field":"maybe"}));
    assert_eq!(
        set_maybe.evaluate(&[holder, Value::Some(Arc::new(Value::None))]),
        Ok(record(HOLDER, vec![Some(Value::None), Some(Value::U32(1))]))
    );
    assert_eq!(
        set_x.evaluate(&[point.clone(), s("nine")]),
        Err(KernelError::InputContract)
    );
    assert_eq!(set_x.evaluate(&[point]), Err(KernelError::InputContract));
    let (package, snapshot) = catalog();
    assert_eq!(
        bind_in(
            &package,
            snapshot,
            Op::SetField,
            json!({"type":SHAPE,"field":"x"})
        )
        .unwrap_err(),
        BindingError::InvalidConfig
    );
}

#[test]
fn compiled_set_field_uses_its_derived_contract() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            node(
                1,
                Op::MakeRecord,
                json!({"type":POINT}),
                json!({
                "x":literal("core/i64@1", &Value::I64(1)),
                "label":literal("core/option@1<core/string@1>", &Value::None)}),
            ),
            node(
                2,
                Op::SetField,
                json!({"type":POINT,"field":"label"}),
                json!({
                "value":literal("core/option@1<core/string@1>", &s("set"))}),
            ),
        ],
        vec![edge(1, 1, "result", 2, "record")],
        vec![json!({"id":"updated","node":id(2),"port":"result"})],
    );
    let doc = with_signatures(doc, &snapshot);
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
    assert_eq!(
        plan.constants()[plan.exports()[0].slot.index()],
        FoldedValue::Value(record(POINT, vec![Some(Value::I64(1)), Some(s("set"))]))
    );
}
