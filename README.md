# OrangeEditor

A reusable Rust node editor foundation: an embeddable graph core, a separate usable and extensible egui editor, and a basic application with desktop and headless execution. The selected scope is typed logic/math, variables/enums, loops/time/branching, reusable graphs and import/export, configurable triggers, files, HTTP/HTTPS/WebSocket, and basic audio playback/input/level events. MQTT and frequency/pitch analysis are conditional extensions. Specialized integrations such as OBS, Twitch and browser-canvas servers are outside this delivery; installed plugins can add them through public contracts (ADR-22).

The [orange-model](app/crates/orange-model) crate provides graph documents, transactions/undo, indexes, signatures, copy/paste, typed values, types with generic inference and converter planning, and read-only domain, connection, provider-resolution, input, graph-local function and port-level data-cycle analysis. Get/Set and `core/update-variable` Add/Sub/Min/Max/Compare-and-set/And/Or/Xor/Append edges use fresh declaration-owned ports; Update accepts local/run variables only, requiring exact numeric-operation evidence for Add/Sub, exact Ord evidence for Min/Max, exact Eq evidence for Compare-and-set, and exact built-in bool/integer types for And/Or/Xor. Append accepts exact string/bytes/list concatenation. Workspace-only Set-shared/Update-shared expose asynchronous contracts; the host actor remains unimplemented. Call-site type interfaces and generic authoring are separate from execution metadata. For the supported control vocabulary, validation includes accepted calls, body/interface conformance, definition/config legality, regions, value availability, control cycles and parallel-write conflicts. Variable and event declaration types are checked even when unused. Combined analysis applies configurable admission quotas and cooperative cancellation between passes; standalone pass APIs remain caller-controlled. Semantic validation can report readiness to begin compiler lowering, while constant folding and constant-error preservation remain a separate compiler-owned prerequisite for publishing a completed plan. Combined analysis first accepts only absent or explicit control domains; unsupported or malformed domain metadata is preserved and blocks control analysis. A successful `ValidationReport` covers only the passes it ran. `ValidationAnalysis` exposes actual and missing validation checks, binds results to semantic document state and registry generation (layout edits preserve freshness), and reports compile readiness only when all required checks exist; general compilation readiness remains false; the compiler proves its supported pure/seeded-entropy root-data scope. Checked data plans and a bounded structured-control plan/runtime are implemented; remaining control/function families and general services stay open. Current work prioritizes a tested headless core before concrete host services and editor integration. OrangeEditor is a foundation: the core has no UI, window or engine dependency, and the editor UI will be a separate egui library ([ADR-02, ADR-21](docs/08-decisions.md)).

The first shipped scalar kernel slice is [orange-bundled](app/crates/orange-bundled): native Add/Subtract/Multiply/Divide for i32/i64/u32/u64/f32/f64 and boolean Not, with exact provider binding, explicit numeric policies and registry snapshot retention. It also executes the four registered Core numeric conversions. The new [orange-compiler](app/crates/orange-compiler) consumes fresh semantic analysis to build immutable typed plans for pure scalar/JSON root exports, including constant Select/And/Or short-circuit demand, folds shared dependencies once, and retains observable constant faults and source maps. PurePlan::evaluate exposes freshness-checked borrowed results and original graph faults, with export limits and cancellation; it reads folded values without rerunning operations. A test-only independent demand evaluator now checks bounded integer/float/boolean graphs and all four Core numeric conversions against the public compiled-export API, including typed loss metadata. The root-data runtime also evaluates seeded entropy and dynamic lazy forms in explicit epochs, checked against a source-graph reference evaluator. Structured headless execution includes all four Fork joins; graph-local Pure calls accept epoch-dependent inputs, while action/async functions remain future work.

The compiler also supports nested options over its scalar/JSON types and constant Coalesce: only outer None demands fallback; present values, JSON null and upstream faults remain distinct. PlanType represents recursive options and exported type metadata is borrowed. Try captures pure input-cone graph faults into typed Result values with original producer identity; nested successes preserve Ok(Err) distinctly. DataActivation executes the supported pure/seeded-entropy root-data subset, including dynamic Select/And/Or/Coalesce/Try. The control runtime admits a bounded subset including Pure and seeded-volatile calls in data demands; action/async functions and remaining control families stay open. The same bundled crate executes the four existing explicit core converters: i32↔i64 and f32↔f64. Narrowing checks range; lossy float conversion returns JSON loss metadata. These preserve exact entry/provider choices and require complete saved policies; they introduce no implicit conversions.

## Development

The [headless control example](app/crates/orange-bundled/examples/headless_control.rs) compiles a saved graph, accumulates a run variable through Count, waits on virtual time and selects a Branch. `Run::pump` budgets both control and resumable data work; `advance_to` never executes graph work inline. Local/run Get/Set, Gate/DoOnce/FlipFlop/Register, scalar Switch, Match-result, Sequence, Count/While, Break/Continue, Wait/Yield and Fork all/any/race/detach are supported. Graceful stop runs On Stop once and drains to a deadline; immediate stop retires waits and permits bounded safe cleanup. Action/async functions, Remaining Switch enum/custom-key families, iterators/zones, Update/shared state, event/reactivity delivery and general resources/services remain open.

```sh
cargo run -p orange-bundled --example headless_control --locked
```

The [Fork example](app/crates/orange-bundled/examples/headless_fork.rs) compares `all`, `any`, `race` and `detach` using two virtual waits. It shows when a join publishes completion and when all owned work retires:

```sh
cargo run -p orange-bundled --example headless_fork --locked
```

Saved graphs use versioned compact JSON for editing and interchange. Compilation produces typed Rust values and dense slot indices in memory, following the four representations in [12](docs/12-graph-data-structures.md). Execution does not require repeatedly parsing the saved JSON. Binary persistence and compiled-plan caching are separate choices; the measured format comparison and JSON import improvement are recorded in [10](docs/10-tooling-and-sources.md).

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

The checked DataPlan compiler lowers pure/seeded-entropy root data exports to dense typed instructions, folding shared constant islands while leaving entropy unevaluated. Static lazy arms are pruned before binding. The existing PurePlan API remains; DataActivation executes these plans. Independent source-graph comparisons cover bounded seeded arithmetic and dynamic lazy/Option/Try graphs across epochs. Remaining control families, event scheduling, resources and service integration remain next.

The control runtime snapshots only each consumer's potential inputs and publishes them on actual lazy demand. Pending Set pulls retain read-modify-write ownership across host pump slices. Shutdown frames use a separately bounded startup reserve (RunLimits::max_stop_activations), allowing graceful stop at ordinary activation capacity. These guarantees cover the supported control subset; broader core work remains open.

For repeated authoring, plan_edit_indexed and plan_connection_indexed reuse a retained DocIndex and journaled trial edits while preserving the existing read-only planning APIs. Validation shares scoped port projections; runtime activations share immutable preparation while retaining separate values, entropy counters, limits and cancellation. Provider locks and public resolver contracts are unchanged.

`AnalysisSession` keeps validation current through committed edits and undo/redo, with borrowed diagnostic/signature views and a retained index for trial planning. Layout edits preserve semantic evidence without validation work. Semantic edits reuse unchanged configuration and declaration-type results; admission, other validation passes and result publication still inspect the wider document. Work counters distinguish cache computations/hits from input sizes. This is V12a plus bounded V12b work, not completed incremental validation or NFR-15 qualification; no UI dependency or saved-format change is introduced.

Graph-local pure functions now compile through `compile_pure`, including nested interface-visible generic calls, isolated memoization, typed results and original-node faults. Potential expansion/depth is admitted before callbacks. Pure and seeded-volatile calls also lower through `compile_data` and the data demands of `compile_control`, with independent call slots/memos, strict ordered arguments/results and resumable host pumping. Other volatile calls (state/clock reads), action/async calls, erased generic parameters, package bodies and Return cleanup remain future work. See [the pure-call contract](docs/11-graph-language.md#pure-call-execution-evidence-2026-10-26-utc).

Control dispatch now includes exact Core bool/integer/string Switch keys with checked case tables and Match-result with outcome-specific typed projections. String values work through existing pure/data plans and quotas. Enum/custom-key dispatch and broader types remain pending.

State controls now execute Gate commands, resettable Do Once, alternating Flip Flop and typed run/activation Register cells. Compiler-resolved destination commands preserve named exec inputs across loops, waits and cancellation; charged state publication and Register mutation ownership survive host pump slices. State inspection reports write intents; event delivery and action/async function frames remain future work.
