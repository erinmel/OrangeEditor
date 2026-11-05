//! Function frames, arguments and results against the run's aggregate budget.
use super::action_functions::{action_call, compile_plan, initialized, local_function, set, start};
use super::*;
use crate::budget_drive::boundary;
use orange_runtime::RunError;

#[test]
fn function_frames_arguments_and_results_reserve_from_the_aggregate_budget() {
    let (package, snapshot) = catalog();
    let mut count = core(2, "count");
    count["inputs"] = json!({"count":{"type":"core/i64@1","value":"3"}});
    let doc = document(
        &package,
        vec![start(), count, action_call(3, 100, 42), set(4, 11, 0)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 3, "result", 4, "value"),
        ],
        vec![],
        vec![local_function(100, 10)],
        vec![initialized(10, "local", 7), initialized(11, "run", 0)],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    // Each call reserves its frame, retains its result, then releases the frame; the
    // repeated calls replace the previous result, so the peak does not grow with them.
    let refusal = boundary(|limit| {
        let mut options = RunOptions::default();
        options.limits.max_run_retained_bytes = limit;
        Run::start(plan.clone(), &doc, &snapshot, 1, options)
    });
    assert_eq!(refusal, RunError::AggregateLimit);
}
