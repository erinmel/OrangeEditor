# Share values and events between graphs

One host can run several graphs at once, each on its own thread, so one slow or
paused graph never holds the others. This guide shares state and signals between
them.

## Shared variables

A shared variable belongs to the workspace, not to one graph. Every running graph
sees a write at its next turn, and writes from different graphs are applied one at a
time, so none is lost.

In a graph:

1. Declare the variable with the scope shared by all graphs.
2. Read it with **Get variable**.
3. Write it with **Set shared**, or change it from its current value with
   **Update shared**, such as adding one.

Every graph that uses the variable declares it with the same ID and type.

<!-- tutorial-graph: shared-score -->

## Shared events

A workspace event lets one graph start work in others. A **Dispatch** on a workspace
event fires the matching **Listen** in every *other* running graph, never in the graph
that sent it. Each Listen starts its own activation with the payload.

The editor does not yet declare workspace events, so declare them in the documents
your application shares (below).

## From Rust

An application embedding [`orange-host`](../embedding/) declares what its graphs
share once, from one document, before it opens graphs that use it:

```rust,ignore
use orange_host::{Host, HostOptions};

let (mut host, events) = Host::new(HostOptions::default())?;
// The workspace-scope variables and events of this document are shared.
host.share_variables(&workspace_json)?;
let scoring = host.open(&scoring_json)?;
let overlay = host.open(&overlay_json)?;
```

A document whose shared variables or events do not match the declaration does not
open. While graphs run, `Host::shared_variable` reads a value and
`Host::shared_writes` tells you when to read again.

## Limits

- Graphs that use workspace events take turns with each other, so a heavy one slows
  the others' event handling. Graphs without workspace events never wait.
- Events waiting for a graph that you edit while it runs are dropped, as they are for
  a stopped graph.

## See also

- [Embed the runtime](../embedding/)
- [Graph language reference](../reference/graph-language/)
