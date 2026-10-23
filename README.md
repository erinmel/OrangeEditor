# OrangeEditor

A reusable Rust node editor foundation: an embeddable graph core, a separate usable and extensible egui editor, and a basic application with desktop and headless execution. The selected scope is typed logic/math, variables/enums, loops/time/branching, reusable graphs and import/export, configurable triggers, files, HTTP/HTTPS/WebSocket, and basic audio playback/input/level events. MQTT and frequency/pitch analysis are conditional extensions. Specialized integrations such as OBS, Twitch and browser-canvas servers are outside this delivery; installed plugins can add them through public contracts (ADR-22).

The working code is [orange-model](app/crates/orange-model): graph documents, transactions/undo, indexes, signatures, copy/paste, typed values, types with generic inference and converter planning, and read-only connection, provider-resolution, input and graph-local function analysis. Get/Set and `core/update-variable` Add/Sub edges use fresh declaration-owned ports; Update accepts local/run variables only and requires exact Add/Sub type evidence. Other Update operations, shared-variable and call/interface ports, definition/config legality, regions, availability, cycles and state-conflict checks remain incomplete. A successful `ValidationReport` covers only the passes it ran. `ValidationAnalysis` exposes actual and missing validation checks, binds results to exact document state and registry generation, and reports compile readiness only when all required checks exist; readiness remains false while validation is partial. The compiler and executor remain to be implemented. Current work prioritizes a tested headless core before concrete host services and editor integration. OrangeEditor is a foundation: the core has no UI, window or engine dependency, and the editor UI will be a separate egui library ([ADR-02, ADR-21](docs/08-decisions.md)).

## Development

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

Source: [erinmel/OrangeEditor](https://github.com/erinmel/OrangeEditor). Measured results are recorded separately from proposed performance targets and unimplemented capabilities.
