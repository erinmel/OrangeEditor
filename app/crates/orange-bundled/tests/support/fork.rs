use super::*;
use orange_runtime::{Run, RunError, RunOptions};

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
