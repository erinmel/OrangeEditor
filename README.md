# OrangeEditor

OrangeEditor has a working headless Rust graph core and CLI. The CLI runs multiple
flows, loads, lists, removes and updates expansion packages, and includes HTTP/HTTPS
requests, WebSocket text send and receive over WS or WSS, WAV and FLAC playback on a
chosen audio device with loop, pause, seek and volume, and a trigger on input level. The separate egui desktop editor is planned.
See [CLI usage](#development), [service behavior and limits](docs/05-services-and-platforms.md#connection-manager)
and [verification results](docs/10-tooling-and-sources.md).

Bounded stream bridges are implemented: hosts can push typed, sequenced items
through explicit finite queues, and graph `core/tee` splits one affine stream
into two consumers with declared backpressure, drop-oldest, drop-newest or fault
overflow. Closing the last branch closes its upstream once. Portable replay, stateful authoring preview and physical performance qualification remain open. See [stream contracts](docs/03-data-and-execution.md).

Runs can be paused and resumed, and `RunDriver` gives many host threads one bounded
mailbox into a run. Installed packages can implement pure, action and async nodes, with
bytes and dense numeric lists crossing the plugin boundary in binary. Trusted native services can supply package event sources. Installed Wasm guests still need host service imports and resource support. See
[execution](docs/03-data-and-execution.md) and [expansion packages](docs/04-expansion-plugins.md).

A reusable Rust node editor foundation: an embeddable graph core, a separate usable and extensible egui editor, and a basic application with desktop and headless execution. The selected scope is typed logic/math, variables/enums, loops/time/branching, reusable graphs and import/export, configurable triggers, files, HTTP/HTTPS/WebSocket, and basic audio playback/input/level events. MQTT and frequency/pitch analysis are conditional extensions. Specialized integrations such as OBS, Twitch and browser-canvas servers are outside this delivery; installed plugins can add them through public contracts (ADR-22).

The [orange-model](app/crates/orange-model) crate provides graph documents, transactions/undo, indexes, signatures, copy/paste, typed values, types with generic inference and converter planning, and read-only domain, connection, provider-resolution, input, graph-local function and port-level data-cycle analysis. Get/Set and `core/update-variable` Add/Sub/Min/Max/Compare-and-set/And/Or/Xor/Append edges use fresh declaration-owned ports; Update accepts local/run variables only, requiring exact numeric-operation evidence for Add/Sub, exact Ord evidence for Min/Max, exact Eq evidence for Compare-and-set, and exact built-in bool/integer types for And/Or/Xor. Append accepts exact string/bytes/list concatenation. Workspace-only Set-shared/Update-shared execute through the host workspace actor with authenticated asynchronous completion. Call-site type interfaces and generic authoring are separate from execution metadata. For the supported control vocabulary, validation includes accepted calls, body/interface conformance, definition/config legality, regions, value availability, control cycles and parallel-write conflicts. Variable and event declaration types are checked even when unused. Combined analysis applies configurable admission quotas and cooperative cancellation between passes; standalone pass APIs remain caller-controlled. Semantic validation can report readiness to begin compiler lowering, while constant folding and constant-error preservation remain a separate compiler-owned prerequisite for publishing a completed plan. Combined analysis first accepts only absent or explicit control domains; unsupported or malformed domain metadata is preserved and blocks control analysis. A successful `ValidationReport` covers only the passes it ran. `ValidationAnalysis` exposes actual and missing validation checks, binds results to semantic document state and registry generation (layout edits preserve freshness), and reports compile readiness only when all required checks exist; general compilation readiness remains false; the compiler proves its supported pure/seeded-entropy root-data scope. Checked data plans and a bounded structured-control plan/runtime are implemented; remaining control/function families and general services stay open. OrangeEditor is a foundation: the core has no UI, window or engine dependency, and the editor UI will be a separate egui library ([ADR-02, ADR-21](docs/08-decisions.md)).

The first shipped scalar kernel slice is [orange-bundled](app/crates/orange-bundled): native Add/Subtract/Multiply/Divide for i32/i64/u32/u64/f32/f64 and boolean Not, with exact provider binding, explicit numeric policies and registry snapshot retention. It also executes the four registered Core numeric conversions. The new [orange-compiler](app/crates/orange-compiler) consumes fresh semantic analysis to build immutable typed plans for pure scalar/JSON/collection root exports, including constant Select/And/Or short-circuit demand, folds shared dependencies once, and retains observable constant faults and source maps. PurePlan::evaluate exposes freshness-checked borrowed results and original graph faults, with export limits and cancellation; it reads folded values without rerunning operations. A test-only independent demand evaluator now checks bounded integer/float/boolean graphs and all four Core numeric conversions against the public compiled-export API, including typed loss metadata. The root-data runtime also evaluates seeded entropy and dynamic lazy forms in explicit epochs, checked against a source-graph reference evaluator. Structured headless execution includes all four Fork joins; graph-local Pure calls accept epoch-dependent inputs, with owned action/async call frames and Return cleanup.

The bundled integer vocabulary also includes remainder and Euclidean modulo,
negate/absolute, min/max/clamp, bitwise logic and shifts for all four integer widths.
Exact overflow/count policies and safe faults apply in folded and running graphs;
the finite catalog is in [12 §4.2](docs/12-graph-data-structures.md).
Float math adds square root, exponential, logarithms, power, rounding and float
negate/absolute/min/max/clamp/remainder for f32/f64 under finite or IEEE policy
([12 §4.3](docs/12-graph-data-structures.md)). Trigonometry, IEEE comparisons,
total order, NaN/infinity checks and boolean XOR are in the same catalog. Every
pair of numeric types has an explicit Core converter with checked, lossy or
rounding semantics ([11 §6](docs/11-graph-language.md)). Invariant text parse/display
and strict or replacing UTF-8 decoding are explicit converters too. The
`org.orange.text` package adds lengths, search, scalar/byte substring, replace,
trim, case mapping, split and join ([12 §4.4](docs/12-graph-data-structures.md)).
The `org.orange.collections` package adds generic list, map and option nodes
([12 §4.5](docs/12-graph-data-structures.md)) and record/enum construction and
projection ([12 §4.6](docs/12-graph-data-structures.md)). `org.orange.json` reads JSON
values with exact source text ([12 §4.7](docs/12-graph-data-structures.md)).

The compiler also supports nested options over its scalar/JSON types and constant Coalesce: only outer None demands fallback; present values, JSON null and upstream faults remain distinct. PlanType represents recursive options and exported type metadata is borrowed. Try captures pure input-cone graph faults into typed Result values with original producer identity; nested successes preserve Ok(Err) distinctly. DataActivation executes the supported pure/seeded-entropy root-data subset, including dynamic Select/And/Or/Coalesce/Try. The control runtime admits a bounded subset including Pure and seeded-volatile calls in data demands; remaining control families, clock readers and general services stay open. Core numeric and invariant text converters execute with exact provider choices and saved policies, typed faults and explicit loss metadata; they introduce no implicit conversions.

Reference-only recording and replay now reproduce a sealed in-memory run history using its pinned plan, seed, clock and effect outcomes. Replay invokes no Action callback and exposes no request to an external executor. Hosts supply the payload archive explicitly; raw values are excluded from journal entries. Portable log formats and live resource replay remain open.

Pure authoring preview now publishes checked immutable exports, reuses unchanged typed calculations and keeps existing runs and providers pinned. It accepts pure cones in mixed control/service graphs without executing those services; cache limits affect reuse, while normal compiler quotas and freshness still apply.

A host workspace value actor now serializes typed Set/Update/CAS and publishes immutable versioned snapshots with bounded lifetime accounting. Graph Get and asynchronous Set-shared/Update-shared now use explicit workspace attachment, single-use commands and authenticated receipts; successful completion installs the snapshot before continuing. A bounded typed workspace bus connects Emit to other runs' On Event/Await Event through explicit host driving, with causal metadata and lifetime-charged deliveries. Selected persistent values now use [portable bounded checkpoints](docs/20-value-checkpoints.md) to bootstrap fresh actors; durable file storage remains host work.

## Development

The [orange-host](app/crates/orange-host) crate is the session a front end attaches to: it installs packages, runs any number of documents at once as independent flows (one run per thread, on real time), executes package effects on worker threads and reports what happens on one bounded event channel. It also lists every usable node with its display text and can report a flow's variable values and the values on its wires as they change. Native plugins are written against [orange-sdk](app/crates/orange-sdk), as the base network package [orange-net](app/crates/orange-net) is. The `orange` command ([orange-cli](app/crates/orange-cli)) is its terminal front end:

```console
cargo run -p orange-cli --locked -- check app/crates/orange-cli/graphs/set-on-trigger.json
cargo run -p orange-cli --locked -- run app/crates/orange-cli/graphs/set-on-trigger.json
```

A front end attaches through `orange-host` alone; [examples/attach.rs](app/crates/orange-host/examples/attach.rs) is a complete one in a single file (`cargo run -p orange-host --example attach --locked`). `orange nodes` lists every usable node with its name and category, `orange packages` the built-in and stored packages, `orange remove <id> <version>` takes one out of the store, and `orange updates [--apply]` reads each stored package's update feed and installs newer releases after checking their digest. [orange-audio](app/crates/orange-audio) is the base audio package: output and input device connections (`orange devices` lists the names), play, stop and input-level nodes and an entry fired when the input level reaches a threshold; `--no-audio` leaves it out. Linux builds need the ALSA development headers (`libasound2-dev`).

A Rust plugin can also be a dynamic library: [orange-plugin-example](app/crates/orange-plugin-example) is one (`crate-type = ["cdylib"]`, `orange-sdk`, one `export_plugin!` line), and `orange --plugins <dir>` loads every library in a directory at start through [orange-native](app/crates/orange-native). Such a plugin must be built with the application's compiler and lockfile, is not sandboxed, and runs at your own risk.

An installed package's code reaches outside itself only with a grant given when it is installed: `--grant-files` (its own data directory), `--grant-http` (HTTPS requests) and `--grant-log`. A package can name the key that signs its update feed; `orange updates` then refuses a feed that key did not sign, and it also updates native plugin libraries that name a feed.

Files, credentials and state: the built-in [file package](app/crates/orange-files) reads, writes, lists and watches files inside a folder a graph declares. `orange secret set <name>` keeps a credential in the operating system's store ([orange-vault](app/crates/orange-vault)); a connection names it as `credential://<name>` in its `bearer`, `basic` or `headers` settings, so graph files hold no secrets. HTTP and WebSocket connections send those headers, and binary request, response and message nodes carry bytes such as protobuf or an encrypted payload. Variables a graph declares `persistent` are stored in `--state <dir>` and restored at the next run, and `set <flow> <variable-id> <literal>` changes one while it runs. `orange publish` turns a function of a graph into a package node, and `orange export` / `import` move a graph together with the packages it uses.

`--store <dir>` keeps installed packages in a directory, so later commands with the same `--store` have them without `--package`. While `run` is active, type `trigger 1 10000000-0000-4000-8000-000000000001`, `pause 1`, `resume 1`, `stop 1` or `quit`. Several graph files start several flows; `--package <file.orangeplug>` installs a package first. A package node can be a graph instead of code: [packages/remember](app/crates/orange-cli/packages/remember) is one, built with `orange pack <dir> <out.orangeplug>` and used by [remember-on-trigger.json](app/crates/orange-cli/graphs/remember-on-trigger.json) with `--package`. Documents declare connections that nodes use by reference, private to one diagram or shared across diagrams with `--share <id> <adapter> <settings.json>`. [orange-net](app/crates/orange-net) is the base network package built into the CLI: HTTP and WebSocket connection adapters, a request node, a send node and an entry that fires on each received WebSocket message, with TLS. A running graph can be edited live: `Host::update`, or `reload <flow>` in the CLI, replaces it with the edited file, keeps its variables and connections, and refuses an invalid edit without disturbing it. Flows share variables the application declares (`Host::share_variables`). `orange packages --store <dir>` lists the base, native and stored packages; `orange disable <id> --store <dir>` leaves one out from the next command on, and `enable` brings it back.

The [headless control example](app/crates/orange-bundled/examples/headless_control.rs) compiles a saved graph, accumulates a run variable through Count, waits on virtual time and selects a Branch. `Run::pump` budgets both control and resumable data work; `advance_to` never executes graph work inline. Local/run Get/Set/Update, Gate/DoOnce/FlipFlop/Register, scalar Switch, Match-result, Sequence, Count/While/For-each, Break/Continue, Wait/Yield and Fork all/any/race/detach are supported. Repeat/Fold/Map/Filter data zones also execute. Run-scoped On Event and Emit support typed bounded Queue, Drop, Replace and Parallel handlers with causal-depth limits. Drop skips while a delivery is pending or its root remains live; Parallel owns at most its checked `max_parallel` roots, with separate pending capacity. Replace retains one latest pending delivery and cancels the old root once, after the emitting strand returns to actor ownership; the latest starts only after all old descendants and bounded cleanup retire. Its latest-only rule supersedes configured pending overflow, while ordinary admission quotas still apply without crediting the live old payload. Graceful stop starts On Stop and drains accepted deliveries to a deadline; immediate stop unregisters Await Event waiters and retires pending event work through bounded cleanup. Typed run-scope Await Event resumes the existing strand on the next matching accepted Emit, with optional virtual timeout, cancellation and budgeted payload publication. Hosts register and revoke run-owned event sources and admit typed, sequenced input through atomic delivery checks. Root `core/on-manual` entries are registered and run through `Run::trigger` with the same bounded handler admission, and root `core/on-timer`/`core/on-timeout` entries admit ticks on the virtual clock with exact missed-tick policies; `core/now` reads that clock once per epoch. Root `core/on-variable-changed` entries receive one equal-aware notification per changed run variable at each segment end. Runtime Watch now observes supported pure cones over initialized run variables at segment boundaries. Trusted Action/Async effect bindings execute through a separate checked interface, and host and graph pull streams support affine Next Item, bounded Collect, For Each Item and cleanup-safe Close Stream. Iterators wait for body descendants before pulling again; Break/Return/Race/stop close unfinished producers. Direct Async stream results and function ownership transfers are checked before publication. Portable replay, stateful authoring preview, custom package equality dispatch and concrete services remain open. A run owns resources through authenticated generation-checked handle values. Direct handles cross trusted effects and Action/Async functions; acquisition belongs to its dynamic region, Return transfers returned ownership, and cleanup-safe Close Handle or host revocation invalidates aliases. Region retirement closes unreturned resources exactly once. Concrete resource services remain open. Verification and limits are recorded in [10](docs/10-tooling-and-sources.md).

```sh
cargo run -p orange-bundled --example headless_control --locked
```

The [Fork example](app/crates/orange-bundled/examples/headless_fork.rs) compares `all`, `any`, `race` and `detach` using two virtual waits. It shows when a join publishes completion and when all owned work retires:

```sh
cargo run -p orange-bundled --example headless_fork --locked
```

Saved graphs use versioned compact JSON for editing and interchange. Compilation produces typed Rust values and dense slot indices in memory, following the four representations in [12](docs/12-graph-data-structures.md). Execution does not require repeatedly parsing the saved JSON. Binary persistence and compiled-plan caching are separate choices; the measured format comparison and JSON import improvement are recorded in [10](docs/10-tooling-and-sources.md).

The [headless authoring example](app/crates/orange-bundled/examples/headless_authoring.rs) composes editing, bounded undo/redo, retained analysis, compilation and execution on one worker. Capacity-one channels carry ordered commands and metadata acknowledgements; the document/session are not cloned for each edit. Active run, plan and inspection share one identity; terminal results stay separate. The workflow edits during a real virtual wait, resumes the pinned old plan, stops it, and verifies that a replacement begins with fresh state. Recoverable editing errors preserve the worker; explicit graceful shutdown runs On Stop, while terminal transport/fatal errors use bounded immediate cleanup. Its synchronous caller is a deterministic demonstration, not a UI adapter or a latency guarantee.

```sh
cargo run -p orange-bundled --example headless_authoring --locked
cargo test -p orange-bundled --example headless_authoring --locked
```

Run from the repository root with the toolchain in [rust-toolchain.toml](rust-toolchain.toml) and the checked-in Cargo.lock:

```sh
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Repository helpers use Node.js 24.18.0:

```sh
node tools/check-cargo-metadata.mjs
node --test tools/release.test.mjs
node tools/release.mjs check
```

The metadata check enforces the core dependency boundary. The release configuration is intentionally disabled until an executable and qualified packaging exist; its [contract](tools/release/README.md) describes activation. [Validation policy](docs/06-quality-and-validation.md) owns CI, security and platform checks. Use focused Rust tests for real behavior and regressions as implementation lands.

## Project references

- [Contributing](CONTRIBUTING.md) and [agent instructions](AGENTS.md): work scope, checks and the [Git workflow](docs/14-git-workflow.md). Topic PRs target `dev`; `main` receives promotions.
- [Decisions](docs/08-decisions.md): selected product and engineering choices. Local delivery plans and WU task packets live in ignored `.artifacts/` and are not committed.
- [Graph language](docs/11-graph-language.md), [data structures](docs/12-graph-data-structures.md) and [localization](docs/13-internationalization.md): normative core contracts.
- [Requirements](docs/01-product-requirements.md), [architecture](docs/02-architecture.md), [expansion packages](docs/04-expansion-plugins.md) and [source layout](docs/15-monorepo-layout.md): product scope and boundaries.
- [Data/execution](docs/03-data-and-execution.md), [services/platforms](docs/05-services-and-platforms.md), [feasibility probes](docs/09-feasibility-probes.md) and [tooling evidence](docs/10-tooling-and-sources.md): detailed contracts and recorded limitations.
- [Incremental validation](docs/16-incremental-validation.md): implemented analysis session, bounded local caching and the remaining V12/NFR-15 design.

Source: [erinmel/OrangeEditor](https://github.com/erinmel/OrangeEditor). Measured results are recorded separately from proposed performance targets and unimplemented capabilities.

The [orange-runtime](app/crates/orange-runtime) crate evaluates checked root data plans with versioned deterministic entropy streams and activation-owned consumption epochs. Bound node reads share a sample within an epoch; skipped nodes do not advance their stream. The host-driven Run actor also schedules the supported control subset with virtual time and bounded lifecycle management.

Fresh registered definitions carry typed volatility/determinism metadata. Publication enforces pure determinism, seeded package entropy and host-owned state/clock classification before future execution lowering.

MathPackage also registers org.orange.math/random-u64 with a separate trusted entropy binding. It draws from a host-owned stream and cannot run through the pure folder; DataActivation now evaluates the checked data plan in explicit epochs.

The checked DataPlan compiler lowers pure/seeded-entropy root data exports to dense typed instructions, folding shared constant islands while leaving entropy unevaluated. Static lazy arms are pruned before binding. The existing PurePlan API remains; DataActivation executes these plans. Independent source-graph comparisons cover bounded seeded arithmetic and dynamic lazy/Option/Try graphs across epochs. Concrete services, package function bodies, asynchronous guests and executable package lifecycle integration remain open.

The control runtime snapshots only each consumer's potential inputs and publishes them on actual lazy demand. Pending Set pulls retain read-modify-write ownership across host pump slices. Shutdown frames use a separately bounded startup reserve (RunLimits::max_stop_activations), allowing graceful stop at ordinary activation capacity. These guarantees cover the supported control subset; broader core work remains open.

For repeated authoring, plan_edit_indexed and plan_connection_indexed reuse a retained DocIndex and journaled trial edits while preserving the existing read-only planning APIs. Validation shares scoped port projections; runtime activations share immutable preparation while retaining separate values, entropy counters, limits and cancellation. Provider locks and public resolver contracts are unchanged.

`AnalysisSession` keeps validation current through committed edits and undo/redo, with borrowed diagnostic/signature views and a retained index for trial planning. Layout edits preserve semantic evidence without validation work. Semantic edits reuse unchanged configuration and declaration-type results; admission, other validation passes and result publication still inspect the wider document. Work counters distinguish cache computations/hits from input sizes. This is V12a plus bounded V12b work, not completed incremental validation or NFR-15 qualification; no UI dependency or saved-format change is introduced.

Graph-local pure functions now compile through `compile_pure`, including nested interface-visible generic calls, isolated memoization, typed results and original-node faults. Potential expansion/depth is admitted before callbacks. Pure and seeded-volatile calls also lower through `compile_data` and the data demands of `compile_control`, with independent call slots/memos, strict ordered arguments/results and resumable host pumping. Volatile calls can also read checked host state, including run variables and independent read-only call locals. Remaining callable type/execution forms and package function bodies remain future work. See [the pure-call contract](docs/11-graph-language.md#pure-call-execution-evidence-2026-10-26-utc).

Control dispatch now includes exact Core bool/integer/string Switch keys with checked case tables and Match-result with outcome-specific typed projections. String values work through existing pure/data plans and quotas. Package enum tag dispatch is implemented; record/custom equality keys remain pending.

State controls now execute Gate commands, resettable Do Once, alternating Flip Flop and typed run/activation Register cells. Compiler-resolved destination commands preserve named exec inputs across loops, waits and cancellation; charged state publication and Register mutation ownership survive host pump slices. State inspection reports write intents. Event delivery and Action/Async function ownership are described above.

Synchronous core/update-variable now executes all nine operations over the documented supported Core types, with atomic local/run mutation, strict one-epoch operands and typed old/new/CAS results. It shares scalar arithmetic policies with bundled math. Package-defined constraint kernels remain open; workspace writes and change/Watch delivery use the actor paths described above.

Executable values now include bytes, timestamps, lists and ordered maps. For-each captures immutable list/map input, preserves source order across waits and keeps item/index context during cancellation cleanup. Typed checks share bounded nominal charging; string/bytes/list Append, timestamp Min/Max and supported container CAS use the same atomic Update path. Concrete service adapters and package-defined constraint kernels remain open.

Volatile graph-local functions can read state through compiler-owned host-input slots. Run binds called Get nodes to the current consumption-epoch snapshot; read-only function locals use their declaration initializer independently of the caller activation. Original Get faults, strict arguments, lazy demand and per-call memoization remain intact.

Graph-local Action/Async calls now compile to scoped control/data plans and actor-owned invocation frames. Arguments and Return results are strict ordered collections; locals initialize for each invocation, while run variables are shared. Wait/Yield and Fork retain region ownership, and Return cancels detached work and finishes bounded cleanup before publishing results or resuming the caller. Return coverage follows sparse Sequence continuations and rejects silent suppression paths without results. Package bodies, erased generics and remaining callable/core gates stay open.

Data plans execute Repeat, Fold, Map and Filter as resumable, fuel-bounded work, with zone state captured per outer epoch. Stream iterators execute through the affine ownership APIs described above.
