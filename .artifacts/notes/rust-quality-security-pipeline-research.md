# Rust quality and dependency security pipeline research

Research date: 2026-10-20. Workspace HEAD inspected: `c6551faa0b783dbd42da268c2e71f4e770967875`. This is a recommendation for the current pure-Rust `orange-model` workspace, not implementation or evidence that checks passed.

## Current baseline

The root pins Rust 1.99.1 with rustfmt and Clippy in `rust-toolchain.toml`; Cargo declares edition 2024, resolver 3, `rust-version = "1.97.1"`, `publish = false`, one workspace member (`app/crates/orange-model`) and a committed lockfile. The documented current lock inventory is all crates.io sourced, with dependency licenses and declared MSRVs recorded in `docs/10-tooling-and-sources.md`. That document explicitly says no advisory review has run. The workspace contains 100 tests per the assignment context; this research did not run them or any audit.

`.github/workflows/checks.yml` already uses full-SHA pins for `actions/checkout` and `actions/setup-node`, `persist-credentials: false`, workflow-wide `contents: read`, locked Clippy and cross-OS tests (Ubuntu 24.04, Windows 2022, macOS 15), plus a fail-closed `rust` aggregate. It has no Rust documentation warning gate, explicit MSRV job, dependency advisory/source check, scheduled rescan, or Dependabot config. No project-level license is selected. These facts align with docs 06/10/14: toolchain/lock review is a merge requirement for dependency work; CI may not expose secrets or grant write access to untrusted PR code; release/deploy work waits for a real artifact and WU-12.

## Recommended minimal maintained pipeline

Keep the existing pinned 1.99.1 quality job and add Rust documentation to it. Use the version-controlled toolchain rather than floating stable, and keep the existing per-workspace feature selection (Cargo default features only). The commands are:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

This gives formatting, the current `clippy::all` policy plus compiler warnings-as-errors, and rustdoc warnings-as-errors for workspace packages. Do not add `--all-features` until features exist and a real supported feature matrix is defined. If toolchain updates introduce new warning failures, fix or narrowly justify the code/lint at the update; do not suppress the aggregate.

Add a separate MSRV job on Ubuntu using the exact declared floor, Rust 1.97.1, and a full test build so dev dependencies and test targets are exercised:

```sh
rustup toolchain install 1.97.1 --profile minimal
cargo +1.97.1 test --workspace --locked
```

Record `rustc +1.97.1 --version --verbose` in that job. Do not use `--ignore-rust-version`. Keep this as a separate required job/status feeding the stable Rust aggregate. The existing 1.99.1 matrix remains the platform compatibility evidence; the MSRV job proves the declared floor on Linux. A future Bevy/app adoption may require revisiting cost and feature/platform coverage, but the documented 1.97.1 release floor and current lock make this a small, direct check.

For dependency policy, use **cargo-deny only**, not both cargo-deny and cargo-audit. Pin the CLI install exactly at the verified `cargo-deny` 0.20.2 release (`cargo install --locked cargo-deny --version 0.20.2`), then run:

```sh
cargo deny check advisories sources
```

0.20.2 was released 2026-07-09, declares Rust 1.88.0, and its CLI/config supports the advisory, source, license and crate-ban checks. A RustSec advisory DB check covers known published crate advisories; source checking can enforce crates.io-only resolution. cargo-audit 0.22.2 is also available and focused on advisories, but would duplicate the advisory function without providing cargo-deny's source policy. Keep the tool install pinned and locked; let the advisory database refresh on each invocation, including a weekly scheduled run. Do not pin the database to a stale commit.

Suggested minimal `.cargo-deny.toml`/`deny.toml` source policy (cargo-deny recognizes `deny.toml`):

```toml
[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

The cargo-deny default allowed registry is crates.io. Explicit `unknown-git = "deny"` rejects new Git dependencies unless individually and consciously allowed. Keep `[advisories].ignore` absent/empty; an advisory exception needs a specific ID, owner, reason, upstream link and review/expiry date, and should be rare. Do not configure broad blanket suppressions. Run the advisory/source check on every PR and push and on a weekly schedule (for example Monday 07:00 UTC). A small independent job can install the pinned tool, check the graph, and feed a stable `rust-security` aggregate which is included in the existing `rust` aggregate. The aggregate must fail if the job fails, is cancelled or skipped, using the same fail-closed pattern already used by `rust`.

Do not turn on cargo-deny's `licenses` check with a guessed allowlist. The repository has not selected its own license, and an empty allowlist is not a meaningful approval policy. Keep dependency license inventory in dependency-change review using the existing `docs/10` evidence process. Once the maintainer selects an actual distribution/project licensing policy, add a reviewed SPDX allowlist to cargo-deny, run `cargo deny check licenses`, and update the repository and docs consistently. That is an explicit unresolved policy choice, not an assumed MIT/Apache choice. `cargo-deny check advisories sources` intentionally omits licenses until that choice exists.

Add `.github/dependabot.yml` with weekly updates for both `cargo` at `/` and `github-actions` at `/`, a low open-PR limit, and grouped routine updates if they remain reviewable. Dependabot should open proposals; CI and the normal PR review decide adoption. Enable GitHub Dependabot security updates/alerts for Rust dependencies if repository settings permit. This helps update the actual lock and action pins, while the cargo-deny schedule independently detects newly published RustSec advisories against the existing lock. Keep full SHA references for all workflow actions and let Dependabot propose refreshed SHAs; do not replace them with mutable tags.

Keep `permissions: contents: read`, checkout with `persist-credentials: false`, and no repository secrets in quality/security jobs. Continue using `pull_request`, never `pull_request_target` for checkout/build of contributor code. cargo-deny and Cargo metadata process manifests/lockfiles supplied by the PR, so execute them with the same read-only/no-secret posture as compile jobs. Fork and Dependabot PR workflows then remain useful with read-only tokens and no secrets. Do not cache advisory data in a way that can make scheduled results stale; avoid adding caches until measured CI time justifies them.

## Keep out of this baseline

Do not add cargo-audit alongside cargo-deny; it duplicates the advisory scan. Do not add cargo-outdated, cargo-bloat, cargo-udeps, cargo-geiger, cargo-vet, OSV-Scanner/Trivy/Snyk, SBOM generators, or action wrappers now: they add overlapping policy surfaces, setup and maintenance without a current requirement. Reconsider focused tools when a release/product/distribution gate needs their evidence. Do not add nightly, fuzz, Miri, coverage, benchmark regression or all-features required CI gates on an early model crate; docs 06 assigns fuzz/lifecycle/bench work to later bounded/nightly gates and does not treat line coverage as behavior proof. No installable app artifact exists, so do not add release/deployment, signing or publish jobs.

## Sources and evidence limits

All online references checked 2026-10-20; only official Rust/Cargo/Clippy, RustSec, cargo-deny project and GitHub documentation/release sources were used.

- Cargo `rust-version` semantics and supported compiler declaration: https://doc.rust-lang.org/cargo/reference/rust-version.html
- Cargo `--locked` contract and doc options: https://doc.rust-lang.org/cargo/commands/cargo-doc.html
- Clippy CI guidance for `-D warnings`: https://doc.rust-lang.org/clippy/continuous_integration/index.html and https://doc.rust-lang.org/clippy/usage.html
- rustdoc flags source precedence: https://doc.rust-lang.org/cargo/reference/config.html
- cargo-deny 0.20.2 release (2026-07-09): https://github.com/EmbarkStudios/cargo-deny/releases/tag/0.20.2
- cargo-deny 0.20.2 declared Rust 1.88.0: https://github.com/EmbarkStudios/cargo-deny/blob/0.20.2/Cargo.toml
- cargo-deny check capabilities and install command: https://github.com/EmbarkStudios/cargo-deny/tree/0.20.2 and https://embarkstudios.github.io/cargo-deny/
- cargo-deny source config and default crates.io registry: https://embarkstudios.github.io/cargo-deny/checks/sources/cfg.html
- RustSec Advisory Database and its consumers (`cargo-audit`, `cargo-deny`): https://github.com/RustSec/advisory-db
- cargo-audit 0.22.2 release list / requirements: https://github.com/rustsec/rustsec/releases and https://github.com/rustsec/rustsec/blob/main/cargo-audit/README.md
- GitHub recommendation to pin actions to full commit SHA: https://docs.github.com/en/actions/security-for-github-actions/security-guides/security-hardening-for-github-actions#using-third-party-actions
- GitHub Dependabot manifest, package and Actions update behavior: https://docs.github.com/en/code-security/concepts/supply-chain-security/dependabot-version-updates
- GitHub fork PR secrets and token restrictions: https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows and https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax

No package/tool was installed and no Cargo check, MSRV build, advisory scan, license audit, remote CI run, or repository setting change was performed. Since this environment did not run cargo-deny/audit against `Cargo.lock`, this note makes no claim that the current graph is advisory-free. Browser research did not establish a lockfile-wide clean result. The current installed Rust 1.99.1 success evidence in docs/10 is existing recorded evidence, not rerun here.
