//! cargo run -p orange-bundled --example headless_control --locked
//! An embedding host owns the clock and drives work; no window, OS timer or I/O.
use orange_bundled::{MathPackage, math_definitions};
use orange_compiler::{CompileOptions, compile_control};
use orange_model::catalog::{Ports, RegistryBuilder};
use orange_model::validation::HostOrigin;
use orange_model::{analyze, from_json};
use orange_runtime::{Run, RunOptions, RunState, StopMode};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn core(n: u32, name: &str) -> Value {
    json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}})
}
fn edge(n: u32, from: u32, output: &str, to: u32, input: &str) -> Value {
    json!({"id":format!("30000000-0000-4000-8000-{n:012x}"),"from":{"node":id(from),"port":output},"to":{"node":id(to),"port":input}})
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Illustrative build identity. A production host authenticates actual provider bytes.
    let digest = format!("sha256:{}", "e".repeat(64));
    let package = MathPackage::new(digest.clone())?;
    let registry = package
        .register(RegistryBuilder::core(HostOrigin::new("0.0.0", digest)?))?
        .build()?;
    let add = math_definitions()?
        .into_iter()
        .find(|d| d.definition.id.as_str() == "org.orange.math/add")
        .expect("bundled Add");
    let Ports::Declared(ports) = add.ports else {
        unreachable!("bundled declared ports")
    };
    let args = BTreeMap::from([("T".into(), "core/u64@1".parse()?)]);
    let signature = orange_model::signature::project(add.kind, ports, &args);
    let mut start = core(1, "on-start");
    start["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
    let mut sequence = core(2, "sequence");
    sequence["variadic"] = json!({"then":{"order":[0,1],"next":2}});
    let mut count = core(3, "count");
    count["inputs"] = json!({"count":{"type":"core/i64@1","value":"3"}});
    let mut get = core(4, "get-variable");
    get["config"] = json!({"variable":id(100)});
    let key = package.provider_key();
    let add_node = json!({"id":id(5),"definition_id":"org.orange.math/add","definition_version":1,"config_version":1,"config":{"int_overflow":"checked"},"signature":"add-u64","provider":{"package":key.package,"version":key.version,"id":key.id},"inputs":{"b":{"type":"core/u64@1","value":"1"}}});
    let mut set = core(6, "set-variable");
    set["config"] = json!({"variable":id(100)});
    let mut wait = core(7, "wait");
    wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"10"}});
    let mut branch = core(8, "branch");
    branch["inputs"] = json!({"condition":{"type":"core/bool@1","value":true}});
    let saved = json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001","revision":0,
        "package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],"signatures":{"add-u64":signature},
        "variables":[{"id":id(100),"name":"total","type":"core/u64@1","scope":"run","init":{"type":"core/u64@1","value":"0"}}],
        "nodes":[start,sequence,count,get,add_node,set,wait,branch],
        "edges":[edge(1,1,"then",2,"in"),edge(2,2,"then#0",3,"in"),edge(3,3,"body",6,"in"),edge(4,4,"value",5,"a"),edge(5,5,"sum",6,"value"),edge(6,2,"then#1",7,"in"),edge(7,7,"then",8,"in")]});
    let document = from_json(&saved.to_string())?;
    let analysis = analyze(&document, &registry);
    let plan = Arc::new(compile_control(
        &document,
        registry.clone(),
        &analysis,
        &package,
        CompileOptions::default(),
    )?);
    let mut run = Run::start(plan, &document, &registry, 42, RunOptions::default())?;
    loop {
        let progress = run.pump(64)?;
        if !progress.ready {
            if let Some(at) = progress.next_wakeup {
                run.advance_to(at)?;
            } else {
                break;
            }
        }
    }
    assert_eq!(
        run.variable(id(100).parse()?),
        Some(&orange_model::Value::U64(3))
    );
    println!("total=3; virtual_time_ns={}", run.now());
    run.request_stop(StopMode::Graceful)?;
    assert_eq!(run.state(), RunState::Stopped);
    run.dispose()?;
    Ok(())
}
