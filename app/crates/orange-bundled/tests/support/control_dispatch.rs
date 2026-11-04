use super::*;
use orange_model::{GraphDocument, Value, to_json};
use orange_runtime::{ControlOutcome, Run, RunError, RunOptions};
use std::sync::{Arc, atomic::Ordering};

fn lit(ty: &str, value: Json) -> Json {
    let value = if matches!(ty, "core/i64@1" | "core/u64@1") {
        json!(value.to_string())
    } else {
        value
    };
    json!({"type":ty,"value":value})
}
fn switch(n: u32, ty: &str, key: Json, cases: &[(u16, Json)]) -> Json {
    let mut node = core(n, "switch");
    node["type_pins"] = json!({"K":ty});
    node["inputs"] = json!({"key":lit(ty,key)});
    node["config"] = json!({"cases":cases.iter().map(|(k,v)|(k.to_string(),v.clone())).collect::<serde_json::Map<_,_>>()});
    node["variadic"] = json!({"case":{"order":cases.iter().map(|(k,_)|k).collect::<Vec<_>>(),"next":cases.iter().map(|(k,_)|u32::from(*k)+1).max().unwrap()}});
    node
}
fn attempt(n: u32, ty: &str) -> Json {
    let mut n = core(n, "try");
    n["type_pins"] = json!({"T":ty});
    n
}
fn matcher(n: u32, ty: &str) -> Json {
    let mut n = core(n, "match-result");
    n["type_pins"] = json!({"T":ty});
    n
}
fn build(
    nodes: Vec<Json>,
    edges: Vec<Json>,
    variables: Vec<Json>,
) -> (
    GraphDocument,
    Arc<orange_model::catalog::RegistrySnapshot>,
    Counting,
    Arc<orange_compiler::ControlPlan>,
) {
    let (package, snapshot) = catalog();
    let base = graph(&package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!(variables);
    let doc = from_json(&raw.to_string()).unwrap();
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(
        compile_control(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    (doc, snapshot, resolver, plan)
}
fn run_all(run: &mut Run, budget: usize) {
    for _ in 0..4096 {
        run.pump(budget).unwrap();
        if run.ownership().strands == 0 {
            return;
        }
    }
    panic!("control did not complete");
}

#[test]
fn switch_matches_source_cases_preserving_sparse_instances_and_unconnected_arms() {
    let groups = [
        (
            "core/i32@1",
            vec![(42, json!(i32::MIN)), (2, json!(i32::MAX)), (9, json!(0))],
            vec![json!(i32::MIN), json!(i32::MAX), json!(0), json!(5)],
        ),
        (
            "core/u32@1",
            vec![(42, json!(u32::MAX)), (2, json!(0)), (9, json!(1))],
            vec![json!(u32::MAX), json!(0), json!(1), json!(9)],
        ),
        (
            "core/i64@1",
            vec![(42, json!(-7)), (2, json!(1000)), (9, json!(0))],
            vec![json!(-7), json!(0), json!(1000), json!(5), json!(i64::MIN)],
        ),
        (
            "core/u64@1",
            vec![(42, json!(u64::MAX)), (2, json!(0)), (9, json!(1))],
            vec![json!(u64::MAX), json!(0), json!(1), json!(33)],
        ),
        (
            "core/bool@1",
            vec![(42, json!(false)), (2, json!(true))],
            vec![json!(false), json!(true)],
        ),
        (
            "core/string@1",
            vec![
                (42, json!("private-key-z")),
                (2, json!("á العربية")),
                (9, json!("")),
            ],
            vec![
                json!("private-key-z"),
                json!("á العربية"),
                json!(""),
                json!("unknown"),
            ],
        ),
    ];
    for (ty, cases, keys) in groups {
        for key in keys {
            for budget in [1, 7] {
                let expected = cases
                    .iter()
                    .find(|(_, value)| *value == key)
                    .map(|(instance, _)| *instance);
                let mut edges = vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "default", 5, "in")];
                // Case42 deliberately has no connection. A matched empty arm cannot fall through.
                for (instance, _) in &cases {
                    if *instance != 42 {
                        edges.push(edge(
                            10 + u32::from(*instance),
                            2,
                            &format!("case#{instance}"),
                            3,
                            "in",
                        ));
                    }
                }
                let (doc, snapshot, _, plan) = build(
                    vec![
                        core(1, "on-start"),
                        switch(2, ty, key.clone(), &cases),
                        branch(3, true),
                        branch(5, false),
                    ],
                    edges,
                    vec![],
                );
                assert!(!format!("{:?}", plan.nodes()[1].operation()).contains("private-key-z"));
                let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
                run_all(&mut run, budget);
                let event = run
                    .trace()
                    .iter()
                    .find(|e| e.node.to_string() == id(2))
                    .unwrap();
                assert_eq!(
                    event.outcome,
                    expected.map_or(ControlOutcome::Default, ControlOutcome::Case)
                );
                let visited: Vec<_> = run.trace().iter().map(|e| e.node.to_string()).collect();
                assert_eq!(visited.contains(&id(5)), expected.is_none());
                assert_eq!(visited.contains(&id(3)), expected.is_some_and(|i| i != 42));
            }
        }
    }
}

#[test]
fn switch_case_ranges_and_table_quotas_fail_before_provider_callbacks() {
    let (package, snapshot) = catalog();
    for (ty, bad) in [
        ("core/i32@1", json!(i64::from(i32::MAX) + 1)),
        ("core/u32@1", json!(u64::from(u32::MAX) + 1)),
        ("core/u64@1", json!(-1)),
    ] {
        let doc = graph(
            &package,
            vec![
                core(1, "on-start"),
                switch(2, ty, json!(0), &[(0, bad)]),
                math(90, "add", 1, 2),
            ],
            vec![edge(1, 1, "then", 2, "in")],
            vec![export(90, "sum")],
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
        assert!(
            analysis
                .report()
                .diagnostics
                .iter()
                .any(|d| d.code == orange_model::validation::DiagnosticCode::SwitchCase)
        );
        let resolver = Counting::new(package.clone());
        assert!(matches!(
            compile_control(
                &doc,
                snapshot.clone(),
                &analysis,
                &resolver,
                CompileOptions::default()
            ),
            Err(CompileError::ValidationIncomplete)
        ));
        assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
    }
    let doc = graph(
        &package,
        vec![
            core(1, "on-start"),
            switch(
                2,
                "core/string@1",
                json!(""),
                &[(0, json!("X".repeat(4096)))],
            ),
            math(90, "add", 1, 2),
        ],
        vec![edge(1, 1, "then", 2, "in")],
        vec![export(90, "sum")],
    );
    let analysis = analyze(&doc, &snapshot);
    let resolver = Counting::new(package);
    let options = CompileOptions {
        limits: orange_compiler::CompileLimits {
            max_constant_bytes: 128,
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        compile_control(&doc, snapshot, &analysis, &resolver, options).unwrap_err(),
        CompileError::ResourceLimit
    );
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
}

#[test]
fn try_match_routes_values_and_original_errors_without_duplicate_projection_slots() {
    for denominator in [0, 2] {
        for budget in [1, 11] {
            let (doc, snapshot, _, plan) = build(
                vec![
                    core(1, "on-start"),
                    math(2, "divide", 8, denominator),
                    attempt(3, "core/u64@1"),
                    matcher(4, "core/u64@1"),
                    var_node(5, "set-variable", 100),
                    var_node(6, "set-variable", 101),
                ],
                vec![
                    edge(1, 1, "then", 4, "in"),
                    edge(2, 2, "quotient", 3, "value"),
                    edge(3, 3, "result", 4, "result"),
                    edge(4, 4, "ok", 5, "in"),
                    edge(5, 4, "value", 5, "value"),
                    edge(6, 4, "err", 6, "in"),
                    edge(7, 4, "error", 6, "value"),
                ],
                vec![
                    variable(100, "run", "core/u64@1", None),
                    variable(101, "run", "core/error@1", None),
                ],
            );
            let owner = plan
                .nodes()
                .iter()
                .position(|n| n.node().to_string() == id(4))
                .unwrap();
            let sources: Vec<_> = plan
                .host_sources()
                .iter()
                .filter(|s| matches!(s,orange_compiler::HostSource::Error(i) if i.index()==owner))
                .collect();
            assert_eq!(sources.len(), 1);
            let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
            run_all(&mut run, budget);
            let outcome = run
                .trace()
                .iter()
                .find(|e| e.node.to_string() == id(4))
                .unwrap();
            assert!(outcome.fault.is_none());
            if denominator == 0 {
                assert_eq!(outcome.outcome, ControlOutcome::MatchedErr);
                let Some(Value::Err(error)) = run.variable(id(101).parse().unwrap()) else {
                    panic!("missing captured error")
                };
                assert_eq!(error.node, Some(id(2).parse().unwrap()));
                assert_eq!(error.code, "core/DivideByZero");
                assert!(run.variable(id(100).parse().unwrap()).is_none());
            } else {
                assert_eq!(outcome.outcome, ControlOutcome::MatchedOk);
                assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(4)));
                assert!(run.variable(id(101).parse().unwrap()).is_none());
            }
        }
    }
}

#[test]
fn nested_ok_error_remains_a_successful_error_value() {
    let (doc, snapshot, _, plan) = build(
        vec![
            core(1, "on-start"),
            math(2, "divide", 8, 0),
            attempt(3, "core/u64@1"),
            matcher(4, "core/u64@1"),
            attempt(7, "core/error@1"),
            matcher(8, "core/error@1"),
            var_node(9, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 4, "in"),
            edge(2, 2, "quotient", 3, "value"),
            edge(3, 3, "result", 4, "result"),
            edge(4, 4, "err", 8, "in"),
            edge(5, 4, "error", 7, "value"),
            edge(6, 7, "result", 8, "result"),
            edge(7, 8, "ok", 9, "in"),
            edge(8, 8, "value", 9, "value"),
        ],
        vec![variable(100, "run", "core/error@1", None)],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    run_all(&mut run, 1);
    assert_eq!(
        run.trace()
            .iter()
            .find(|e| e.node.to_string() == id(8))
            .unwrap()
            .outcome,
        ControlOutcome::MatchedOk
    );
    let Some(Value::Err(error)) = run.variable(id(100).parse().unwrap()) else {
        panic!("Ok(error) was lost")
    };
    assert_eq!(error.node, Some(id(2).parse().unwrap()));
}

#[test]
fn match_success_none_is_present_and_not_a_missing_projection() {
    let ty = "core/option@1<core/u64@1>";
    let (doc, snapshot, _, plan) = build(
        vec![
            core(1, "on-start"),
            var_node(2, "get-variable", 100),
            attempt(3, ty),
            matcher(4, ty),
            var_node(5, "set-variable", 101),
        ],
        vec![
            edge(1, 1, "then", 4, "in"),
            edge(2, 2, "value", 3, "value"),
            edge(3, 3, "result", 4, "result"),
            edge(4, 4, "ok", 5, "in"),
            edge(5, 4, "value", 5, "value"),
        ],
        vec![
            variable(100, "run", ty, Some(json!({"none":true}))),
            variable(101, "run", ty, None),
        ],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    run_all(&mut run, 1);
    assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::None));
}

fn string_match(
    repeats: i64,
    payload: &str,
    scope: &str,
) -> (
    GraphDocument,
    Arc<orange_model::catalog::RegistrySnapshot>,
    Counting,
    Arc<orange_compiler::ControlPlan>,
) {
    let mut source = attempt(3, "core/string@1");
    source["inputs"] = json!({"value":lit("core/string@1",json!(payload))});
    let mut count = core(2, "count");
    count["inputs"] = json!({"count":lit("core/i64@1",json!(repeats))});
    build(
        vec![
            core(1, "on-start"),
            count,
            source,
            matcher(4, "core/string@1"),
            var_node(5, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 4, "in"),
            edge(3, 3, "result", 4, "result"),
            edge(4, 4, "ok", 5, "in"),
            edge(5, 4, "value", 5, "value"),
        ],
        vec![variable(100, scope, "core/string@1", None)],
    )
}

#[test]
fn matched_strings_replace_charges_and_respect_activation_state_limits() {
    let (doc, snapshot, _, plan) = string_match(128, &"x".repeat(60), "run");
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 96;
    let mut run = Run::start(plan, &doc, &snapshot, 1, options.clone()).unwrap();
    run_all(&mut run, 1);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::Str(Arc::new("x".repeat(60))))
    );
    for scope in ["run", "local"] {
        let payload = if scope == "run" {
            "x".repeat(1024)
        } else {
            "x".repeat(60)
        };
        let (doc, snapshot, _, plan) = string_match(1, &payload, scope);
        let mut run = Run::start(plan, &doc, &snapshot, 1, options.clone()).unwrap();
        let mut error = None;
        for _ in 0..4096 {
            if let Err(fault) = run.pump(1) {
                error = Some(fault);
                break;
            }
        }
        assert_eq!(error, Some(RunError::ResourceLimit));
        assert!(
            !run.trace()
                .iter()
                .any(|e| e.node.to_string() == id(5) && e.outcome == ControlOutcome::Then)
        );
    }
}
#[test]
fn uncaptured_input_fault_takes_failed_instead_of_the_result_err_arm() {
    let (doc, snapshot, _, plan) = build(
        vec![
            core(1, "on-start"),
            var_node(2, "get-variable", 100),
            matcher(4, "core/u64@1"),
            branch(5, true),
            branch(6, false),
        ],
        vec![
            edge(1, 1, "then", 4, "in"),
            edge(2, 2, "value", 4, "result"),
            edge(3, 4, "failed", 5, "in"),
            edge(4, 4, "err", 6, "in"),
        ],
        vec![variable(100, "run", "core/result@1<core/u64@1>", None)],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    run_all(&mut run, 1);
    let event = run
        .trace()
        .iter()
        .find(|e| e.node.to_string() == id(4))
        .unwrap();
    assert_eq!(event.outcome, ControlOutcome::Failed);
    assert_eq!(event.fault.as_ref().unwrap().source, id(2).parse().unwrap());
    assert!(run.trace().iter().any(|e| e.node.to_string() == id(5)));
    assert!(!run.trace().iter().any(|e| e.node.to_string() == id(6)));
}

#[test]
fn wide_case_validation_rejects_duplicates_without_changing_case_identity() {
    let (package, snapshot) = catalog();
    let cases = (0..1024u16)
        .map(|i| (i, json!(u64::from(i))))
        .collect::<Vec<_>>();
    let mut selected = switch(2, "core/u64@1", json!(1023), &cases);
    let base = graph(
        &package,
        vec![core(1, "on-start"), selected.clone()],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let analysis = analyze(&base, &snapshot);
    assert!(analysis.is_ready_for_lowering(&base, &snapshot));
    let plan = Arc::new(
        compile_control(
            &base,
            snapshot.clone(),
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let mut run = Run::start(plan, &base, &snapshot, 1, RunOptions::default()).unwrap();
    run_all(&mut run, 1);
    assert_eq!(
        run.trace().back().unwrap().outcome,
        ControlOutcome::Case(1023)
    );
    selected["config"]["cases"]["1023"] = json!(0);
    let invalid = graph(
        &package,
        vec![core(1, "on-start"), selected],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let report = analyze(&invalid, &snapshot);
    assert!(!report.is_ready_for_lowering(&invalid, &snapshot));
    assert_eq!(
        report
            .report()
            .diagnostics
            .iter()
            .filter(|d| d.code == orange_model::validation::DiagnosticCode::SwitchCase)
            .count(),
        1
    );
}

const LIGHT: &str = "org.example/light@1";

/// The math package plus a package enum `light` (red, amber(u32), green) deriving `eq`.
fn enum_catalog() -> (
    orange_bundled::MathPackage,
    Arc<orange_model::catalog::RegistrySnapshot>,
) {
    enum_catalog_with(&[])
}

/// [`enum_catalog`] whose `light` schema also lists payload-free `extra` variants.
fn enum_catalog_with(
    extra: &[&str],
) -> (
    orange_bundled::MathPackage,
    Arc<orange_model::catalog::RegistrySnapshot>,
) {
    use orange_model::type_term::TypeTerm;
    use orange_model::types::{Constraint, TypeRegistryBuilder};
    use orange_model::value::literal_codec::{EnumSchema, VariantSchema};
    let light: orange_model::id::TypeExpr = LIGHT.parse().unwrap();
    let mut types = TypeRegistryBuilder::core();
    let variant = |id: &str, payload: Option<&str>| VariantSchema {
        variant_id: id.into(),
        payload: payload.map(|ty| TypeTerm::parse(&ty.parse().unwrap()).unwrap()),
    };
    let mut variants = vec![
        variant("red", None),
        variant("amber", Some("core/u32@1")),
        variant("green", None),
    ];
    variants.extend(extra.iter().map(|id| variant(id, None)));
    types
        .register_enum("org.example", &light, EnumSchema { variants })
        .unwrap();
    types
        .derive_constraint("org.example", &light, Constraint::Eq)
        .unwrap();
    let digest = format!("sha256:{}", "a".repeat(64));
    let package = orange_bundled::MathPackage::new(digest.clone()).unwrap();
    let builder = orange_model::catalog::RegistryBuilder::core(
        orange_model::validation::HostOrigin::new("0.0.0", digest).unwrap(),
    )
    .types(Arc::new(types.build().unwrap()));
    let snapshot = package.register(builder).unwrap().build().unwrap();
    (package, snapshot)
}

/// on-start -> switch(2) over `cases`; every case and (optionally) default end in a branch.
fn light_graph(
    package: &orange_bundled::MathPackage,
    key: Json,
    cases: &[(u16, Json)],
    default: bool,
) -> GraphDocument {
    let mut nodes = vec![core(1, "on-start"), switch(2, LIGHT, key, cases)];
    let mut edges = vec![edge(1, 1, "then", 2, "in")];
    for (instance, _) in cases {
        let n = 10 + u32::from(*instance);
        nodes.push(branch(n, true));
        edges.push(edge(n, 2, &format!("case#{instance}"), n, "in"));
    }
    if default {
        nodes.push(branch(5, false));
        edges.push(edge(5, 2, "default", 5, "in"));
    }
    graph(package, nodes, edges, vec![])
}

#[test]
fn enum_switch_routes_by_variant_tag_ignoring_payloads() {
    use orange_compiler::SwitchKey;
    let (package, snapshot) = enum_catalog();
    let partial = [(4, json!("amber")), (1, json!("red"))];
    let exhaustive = [(0, json!("green")), (4, json!("amber")), (1, json!("red"))];
    for (cases, default) in [(&partial[..], true), (&exhaustive[..], false)] {
        for (key, expected) in [
            (json!({"variant":"red"}), Some(1)),
            (json!({"variant":"amber","payload":7}), Some(4)),
            (json!({"variant":"amber","payload":9}), Some(4)),
            (json!({"variant":"green"}), (!default).then_some(0)),
        ] {
            let doc = light_graph(&package, key, cases, default);
            let analysis = analyze(&doc, &snapshot);
            assert!(
                analysis.is_ready_for_lowering(&doc, &snapshot),
                "{:?}",
                analysis.report().diagnostics
            );
            let plan = compile_control(
                &doc,
                snapshot.clone(),
                &analysis,
                &package,
                CompileOptions::default(),
            )
            .unwrap();
            // Cases lower to schema tags in tag order, keeping their source instances.
            let ControlOperation::Switch { cases: table, .. } = plan.nodes()[1].operation() else {
                panic!("not a switch");
            };
            let lowered: Vec<_> = table
                .iter()
                .map(|c| (c.key().clone(), c.instance()))
                .collect();
            let mut want = vec![(SwitchKey::Variant(0), 1), (SwitchKey::Variant(1), 4)];
            if !default {
                want.push((SwitchKey::Variant(2), 0));
            }
            assert_eq!(lowered, want);
            let mut run =
                Run::start(Arc::new(plan), &doc, &snapshot, 1, RunOptions::default()).unwrap();
            run_all(&mut run, 1);
            let event = run
                .trace()
                .iter()
                .find(|e| e.node.to_string() == id(2))
                .unwrap();
            assert_eq!(
                event.outcome,
                expected.map_or(ControlOutcome::Default, ControlOutcome::Case)
            );
            let visited: Vec<_> = run.trace().iter().map(|e| e.node.to_string()).collect();
            assert_eq!(visited.contains(&id(5)), expected.is_none());
        }
    }
}

#[test]
fn enum_switch_cases_must_name_distinct_variants_and_cover_the_schema_or_default() {
    let (package, snapshot) = enum_catalog();
    let red = json!({"variant":"red"});
    for (cases, default) in [
        (vec![(0, json!("red")), (1, json!("blue"))], true),
        (vec![(0, json!("red")), (1, json!("red"))], true),
        (vec![(0, json!("red")), (1, json!(1))], true),
        (vec![(0, json!("red")), (1, json!("amber"))], false),
    ] {
        let doc = light_graph(&package, red.clone(), &cases, default);
        let analysis = analyze(&doc, &snapshot);
        assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
        let faults: Vec<_> = analysis
            .report()
            .diagnostics
            .iter()
            .filter(|d| d.code == orange_model::validation::DiagnosticCode::SwitchCase)
            .map(|d| d.subject.node.map(|n| n.to_string()))
            .collect();
        assert_eq!(faults, [Some(id(2))], "{cases:?}");
        let resolver = Counting::new(package.clone());
        assert_eq!(
            compile_control(
                &doc,
                snapshot.clone(),
                &analysis,
                &resolver,
                CompileOptions::default()
            )
            .unwrap_err(),
            CompileError::ValidationIncomplete
        );
        assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
    }
}

fn switch_faults(analysis: &orange_model::validation::ValidationAnalysis) -> usize {
    analysis
        .report()
        .diagnostics
        .iter()
        .filter(|d| d.code == orange_model::validation::DiagnosticCode::SwitchCase)
        .count()
}

/// A partial enum switch depends on its default edge: a retained session must add and
/// clear E-SWITCH-CASE as that edge goes and comes back, as the full analysis does.
#[test]
fn retained_session_tracks_an_enum_switch_default_edge() {
    use orange_model::document::Edge;
    use orange_model::validation::{AnalysisOptions, AnalysisSession, analyze_with_options};
    use orange_model::{DocCommand, GraphDocument};
    use std::collections::BTreeMap;
    let (package, snapshot) = enum_catalog();
    let partial = [(4, json!("amber")), (1, json!("red"))];
    let mut doc = light_graph(&package, json!({"variant":"red"}), &partial, true);
    let options = AnalysisOptions::default();
    let mut session = AnalysisSession::new(&doc, &snapshot, BTreeMap::new(), options);
    assert_eq!(switch_faults(session.analysis()), 0);
    let default_edge = || serde_json::from_value::<Edge>(edge(5, 2, "default", 5, "in")).unwrap();
    let mut step = |doc: &mut GraphDocument, command: DocCommand, faults: usize| {
        let committed = doc.apply(vec![command]).unwrap();
        session.update(doc, &snapshot, &committed.change_set, options);
        let full = analyze_with_options(doc, &snapshot, BTreeMap::new(), options);
        assert_eq!(session.analysis(), &full);
        assert_eq!(switch_faults(&full), faults);
    };
    for _ in 0..2 {
        let disconnect = DocCommand::Disconnect {
            edge: default_edge().id,
        };
        step(&mut doc, disconnect, 1);
        let connect = DocCommand::Connect {
            edge: default_edge(),
        };
        step(&mut doc, connect, 0);
    }
}

/// Exhaustiveness is checked against the current snapshot's schema: a new variant makes
/// a previously exhaustive switch without default invalid on re-analysis.
#[test]
fn a_schema_gaining_a_variant_invalidates_an_exhaustive_switch_without_default() {
    let (package, snapshot) = enum_catalog();
    let exhaustive = [(0, json!("green")), (4, json!("amber")), (1, json!("red"))];
    let doc = light_graph(&package, json!({"variant":"red"}), &exhaustive, false);
    let analysis = analyze(&doc, &snapshot);
    assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
    let (_, grown) = enum_catalog_with(&["blue"]);
    let analysis = analyze(&doc, &grown);
    assert!(!analysis.is_ready_for_lowering(&doc, &grown));
    assert_eq!(switch_faults(&analysis), 1);
    let with_default = light_graph(&package, json!({"variant":"red"}), &exhaustive, true);
    assert!(analyze(&with_default, &grown).is_ready_for_lowering(&with_default, &grown));
}
