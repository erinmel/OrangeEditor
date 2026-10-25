use super::*;
use orange_runtime::{Run, RunError, RunOptions};

fn fork(n: u32, join: &str, order: &[u32]) -> Json {
    let mut node = core(n, "fork");
    node["config"] = json!({"join":join});
    node["variadic"] = json!({"branch":{"next":order.iter().max().unwrap()+1,"order":order}});
    node
}

#[test]
fn checked_fork_lowering_keeps_sparse_branch_order_and_refuses_unknown_join() {
    for join in ["all", "any", "race", "detach"] {
        let (_, plan) = compiled(
            vec![core(1, "on-start"), fork(2, join, &[7, 2]), branch(3, true)],
            vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "branch#2", 3, "in")],
        );
        let ControlOperation::Fork { branches, .. } = plan.nodes()[1].operation() else {
            panic!()
        };
        assert!(branches[0].is_none());
        assert_eq!(branches[1].unwrap().index(), 2);
    }
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![core(1, "on-start"), fork(2, "unknown", &[0])],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let resolver = Counting::new(package.clone());
    assert!(
        compile_control(
            &doc,
            snapshot.clone(),
            &analyze(&doc, &snapshot),
            &resolver,
            CompileOptions::default()
        )
        .is_err()
    );
    assert_eq!(resolver.pure.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn root_ownership_is_admitted_before_publishing_a_run() {
    let (doc, plan) = compiled(vec![core(1, "on-start")], vec![]);
    for strand_limit in [false, true] {
        let mut options = RunOptions::default();
        if strand_limit {
            options.limits.max_strands = 0;
        } else {
            options.limits.max_regions = 0;
        }
        assert!(matches!(
            Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options),
            Err(RunError::ResourceLimit)
        ));
    }
}

fn drain(run: &mut Run, budget: usize) {
    for _ in 0..100_000 {
        if !run.pump(budget).unwrap().ready {
            return;
        }
    }
    panic!("run did not reach a suspension or completion");
}
fn flag(run: &Run, n: u32) -> bool {
    run.variable(id(n).parse().unwrap()) == Some(&orange_model::Value::Bool(true))
}

#[test]
fn all_and_detach_keep_enclosing_sequence_alive_until_every_descendant_ends() {
    for join in ["all", "detach"] {
        for budget in [1, 7, 1000] {
            let (doc, plan) = state_plan(
                vec![
                    core(1, "on-start"),
                    sequence(2, &[0, 1]),
                    fork(3, join, &[7, 2]),
                    wait(4, 10),
                    wait(5, 5),
                    set_bool(6, 100, true),
                    set_bool(7, 101, true),
                    set_bool(8, 102, true),
                    set_bool(9, 103, true),
                ],
                vec![
                    edge(1, 1, "then", 2, "in"),
                    edge(2, 2, "then#0", 3, "in"),
                    edge(3, 3, "branch#7", 4, "in"),
                    edge(4, 3, "branch#2", 5, "in"),
                    edge(5, 4, "then", 6, "in"),
                    edge(6, 5, "then", 7, "in"),
                    edge(7, 3, "completed", 8, "in"),
                    edge(8, 2, "then#1", 9, "in"),
                ],
                (100..104)
                    .map(|n| variable(n, "run", "core/bool@1", Some(json!(false))))
                    .collect(),
            );
            let mut run = Run::start(
                plan.clone(),
                &doc,
                plan.data().snapshot(),
                1,
                RunOptions::default(),
            )
            .unwrap();
            drain(&mut run, budget);
            assert_eq!(flag(&run, 102), join == "detach");
            assert!(!flag(&run, 103));
            assert_eq!(run.pending_continuations(), 2);
            run.advance_to(5).unwrap();
            drain(&mut run, budget);
            assert!(flag(&run, 101));
            assert!(!flag(&run, 100));
            assert!(!flag(&run, 103));
            run.advance_to(10).unwrap();
            drain(&mut run, budget);
            assert!((100..104).all(|n| flag(&run, n)));
            assert_eq!(
                run.trace()
                    .iter()
                    .filter(|e| e.node.to_string() == id(3)
                        && e.outcome == orange_runtime::ControlOutcome::Completed)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn fork_shares_activation_local_cells_and_empty_branches_complete() {
    for join in ["all", "detach"] {
        let mut read = branch(6, false);
        read.as_object_mut().unwrap().remove("inputs");
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                fork(2, join, &[8, 2, 5]),
                set_bool(3, 100, true),
                wait(4, 1),
                var_node(5, "get-variable", 100),
                read,
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "branch#8", 3, "in"),
                edge(3, 2, "branch#2", 4, "in"),
                edge(4, 4, "then", 6, "in"),
                edge(5, 5, "value", 6, "condition"),
            ],
            vec![variable(100, "local", "core/bool@1", Some(json!(false)))],
        );
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        drain(&mut run, 1);
        run.advance_to(1).unwrap();
        drain(&mut run, 1);
        assert_eq!(
            run.trace()
                .iter()
                .find(|e| e.node.to_string() == id(6))
                .unwrap()
                .outcome,
            orange_runtime::ControlOutcome::True
        );
        assert_eq!(run.pending_continuations(), 0);
    }
}

#[test]
fn any_retains_losers_race_retires_them_and_both_publish_once() {
    for join in ["any", "race"] {
        for budget in [1, 9, 1000] {
            let (doc, plan) = state_plan(
                vec![
                    core(1, "on-start"),
                    sequence(2, &[0, 1]),
                    fork(3, join, &[0, 1]),
                    wait(4, 10),
                    wait(5, 5),
                    set_bool(6, 100, true),
                    set_bool(7, 101, true),
                    set_bool(8, 102, true),
                    set_bool(9, 103, true),
                ],
                vec![
                    edge(1, 1, "then", 2, "in"),
                    edge(2, 2, "then#0", 3, "in"),
                    edge(3, 3, "branch#0", 4, "in"),
                    edge(4, 3, "branch#1", 5, "in"),
                    edge(5, 4, "then", 6, "in"),
                    edge(6, 5, "then", 7, "in"),
                    edge(7, 3, "completed", 8, "in"),
                    edge(8, 2, "then#1", 9, "in"),
                ],
                (100..104)
                    .map(|n| variable(n, "run", "core/bool@1", Some(json!(false))))
                    .collect(),
            );
            let mut run = Run::start(
                plan.clone(),
                &doc,
                plan.data().snapshot(),
                1,
                RunOptions::default(),
            )
            .unwrap();
            drain(&mut run, budget);
            assert!(!flag(&run, 102));
            run.advance_to(5).unwrap();
            drain(&mut run, budget);
            assert!(flag(&run, 101) && flag(&run, 102));
            assert_eq!(flag(&run, 103), join == "race");
            assert_eq!(run.pending_continuations(), usize::from(join == "any"));
            run.advance_to(10).unwrap();
            drain(&mut run, budget);
            assert_eq!(flag(&run, 100), join == "any");
            assert!(flag(&run, 103));
            assert_eq!(
                run.trace()
                    .iter()
                    .filter(|e| e.node.to_string() == id(3)
                        && e.outcome == orange_runtime::ControlOutcome::Completed)
                    .count(),
                1
            );
            assert_eq!(
                run.trace()
                    .iter()
                    .filter(|e| e.node.to_string() == id(4)
                        && e.outcome == orange_runtime::ControlOutcome::Cancelled)
                    .count(),
                usize::from(join == "race")
            );
        }
    }
}

#[test]
fn first_empty_or_synchronous_race_branch_wins_in_persisted_order() {
    for empty in [true, false] {
        let edges = if empty {
            vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "branch#2", 3, "in")]
        } else {
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "branch#7", 4, "in"),
                edge(3, 2, "branch#2", 3, "in"),
            ]
        };
        let (doc, plan) = compiled(
            vec![
                core(1, "on-start"),
                fork(2, "race", &[7, 2]),
                branch(3, true),
                branch(4, false),
            ],
            edges,
        );
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        drain(&mut run, 1);
        assert!(
            !run.trace().iter().any(|e| e.node.to_string() == id(3)),
            "empty={empty} {:?}",
            run.trace()
        );
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(4))
                .count(),
            usize::from(!empty)
        );
    }
}

#[test]
fn fork_loop_terminators_cancel_the_complete_body_before_advancing() {
    for join in ["all", "any", "race", "detach"] {
        for terminator in ["break", "continue"] {
            for budget in [1, 1000] {
                let mut loop_node = core(2, "count");
                loop_node["inputs"] = json!({"count":{"type":"core/i64@1","value":"2"}});
                let (doc, plan) = state_plan(
                    vec![
                        core(1, "on-start"),
                        loop_node,
                        fork(3, join, &[0, 1]),
                        wait(4, 100),
                        core(5, terminator),
                        set_bool(6, 100, true),
                        set_bool(7, 101, true),
                        branch(8, true),
                    ],
                    vec![
                        edge(1, 1, "then", 2, "in"),
                        edge(2, 2, "body", 3, "in"),
                        edge(3, 3, "branch#0", 4, "in"),
                        edge(4, 3, "branch#1", 5, "in"),
                        edge(5, 4, "cancelled", 6, "in"),
                        edge(6, 4, "then", 8, "in"),
                        edge(7, 2, "completed", 7, "in"),
                    ],
                    (100..102)
                        .map(|n| variable(n, "run", "core/bool@1", Some(json!(false))))
                        .collect(),
                );
                let mut run = Run::start(
                    plan.clone(),
                    &doc,
                    plan.data().snapshot(),
                    1,
                    RunOptions::default(),
                )
                .unwrap();
                drain(&mut run, budget);
                assert!(flag(&run, 100) && flag(&run, 101));
                assert_eq!(run.pending_continuations(), 0);
                assert_eq!(
                    run.trace()
                        .iter()
                        .filter(|e| e.node.to_string() == id(4)
                            && e.outcome == orange_runtime::ControlOutcome::Cancelled)
                        .count(),
                    if terminator == "break" { 1 } else { 2 }
                );
                assert!(!run.trace().iter().any(|e| e.node.to_string() == id(8)));
                run.advance_to(100).unwrap();
                drain(&mut run, budget);
                assert!(!run.trace().iter().any(|e| e.node.to_string() == id(8)));
            }
        }
    }
}

#[test]
fn unhandled_fork_failure_cancels_siblings_and_does_not_win_the_join() {
    for join in ["all", "any", "race", "detach"] {
        let mut consumer = branch(5, false);
        consumer.as_object_mut().unwrap().remove("inputs");
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                fork(2, join, &[0, 1]),
                wait(3, 100),
                var_node(4, "get-variable", 101),
                consumer,
                set_bool(6, 100, true),
                branch(7, true),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "branch#0", 3, "in"),
                edge(3, 2, "branch#1", 5, "in"),
                edge(4, 4, "value", 5, "condition"),
                edge(5, 3, "cancelled", 6, "in"),
                edge(6, 2, "completed", 7, "in"),
            ],
            vec![
                variable(100, "run", "core/bool@1", Some(json!(false))),
                variable(101, "local", "core/bool@1", None),
            ],
        );
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        drain(&mut run, 1);
        assert!(flag(&run, 100));
        assert_eq!(run.pending_continuations(), 0);
        assert_eq!(run.state(), orange_runtime::RunState::Running);
        assert!(!run.trace().iter().any(|e| e.node.to_string() == id(7)));
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(2)
                    && e.outcome == orange_runtime::ControlOutcome::Completed)
                .count(),
            usize::from(join == "detach")
        );
    }
}

#[test]
fn shutdown_cleans_every_wait_in_one_activation_and_deadline_or_second_stop_aborts() {
    use orange_runtime::{RunState, StopMode};
    for stop in [StopMode::Immediate, StopMode::Graceful] {
        for abort in [0, 1, 2] {
            let (doc, plan) = state_plan(
                vec![
                    core(1, "on-start"),
                    fork(2, "all", &[0, 1]),
                    wait(3, 100),
                    wait(4, 100),
                    set_bool(5, 100, true),
                    set_bool(6, 101, true),
                ],
                vec![
                    edge(1, 1, "then", 2, "in"),
                    edge(2, 2, "branch#0", 3, "in"),
                    edge(3, 2, "branch#1", 4, "in"),
                    edge(4, 3, "cancelled", 5, "in"),
                    edge(5, 4, "cancelled", 6, "in"),
                ],
                (100..102)
                    .map(|n| variable(n, "run", "core/bool@1", Some(json!(false))))
                    .collect(),
            );
            let options = RunOptions {
                drain_timeout_ns: 2,
                cleanup_timeout_ns: 1,
                ..RunOptions::default()
            };
            let mut run =
                Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
            drain(&mut run, 1);
            run.request_stop(stop).unwrap();
            if stop == StopMode::Graceful {
                run.advance_to(2).unwrap();
                run.pump(0).unwrap();
            }
            if abort == 1 && run.state() != RunState::Stopped {
                run.stop().unwrap();
            } else if abort == 2 && run.state() != RunState::Stopped {
                run.advance_to(run.now() + 1).unwrap();
                run.pump(0).unwrap();
            }
            if run.state() != RunState::Stopped {
                drain(&mut run, 1);
            }
            assert_eq!(run.state(), RunState::Stopped);
            assert_eq!(run.pending_continuations(), 0);
            assert_eq!(
                run.ownership(),
                orange_runtime::RunOwnership {
                    activations: 0,
                    strands: 0,
                    regions: 0,
                    timers: 0
                }
            );
            // Graceful expiry has no remaining cleanup deadline.
            let cleaned = abort == 0 && stop == StopMode::Immediate;
            assert_eq!(flag(&run, 100), cleaned);
            assert_eq!(flag(&run, 101), cleaned);
            run.dispose().unwrap();
        }
    }
}

#[test]
fn fork_fanout_admission_is_transactional_and_exact_limits_work() {
    for resource in [0, 1, 2] {
        let (doc, plan) = compiled(
            vec![
                core(1, "on-start"),
                fork(2, "all", &[0, 1]),
                branch(3, true),
                branch(4, false),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "branch#0", 3, "in"),
                edge(3, 2, "branch#1", 4, "in"),
            ],
        );
        for fits in [false, true] {
            let mut options = RunOptions::default();
            match resource {
                0 => options.limits.max_strands = if fits { 3 } else { 2 },
                1 => options.limits.max_regions = if fits { 4 } else { 3 },
                _ => options.limits.max_region_depth = if fits { 2 } else { 1 },
            }
            let mut run =
                Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
            if fits {
                drain(&mut run, 1);
            } else {
                assert_eq!(run.pump(1000), Err(RunError::ResourceLimit));
                assert!(
                    !run.trace()
                        .iter()
                        .any(|e| e.node.to_string() == id(3) || e.node.to_string() == id(4))
                );
                assert_eq!(run.pending_continuations(), 0);
            }
        }
    }
}

#[test]
fn race_cleanup_finishes_before_enclosing_sequence_and_preserves_normal_fuel() {
    for budget in [1, 1000] {
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                sequence(2, &[0, 1]),
                fork(3, "race", &[0, 1]),
                wait(4, 100),
                wait(5, 1),
                set_bool(6, 100, true),
                set_bool(7, 101, true),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "then#0", 3, "in"),
                edge(3, 3, "branch#0", 4, "in"),
                edge(4, 3, "branch#1", 5, "in"),
                edge(5, 4, "cancelled", 6, "in"),
                edge(6, 2, "then#1", 7, "in"),
            ],
            (100..102)
                .map(|n| variable(n, "run", "core/bool@1", Some(json!(false))))
                .collect(),
        );
        let mut options = RunOptions::default();
        options.data.limits.max_activation_operations = 3;
        options.max_cleanup_operations = 8;
        let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
        drain(&mut run, budget);
        run.advance_to(1).unwrap();
        drain(&mut run, budget);
        assert!(flag(&run, 100) && flag(&run, 101));
        let events: Vec<_> = run.trace().iter().map(|e| e.node.to_string()).collect();
        assert!(
            events.iter().position(|n| *n == id(6)).unwrap()
                < events.iter().position(|n| *n == id(7)).unwrap()
        );
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases:96,rng_seed:proptest::test_runner::RngSeed::Fixed(20261025),..proptest::test_runner::Config::default() })]
    #[test]
    fn nested_forks_match_independent_future_source_interpreter(
        outer in 0usize..4, inner in 0usize..4, iterations in 0i64..4,
        seed in proptest::num::u64::ANY, handled_fault in proptest::bool::ANY, reverse in proptest::bool::ANY,
    ) {
        let modes=["all","any","race","detach"];
        let mut nodes=vec![core(1,"on-start"),count(2,iterations),fork(3,modes[outer],&[7,2]),fork(4,modes[inner],&[5,1]),wait(5,1),wait(6,3),var_node(7,"set-variable",100),wait(8,9),var_node(9,"set-variable",101),set_bool(10,102,true),wait(11,50),set_bool(12,103,true),set_bool(13,104,true),sequence(14,&[2,0]),var_node(16,"get-variable",105),set_bool(17,106,true),random(30),random(31)];
        let mut edges=vec![edge(1,1,"then",14,"in"),edge(2,14,"then#2",2,"in"),edge(3,14,"then#0",13,"in"),edge(4,2,"body",3,"in"),edge(5,3,"branch#7",5,"in"),edge(6,5,"then",4,"in"),edge(7,3,"branch#2",11,"in"),edge(8,11,"then",12,"in"),edge(9,4,"branch#5",6,"in"),edge(10,6,"then",7,"in"),edge(11,4,"branch#1",8,"in"),edge(12,8,"then",9,"in"),edge(13,4,"completed",10,"in"),edge(14,if handled_fault {16}else{30},"value",7,"value"),edge(15,31,"value",9,"value"),edge(16,7,"failed",17,"in")];
        if reverse{nodes.reverse();edges.reverse();}
        let (doc,plan)=state_plan(nodes,edges,vec![variable(100,"run","core/u64@1",Some(json!("0"))),variable(101,"run","core/u64@1",Some(json!("0"))),variable(102,"run","core/bool@1",Some(json!(false))),variable(103,"run","core/bool@1",Some(json!(false))),variable(104,"run","core/bool@1",Some(json!(false))),variable(105,"local","core/u64@1",None),variable(106,"run","core/bool@1",Some(json!(false)))]);
        let analysis=analyze(&doc,plan.data().snapshot());
        let oracle=fork_reference::ForkReference::new(&doc,plan.data().snapshot(),&analysis,seed);
        oracle.run();
        for budget in [1,7,1000] {
            let mut run=Run::start(plan.clone(),&doc,plan.data().snapshot(),seed,RunOptions::default()).unwrap();
            let mut observations=vec![];let mut seen=0;
            for _ in 0..10000 {
                let progress=run.pump(budget).unwrap();assert!(progress.operations<=budget);
                for event in run.trace().iter().skip(seen) {
                    let definition=doc.root().nodes[&event.node].definition.id.as_str();
                    let label=match (definition,event.outcome) {
                        ("core/set-variable",orange_runtime::ControlOutcome::Then)=>Some("then"),
                        ("core/wait",orange_runtime::ControlOutcome::Then)=>Some("then"),
                        ("core/wait",orange_runtime::ControlOutcome::Cancelled)=>Some("cancelled"),_=>None,
                    };
                    if let Some(label)=label{observations.push((run.now(),event.node,label));}
                }
                seen=run.trace().len();
                if !progress.ready {if let Some(at)=progress.next_wakeup{run.advance_to(at).unwrap();}else{break;}}
            }
            for (variable,value) in oracle.vars.borrow().iter() {
                if doc.variables()[variable].scope==orange_model::document::VarScope::Run{assert_eq!(run.variable(*variable),value.as_ref());}
            }
            observations.sort();let mut expected=oracle.observations.borrow().clone();expected.sort();
            assert_eq!(observations,expected);
            let mut faults:Vec<_>=run.trace().iter().filter_map(|e|e.fault.as_ref().map(|f|(e.node,f.source,f.fault.code))).collect();
            faults.sort();let mut expected=oracle.faults.borrow().clone();expected.sort();assert_eq!(faults,expected);
            assert_eq!(run.ownership().activations,0);assert_eq!(run.ownership().regions,0);
        }
    }
}
