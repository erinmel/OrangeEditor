use super::*;
use orange_compiler::{ExecInput, HostSource};
use orange_model::{GraphDocument, Value, to_json};
use orange_runtime::{ControlOutcome, Run, RunError, RunOptions, RunState};
use std::sync::{Arc, atomic::Ordering};

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
fn idle(run: &mut Run, budget: usize) {
    for _ in 0..20_000 {
        if !run.pump(budget).unwrap().ready {
            return;
        }
    }
    panic!("bounded fixture did not idle");
}
fn route(n: u32, commands: &[&str], target: u32) -> (Json, Vec<Json>) {
    let mut sw = core(n, "switch");
    sw["type_pins"] = json!({"K":"core/i64@1"});
    sw["variadic"] =
        json!({"case":{"order":(0..commands.len()).collect::<Vec<_>>(),"next":commands.len()}});
    sw["config"] = json!({"cases":commands.iter().enumerate().map(|(i,_)|(i.to_string(),json!(i))).collect::<serde_json::Map<_,_>>()});
    let edges = commands
        .iter()
        .enumerate()
        .map(|(i, port)| edge(100 + i as u32, n, &format!("case#{i}"), target, port))
        .collect();
    (sw, edges)
}
fn register(n: u32, scope: &str, initial: Option<Json>) -> Json {
    let mut reg = core(n, "register");
    reg["type_pins"] = json!({"T":"core/u64@1"});
    reg["config"] = json!({"scope":scope});
    reg["inputs"] = json!({"next":literal(0)});
    if let Some(initial) = initial {
        reg["config"]["initial"] = json!({"type":"core/u64@1","value":initial.to_string()});
    }
    reg
}

#[test]
fn gate_commands_keep_input_identity_and_only_enter_passes_then() {
    let commands = [
        "open", "enter", "close", "enter", "toggle", "in", "toggle", "enter",
    ];
    for budget in [1, 17] {
        let (sw, mut edges) = route(3, &commands, 4);
        edges.extend([
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 2, "index", 3, "key"),
            edge(4, 4, "then", 5, "in"),
            edge(5, 4, "is_open", 5, "condition"),
        ]);
        let mut observer = branch(5, false);
        observer["inputs"] = json!({});
        let (doc, snapshot, _, plan) = build(
            vec![
                core(1, "on-start"),
                count(2, commands.len() as i64),
                sw,
                core(4, "gate"),
                observer,
            ],
            edges,
            vec![],
        );
        let ControlOperation::Switch { cases, .. } = plan
            .nodes()
            .iter()
            .find(|n| n.node().to_string() == id(3))
            .unwrap()
            .operation()
        else {
            panic!()
        };
        assert_eq!(cases[0].target().unwrap().input(), ExecInput::Open);
        assert_eq!(cases[1].target().unwrap().input(), ExecInput::Enter);
        assert_eq!(cases[5].target().unwrap().input(), ExecInput::Enter);
        assert!(
            plan.host_sources()
                .iter()
                .any(|s| matches!(s, HostSource::State(_)))
        );
        let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
        assert_eq!(
            run.node_state(id(4).parse().unwrap()),
            Some(&Value::Bool(false))
        );
        idle(&mut run, budget);
        let actual: Vec<_> = run
            .trace()
            .iter()
            .filter(|e| e.node.to_string() == id(4))
            .map(|e| e.outcome)
            .collect();
        let (mut open, mut expected) = (false, Vec::new());
        for command in commands {
            expected.push(match command {
                "open" => {
                    open = true;
                    ControlOutcome::StateChanged
                }
                "close" => {
                    open = false;
                    ControlOutcome::StateChanged
                }
                "toggle" => {
                    open = !open;
                    ControlOutcome::StateChanged
                }
                _ => {
                    if open {
                        ControlOutcome::Then
                    } else {
                        ControlOutcome::Suppressed
                    }
                }
            });
        }
        assert_eq!(actual, expected);
        assert_eq!(
            run.node_state(id(4).parse().unwrap()),
            Some(&Value::Bool(open))
        );
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(5))
                .count(),
            2
        );
        assert!(
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(5))
                .all(|e| e.outcome == ControlOutcome::True)
        );
    }
}

#[test]
fn do_once_reset_rearms_without_firing_then() {
    let commands = ["in", "in", "reset", "in", "in", "reset", "in"];
    let (sw, mut edges) = route(3, &commands, 4);
    edges.extend([
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "body", 3, "in"),
        edge(3, 2, "index", 3, "key"),
        edge(4, 4, "then", 5, "in"),
    ]);
    let (doc, snapshot, _, plan) = build(
        vec![
            core(1, "on-start"),
            count(2, 7),
            sw,
            core(4, "do-once"),
            branch(5, true),
        ],
        edges,
        vec![],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    idle(&mut run, 1);
    let actual: Vec<_> = run
        .trace()
        .iter()
        .filter(|e| e.node.to_string() == id(4))
        .map(|e| e.outcome)
        .collect();
    assert_eq!(
        actual,
        [
            ControlOutcome::Then,
            ControlOutcome::Suppressed,
            ControlOutcome::StateChanged,
            ControlOutcome::Then,
            ControlOutcome::Suppressed,
            ControlOutcome::StateChanged,
            ControlOutcome::Then
        ]
    );
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.node.to_string() == id(5))
            .count(),
        3
    );
}

#[test]
fn flip_flop_starts_at_a_and_state_snapshots_follow_selected_outcomes() {
    let mut observer = branch(4, false);
    observer["inputs"] = json!({});
    let (doc, snapshot, _, plan) = build(
        vec![
            core(1, "on-start"),
            count(2, 8),
            core(3, "flip-flop"),
            observer,
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "a", 4, "in"),
            edge(4, 3, "b", 4, "in"),
            edge(5, 3, "is_a", 4, "condition"),
        ],
        vec![],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    idle(&mut run, 1);
    let flips: Vec<_> = run
        .trace()
        .iter()
        .filter(|e| e.node.to_string() == id(3))
        .map(|e| e.outcome)
        .collect();
    let observed: Vec<_> = run
        .trace()
        .iter()
        .filter(|e| e.node.to_string() == id(4))
        .map(|e| e.outcome)
        .collect();
    for i in 0..8 {
        assert_eq!(
            flips[i],
            if i % 2 == 0 {
                ControlOutcome::FlipA
            } else {
                ControlOutcome::FlipB
            }
        );
        assert_eq!(
            observed[i],
            if i % 2 == 0 {
                ControlOutcome::True
            } else {
                ControlOutcome::False
            }
        );
    }
    assert_eq!(
        run.node_state(id(3).parse().unwrap()),
        Some(&Value::Bool(false))
    );
}

#[test]
fn register_feedback_is_atomic_across_one_unit_pumps_and_reports_state_writes() {
    let mut reg = register(3, "run", Some(json!(0)));
    reg["inputs"] = json!({});
    let mut add = math(4, "add", 0, 1);
    add["inputs"].as_object_mut().unwrap().remove("a");
    let (doc, snapshot, resolver, plan) = build(
        vec![
            core(1, "on-start"),
            count(2, 3),
            reg,
            add,
            var_node(5, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "store"),
            edge(3, 3, "current", 4, "a"),
            edge(4, 4, "sum", 3, "next"),
            edge(5, 3, "then", 5, "in"),
            edge(6, 3, "current", 5, "value"),
        ],
        vec![variable(100, "run", "core/u64@1", None)],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    let mut saw_write = false;
    for _ in 0..1024 {
        let progress = run.pump(1).unwrap();
        saw_write |= run
            .segment_state_writes()
            .is_some_and(|(_, ids)| ids.contains(&id(3).parse().unwrap()));
        if !progress.ready {
            break;
        }
    }
    assert!(saw_write);
    assert_eq!(run.node_state(id(3).parse().unwrap()), Some(&Value::U64(3)));
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(3)));
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 3);
    run.stop().unwrap();
    run.dispose().unwrap();
    assert_eq!(run.state(), RunState::Disposed);
    assert!(run.node_state(id(3).parse().unwrap()).is_none());
}

#[test]
fn register_reset_uses_pinned_initial_without_demanding_faulting_next() {
    let mut reg = register(2, "run", Some(json!(7)));
    reg["inputs"] = json!({});
    let (doc, snapshot, _, plan) = build(
        vec![
            core(1, "on-start"),
            reg,
            math(3, "divide", 8, 0),
            var_node(4, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "reset"),
            edge(2, 3, "quotient", 2, "next"),
            edge(3, 2, "then", 4, "in"),
            edge(4, 2, "current", 4, "value"),
        ],
        vec![variable(100, "run", "core/u64@1", None)],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    idle(&mut run, 1);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(7)));
    assert!(
        !run.trace()
            .iter()
            .any(|e| e.outcome == ControlOutcome::Failed)
    );
}

#[test]
fn register_uninitialized_read_preserves_the_cell_producer_fault() {
    let (doc, snapshot, _, plan) = build(
        vec![
            core(1, "on-start"),
            register(2, "run", None),
            var_node(3, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 3, "in"),
            edge(2, 2, "current", 3, "value"),
        ],
        vec![variable(100, "run", "core/u64@1", None)],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    idle(&mut run, 1);
    let event = run.trace().back().unwrap();
    assert_eq!(event.outcome, ControlOutcome::Failed);
    let fault = event.fault.as_ref().unwrap();
    assert_eq!(fault.source, id(2).parse().unwrap());
    assert_eq!(fault.fault.code, "core/UninitializedRegister");
}

#[test]
fn register_activation_cells_are_distinct_while_run_cells_are_shared() {
    for scope in ["run", "activation"] {
        let mut reg = register(2, scope, Some(json!(7)));
        reg["inputs"] = json!({"next":literal(11)});
        let (doc, snapshot, _, plan) = build(
            vec![
                core(1, "on-start"),
                reg,
                core(10, "on-start"),
                wait(11, 1),
                var_node(12, "set-variable", 100),
            ],
            vec![
                edge(1, 1, "then", 2, "store"),
                edge(2, 10, "then", 11, "in"),
                edge(3, 11, "then", 12, "in"),
                edge(4, 2, "current", 12, "value"),
            ],
            vec![variable(100, "run", "core/u64@1", None)],
        );
        let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
        idle(&mut run, 1);
        run.advance_to(1).unwrap();
        idle(&mut run, 1);
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&Value::U64(if scope == "run" { 11 } else { 7 }))
        );
    }
}

#[test]
fn state_quota_rejection_does_not_mutate_register_or_emit_a_write_intent() {
    let mut reg = core(2, "register");
    reg["type_pins"] = json!({"T":"core/string@1"});
    reg["config"] = json!({"initial":{"type":"core/string@1","value":"old"}});
    reg["inputs"] = json!({"next":{"type":"core/string@1","value":"X".repeat(1024)}});
    let (doc, snapshot, _, plan) = build(
        vec![core(1, "on-start"), reg],
        vec![edge(1, 1, "then", 2, "store")],
        vec![],
    );
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 64;
    let mut run = Run::start(plan, &doc, &snapshot, 1, options).unwrap();
    let mut error = None;
    for _ in 0..1024 {
        if let Err(fault) = run.pump(1) {
            error = Some(fault);
            break;
        }
    }
    assert_eq!(error, Some(RunError::ResourceLimit));
    assert_eq!(
        run.node_state(id(2).parse().unwrap()),
        Some(&Value::Str(Arc::new("old".into())))
    );
    assert!(
        run.segment_state_writes()
            .is_none_or(|(_, ids)| ids.is_empty())
    );
}

#[test]
fn gate_configuration_and_register_copy_quota_reject_before_binding() {
    let (package, snapshot) = catalog();
    let mut gate = core(2, "gate");
    gate["config"] = json!({"start_open":"invalid"});
    let doc = graph(
        &package,
        vec![core(1, "on-start"), gate],
        vec![edge(1, 1, "then", 2, "enter")],
        vec![],
    );
    let report = analyze(&doc, &snapshot);
    assert!(!report.is_ready_for_lowering(&doc, &snapshot));
    let mut reg = core(2, "register");
    reg["type_pins"] = json!({"T":"core/string@1"});
    reg["config"] = json!({"initial":{"type":"core/string@1","value":"X".repeat(4096)}});
    reg["inputs"] = json!({"next":{"type":"core/string@1","value":""}});
    let doc = graph(
        &package,
        vec![core(1, "on-start"), reg, math(90, "add", 1, 2)],
        vec![edge(1, 1, "then", 2, "reset")],
        vec![export(90, "sum")],
    );
    let report = analyze(&doc, &snapshot);
    assert!(report.is_ready_for_lowering(&doc, &snapshot));
    let resolver = Counting::new(package);
    let limits = orange_compiler::CompileLimits {
        max_constant_bytes: 128,
        ..Default::default()
    };
    assert_eq!(
        compile_control(
            &doc,
            snapshot,
            &report,
            &resolver,
            CompileOptions {
                limits,
                ..Default::default()
            }
        )
        .unwrap_err(),
        CompileError::ResourceLimit
    );
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
}
#[test]
fn register_store_keeps_its_mutation_segment_ahead_of_other_activations() {
    let mut reg = register(2, "run", Some(json!(7)));
    reg["inputs"] = json!({});
    let mut add = math(4, "add", 0, 1);
    add["inputs"].as_object_mut().unwrap().remove("a");
    let mut other = var_node(11, "set-variable", 100);
    other["inputs"] = json!({"value":literal(99)});
    let (doc, snapshot, _, plan) = build(
        vec![
            core(1, "on-start"),
            reg,
            var_node(3, "get-variable", 100),
            add,
            core(10, "on-start"),
            other,
        ],
        vec![
            edge(1, 1, "then", 2, "store"),
            edge(2, 3, "value", 4, "a"),
            edge(3, 4, "sum", 2, "next"),
            edge(4, 10, "then", 11, "in"),
        ],
        vec![variable(100, "run", "core/u64@1", Some(json!("0")))],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    idle(&mut run, 1);
    let order: Vec<_> = run
        .trace()
        .iter()
        .filter(|e| e.node.to_string() == id(2) || e.node.to_string() == id(11))
        .map(|e| e.node.to_string())
        .collect();
    assert_eq!(order, [id(2), id(11)]);
    assert_eq!(run.node_state(id(2).parse().unwrap()), Some(&Value::U64(1)));
}

#[test]
fn stopping_a_pending_store_preserves_state_and_plans_do_not_share_mutations() {
    let mut reg = register(2, "run", Some(json!(7)));
    reg["inputs"] = json!({"next":literal(11)});
    let (doc, snapshot, _, plan) = build(
        vec![core(1, "on-start"), reg],
        vec![edge(1, 1, "then", 2, "store")],
        vec![],
    );
    let mut first = Run::start(plan.clone(), &doc, &snapshot, 1, RunOptions::default()).unwrap();
    let mut second = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    first.pump(1).unwrap();
    first.pump(1).unwrap();
    first.stop().unwrap();
    assert_eq!(
        first.node_state(id(2).parse().unwrap()),
        Some(&Value::U64(7))
    );
    assert_eq!(first.ownership().strands, 0);
    assert_eq!(
        second.node_state(id(2).parse().unwrap()),
        Some(&Value::U64(7))
    );
    idle(&mut second, 1);
    assert_eq!(
        second.node_state(id(2).parse().unwrap()),
        Some(&Value::U64(11))
    );
    assert_eq!(
        first.node_state(id(2).parse().unwrap()),
        Some(&Value::U64(7))
    );
}

#[test]
fn timer_cancellation_routes_the_named_gate_command_during_cleanup() {
    let mut gate = core(3, "gate");
    gate["config"] = json!({"start_open":true});
    let (doc, snapshot, _, plan) = build(
        vec![core(1, "on-start"), wait(2, 10), gate],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "cancelled", 3, "close"),
        ],
        vec![],
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    idle(&mut run, 1);
    assert_eq!(run.pending_continuations(), 1);
    assert_eq!(
        run.node_state(id(3).parse().unwrap()),
        Some(&Value::Bool(true))
    );
    run.stop().unwrap();
    idle(&mut run, 1);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(
        run.node_state(id(3).parse().unwrap()),
        Some(&Value::Bool(false))
    );
    assert!(
        run.trace()
            .iter()
            .any(|e| e.node.to_string() == id(3) && e.outcome == ControlOutcome::StateChanged)
    );
}
