# Monorepo layout and ownership

Status: **selected source layout**, 2026-10-20. The user selected one public GitHub repository, `erinmel/OrangeEditor`, containing the application and authored expansion plugins. A directory exists only once it has content: current implemented core members are `orange-model`, `orange-compiler`, `orange-bundled` and the supported headless `orange-runtime` subset, alongside `tools/` and `.github/`. The other directories are reserved names; no SDK or plugin implementation is implied. Repository bootstrap evidence is recorded separately in 10 and 14.

```text
OrangeEditor/
  app/
    apps/          desktop and CLI entrypoints
    crates/        model, compiler, runtime, host, editor and services
  plugins/         one directory per installable expansion
  sdk/             versioned public plugin interfaces/schemas/bindings
  examples/        validated workflow and SDK examples
  docs/            requirements, architecture, decisions and evidence
  tools/           CI support scripts
  .github/         PR template and CI workflows
```

## Why one repository

The application and plugins share node/type, package, error, localization and lifecycle contracts. One repository lets a contract change include its SDK update, host adapter, example plugin and compatibility tests in the same reviewed PR. It also provides one issue/roadmap location and one reproducible commit identifying the complete source combination. Separate folders keep ownership and dependencies visible.

The tradeoff is shared repository access/history and a larger CI change surface. Keep work scoped, use affected-component checks and preserve package release boundaries. Splitting a plugin into its own repository later remains possible because it uses published SDK contracts rather than private host imports. A monorepo is a source organization decision, not a replacement for installable packages or exact running-provider locks.

## Boundaries and future builds

| Area | Boundary |
|---|---|
| `app/apps`, `app/crates` | Root Cargo workspace members for the host application; graph core remains independent of UI and engines. `orange-model` groups its source by layer: `id` and `type_term`, `document/`, `catalog/` (node contracts, providers and the registry snapshot), `value/`, `edit/`, then `index`, `signature` and `paste` |
| `plugins/<name>` | Independently built/package-versioned expansion; Rust guest/worker workspaces are excluded from root host defaults and use their own lock/toolchain as needed |
| `sdk` | Public versioned WIT/wire/schema/packaging contract, exported deliberately from the host design; not a second private runtime API |
| `examples` | Small version-locked workflows and SDK usage with expected results |
| `tools`, `.github` | Repository checks and automation; no product runtime responsibilities |

The repository bootstrap added no Cargo manifest. WU-00/00b.1 added the root workspace (`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`) with its first member `app/crates/orange-model`; `plugins` is excluded. WU-01 verifies the UI pins before any UI member is added, and WU-00 grows the host core incrementally. Once manifests exist, commit the root application lockfile and each independently built plugin lockfile. Code location does not turn an installable Wasm/worker expansion into a build-time crate dependency. Privileged statically linked base kernels remain `app/crates/orange-bundled`, under the exception in 04.

## Change and release rules

- Use the topic-branch/PR rules in [14](14-git-workflow.md). A change affecting a shared SDK contract includes host/plugin conformance evidence and relevant FR/NFR/ADR updates in the same work unit.
- Root/application source changes run the applicable host matrix. SDK/schema/contract changes run affected plugin compatibility checks too. A plugin-only implementation change runs its packaging/tests plus SDK compatibility; it does not imply rebuilding/releasing the host.
- Application tags use `v<version>` per 14. SDK-only releases use `sdk/v<version>` and plugin releases `plugins/<name>/v<version>` when those artifacts exist. Each tag identifies an exact repository commit and its own artifact/version evidence; unrelated package versions do not advance automatically.
- Public catalogues, examples, manifests and traces contain no credential material. Machine IDE state, output directories and local configuration are ignored.
- Maintainer: `erinmel`, with solo-review mode initially. License selection and artifact-signing identities remain future distribution decisions; this bootstrap does not choose a license for the user.

Architecture ownership is in [02](02-architecture.md) and package isolation/lifecycle in [04](04-expansion-plugins.md). Delivery sequencing and WU task packets are local working artifacts under ignored `.artifacts/`. This document changes source placement, not graph semantics, public package identity or execution rules. ADR-19 records the decision under NFR-14.

N1a adds the second root workspace member, app/crates/orange-bundled, for ordinary product math definitions and privileged native scalar kernels. At that checkpoint its only product dependency was orange-model; no UI, engine, host service or installable-package runtime is linked. Workspace exact dependency pins/MSRV/lints apply. Cargo.lock gains the local member entry without changing external package versions. The metadata policy includes this member in direct/transitive UI/engine checks.
E1a adds app/crates/orange-compiler as the third workspace member. Its product dependency is orange-model; orange-bundled now depends on both model and compiler to implement the open trusted resolver, with no reverse compiler-to-bundled dependency. Existing exact external pins/features remain unchanged; Cargo.lock adds only the local compiler and dependency edge. The metadata policy checks all three members for UI/engine isolation. The compiler is currently a restricted pure-export library, not a complete runtime or editor integration.

E2g1 adds app/crates/orange-runtime as the fourth workspace member, initially limited to deterministic entropy/consumption-scope primitives. It depends on orange-model and already-pinned sha2/thiserror, without a compiler/UI/engine/service dependency. Cargo.lock gains only the local runtime package entry; no external version changes. General actor/scheduler/runtime ownership remains later work; the existing metadata policy already includes this core crate.

E2g4c connects orange-runtime to orange-compiler normally for checked DataPlan consumption. orange-bundled has a dev-only runtime dependency for external API integration tests; normal bundled code still has no runtime/UI/service dependency. Lockfile changes add only these local edges, not external versions/features. Runtime currently evaluates synchronous root data with epochs, not general run actors/control/functions.

The four current crates enforce dependency direction: orange-model owns documents/types/registries/editing and validation; orange-compiler depends on model and exposes open trusted kernel-resolution interfaces; orange-runtime depends on model/compiler and executes checked plans; orange-bundled depends on model/compiler and supplies native kernels (runtime is a development dependency for its execution tests/examples). Compiler and runtime have no product dependency on bundled implementations. This lets downstream hosts use the model alone or choose their own providers. A single crate could organize the same responsibilities as modules, but Cargo would no longer enforce these package boundaries. This is source/library organization, distinct from installable expansion packages.

The structured-concurrency cohort keeps the same four crates: model validation and saved graphs, checked Fork lowering in compiler, strand/region execution in runtime, and independent source tests plus headless_fork embedding example in bundled. No scheduler framework, concrete host service, UI dependency or fifth crate is required. Provider override/resolver boundaries are unchanged.

Measurement harness member (2026-10-27): `app/crates/orange-measure` is a fifth, development-only workspace member. It is a binary with `publish = false`, depends on the four core crates and the workspace's pinned `serde_json`, and adds no product dependency or external version. It keeps its own lints so that its single documented `unsafe impl GlobalAlloc` is allowed, while the library crates keep `unsafe_code = "forbid"`. The metadata policy covers it like the other members.
