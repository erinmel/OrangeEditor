//! Package records from literal to run (11 §5.2, §10.2, §11.1; 12 §7): record literals,
//! variables, graph-local calls and event payloads resolve through the plan snapshot's
//! locked schemas, and the runtime handles the checked values generically.
use crate::data_graph as support;

use std::sync::Arc;

use orange_bundled::MathPackage;
use orange_compiler::{CompileError, CompileOptions, ControlPlan, compile_control};
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::type_term::TypeTerm;
use orange_model::types::{Constraint, TypeRegistryBuilder};
use orange_model::validation::{DiagnosticCode, HostOrigin};
use orange_model::value::literal_codec::{EnumSchema, FieldSchema, RecordSchema, VariantSchema};
use orange_model::value::{ListRepr, MapRepr, RecordRepr};
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use orange_runtime::{Run, RunOptions, RunState};
use serde_json::{Value as Json, json};
use support::{edge, graph, id};

const POINT: &str = "org.example/point@1";
const PATH: &str = "org.example/path@1";
const SHAPE: &str = "org.example/shape@1";
const LINK: &str = "org.example/link@1";
const EVENT: &str = "40000000-0000-4000-8000-000000000001";

/// The math package plus `point { x: i64, label?: string }` deriving `eq` and `hash`;
/// `path { points: list<point>, meta: map<string,i64> }` and the enum
/// `shape { dot(point), path(path), empty }` deriving `eq`; and the recursive
/// `link { next?: link }`.
fn catalog() -> (MathPackage, Arc<RegistrySnapshot>) {
    let mut types = TypeRegistryBuilder::core();
    let term = |ty: &str| TypeTerm::parse(&ty.parse().unwrap()).unwrap();
    let field = |id: &str, ty: &str, optional| FieldSchema {
        field_id: id.into(),
        ty: term(ty),
        optional,
    };
    let records = [
        (
            POINT,
            vec![
                field("x", "core/i64@1", false),
                field("label", "core/string@1", true),
            ],
        ),
        (
            PATH,
            vec![
                field("points", &format!("core/list@1<{POINT}>"), false),
                field("meta", "core/map@1<core/string@1,core/i64@1>", false),
            ],
        ),
        (LINK, vec![field("next", LINK, true)]),
    ];
    for (ty, fields) in records {
        types
            .register_record("org.example", &ty.parse().unwrap(), RecordSchema { fields })
            .unwrap();
    }
    let variant = |id: &str, payload: Option<&str>| VariantSchema {
        variant_id: id.into(),
        payload: payload.map(term),
    };
    let variants = vec![
        variant("dot", Some(POINT)),
        variant("path", Some(PATH)),
        variant("empty", None),
    ];
    types
        .register_enum(
            "org.example",
            &SHAPE.parse().unwrap(),
            EnumSchema { variants },
        )
        .unwrap();
    let derive = |types: &mut TypeRegistryBuilder, ty: &str, constraint| {
        types
            .derive_constraint("org.example", &ty.parse().unwrap(), constraint)
            .unwrap();
    };
    derive(&mut types, POINT, Constraint::Eq);
    derive(&mut types, POINT, Constraint::Hash);
    derive(&mut types, PATH, Constraint::Eq);
    derive(&mut types, SHAPE, Constraint::Eq);
    let digest = format!("sha256:{}", "a".repeat(64));
    let package = MathPackage::new(digest.clone()).unwrap();
    let builder = RegistryBuilder::core(HostOrigin::new("0.0.0", digest).unwrap())
        .types(Arc::new(types.build().unwrap()));
    let snapshot = package.register(builder).unwrap().build().unwrap();
    (package, snapshot)
}

fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id":id(n),"definition_id":format!("core/{name}"),
        "definition_version":1,"config_version":1,"config":{}});
    if matches!(name, "on-start" | "on-event") {
        node["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest",
            "on_fault":"continue","order":0});
    }
    node
}

fn var_node(n: u32, name: &str, var: u32) -> Json {
    let mut node = core(n, name);
    node["config"] = json!({"variable": id(var)});
    node
}

fn set(n: u32, var: u32, ty: &str, value: Json) -> Json {
    let mut node = var_node(n, "set-variable", var);
    node["inputs"] = json!({"value": {"type": ty, "value": value}});
    node
}

fn variable(n: u32, ty: &str, init: Option<Json>) -> Json {
    let mut v = json!({"id": id(n), "name": "secret name", "type": ty, "scope": "run"});
    if let Some(value) = init {
        v["init"] = json!({"type": ty, "value": value});
    }
    v
}

fn port(name: &str, direction: &str, ty: &str) -> Json {
    json!({"port_id":name,"direction":direction,"kind":"data","type":ty})
}

/// A non-generic pure function passing one `ty` value through.
fn pass(f: u32, ty: &str) -> Json {
    json!({"id":id(f),"name":"pass","kind":"pure",
        "inputs":[port("value","input",ty)],"outputs":[port("result","output",ty)],
        "nodes":[core(f+1,"function-entry"),core(f+2,"function-outputs")],
        "edges":[edge(f+1,f+1,"value",f+2,"result")]})
}

fn document(
    package: &MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    variables: Vec<Json>,
    events: Vec<Json>,
    functions: Vec<Json>,
) -> GraphDocument {
    let base = graph(package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!(variables);
    raw["events"] = json!(events);
    raw["functions"] = json!(functions);
    from_json(&raw.to_string()).unwrap()
}

fn compile(
    doc: &GraphDocument,
    package: &MathPackage,
    snapshot: &Arc<RegistrySnapshot>,
) -> Result<Arc<ControlPlan>, CompileError> {
    let analysis = analyze(doc, snapshot);
    assert!(
        analysis.is_ready_for_lowering(doc, snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    compile_control(
        doc,
        snapshot.clone(),
        &analysis,
        package,
        CompileOptions::default(),
    )
    .map(Arc::new)
}

fn run_to_idle(doc: &GraphDocument, plan: Arc<ControlPlan>) -> Run {
    let mut run = Run::start(
        plan.clone(),
        doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    for _ in 0..4096 {
        if run.state() == RunState::Stopped {
            return run;
        }
        let progress = run.pump(64).unwrap();
        if !progress.ready && run.event_stats().pending == 0 {
            return run;
        }
    }
    panic!("fixture did not idle");
}

fn text(s: &str) -> Value {
    Value::Str(Arc::new(s.into()))
}

fn record(ty: &str, fields: Vec<Option<Value>>) -> Value {
    Value::Record(Arc::new(RecordRepr {
        ty: ty.parse().unwrap(),
        fields: fields.into(),
    }))
}

fn point(x: i64, label: Option<&str>) -> Value {
    record(POINT, vec![Some(Value::I64(x)), label.map(text)])
}

fn path(points: Vec<Value>, meta: Vec<(&str, i64)>) -> Value {
    let meta = meta.into_iter().map(|(k, v)| (text(k), Value::I64(v)));
    record(
        PATH,
        vec![
            Some(Value::List(Arc::new(ListRepr::Values(points.into())))),
            Some(Value::Map(Arc::new(
                MapRepr::new(meta.collect::<Vec<_>>()).unwrap(),
            ))),
        ],
    )
}

fn stored(run: &Run, var: u32) -> Option<&Value> {
    run.variable(id(var).parse().unwrap())
}

/// on-start -> set 101 from a record literal -> set 102 from pass(get 100).
#[test]
fn record_literals_variables_and_calls_execute() {
    let (package, snapshot) = catalog();
    let mut call = core(5, "call");
    call["config"] = json!({"function": id(200)});
    let doc = document(
        &package,
        vec![
            core(1, "on-start"),
            set(2, 101, POINT, json!({"x": "5"})),
            var_node(3, "get-variable", 100),
            var_node(4, "set-variable", 102),
            call,
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 4, "in"),
            edge(3, 3, "value", 5, "value"),
            edge(4, 5, "result", 4, "value"),
        ],
        vec![
            variable(100, POINT, Some(json!({"x": "-1", "label": "a"}))),
            variable(101, POINT, None),
            variable(102, POINT, None),
        ],
        vec![],
        vec![pass(200, POINT)],
    );
    let run = run_to_idle(&doc, compile(&doc, &package, &snapshot).unwrap());
    // A missing optional field stays missing: an absent slot, never None or a default.
    assert_eq!(stored(&run, 101), Some(&point(5, None)));
    assert_eq!(stored(&run, 102), Some(&point(-1, Some("a"))));
    assert!(!run.trace().iter().any(|e| e.fault.is_some()));
    assert!(!format!("{run:?}").contains("secret"));
}

/// on-start -> emit(payload) ; on-event -> set var 100 from the payload.
fn emit_into_variable(package: &MathPackage, ty: &str, payload: Json) -> GraphDocument {
    let mut emit = core(2, "emit");
    emit["config"] = json!({"event": EVENT});
    emit["inputs"] = json!({"payload": {"type": ty, "value": payload}});
    let mut handler = core(3, "on-event");
    handler["config"] = json!({"event": EVENT});
    document(
        package,
        vec![
            core(1, "on-start"),
            emit,
            handler,
            var_node(4, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 3, "payload", 4, "value"),
        ],
        vec![variable(100, ty, None)],
        vec![json!({"id": EVENT, "name": "secret event", "payload": ty, "scope": "run"})],
        vec![],
    )
}

/// Records nested in records, lists, maps and enum payloads, and records keying maps
/// when their `eq` and `hash` are derived, travel through events typed and intact.
#[test]
fn record_event_payloads_nest_through_containers_and_enums() {
    let (package, snapshot) = catalog();
    let walk = json!({"points": [{"x": "1"}, {"x": "2", "label": "b"}],
        "meta": [["z", "1"], ["a", "2"]]});
    let walked = path(
        vec![point(1, None), point(2, Some("b"))],
        vec![("z", 1), ("a", 2)],
    );
    let keyed = format!("core/map@1<{POINT},core/u32@1>");
    let keys = MapRepr::new(vec![
        (point(1, None), Value::U32(5)),
        (point(1, Some("b")), Value::U32(6)),
    ])
    .unwrap();
    for (ty, payload, expected) in [
        (POINT, json!({"x": "7", "label": "p"}), point(7, Some("p"))),
        (PATH, walk.clone(), walked.clone()),
        (
            SHAPE,
            json!({"variant": "path", "payload": walk}),
            Value::Enum {
                tag: 1,
                payload: Some(Arc::new(walked)),
            },
        ),
        (
            &keyed,
            json!([[{"x": "1"}, 5], [{"x": "1", "label": "b"}, 6]]),
            Value::Map(Arc::new(keys)),
        ),
    ] {
        let doc = emit_into_variable(&package, ty, payload);
        let run = run_to_idle(&doc, compile(&doc, &package, &snapshot).unwrap());
        assert_eq!(stored(&run, 100), Some(&expected), "{ty}");
        assert_eq!(run.event_stats().delivered, 1, "{ty}");
    }
}

/// A literal that does not match its schema never reaches lowering; a recursive record
/// is valid metadata but has no executable shape, so lowering refuses it.
#[test]
fn bad_record_literals_and_recursive_records_are_refused() {
    let (package, snapshot) = catalog();
    let declared = |ty: &str, init: Json| {
        document(
            &package,
            vec![core(1, "on-start"), var_node(2, "get-variable", 100)],
            vec![],
            vec![variable(100, ty, Some(init))],
            vec![],
            vec![],
        )
    };
    for init in [
        json!({"label": "missing x"}),
        json!({"x": "1", "extra": true}),
        json!({"x": 1}),
        json!({"x": "1", "label": null}),
        json!(["1"]),
    ] {
        let doc = declared(POINT, init.clone());
        let analysis = analyze(&doc, &snapshot);
        assert!(!analysis.is_ready_for_lowering(&doc, &snapshot), "{init}");
        assert!(
            analysis
                .report()
                .diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::LiteralType),
            "{init}"
        );
    }
    let doc = declared(LINK, json!({"next": {}}));
    assert_eq!(
        compile(&doc, &package, &snapshot).unwrap_err(),
        CompileError::UnsupportedNode {
            node: id(2).parse().unwrap()
        }
    );
}
