# Run a graph from Rust

In this tutorial you write a small Rust program that opens the counter graph, presses
its **Increment** button three times and prints **Counter**. You meet the
[`Host`](@shared/api/orange_host/struct.Host.html), which every application uses to
check and run graphs, and the event channel that reports what a running graph does.
No window is involved. It takes about fifteen minutes.

You need Rust 1.97.1 or later and a clone of the OrangeEditor repository. It helps to
have done [Count button presses](../count-button-presses/) first, but the counter
graph is also in the repository.

The finished program is
[`examples/run_counter.rs`](@source/app/crates/platform/orange-host/examples/run_counter.rs).
From the repository root, `cargo run -p orange-host --example run_counter --locked`
runs it.

## Create the project

Create a project next to your clone of OrangeEditor:

```sh
cargo new counter-runner
cd counter-runner
```

Add `orange-host` from the clone, and `serde_json` to read names from the graph file,
in `Cargo.toml`:

```toml
[dependencies]
orange-host = { path = "../OrangeEditor/app/crates/platform/orange-host" }
serde_json = "1"
```

Copy the graph you saved in the counter tutorial to `counter.json` in the project
folder, or copy `app/crates/apps/orange-cli/graphs/counter-with-reset.json` from the
clone.

## Check the graph

A host holds the node catalogue and runs graphs. One is enough for an application.
[`Host::check`](@shared/api/orange_host/struct.Host.html#method.check) reports what
the editor's **Problems** panel would show, without running anything. Replace
`src/main.rs` with:

```rust
use orange_host::{FlowEvent, Host, HostOptions, NodeId, Severity, StopMode, Value, VariableId};
use std::error::Error;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
    let text = std::fs::read_to_string("counter.json")?;
    let (mut host, events) = Host::new(HostOptions::default())?;

    let mut blocked = false;
    for problem in host.check(&text)? {
        eprintln!("{:?}: {}", problem.severity, problem.message);
        blocked |= problem.severity == Severity::Error;
    }
    if blocked {
        return Err("the graph has errors; open it in the editor to fix them".into());
    }
    Ok(())
}
```

Run `cargo run`. It prints nothing and exits: the graph has no problems. Change
`"core/i64@1"` in the variable's `type` to `"core/i64@9"` and run it again to see an
error, then change it back.

## Find the button and the variable

A graph refers to nodes and variables by ID, and a person names them. Add this
function above `main` to look an ID up by name in the graph file:

```rust
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
```

Then, in `main`, before `Ok(())`:

```rust
    let graph: serde_json::Value = serde_json::from_str(&text)?;
    let increment: NodeId = id_named(&graph, "nodes", "Increment").ok_or("no Increment node")?;
    let counter: VariableId =
        id_named(&graph, "variables", "Counter").ok_or("no Counter variable")?;
```

## Press the button

[`Host::open`](@shared/api/orange_host/struct.Host.html#method.open) starts the graph
as a flow on its own thread and returns at once.
[`Host::trigger`](@shared/api/orange_host/struct.Host.html#method.trigger) does what
**Fire** does in the editor. Each call queues one activation.

```rust
    let flow = host.open(&text)?;
    host.observe(flow, true)?;
    for _ in 0..3 {
        host.trigger(flow, increment, None)?;
    }
```

`observe` asks the flow to report its variables. Without it, the flow runs just the
same but stays quiet.

## Read what happens

Everything every flow does arrives on the `events` channel that `Host::new` returned.
Wait for **Counter** to reach 3, then stop the flow and wait for it to finish:

```rust
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
```

Run `cargo run`. The last line it prints is:

```text
Counter = Some(I64(3))
```

You may also see `Some(I64(1))` or `Some(I64(2))` first. A report holds the latest
value of each variable that changed since the previous report, so fast changes can
arrive as one.

## What you built

- **One host, many flows.** `Host::open` can run several graphs at once, each on its
  own thread, and every one reports on the same channel. `event.flow` says which.
- **The host is the editor's engine.** The desktop editor uses these same calls; it
  draws the events instead of printing them.
- **Graph files are data.** Your program reads names from the JSON, but the host
  checks and runs it; you never execute anything from the file yourself.

Next, give the editor's window to your own app in
[Embed and customize the egui editor](../egui/), or read the
[`orange-host` API](@shared/api/orange_host/) and [Embed the runtime](../embedding/)
for services, packages and shared variables.
