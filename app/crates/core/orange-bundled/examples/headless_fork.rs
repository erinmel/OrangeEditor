//! cargo run -p orange-bundled --example headless_fork --locked
//! Compare the four joins using saved graphs and a host-owned virtual clock.
use orange_bundled::MathPackage;
use orange_compiler::{CompileOptions, compile_control};
use orange_model::catalog::RegistryBuilder;
use orange_model::validation::HostOrigin;
use orange_model::{analyze, from_json};
use orange_runtime::{ControlOutcome, Run, RunOptions};
use serde_json::{Value, json};
use std::sync::Arc;

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn core(n: u32, name: &str) -> Value {
    json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}})
}
fn edge(n: u32, from: u32, port: &str, to: u32) -> Value {
    json!({"id":format!("30000000-0000-4000-8000-{n:012x}"),"from":{"node":id(from),"port":port},"to":{"node":id(to),"port":"in"}})
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Illustrative host identity; no package executable is needed by these graphs.
    let digest = format!("sha256:{}", "e".repeat(64));
    let package = MathPackage::new(digest.clone())?;
    let registry = RegistryBuilder::core(HostOrigin::new("0.0.0", digest)?).build()?;
    for (join, expected) in [("all", 10), ("any", 5), ("race", 5), ("detach", 0)] {
        let mut start = core(1, "on-start");
        start["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
        let mut fork = core(2, "fork");
        fork["config"] = json!({"join":join});
        fork["variadic"] = json!({"branch":{"order":[7,2],"next":8}});
        let mut slow = core(3, "wait");
        slow["inputs"] = json!({"duration":{"type":"core/duration@1","value":"10"}});
        let mut fast = core(4, "wait");
        fast["inputs"] = json!({"duration":{"type":"core/duration@1","value":"5"}});
        let mut completed = core(5, "branch");
        completed["inputs"] = json!({"condition":{"type":"core/bool@1","value":true}});
        let saved = json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001","revision":0,"package_lock":[],"signatures":{},
            "nodes":[start,fork,slow,fast,completed],"edges":[edge(1,1,"then",2),edge(2,2,"branch#7",3),edge(3,2,"branch#2",4),edge(4,2,"completed",5)]});
        let document = from_json(&saved.to_string())?;
        let plan = Arc::new(compile_control(
            &document,
            registry.clone(),
            &analyze(&document, &registry),
            &package,
            CompileOptions::default(),
        )?);
        let mut run = Run::start(plan, &document, &registry, 42, RunOptions::default())?;
        let mut completed_at = None;
        loop {
            let progress = run.pump(7)?;
            if completed_at.is_none()
                && run
                    .trace()
                    .iter()
                    .any(|e| e.node.to_string() == id(5) && e.outcome == ControlOutcome::True)
            {
                completed_at = Some(run.now());
            }
            if !progress.ready {
                if let Some(at) = progress.next_wakeup {
                    run.advance_to(at)?;
                } else {
                    break;
                }
            }
        }
        assert_eq!(completed_at, Some(expected));
        assert_eq!(run.ownership().activations, 0);
        println!(
            "{join}: completed_at={}ns; all_owned_work_finished_at={}ns",
            completed_at.unwrap(),
            run.now()
        );
        run.stop()?;
        run.dispose()?;
    }
    Ok(())
}
