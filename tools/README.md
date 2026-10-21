# Repository tooling

`check-cargo-metadata.mjs` supports the code pipeline. Run it from the repository root with Node.js 24.18.0:

```sh
node tools/check-cargo-metadata.mjs --self-test
node tools/check-cargo-metadata.mjs
node tools/check-cargo-metadata.mjs --msrv
cargo deny --locked check advisories sources
```

The Cargo metadata policy check validates Cargo's locked resolved graph, derives the single workspace `rust-version` for the MSRV job, enforces exact Bevy `=0.20.0` member requirements and rejects direct or transitive `bevy` / `bevy_*` framework dependencies under known graph-core crates. Its fixture mode covers expected pass/fail cases. It does not ban Bevy from host adapters or equate `bevy_*` utility crate versions with the Bevy engine version.

`cargo-deny` is pinned at 0.20.2 and checks advisories and dependency sources only. The advisory database is fetched or refreshed on each run; a database or registry/network failure fails the job. Vulnerability, unmaintained and unsound advisories are errors under cargo-deny's defaults; yanked versions are warnings in this configuration. The source check rejects unrecognized registries and Git sources while allowing crates.io. This is not a license compatibility audit; the project license and allowed third-party SPDX expressions remain unselected.

The [Rust CI workflow](../.github/workflows/checks.yml) runs on pushes to `main`, every PR, manual dispatch and a weekly schedule; scheduled runs start once the workflow is on the default branch. The single required status is `rust`. Independent matrix jobs report `rust-format`, `rust-clippy` and `rustdoc`, with fail-fast disabled so a formatting error does not hide lint or API-documentation results. `rust-architecture` runs the metadata check and derives the MSRV; `rust-msrv` tests on that compiler. Locked workspace tests run on Linux x64 (`ubuntu-24.04`), Windows x64 (`windows-2022`) and macOS arm64 (`macos-15`), also without fail-fast. `dependency-security` runs cargo-deny. The `rust` aggregate fails on any failure, cancellation or unexpected skip. Every job has a timeout. Dependabot proposes weekly Cargo and GitHub Actions updates after merge.

The security job caches only the pinned cargo-deny installation (actions/cache v6.1.0, full commit SHA), keyed by runner OS/architecture, `rust-toolchain.toml` hash and cargo-deny version, with no fallback restore keys. Both cache hits and misses verify the installed version and scan current online advisory data; the advisory database is never cached. Cargo registry/build caches remain deferred until they bring a material benefit.

Permissions are read-only, checkout credentials are not persisted, and actions are pinned to full commits. There is no deployment workflow yet; an application executable, packaging contract and release qualification belong to later delivery work. Passing the core tests establishes no Bevy, SDK, plugin-host or hardware capability.
