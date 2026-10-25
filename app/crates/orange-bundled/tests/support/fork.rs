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
