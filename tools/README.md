# Repository tooling

Run from the repository root with Node.js 24.18.0 and Git:

```sh
node tools/check-docs.mjs
node tools/check-repo-policy.mjs
node tools/check-cargo-metadata.mjs --self-test
node tools/check-cargo-metadata.mjs
cargo deny --locked check advisories sources
```

The documentation check is an optional local maintenance helper, outside the code pipeline. It validates local file links, fences, JSON syntax, requirement/decision/fixture references, roadmap coverage, translated definition shapes and repeated example consistency. It does not validate external links, every schema rule or product behavior.

The generic repository-policy check is also an optional local helper, outside CI. It inspects tracked paths and high-confidence credential patterns without printing matches, and checks Git whitespace. Review the staged diff too; a clean scan is not proof that no secret exists. Run it after staging the files to be committed. Before the first commit it checks staged whitespace; otherwise it checks the current commit. These scripts require no npm dependencies or credentials.

The Cargo metadata policy check validates Cargo's locked resolved graph, derives the single workspace `rust-version` for the MSRV job, enforces exact Bevy `=0.20.0` member requirements and rejects direct or transitive `bevy` / `bevy_*` framework dependencies under known graph-core crates. Its fixture mode covers expected pass/fail cases. It does not ban Bevy from host adapters or equate `bevy_*` utility crate versions with the Bevy engine version.

`cargo-deny` is pinned at 0.20.2 and checks advisories and dependency sources only. The advisory database is fetched or refreshed on each run; a database or registry/network failure fails the job. Vulnerability, unmaintained and unsound advisories are errors under cargo-deny's defaults; yanked versions are warnings in this configuration. The source check rejects unrecognized registries and Git sources while allowing crates.io. This is not a license compatibility audit; the project license and allowed third-party SPDX expressions remain unselected.

The [Rust CI workflow](../.github/workflows/checks.yml) is configured for pushes to `main`, every PR, manual dispatch and weekly schedule. GitHub begins scheduled runs after the workflow is merged to the default branch. The single required status is `rust`. Independent matrix jobs report `rust-format`, `rust-clippy` and `rustdoc`; fail-fast is disabled so a formatting error does not hide lint or API-documentation results. `rust-architecture` checks Cargo metadata and derives MSRV; `rust-msrv` tests that compiler floor. Locked workspace tests run on Linux x64 (`ubuntu-24.04`), Windows x64 (`windows-2022`) and macOS arm64 (`macos-15`). `dependency-security` runs cargo-deny. The `rust` aggregate runs after all code checks and fails for any failure, cancellation or unexpected skip. The OS matrix also disables fail-fast; each job has a timeout. Dependabot proposes weekly Cargo and GitHub Actions updates after merge.

The security job caches only the pinned cargo-deny installation using actions/cache v6.1.0, pinned to its full commit SHA. The exact key includes runner OS/architecture, `rust-toolchain.toml` hash and cargo-deny version. There are no fallback restore keys. Cache hits and misses both verify the installed version and scan against current online advisory data; the advisory database is not cached. Cargo registry/build caches remain deferred pending a material benefit.

Permissions remain read-only, checkout credentials are not persisted, and action references remain pinned to full commits. There is no deployment workflow yet: an application executable, packaging contract and release qualification belong to later delivery work. Passing these core tests establishes no Bevy, SDK, plugin-host or hardware capability.

The documentation traversal skips only ignored `.artifacts/local/` scratch worktrees; other tracked `.artifacts/` planning context remains in scope. This prevents temporary checkout documentation from being mistaken for repository documentation.
