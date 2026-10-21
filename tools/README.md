# Repository tooling

The metadata and release helpers support code validation and release CD. Run them from the repository root with Node.js 24.18.0:

```sh
node tools/check-cargo-metadata.mjs --self-test
node tools/check-cargo-metadata.mjs
node tools/check-cargo-metadata.mjs --msrv
node --test tools/release.test.mjs
node tools/release.mjs check
cargo deny --locked check advisories sources
```

The Cargo metadata policy check validates Cargo's locked resolved graph, derives the single workspace `rust-version` for the MSRV job, enforces exact Bevy `=0.20.0` member requirements and rejects direct or transitive `bevy` / `bevy_*` framework dependencies under known graph-core crates. Its fixture mode covers expected pass/fail cases. It does not ban Bevy from host adapters or equate `bevy_*` utility crate versions with the Bevy engine version.

`cargo-deny` is pinned at 0.20.2 and checks advisories and dependency sources only. The advisory database is fetched or refreshed on each run; a database or registry/network failure fails the job. Vulnerability, unmaintained and unsound advisories are errors under cargo-deny's defaults; yanked versions are warnings in this configuration. The source check rejects unrecognized registries and Git sources while allowing crates.io. This is not a license compatibility audit; the project license and allowed third-party SPDX expressions remain unselected.

The [PR validation workflow](../.github/workflows/checks.yml) runs on PRs, dev/main pushes and manual dispatch. Tests execute once on Linux. Native Windows/macOS jobs compile/link all test targets with `--no-run`; MSRV checks every target with `cargo check` on the declared compiler. Format/Clippy/rustdoc remain independent, as do Cargo architecture, workflow-contract and security checks. `workflow-contracts` checks CI/CD YAML with pinned/checksummed actionlint and tests the release helper/configuration. The required `rust` aggregate rejects any non-success job. The weekly schedule and release caller enable full test execution on every native OS. The schedule and Dependabot activate after merge; every job has a timeout.

The security job caches the pinned cargo-deny installation (actions/cache v6.1.0, full SHA), keyed by runner OS/architecture, toolchain-file hash and CLI version. Hits and misses verify its version and scan current online advisory data; the database is not cached. Compiled jobs use full-SHA Rust-cache v2.9.3, with separate job/platform/MSRV/release keys and its compiler/environment/lock/config invalidation. Installed binaries are excluded; formatting does not restore compiled dependencies. Cache hits never skip validation.

PR/build permissions are read-only, checkout credentials are not persisted and Actions use full-SHA pins. [Release CD](../.github/workflows/release.yml) reuses full native validation, builds optimized binaries, then separates credential-bearing packaging/verification and final provenance/publication jobs. Its preflight deliberately fails while the release executable, native packagers and qualification are absent. The [release contract](release/README.md) defines activation and output requirements. Passing helper or core tests establishes no Bevy, signing, installer or hardware capability.
