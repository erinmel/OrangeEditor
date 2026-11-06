//! Enum-typed variables, event payloads and enum map keys from declaration to run
//! (11 §5.2, §5.4, §10.2, §11.1): declarations lower through the plan snapshot's locked
//! schemas, and the runtime handles the checked values generically.
use crate::data_graph as support;

use std::sync::Arc;

use orange_bundled::MathPackage;
use orange_compiler::{CompileError, CompileOptions, ControlPlan, compile_control};
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::document::ProviderBinding;
use orange_model::type_term::TypeTerm;
use orange_model::types::{
    Constraint, ConstraintId, ConstraintImplementationRef, TypeRegistryBuilder,
};
use orange_model::validation::{DiagnosticCode, HostOrigin};
use orange_model::value::MapRepr;
use orange_model::value::literal_codec::{EnumSchema, VariantSchema};
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use orange_runtime::{ControlOutcome, Run, RunOptions, RunState};
use serde_json::{Value as Json, json};
use support::{edge, graph, id};

const LIGHT: &str = "org.example/light@1";
const TAGGED: &str = "org.example/tagged@1";
const EVENT: &str = "40000000-0000-4000-8000-000000000001";

/// The math package plus `light` (red, amber(u32), green, then `extra`) deriving `eq`
/// and `hash`, and `tagged` (one, two) deriving `eq` with a package-implemented `hash`.
fn catalog(extra: &[&str]) -> (MathPackage, Arc<RegistrySnapshot>) {
    let mut types = TypeRegistryBuilder::core();
    let variant = |id: &str, payload: Option<&str>| VariantSchema {
        variant_id: id.into(),
        payload: payload.map(|ty| TypeTerm::parse(&ty.parse().unwrap()).unwrap()),
    };
    let mut lights = vec![
        variant("red", None),
        variant("amber", Some("core/u32@1")),
        variant("green", None),
    ];
    lights.extend(extra.iter().map(|id| variant(id, None)));
    let tags = vec![variant("one", None), variant("two", None)];
    for (ty, variants) in [(LIGHT, lights), (TAGGED, tags)] {
        types
            .register_enum("org.example", &ty.parse().unwrap(), EnumSchema { variants })
            .unwrap();
    }
    let light = LIGHT.parse().unwrap();
    for constraint in [Constraint::Eq, Constraint::Hash] {
        types
            .derive_constraint("org.example", &light, constraint)
            .unwrap();
    }
    let tagged = TAGGED.parse().unwrap();
    types
        .derive_constraint("org.example", &tagged, Constraint::Eq)
        .unwrap();
    types
        .register_constraint_implementation(
            "org.example",
            &TypeTerm::parse(&tagged).unwrap(),
            ConstraintId::Core(Constraint::Hash),
            ConstraintImplementationRef {
                provider: ProviderBinding {
                    package: "org.example".into(),
                    version: "1.0.0".into(),
                    id: "types".into(),
                    extensions: Default::default(),
                },
                content_digest: format!("sha256:{}", "b".repeat(64)),
                implementation_id: "tagged-hash".into(),
            },
        )
        .unwrap();
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

fn document(
    package: &MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    variables: Vec<Json>,
    events: Vec<Json>,
) -> GraphDocument {
    let base = graph(package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!(variables);
    raw["events"] = json!(events);
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

fn light(tag: u32, amber: Option<u32>) -> Value {
    Value::Enum {
        tag,
        payload: amber.map(|n| Arc::new(Value::U32(n))),
    }
}

/// on-start -> [set var 100] -> switch(get var 100) over every `light` variant.
fn switch_on_variable(package: &MathPackage, init: Json, assign: Option<Json>) -> GraphDocument {
    let mut switch = core(3, "switch");
    switch["type_pins"] = json!({"K": LIGHT});
    switch["config"] = json!({"cases": {"0": "red", "1": "amber", "2": "green"}});
    switch["variadic"] = json!({"case": {"order": [0, 1, 2], "next": 3}});
    let mut nodes = vec![
        core(1, "on-start"),
        switch,
        var_node(4, "get-variable", 100),
    ];
    let mut edges = vec![edge(2, 4, "value", 3, "key")];
    match assign {
        Some(value) => {
            nodes.push(set(2, 100, LIGHT, value));
            edges.push(edge(1, 1, "then", 2, "in"));
            edges.push(edge(3, 2, "then", 3, "in"));
        }
        None => edges.push(edge(1, 1, "then", 3, "in")),
    }
    document(
        package,
        nodes,
        edges,
        vec![variable(100, LIGHT, Some(init))],
        vec![],
    )
}

fn switch_outcome(run: &Run) -> ControlOutcome {
    run.trace()
        .iter()
        .find(|e| e.node.to_string() == id(3))
        .unwrap()
        .outcome
}

#[test]
fn enum_variables_initialize_from_their_schema_and_route_a_switch() {
    let (package, snapshot) = catalog(&[]);
    let amber = json!({"variant": "amber", "payload": 3});
    for (assign, stored, case) in [
        (None, light(1, Some(3)), 1),
        (Some(json!({"variant": "green"})), light(2, None), 2),
        (
            Some(json!({"variant": "amber", "payload": 9})),
            light(1, Some(9)),
            1,
        ),
    ] {
        let doc = switch_on_variable(&package, amber.clone(), assign);
        let run = run_to_idle(&doc, compile(&doc, &package, &snapshot).unwrap());
        assert_eq!(run.variable(id(100).parse().unwrap()), Some(&stored));
        assert_eq!(switch_outcome(&run), ControlOutcome::Case(case));
        assert!(!format!("{run:?}").contains("secret"));
    }
}

/// A schema gaining a variant (11 §5.3) keeps a stored initializer's meaning: the
/// variant ID decodes to the new snapshot's tag. An initializer naming a variant the
/// plan's snapshot lacks never reaches lowering.
#[test]
fn stored_enum_initializers_follow_the_plan_snapshot_across_a_variant_addition() {
    let (package, old) = catalog(&[]);
    let (_, grown) = catalog(&["blue"]);
    let doc = |init: Json| {
        document(
            &package,
            vec![
                core(1, "on-start"),
                set(2, 101, LIGHT, json!({"variant": "red"})),
            ],
            vec![edge(1, 1, "then", 2, "in")],
            vec![variable(100, LIGHT, Some(init)), variable(101, LIGHT, None)],
            vec![],
        )
    };
    // Unreferenced declarations are not lowered; reference 100 through a Get as well.
    let referenced = |init: Json| {
        let mut doc: Json = serde_json::from_str(&to_json(&doc(init)).unwrap()).unwrap();
        doc["nodes"]
            .as_array_mut()
            .unwrap()
            .push(var_node(5, "get-variable", 100));
        from_json(&doc.to_string()).unwrap()
    };
    for snapshot in [&old, &grown] {
        let green = referenced(json!({"variant": "green"}));
        let run = run_to_idle(&green, compile(&green, &package, snapshot).unwrap());
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&light(2, None))
        );
    }
    let blue = referenced(json!({"variant": "blue"}));
    let run = run_to_idle(&blue, compile(&blue, &package, &grown).unwrap());
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&light(3, None))
    );
    let analysis = analyze(&blue, &old);
    assert!(!analysis.is_ready_for_lowering(&blue, &old));
    assert!(
        analysis
            .report()
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::LiteralType)
    );
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
    )
}

fn keyed_map(entries: Vec<(Value, u32)>) -> Value {
    let entries: Vec<_> = entries
        .into_iter()
        .map(|(k, v)| (k, Value::U32(v)))
        .collect();
    Value::Map(Arc::new(MapRepr::new(entries).unwrap()))
}

#[test]
fn enum_and_enum_keyed_event_payloads_are_delivered_typed() {
    let (package, snapshot) = catalog(&[]);
    let keyed = format!("core/map@1<{LIGHT},core/u32@1>");
    for (ty, payload, expected) in [
        (
            LIGHT.to_owned(),
            json!({"variant": "amber", "payload": 7}),
            light(1, Some(7)),
        ),
        (
            keyed,
            json!([[{"variant": "green"}, 1], [{"variant": "amber", "payload": 2}, 3]]),
            keyed_map(vec![(light(2, None), 1), (light(1, Some(2)), 3)]),
        ),
    ] {
        let doc = emit_into_variable(&package, &ty, payload);
        let run = run_to_idle(&doc, compile(&doc, &package, &snapshot).unwrap());
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&expected),
            "{ty}"
        );
        assert_eq!(run.event_stats().delivered, 1);
    }
}

/// Map keys of an enum deriving `eq` and `hash` (11 §5.2, §5.4) execute; keys with the
/// same tag and different payloads stay distinct, and insertion order is kept.
#[test]
fn enum_keyed_map_variables_initialize_copy_and_keep_order() {
    let (package, snapshot) = catalog(&[]);
    let ty = format!("core/map@1<{LIGHT},core/u32@1>");
    let init = json!([
        [{"variant": "amber", "payload": 2}, 5],
        [{"variant": "red"}, 6],
        [{"variant": "amber", "payload": 1}, 7]
    ]);
    let doc = document(
        &package,
        vec![
            core(1, "on-start"),
            var_node(2, "set-variable", 101),
            var_node(3, "get-variable", 100),
        ],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 3, "value", 2, "value")],
        vec![variable(100, &ty, Some(init)), variable(101, &ty, None)],
        vec![],
    );
    let run = run_to_idle(&doc, compile(&doc, &package, &snapshot).unwrap());
    let expected = keyed_map(vec![
        (light(1, Some(2)), 5),
        (light(0, None), 6),
        (light(1, Some(1)), 7),
    ]);
    for var in [100, 101] {
        assert_eq!(run.variable(id(var).parse().unwrap()), Some(&expected));
    }
}

/// Validation accepts these, but the host owns no executable semantics for them, so
/// lowering refuses with the existing typed reason instead of inventing one.
#[test]
fn package_hash_keys_and_enum_compare_and_set_are_refused_at_lowering() {
    let (package, snapshot) = catalog(&[]);
    // `tagged`'s hash is package code: the map has no executable shape.
    let ty = format!("core/map@1<{TAGGED},core/u32@1>");
    let doc = document(
        &package,
        vec![core(1, "on-start"), var_node(2, "get-variable", 100)],
        vec![],
        vec![variable(100, &ty, None)],
        vec![],
    );
    assert_eq!(
        compile(&doc, &package, &snapshot).unwrap_err(),
        CompileError::UnsupportedNode {
            node: id(2).parse().unwrap()
        }
    );
    // Derived enum `eq` has no Compare-and-set kernel in the runtime yet.
    let mut cas = var_node(2, "update-variable", 100);
    cas["config"]["operation"] = json!("compare-and-set");
    cas["inputs"] = json!({
        "expected": {"type": LIGHT, "value": {"variant": "red"}},
        "value": {"type": LIGHT, "value": {"variant": "green"}}
    });
    let doc = document(
        &package,
        vec![core(1, "on-start"), cas],
        vec![edge(1, 1, "then", 2, "in")],
        vec![variable(100, LIGHT, Some(json!({"variant": "red"})))],
        vec![],
    );
    assert_eq!(
        compile(&doc, &package, &snapshot).unwrap_err(),
        CompileError::UnsupportedNode {
            node: id(2).parse().unwrap()
        }
    );
}
