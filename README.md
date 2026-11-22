# OrangeEditor

A node editor foundation in Rust: a typed graph language with an embeddable headless
core, an egui editor, a desktop application and a terminal runner. Graphs react to
triggers (manual, timers, keys, files, network messages, audio), compute with typed
values and call services such as HTTP, WebSocket, MQTT, AMQP, files and audio.
Packages and native plugins add nodes, triggers and connection kinds without
rebuilding the application.

It is pre-1.0: the core, editor and base services work; see
[the overview](docs/00-overview.md) for what is done and what is missing.

## Starter workflows

| Graph | What it demonstrates |
| --- | --- |
| [Counter with reset](app/crates/apps/orange-cli/graphs/counter-with-reset.json) | Increment, decrement and reset the same initialized variable. Search for Increment or Bump + in the editor to add a preconfigured update. |
| [Quiet mic gate](app/crates/apps/orange-cli/graphs/quiet-mic-gate.json) | Route a microphone through a gate and meter it. The graph starts on silent devices. Choose your microphone and headphones when ready to listen. |
| [HTTP with failure handling](app/crates/apps/orange-cli/graphs/http-with-failure-handling.json) | Make a GET request and record success, an HTTP error status, or a transport error independently. The default endpoint is `http://127.0.0.1:8080/health`. |
| [HTTP with bounded retry](app/crates/apps/orange-cli/graphs/http-with-bounded-retry.json) | Retry GET once after 250 ms on the request failure route. HTTP error statuses terminate the attempt immediately. |
| [Idle watchdog](app/crates/apps/orange-cli/graphs/idle-watchdog.json) | Fire Heartbeat to start a 750 ms timer. Every new heartbeat replaces the pending wait. The Idle variable turns true once the flow goes quiet. Search for Idle Watchdog to add the three timer cards to another graph. |
| [Forward value with deadband](app/crates/apps/orange-cli/graphs/forward-value-with-deadband.json) | Change Reading in a running graph. Values only forward when they move more than 1 from the last qualifying reading. |

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
`remember-on-trigger.json` uses a node from a package in
[packages/remember](app/crates/apps/orange-cli/packages/remember): pack it, then pass it in.

```sh
cargo run -p orange-cli --locked -- pack app/crates/apps/orange-cli/packages/remember remember.orangeplug
cargo run -p orange-cli --locked -- run app/crates/apps/orange-cli/graphs/remember-on-trigger.json --package remember.orangeplug
```

[power-with-count.json](app/crates/apps/orange-cli/graphs/power-with-count.json) shows a
numeric `for` loop using Count. Open it in the editor, press Run, then Fire on Calculate
power. Edit the Base or Exponent card and fire again. The Variables panel shows `result`
(81 with the default inputs). The graph resets the accumulator to 1, multiplies it by Base
once per iteration, then saves the result on Completed. No wire goes from the body back
to Count. A zero exponent skips the body and leaves the result at 1.

[power-with-repeat.json](app/crates/apps/orange-cli/graphs/power-with-repeat.json)
uses Repeat with State and Multiply. It starts with 1 and multiplies by 3 four
times. [running-total-with-reduce.json](app/crates/apps/orange-cli/graphs/running-total-with-reduce.json)
uses Reduce and Add to sum 2, 3, and 5. Press Run and look for `result` in the
Variables panel. The default results are 81 and 10, respectively.

To send one JSON request after a burst of file changes, add **On file change**
with a Folder connection and select **Debounce** under **Event timing** in
Properties. Connect its execution output to **HTTP JSON Request**, and wire a
JSON object into Body. The HTTP node uses its own HTTP connection. Wire its
optional JSON result to **Match Option** to handle absent or non-JSON responses.

[audio-file-quality.json](app/crates/apps/orange-cli/graphs/audio-file-quality.json)
is a grouped QA graph for seven playback formats, audio triggers, loop controls,
recording, files and HTTP. Launch the editor from the repository root so the
relative fixture paths resolve. The file suite creates `orange-audio-qa-86dc`
under the launch directory. Fire **Run file and folder suite** and then edit a
file in that scratch folder to exercise the watcher.

The two output connections default to the silent test device. For listening tests,
choose a playback device for **QA output** and **QA loop output**. **Start Opus
loop** repeats until **Loop · stop** fires. **Loop stopped** counts completed
stops. **On local source finished** uses the supplied WebM file connection.
**Read-only write rejected** and **Parent path rejected** deliberately log
errors and should each increment their rejection counter.

For the HTTP group, serve the fixtures before pressing Run. One local option is

```sh
python -m http.server 18765 --bind 127.0.0.1 --directory app/crates/services/orange-audio/tests/fixtures
```

Fire **Play remote audio** and **Check fixture server**. Expect one completed
remote clip and status 200. The other groups can be exercised without the server.

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

For scripted packaging, use the same graph and function from the CLI.

```sh
orange publish graph.json "Compute power" org.example.math 0.1.0 power power.orangeplug
```

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

este es un cambio
