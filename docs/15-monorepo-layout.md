# Monorepo layout and ownership

Status: **selected source layout**, 2026-10-20. The user selected one public GitHub repository, `erinmel/OrangeEditor`, containing the application and authored expansion plugins. A directory exists only once it has content: today that is `app/crates/orange-model`, `tools/` and `.github/`. The other directories are reserved names; no SDK or plugin implementation is implied. Repository bootstrap evidence is recorded separately in 10 and 14.

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
| `app/apps`, `app/crates` | Future root Cargo workspace for host application members; graph core remains independent of Bevy |
| `plugins/<name>` | Independently built/package-versioned expansion; Rust guest/worker workspaces are excluded from root host defaults and use their own lock/toolchain as needed |
| `sdk` | Public versioned WIT/wire/schema/packaging contract, exported deliberately from the host design; not a second private runtime API |
| `examples` | Small version-locked workflows and SDK usage with expected results |
| `tools`, `.github` | Repository checks and automation; no product runtime responsibilities |

The repository bootstrap added no Cargo manifest. WU-00/00b.1 added the root workspace (`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`) with its first member `app/crates/orange-model`; `plugins` is excluded. WU-01 still verifies the Bevy pins before any Bevy member is added, and WU-00 grows the host core incrementally. Once manifests exist, commit the root application lockfile and each independently built plugin lockfile. Code location does not turn an installable Wasm/worker expansion into an in-process Bevy developer plugin. Privileged statically linked base kernels remain `app/crates/orange-bundled`, under the exception in 04.

## Change and release rules

- Use the topic-branch/PR rules in [14](14-git-workflow.md). A change affecting a shared SDK contract includes host/plugin conformance evidence and relevant FR/NFR/ADR updates in the same work unit.
- Root/application source changes run the applicable host matrix. SDK/schema/contract changes run affected plugin compatibility checks too. A plugin-only implementation change runs its packaging/tests plus SDK compatibility; it does not imply rebuilding/releasing the host.
- Application tags use `v<version>` per 14. SDK-only releases use `sdk/v<version>` and plugin releases `plugins/<name>/v<version>` when those artifacts exist. Each tag identifies an exact repository commit and its own artifact/version evidence; unrelated package versions do not advance automatically.
- Public catalogues, examples, manifests and traces contain no credential material. Machine IDE state, output directories and local configuration are ignored.
- Maintainer: `erinmel`, with solo-review mode initially. License selection and artifact-signing identities remain future distribution decisions; this bootstrap does not choose a license for the user.

Architecture ownership is in [02](02-architecture.md), package isolation/lifecycle in [04](04-expansion-plugins.md), and delivery gates in [07](07-delivery-plan.md). This document changes source placement, not graph semantics, public package identity or execution rules. ADR-19 records the decision under NFR-14.
