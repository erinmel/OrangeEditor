//! The finished program of the "Run a graph from Rust" tutorial: open the counter graph,
//! press its Increment button three times and print Counter as it changes.
//!
//! Run with `cargo run -p orange-host --example run_counter --locked`, or pass the path
//! of a graph you saved from the editor.
use orange_host::{FlowEvent, Host, HostOptions, NodeId, Severity, StopMode, Value, VariableId};
use std::error::Error;
use std::time::Duration;

const COUNTER: &str = include_str!("../../../apps/orange-cli/graphs/counter-with-reset.json");

/// The ID of the node or variable a graph file gives this name.
fn id_named<T: std::str::FromStr>(graph: &serde_json::Value, list: &str, name: &str) -> Option<T> {
    graph[list]
        .as_array()?
        .iter()
        .find(|item| item["name"] == name)?["id"]
        .as_str()?
        .parse()
        .ok()
}

fn main() -> Result<(), Box<dyn Error>> {
    let text = match std::env::args().nth(1) {
        Some(path) => std::fs::read_to_string(path)?,
        None => COUNTER.to_owned(),
    };

    let (mut host, events) = Host::new(HostOptions::default())?;
    // Checking reports what the editor's Problems panel would show, without running.
    let mut blocked = false;
    for problem in host.check(&text)? {
        eprintln!("{:?}: {}", problem.severity, problem.message);
        blocked |= problem.severity == Severity::Error;
    }
    if blocked {
        return Err("the graph has errors; open it in the editor to fix them".into());
    }

    let graph: serde_json::Value = serde_json::from_str(&text)?;
    let increment: NodeId = id_named(&graph, "nodes", "Increment").ok_or("no Increment node")?;
    let counter: VariableId =
        id_named(&graph, "variables", "Counter").ok_or("no Counter variable")?;

    let flow = host.open(&text)?;
    host.observe(flow, true)?;
    for _ in 0..3 {
        host.trigger(flow, increment, None)?;
    }

    let mut stopping = false;
    loop {
        let event = events.recv_timeout(Duration::from_secs(10))?;
        match event.kind {
            FlowEvent::Variables(changed) => {
                for (variable, value) in changed {
                    if variable != counter {
                        continue;
                    }
                    println!("Counter = {value:?}");
                    if value == Some(Value::I64(3)) && !stopping {
                        stopping = true;
                        host.stop(flow, StopMode::Graceful)?;
                    }
                }
            }
            FlowEvent::Fault(error) => return Err(format!("{error:?}").into()),
            FlowEvent::Finished => break,
            _ => {}
        }
    }
    host.close(flow)?;
    Ok(())
}
