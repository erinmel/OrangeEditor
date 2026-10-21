# OrangeEditor

A Rust visual programming project for typed calculations, state, events and services, with planned desktop and headless execution and installable expansion packages.

The working code is [orange-model](app/crates/orange-model): graph documents, transactions/undo, indexes, signatures, copy/paste and typed values. The compiler, executor, desktop app and package host are still to be implemented. Next is **types and conversions (WU-00/00c)**. OrangeEditor is a foundation: the core has no UI, window or engine dependency, and the editor UI will be a separate egui library ([ADR-02, ADR-21](docs/08-decisions.md)).

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
- [Delivery plan](docs/07-delivery-plan.md) and [decisions](docs/08-decisions.md): implementation order and selected choices.
- [Graph language](docs/11-graph-language.md), [data structures](docs/12-graph-data-structures.md) and [localization](docs/13-internationalization.md): normative core contracts.
- [Requirements](docs/01-product-requirements.md), [architecture](docs/02-architecture.md), [expansion packages](docs/04-expansion-plugins.md) and [source layout](docs/15-monorepo-layout.md): product scope and boundaries.
- [Data/execution](docs/03-data-and-execution.md), [services/platforms](docs/05-services-and-platforms.md), [feasibility probes](docs/09-feasibility-probes.md) and [tooling evidence](docs/10-tooling-and-sources.md): detailed contracts and recorded limitations.

Source: [erinmel/OrangeEditor](https://github.com/erinmel/OrangeEditor). Measured results are recorded separately from proposed performance targets and unimplemented capabilities.
