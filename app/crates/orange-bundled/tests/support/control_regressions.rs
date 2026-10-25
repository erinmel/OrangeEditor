//! Regressions from the pre-merge control/runtime review.
use super::*;

#[test]
fn initializers_and_data_constants_share_one_retained_budget() {
    let (package, snapshot) = catalog();
    let mut set = var_node(2, "set-variable", 100);
    set["inputs"] = json!({"value":{"type":"core/json@1","value":serde_json::to_string(&"y".repeat(160)).unwrap()}});
    let source = graph(
        &package,
        vec![core(1, "on-start"), set],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&source).unwrap()).unwrap();
    raw["variables"] = json!([variable(
        100,
        "run",
        "core/json@1",
        Some(json!(serde_json::to_string(&"x".repeat(160)).unwrap()))
    )]);
    let doc = from_json(&raw.to_string()).unwrap();
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let mut options = CompileOptions::default();
    options.limits.max_constant_bytes = 256;
    assert!(matches!(
        compile_control(&doc, snapshot.clone(), &analysis, &package, options),
        Err(CompileError::ResourceLimit)
    ));
    options.limits.max_constant_bytes = 512;
    let plan = compile_control(&doc, snapshot, &analysis, &package, options).unwrap();
    let initial: usize = plan
        .variables()
        .iter()
        .filter_map(|v| v.initial().and_then(|i| v.charge(i)))
        .sum();
    assert_eq!(initial + plan.data().initial_charge().unwrap(), 356);
}
