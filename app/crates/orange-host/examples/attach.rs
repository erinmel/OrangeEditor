//! The smallest front end. Everything a user interface needs from the core is reached
//! through `orange-host` alone: the node palette, the package list, opening a document
//! as a flow, controlling it and watching its values. A graphical editor attaches the
//! same way and draws what this example prints.
//!
//! Run with `cargo run -p orange-host --example attach --locked`.
use orange_host::{FlowEvent, Host, HostOptions, StopMode};
use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

const GRAPH: &str = include_str!("../../orange-cli/graphs/set-on-trigger.json");

fn main() -> Result<(), Box<dyn Error>> {
    // One host per application. Service packages (network, audio, your own) are added
    // here with `Host::with_services`; this example uses the core and base nodes only.
    let (mut host, events) = Host::new(HostOptions::default())?;

    // The palette: every node, with a name and a category to group it under.
    let mut palette: BTreeMap<String, usize> = BTreeMap::new();
    for node in host.nodes() {
        *palette.entry(node.category().into_owned()).or_default() += 1;
    }
    for (category, nodes) in &palette {
        println!("palette: {category} has {nodes} nodes");
    }

    // The package manager view.
    println!("packages: {}", host.packages().count());

    // Checking a document reports problems without running it.
    let diagnostics = host.check(GRAPH)?;
    println!("check: {} diagnostics", diagnostics.len());

    // Opening it starts a flow on its own thread; many can run at once.
    let flow = host.open(GRAPH)?;
    host.observe(flow, true)?;
    let entry = host
        .info(flow)
        .and_then(|info| info.manual_entries.first().copied())
        .ok_or("the graph has no manual entry")?;
    host.trigger(flow, entry.0, None)?;

    // One channel carries what every flow does. A user interface drains it each frame.
    let mut stopping = false;
    loop {
        let event = events.recv_timeout(Duration::from_secs(10))?;
        match event.kind {
            FlowEvent::State(state) => println!("flow {}: {state:?}", event.flow),
            FlowEvent::Variables(values) => {
                for (variable, value) in values {
                    println!("flow {}: variable {variable} = {value:?}", event.flow);
                    if value.is_some() && !stopping {
                        stopping = true;
                        host.stop(flow, StopMode::Graceful)?;
                    }
                }
            }
            FlowEvent::Ports(ports) => {
                for (node, port, value) in ports {
                    println!("flow {}: {node}.{port} = {value:?}", event.flow);
                }
            }
            FlowEvent::Finished => break,
            _ => {}
        }
    }
    host.close(flow)?;
    Ok(())
}
