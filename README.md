# OrangeEditor

A node editor foundation in Rust: a typed graph language with an embeddable headless
core, an egui editor, a desktop application and a terminal runner. Graphs react to
triggers (manual, timers, keys, files, network messages, audio), compute with typed
values and call services such as HTTP, WebSocket, MQTT, AMQP, files and audio.
Packages and native plugins add nodes, triggers and connection kinds without
rebuilding the application.

It is pre-1.0: the core, editor and base services work; see
[the overview](docs/00-overview.md) for what is done and what is missing.

## Run it

Use the toolchain in [rust-toolchain.toml](rust-toolchain.toml) (Rust 1.99.1) from the
repository root.

```sh
cargo run -p orange-desktop --locked -- graph.json        # the editor
cargo run -p orange-cli --locked -- check app/crates/apps/orange-cli/graphs/set-on-trigger.json
cargo run -p orange-cli --locked -- run app/crates/apps/orange-cli/graphs/set-on-trigger.json
```

Example graphs are in [app/crates/apps/orange-cli/graphs](app/crates/apps/orange-cli/graphs):
`json-extraction.json` works offline, `connection-smoke.json` exercises JSON, feedback and
connections, and `network-lab.json` talks to the [Docker network lab](tools/README.md).

[power-with-count.json](app/crates/apps/orange-cli/graphs/power-with-count.json) shows a
numeric `for` loop using Count. Open it in the editor, press Run, then Fire on Calculate
power. Edit the Base or Exponent card and fire again. The Variables panel shows `result`
(81 with the default inputs). The graph resets the accumulator to 1, multiplies it by Base
once per iteration, then saves the result on Completed. No wire goes from the body back
to Count. A zero exponent skips the body and leaves the result at 1.

Double-click a node name in Properties or on the card to rename it. An empty name
restores the node type's name.

To make a reusable **Compute power** node, leave **Calculate power**, **Base**,
**Exponent**, and **Save result** outside the selection. Shift-click the other cards
and choose **Make function…**. The incoming value wires become the inputs
`a` (base) and `count` (exponent), and the value wire going to **Save result**
becomes the `value` output. Confirm the preview, then press **Run** and **Fire** to
see the result in Variables. Duplicate the **Compute power** call to use the same
function again with its own arguments and wires.

For ordinary exponentiation with floating-point values, add **Power** from the
node search. The Count example demonstrates how to carry state through a loop.

To share a function between graphs, right-click its call and choose
**Export function…**. Review its ports and dependencies, enter the package ID
(such as `org.example.math`), and save the `.orangeplug` file. Import it from
the Packages panel in the other graph. A function that uses a connection exposes
that connection on the imported card so the receiving graph can choose its own.

A front end embeds the core through `orange-host` alone;
[examples/attach.rs](app/crates/platform/orange-host/examples/attach.rs) is a complete one.
[Embedding](docs/02-architecture.md#embedding-surface) documents the supported host API
and the editor's layout, theme, command icon and text hooks.
A Rust plugin can be a dynamic library built on `orange-sdk`, as
[orange-plugin-example](app/crates/dev/orange-plugin-example) shows.

## Check it

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node tools/check-cargo-metadata.mjs
node tools/check-automation.mjs --offline
```

[Development](docs/10-development.md) covers the toolchain, CI, testing, performance
measurement and the Git workflow.

## Documents

| Document | What it holds |
|---|---|
| [00 Overview](docs/00-overview.md) | Vision, scope, status and what is missing |
| [02 Architecture](docs/02-architecture.md) | Crates, layers, how an edit becomes a running graph |
| [03 Data and execution](docs/03-data-and-execution.md) | Runtime semantics: epochs, control, effects, streams |
| [04 Expansion packages](docs/04-expansion-plugins.md) | Package format, install, override, hot update |
| [05 Services and platforms](docs/05-services-and-platforms.md) | Connections, network, files, audio, input, credentials |
| [08 Decisions](docs/08-decisions.md) | Architecture decision records |
| [10 Development](docs/10-development.md) | Toolchain, CI, tests, performance, Git, releases |
| [11 Graph language](docs/11-graph-language.md) | Normative language rules |
| [12 Graph data structures](docs/12-graph-data-structures.md) | Normative document format, values and catalogues |
| [13 Internationalization](docs/13-internationalization.md) | Translatable text contract |
| [16 Incremental validation](docs/16-incremental-validation.md) | How edits are re-checked |
| [20 Value checkpoints](docs/20-value-checkpoints.md) | Persistent value format |
| [22 Editor design](docs/22-editor-design.md) | Editor interaction and visual rules |

[Contributing](CONTRIBUTING.md) and [agent instructions](AGENTS.md) describe how to work
in this repository.
