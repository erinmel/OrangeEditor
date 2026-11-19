# Architecture

## Layers

```text
apps        orange-desktop   orange-editor-ui   orange-cli
              │                 │                  │
platform    orange-host ── orange-packages ── orange-guest (Wasm) ── orange-native (dll)
              │              orange-sdk   orange-vault   orange-sandbox
services    orange-audio   orange-net   orange-files        (ServicePackages on orange-sdk)
              │
core        orange-runtime ← orange-compiler ← orange-model    orange-kernel   orange-bundled
dev         orange-testkit   orange-measure   orange-plugin-example
```

Dependencies point down. The core has no UI, window, network or engine dependency;
`tools/check-cargo-metadata.mjs` enforces that in CI. Every crate lives in
`app/crates/<layer>/<crate>`; installable expansions go in `plugins/` and build on their
own; the Wasm guest interface (WIT) and its toolchain are in `sdk/`.

| Crate | Owns |
|---|---|
| `orange-model` | The graph document, transactions and undo, indexes, types and inference, validation, the node catalogue |
| `orange-compiler` | Turns a checked document into immutable data plans and control plans |
| `orange-runtime` | Runs plans: data epochs, structured control, virtual time, events, streams, resources |
| `orange-kernel` | The traits a node implementation provides: pure, entropy, effect |
| `orange-bundled` | The shipped native nodes: math, logic, text, lists, JSON, comparisons, conversions |
| `orange-packages` | Package archives, manifests, the install store, provider slots and the registry |
| `orange-guest` | Wasm components on Wasmtime with fuel, epochs and memory limits |
| `orange-native` | Loads native plugin libraries that export a `ServicePackage` |
| `orange-sdk` | What a Rust plugin implements: nodes, triggers, live values, connection adapters |
| `orange-sandbox` | File access confined to one directory |
| `orange-vault` | Credentials in the operating system's store |
| `orange-host` | The session every front end attaches to: drafts, flows, effects, services |
| `orange-audio`, `orange-net`, `orange-files` | The base service packages |
| `orange-editor-ui` | The egui editor as a library |
| `orange-desktop`, `orange-cli` | The `orange-editor` window and the `orange` terminal runner |

## From an edit to a running graph

1. **Edit.** A front end holds a `Draft` from `Host::draft`. Every edit (`add_node`,
   `connect`, `set_input`, …) is one model transaction with an exact inverse for undo.
   Type inference writes what the wires settle in the same transaction.
2. **Check.** `Host::problems` keeps an `AnalysisSession` beside the draft and re-checks
   only what an edit touched ([16](16-incremental-validation.md)).
3. **Show.** `Host::draft_view` turns the draft into a `GraphView` of cards and wires.
   It rebuilds only the cards of nodes the edits changed; unchanged cards are shared.
   Previews compute the values of pure outputs that the edit touched, never running an
   effect.
4. **Compile.** `Host::open` resolves every node to its exact provider, validates the
   whole document and compiles it to data and control plans.
5. **Run.** The document becomes a *flow*: one run on its own thread, driven against the
   host's clock. Flows share nothing mutable, so pausing or faulting one never holds
   another.

## The host

- **Control** (`pause`, `resume`, `stop`, `trigger`, `emit`) is a bounded, non-blocking
  send to a flow's mailbox; a full mailbox answers `Busy`.
- **Observation** is one bounded channel of `HostEvent`s. A slow observer loses
  telemetry, counted, but never the end of a flow, and never slows a run.
- **Effects** run on a worker pool that grows while every worker is busy, up to a
  bound, so a sound playing to its end or a slow server never stalls other nodes. The
  queue of waiting effects is bounded too.
- **Services** come from `ServicePackage`s: compiled in (`Host::with_services`) or loaded
  from native libraries. Opening a document opens a session for each connection its
  nodes name; graph-scoped sessions belong to the flow, shared ones to the host.
- **Packages** install, update and uninstall while the editor is open. A running flow
  keeps the provider it was compiled with until it stops ([04](04-expansion-plugins.md)).

## Trust

| Code | Runs where | Bounded by |
|---|---|---|
| Bundled nodes | In process | Reviewed with the core |
| Wasm packages | In process, in Wasmtime | Fuel, epochs, memory, grants given at install |
| Native plugins and services | In process | Nothing: trusted like the application |
| Graphs | In the runtime | Declared limits on time, memory, queues and recursion |

Credentials stay in the OS store; documents, traces and exports hold references and
redacted values only. Product text is translated through Fluent catalogues
([13](13-internationalization.md)); the language of the interface never changes what a
graph means.

## Decisions

The reasons behind these choices are in [08](08-decisions.md): Rust without a game
engine (ADR-02), the foundation split into an embeddable core and a separate editor
(ADR-21), exact provider pinning (ADR-07), and the delivery scope (ADR-22).
