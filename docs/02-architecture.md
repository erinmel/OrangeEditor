# Architecture

## Embedding surface

Use `orange-host` as the Rust application boundary. Create a `Host` with
`HostOptions` and register application services with `Host::with_services`.
Use `Draft` and the host's authoring operations for edits, `GraphView` for
rendering, and `Diagnostic` for validation. For running graphs, hold the returned
`FlowId`, consume `Events`, and use the host for triggers, inspection and stop.
The [headless example](../app/crates/platform/orange-host/examples/attach.rs)
exercises this lifecycle. Use `orange-sdk` to provide node and connection services.

These are the supported integration entry points. Pin the same OrangeEditor
revision for all its Rust crates during the `0.x` series. Source compatibility
is a goal for this surface, and planned breaking changes need a migration note.
Internal compiler and runtime APIs, implementation crate re-exports, and Rust
dynamic-library ABI are outside that promise. Versioned graph and package formats
follow [12](12-graph-data-structures.md) and [04](04-expansion-plugins.md).

For an egui UI, pass `Editor::ui_with_layout` your containers. Compose
`toolbar`, `workspace_bar`, `nodes_panel`, `right_panel`, `log_panel` and
`canvas_panel` as needed. Use the application egui theme, or apply the optional
desktop defaults with `apply_theme`. Install a context-local toolbar and menu
command painter with `icons::set_command_painter`. It receives the icon constant,
the allotted rectangle and the current foreground color, with the stock icon
used when the painter returns `false`. To override editor text, register Fluent
messages with `i18n::add_application_catalogs` before any editor text is read.
Packages can supply their own node text and the host owns file dialogs through
`Editor::take_request`. Set `Editor::docs_url` to a node reference, such as the published one, and F1 opens the selected core or `org.orange.*` node's page there through egui's URL opening (2026-11-26). The [embedding test](../app/crates/apps/orange-editor-ui/tests/embedding.rs)
exercises the public layout, theme, icon and text hooks. The runnable
[embedded editor](../app/crates/apps/orange-desktop/examples/embed_editor.rs)
uses the canvas and an optional inspector inside its own app chrome.

`orange-ui-controls` provides reusable egui components and depends on no graph
or host crate in product builds. Hosts own component state and localized labels.
The default-enabled `icons` feature supplies the `UiIcon` adapter and its
Material, Phosphor and Lucide assets. `native-dialogs` supplies an
`rfd` adapter for file, multi-file and folder selection; a host may handle the
picker response directly. The desktop gallery's `eframe`, WAV and output-device
dependencies are example-only. Meters consume host measurements and the
frequency utilities take caller-owned filter state and caller-provided buffers.
The documentation's browser examples use `orange-docs-web`, a development crate
that draws the shared controls with eframe/WebGL on `wasm32-unknown-unknown`.
Their input values are local to the demo. The editor's `orange-host`
integration still requires the native host and its OS services.

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
   `Host::draft_authoring_view` also includes unwired implicit failure ports and a
   map of their identities by node, so a front end can offer them for wiring.
   `Host::draft_authoring_view_in` projects localized port captions, help and literal
   controls. Connection edits use the fresh graph contract regardless of port
   visibility. A missing exact provider leaves its saved signature available to
   draw existing wires and repair the graph. Host edits cannot use that signature
   to add a connection or write a literal.
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
- **Observation** is a bounded channel of `HostEvent`s. A slow observer can miss
  telemetry. The host retries the latest changed values while the flow is active
  and prioritizes terminal notifications in the overflow queue. `Host::state`
  reports authoritative state while the flow is retained by the host.
  Trace forwarding counts evicted and refused new events as lost. A trace cursor
  belongs to one run and starts fresh on replacement.
- **Effects** run on a worker pool that grows while every worker is busy, up to a
  bound, so a sound playing to its end or a slow server never stalls other nodes. The
  queue of waiting effects is bounded too. A single flow can occupy at most half
  the worker ceiling rounded up, including queued work.
- **Services** come from `ServicePackage`s: compiled in (`Host::with_services`) or loaded
  from native libraries. Opening a document opens a session for each connection its
  nodes name; graph-scoped sessions belong to the flow, shared ones to the host.
- **Workspace** (2026-11-26). `Host::share_variables` declares what flows share: the
  `workspace`-scope variables and events of one document. A flow whose graph uses
  either runs against that workspace; one that uses workspace events joins the
  host's workspace bus when it opens or is replaced. A Dispatch on a workspace event
  reaches every other flow that listens to it, never its sender, on that flow's next
  turn. Flows with workspace events pump while holding the bus, so their turns take
  it one at a time; flows without them never wait on it ([03](03-data-and-execution.md)).
- **Persistent variables** have one writer for each graph ID or workspace state
  directory. A second host or an offline edit must acquire the same file lock before
  writing. A replacement with persistent variables writes the new declarations even
  when their current values equal the previous checkpoint's values.
- **Packages** install, update, turn off and on, and uninstall while the editor is
  open; the base and clock nodes are core and always on. A running flow
  keeps the provider it was compiled with until it stops ([04](04-expansion-plugins.md)).

## Trust

| Code | Runs where | Bounded by |
|---|---|---|
| Bundled nodes | In process | Reviewed with the core |
| Wasm packages | In process, in Wasmtime | Fuel, epochs, memory, grants given at install |
| Native plugins and services | In process | Nothing: trusted like the application |
| Graphs | In the runtime | Declared limits on time, memory, queues and recursion |

Graph-scoped credential approvals stay in the OS store and bind the graph,
connection, adapter and settings before credentials are resolved. Documents, traces and exports hold references and
redacted values only. Product text is translated through Fluent catalogues
([13](13-internationalization.md)); the language of the interface never changes what a
graph means.

## Decisions

The reasons behind these choices are in [08](08-decisions.md): Rust without a game
engine (ADR-02), the foundation split into an embeddable core and a separate editor
(ADR-21), exact provider pinning (ADR-07), and the delivery scope (ADR-22).
