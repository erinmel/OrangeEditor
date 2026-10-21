# Repository tooling

Run from the repository root with Node.js 24.18.0 and Git:

```sh
node tools/check-docs.mjs
node tools/check-repo-policy.mjs
node tools/check-cargo-metadata.mjs --self-test
node tools/check-cargo-metadata.mjs
cargo deny --locked check advisories sources
```

The documentation check validates local file links, fences, JSON syntax, requirement/decision/fixture references, roadmap coverage, translated definition shapes and repeated example consistency. It does not validate external links, every schema rule or product behavior.

The policy check inspects tracked paths and high-confidence credential patterns without printing matches, and checks Git whitespace. Review the staged diff too; a clean scan is not proof that no secret exists. Run it after staging the files to be committed. Before the first commit it checks staged whitespace; CI checks the current commit. These scripts require no npm dependencies or credentials.

The Cargo metadata policy check validates Cargo's locked resolved graph, derives the single workspace `rust-version` for the MSRV job, enforces exact Bevy `=0.20.0` member requirements and rejects direct or transitive `bevy` / `bevy_*` framework dependencies under known graph-core crates. Its fixture mode covers expected pass/fail cases. It does not ban Bevy from host adapters or equate `bevy_*` utility crate versions with the Bevy engine version.

`cargo-deny` is pinned at 0.20.2 and checks advisories and dependency sources only. The advisory database is fetched or refreshed on each run; a database or registry/network failure fails the job. Vulnerability, unmaintained and unsound advisories are errors under cargo-deny's defaults; yanked versions are warnings in this configuration. The source check rejects unrecognized registries and Git sources while allowing crates.io. This is not a license compatibility audit; the project license and allowed third-party SPDX expressions remain unselected.

The [CI workflow](../.github/workflows/checks.yml) is configured for pushes to `main`, every PR, manual dispatch and weekly schedule. GitHub begins scheduled runs after the workflow is merged to the default branch. Stable statuses are `docs`, `policy` and `rust`. Formatting, Clippy and rustdoc run in `rust-quality`; the locked workspace tests run on Linux x64 (`ubuntu-24.04`), Windows x64 (`windows-2022`) and macOS arm64 (`macos-15`). The MSRV test derives the version from workspace Cargo metadata. Dependency security checks run pinned cargo-deny 0.20.2. The `rust` aggregate runs after quality, metadata policy, MSRV, security and all OS test jobs; any failure, cancellation or unexpected skip fails the aggregate. Matrix fail-fast is disabled so each OS reports its result; each job has a timeout. Dependabot is configured to propose weekly Cargo and GitHub Actions updates after the configuration is merged.

Permissions remain read-only, checkout credentials are not persisted, and action references remain pinned to full commits. There is no deployment workflow yet: an application executable, packaging contract and release qualification belong to later delivery work. Passing these core tests establishes no Bevy, SDK, plugin-host or hardware capability.

The documentation traversal skips only ignored `.artifacts/local/` scratch worktrees; other tracked `.artifacts/` planning context remains in scope. This prevents temporary checkout documentation from being mistaken for repository documentation.
