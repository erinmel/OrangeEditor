# Tooling, environment and research register

Checked 2026-10-20. Distinguish verified environment facts, verified source facts, design decisions and untested dependency candidates. Protocol/platform sources are also linked at the relevant contracts in [05](05-services-and-platforms.md); ADRs identify the decisions they support.

## Workspace and environment

- User-selected destination: `C:\Users\eriar\OneDrive\Documentos\Projects\Programming\rust\OraneEditor`. Exact directory spelling is intentional; product name is OrangeEditor. The directory existed and was empty at inspection.
- Planning is staged separately from the existing C# source and then copied into the destination. Existing C# changes are unrelated and must be preserved.
- Toolchain as of 2026-10-20 (reinstalled after the planning observations below): rustup **1.29.1** (`d95a37b6a 2026-08-13`), installed from winget package `Rustlang.Rustup` (installer hash verified by winget). The only installed toolchain is `1.99.1-x86_64-pc-windows-msvc` (active/default), profile `default`.
- Observed on a fresh PATH, all resolving from `%USERPROFILE%\.cargo\bin`:
  - `rustc 1.99.1 (58cf8f9c3 2026-10-13)`
  - `cargo 1.99.1 (5f94df478 2026-08-27)`
  - `clippy 0.1.99 (58cf8f9c35 2026-10-13)`
  - `rustfmt 1.10.0-stable (58cf8f9c35 2026-10-13)`
  - `rust-analyzer 1.99.1 (58cf8f9c 2026-10-13)`
  - Installed components: cargo, clippy, rust-analyzer, rust-docs, rust-std, rustc, rustfmt.
- Install commands used:

  ```powershell
  winget install --id Rustlang.Rustup --exact --source winget --override "-y --default-toolchain none --profile default"
  rustup toolchain install 1.99.1 --profile default --component rust-analyzer,clippy,rustfmt
  rustup default 1.99.1
  ```

- The workspace MSRV is Rust 1.97.1; the installed 1.99.1 satisfies it. MSRV CI coverage is recorded in the CI sections below.
- WU-00/00b.1 (2026-10-20) added `rust-toolchain.toml` (channel `1.99.1`, components rustfmt/clippy, profile minimal), the root Cargo workspace (`resolver = "3"`, edition 2024, `rust-version = "1.97.1"`, member `app/crates/orange-model`, `plugins` excluded) and a tracked `Cargo.lock`. Cargo resolved dependencies to the highest versions compatible with Rust 1.97.1; see the 00b.1 row in the dependency ledger.
- Superseded planning observation: the environment originally reported `rustc`/`cargo` 1.96.1 from a standalone MSI install (`C:\Program Files\Rust stable MSVC 1.96`) that shadowed rustup on PATH, plus rustup toolchains `stable` and `nightly-2026-09-07`. That MSI, the previous rustup install and its toolchains were removed on 2026-10-20 before the reinstall above.
- 00b.1 local evidence (Windows 11, Rust 1.99.1, 2026-10-20): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked` passed (22 tests: 8 unit, 13 codec integration, 1 property test at proptest's default 256 cases). Remote `rust` CI evidence is recorded when it first runs.
- 00b.2 local evidence (same environment, 2026-10-20): the same three commands passed with 46 tests in total. The 24 new tests are 4 for G-36 layout, 2 for G-30 literals, 5 for G-31 variadic ports, 12 for transaction structure and 1 property test of undo-all/redo-all over random edit sequences. No new dependency.
- 00b.3 local evidence (same environment, 2026-10-20): the same three commands passed with 63 tests in total. New tests: 3 signature unit tests, including a sha256 vector computed independently with coreutils `sha256sum`; 5 G-29/G-39 and implicit-port vector tests; 2 index tests, including a property that the incremental index equals a full build; 6 paste tests, including a property of no aliasing, preserved internal edges and undo; 1 `PutSignature` test. New dependency: `sha2` (ledger above).
- 00b.4 local evidence (same environment, 2026-10-20): the same three commands passed with 87 tests in total. New tests: 9 unit tests (Value size and equality, type terms, RFC 4648 base64 vectors); 6 scalar literal tests; 8 aggregate/FR-08 literal tests, including decoding every literal in the 12 §6 example; 1 encode/decode round-trip property. A compile-time assertion keeps `Value` at 16 bytes or less. The codec nesting budget `MAX_NESTING = 64` is a proposed value and an open budget. No new dependency (base64 is implemented locally).
- 00b gaps follow-up (Windows x64, Rust 1.99.1, 2026-10-21): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked` and `cargo test --workspace --locked` passed with 102 tests. New tests cover JSON text depth (64 accepted, 65 and 84 rejected, brackets inside strings ignored), the size budget, hash/equality agreement, and decode/encode of a 20 000-key map with duplicate detection. `MAX_JSON_TEXT_BYTES = 1 MiB` is a proposed value and an open budget, like `MAX_NESTING = 64`.
- 00c.3b analysis guard (2026-10-21): expanded scoped/concrete type terms are limited to a provisional `MAX_EXPANDED_TYPE_NODES = 16_384` nodes per term; nesting keeps `MAX_NESTING = 64`. Limits return typed analysis failures before constructing an oversized output. Alias traversal is iterative; a 10,000-variable alias regression passes without consuming type nesting depth. These are open resource budgets, not a new language type exclusion, measured NFR-15 timing or production memory qualification.
- Apart from `orange-model`, no application implementation, integration test, audio routing check, broker/container setup or OS packaging verification has run. Local availability of Rust is not proof of a working UI build toolchain.

## Model review fixes and slice 4 analysis (2026-10-20)

Review findings on the first four slices were fixed in isolated worktrees based on `8c1f42f`, with explicit source/test ownership, and integrated into the feature checkout. No commit, push, merge or remote CI was part of the fixes.

Corrections: unique transient undo state identities across divergent clones/reloads; atomic function-body node/edge map key validation; default-literal display excluded from semantic projection; same-position ordered-slot replacement without remove/reinsert; exact float-width checks in literal encoding; and map encoding rejecting duplicate/float keys. Contracts and implementation notes are updated in [12](12-graph-data-structures.md), with the identity clarification in ADR-06. FR-03, FR-08 and G-39 regressions cover the affected behavior. The edit-path operation-count test supports NFR-15 investigation but does not measure its timing targets.

Selected toolchain: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, `x86_64-pc-windows-msvc`; `rust-toolchain.toml`, workspace manifests, dependency features and `Cargo.lock` are unchanged. Lockfile SHA-256: `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`.

Executed in the feature checkout after integration (document checks use Node.js v24.18.0):

| Command | Result |
|---|---|
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `cargo test --workspace --locked --offline` | PASS: 100 tests, 0 failures; 0 doctests |
| `node tools/check-docs.mjs` | PASS: 33 Markdown files, 13 JSON blocks, 287 local links |
| `node tools/check-repo-policy.mjs` | PASS: tracked-path, credential-pattern and Git-whitespace checks; limited scan, not a security audit |
| `git diff --check` | PASS |

The 13 added regressions are two internal patch/operation-order tests, four undo/function-invariant tests, three signature tests and four literal-encoding tests. Windows emitted incremental-cache hard-link warnings with a copy fallback. The temporary pre-fix slice-4 probe reproduced encoder failures and observed that JSON text containing 84 nested arrays is accepted despite the typed-container budget of 64; it is not part of the permanent suite. Remaining open items: JSON text nesting/size bounds, quadratic map duplicate-key checks and narrower property coverage of records, enums and JSON. No MSRV 1.97.1 build, Bevy build, cross-OS run, NFR measurement or gate passage is asserted.

## CI matrix and history cleanup (2026-10-20)

The eight feature-branch commits after `fa90e6c` were recreated with commit-message trailers removed. Every content tree, human author/committer identity and timestamp was verified unchanged; `main` remains at `fa90e6c`. A local ignored recovery bundle preserves the original objects. Only the feature branch is eligible for the exact force-with-lease update; protected-main history is unchanged. CONTRIBUTING.md and 14 §3 record the commit and PR attribution policy.

The previous pipeline passed `docs`, `policy` and Ubuntu `rust` on original head `913806b`: [run 39520354817](https://github.com/erinmel/OrangeEditor/actions/runs/39520354817). That result applies to the old head and pipeline. The new workflow separates formatting/Clippy from locked core tests on `ubuntu-24.04` (x64), `windows-2022` (x64) and `macos-15` (arm64). It preserves `docs`, `policy` and the stable `rust` aggregate, which requires both quality and all test jobs to succeed, including failure/cancellation/skip handling. Action pins, the shared Rust 1.99.1 toolchain, dependency features and lockfile remain unchanged. Jobs have explicit timeouts, read-only permissions and no persisted checkout credentials. Runner labels/architectures were checked against the [GitHub runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners); matrix behavior follows [GitHub's matrix documentation](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/run-job-variations).

Local validation on Windows x64: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `cargo test --workspace --locked --offline` (100 tests), `node tools/check-docs.mjs`, `node tools/check-repo-policy.mjs` and `git diff --check` passed with the existing Rust/cargo 1.99.1 and Node.js v24.18.0. Windows incremental-cache hard-link warnings used the copy fallback.

Expanded remote evidence: [run 39522142007](https://github.com/erinmel/OrangeEditor/actions/runs/39522142007) passed all seven jobs on CI commit `72670cd`: `docs`, `policy`, `rust-quality`, the three OS test jobs and the stable `rust` aggregate. This evidence update changes only documentation; the tested workflow and Rust source are unchanged. The PR validation table records its latest head/run separately. There is no local macOS/Linux, MSRV 1.97.1, Bevy, packaging or deployment claim. Deployment/release automation waits for an application artifact and the applicable WU-12 gates. NFR-11/NFR-14 and ADR-18 retain their wider qualification requirements.

## CI quality-gate expansion (2026-10-20; isolated implementation checkout)

This change adds rustdoc warnings, a metadata policy guard, a workspace-derived MSRV test, pinned cargo-deny advisory/source checks, a weekly workflow schedule and weekly Dependabot update proposals. The workflow, Dependabot schedule and required-check configuration are not yet verified on GitHub from this checkout; scheduled Actions and Dependabot run only after these files reach the default branch. Read-only permissions, full-SHA Action pins and non-persisted checkout credentials are retained. No license allowlist was invented, and the advisory/source check is not a license audit.

Recorded environment: Rust/cargo `1.99.1` (`rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`), host `x86_64-pc-windows-msvc`; Node.js `v24.18.0`; one workspace package, `orange-model`, with declared `rust-version = 1.97.1` and no declared Cargo features. The tracked `rust-toolchain.toml` remains `1.99.1`; workspace feature selections and `Cargo.lock` are unchanged. Lockfile SHA-256: `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`.

| Command / evidence | Result |
|---|---|
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `cargo test --workspace --locked --offline` | PASS: 100 tests; no doctests |
| `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked` | PASS |
| `node tools/check-cargo-metadata.mjs --self-test` | PASS: 10 fixture cases, including Bevy subsystem transitive leakage, optional core dependency, exact version pin, missing/empty graph and host-only Bevy allowance |
| `node tools/check-cargo-metadata.mjs` / `--msrv` | PASS: current graph boundary accepted; Bevy pin check is inapplicable because Bevy is absent; derives `1.97.1` |
| `node tools/check-docs.mjs` | PASS: 33 Markdown files, 13 JSON blocks, 287 local links |
| `node tools/check-repo-policy.mjs` / `git diff --check` | PASS: repository policy and whitespace |
| `cargo deny --version` | PASS: `cargo-deny 0.20.2`, installed with `--locked` under ignored `target/cargo-deny/` |
| `cargo deny --locked check advisories sources` | PASS: `advisories ok, sources ok`; RustSec database fetched/refreshed online |
| `rustc +1.97.1 --version --verbose` | PASS: Rust `1.97.1 (8bab26f4f 2026-07-14)`, locally installed under ignored `target/rustup-home/` |
| `cargo +1.97.1 test --workspace --locked --offline` | NOT PASS / environment blocked: Windows `link.exe` reported LNK1104 opening a Rust sysroot archive under the long isolated-checkout toolchain path; the archive exists. MSRV runtime test remains pending Linux CI. |

The deny command keeps `--locked` (lockfile stability) while leaving network enabled to refresh the RustSec database; cargo-deny 0.20.2 exposes offline as a separate option. Its config treats vulnerability, unmaintained and unsound advisories as errors, yanked dependencies as warnings, and unknown registry/Git sources as errors. Current machine validation is Windows x64 only and does not pass NFR-11, Bevy feasibility, G0/G1/G2/G5, or remote CI gates.

Integration repeated formatting, locked offline Clippy and all 100 tests, and locked offline rustdoc with warnings denied on the same Windows/toolchain configuration: PASS. The integrated metadata fixtures, boundary check and MSRV derivation passed; documentation passed for 36 Markdown files, 13 JSON blocks and 293 local links. The pinned scanner repeated its online advisory/source check successfully with access to Cargo's advisory cache. Lockfile hash and toolchain remain unchanged.

Remote evidence: [run 39524649391](https://github.com/erinmel/OrangeEditor/actions/runs/39524649391) on implementation commit `1eb5721` passed all ten jobs: docs, policy, Rust quality, metadata policy, MSRV, dependency security, three native OS tests and the `rust` aggregate. The MSRV log records `rustc 1.97.1 (8bab26f4f 2026-07-14)` on `x86_64-unknown-linux-gnu` with all 100 tests passing; this resolves the remote MSRV evidence gap while preserving the local Windows limitation above. Security logs record cargo-deny 0.20.2 and `advisories ok, sources ok`. After this success, GitHub API readback confirms strict required `docs`, `policy`, `rust` checks bound to Actions app 15368; admin enforcement, solo-mode zero independent approvals, resolved conversations, linear history and disabled force pushes/deletion remain enabled as configured. Negative-path protection tests remain unexecuted.

That run exposed Node 20 deprecation warnings in the inherited Action pins. The follow-up uses official release commit SHAs for [checkout v7.0.1](https://github.com/actions/checkout/releases/tag/v7.0.1) (`3d3c42e5aac5ba805825da76410c181273ba90b1`) and [setup-node v7.1.0](https://github.com/actions/setup-node/releases/tag/v7.1.0) (`949feb2413d6458794dcd2491c4babbbce0c15c1`), resolved using GitHub's commit API and checked against each tag's README/action.yml. Both declare Node 24; hosted runners satisfy their minimum runner requirement. Setup-node's automatic package-manager cache is explicitly disabled. Persisted credentials remain disabled and no elevated-event checkout is introduced. The PR validation table records the final follow-up head/run separately; the successful implementation run above precedes this Action refresh.

## Code-focused CI revision (2026-10-20)

Checks unrelated to code were removed, and formatting failures no longer hide other results. Standalone Markdown/generic repository-policy jobs are removed; their scripts remain optional local helpers. Formatting, Clippy and rustdoc become separate matrix jobs with fail-fast disabled. `rust-architecture` retains the specific Cargo dependency-boundary, exact Bevy pin and MSRV metadata checks. The stable `rust` aggregate still requires all Rust jobs. After remote success, the intended required-status set is `rust` only; the PR records final-head validation and actual protection readback. Earlier docs/policy protection records above are historical.

Measured [run 39525101049](https://github.com/erinmel/OrangeEditor/actions/runs/39525101049) spent 135 seconds compiling cargo-deny and one second scanning; native compile/test steps took 18–34 seconds. Cache only cargo-deny's installed root, with an exact key containing OS, architecture, toolchain-file hash and CLI version. [actions/cache v6.1.0](https://github.com/actions/cache/releases/tag/v6.1.0), SHA `55cc8345863c7cc4c66a329aec7e433d2d1c52a9`, was verified through the official release/commit API and tagged action.yml (Node 24). Both cache hits and misses verify cargo-deny 0.20.2 and execute the online advisory/source scan. No advisory database, credential or Cargo build cache is introduced. Normal GitHub PR cache isolation applies; a CLI version assertion does not authenticate a cached executable cryptographically.

Recorded environment remains Rust/cargo 1.99.1, Node 24.18.0, Windows x64; MSRV 1.97.1; the only package is `orange-model`, without declared Cargo features. Manifests, features, `rust-toolchain.toml` and `Cargo.lock` are unchanged from `3af6ebd`; lockfile SHA-256 remains `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`. Local revised metadata checks and their 10 fixtures, MSRV derivation, documentation-helper check and whitespace check passed. Remote cache miss/hit verification is pending at commit preparation; final PR evidence distinguishes observed paths. No intentional failing-format run or new Bevy/platform/hardware gate passage is claimed.

Implementation evidence: [run 39526751351](https://github.com/erinmel/OrangeEditor/actions/runs/39526751351), commit `9d8cbae`, passed all ten code jobs. Formatting, Clippy and rustdoc now appear independently; architecture, MSRV, all three OS tests, dependency security and the aggregate passed. The security log confirms an initial cache miss, installation/version verification of cargo-deny 0.20.2, `advisories ok, sources ok`, and successful cache save. GitHub protection readback now contains only strict `rust` bound to Actions app 15368; other inspected protection controls remain unchanged, as recorded in 14. This documentation-only evidence follow-up also exercises cache reuse; its final-head result and observed hit/miss are recorded in the PR.

## PR validation and release CD split (2026-10-20)

On baseline `c9fc8e5`, the user requested removing duplicate test execution while preserving native multi-platform PR compatibility and preparing release CD. PRs now execute workspace tests once on Ubuntu; Windows/MSVC and macOS arm64 compile/link all targets with `cargo test --workspace --all-targets --locked --no-run`. MSRV uses `cargo check --workspace --all-targets --locked` rather than another test run. Weekly qualification and release reuse the same validation workflow with full native test execution. Independent format/Clippy/rustdoc and the stable required `rust` aggregate are retained; release-helper tests/configuration are also required code checks.

Compiled jobs use Rust-cache v2.9.3, SHA `d1527c8f67ef9af357290475d4af4b0fab8f12e0`; the official release/tag API and tagged action.yml were checked (Node 24). Compiler/host/environment/manifests/lock/config participate in cache keys, with explicit job, native OS, MSRV and release-target separation. The MSRV environment selects its own actual compiler before caching. Installed binaries are excluded and incremental compilation is disabled by the action. The existing exact cargo-deny cache and fresh online advisory scan remain. No advisory/credential data is added to caches.

Release CD requires an annotated version-matching tag from main ancestry, a qualified executable/license and three real native packagers. It reuses full validation, builds release binaries read-only, then separates credential-bearing packaging/signing/install/launch verification and final checksum/provenance/publication jobs. Live tag identity is rechecked at publication. New full-SHA Actions were resolved from official release APIs: upload-artifact v7.0.2 `cf430e030ddbb5b0abf93d22962f4752f3646cd9`, download-artifact v8.0.2 `9000827ccba6bdab643e8b6fd33ac0654aef8333` (both Node 24), attest v4.2.2 `1e69f48acb82d1966a394da916b4c1698aa569d6`. No release environment, credentials, tag or public release was created. `.github/release.json` is intentionally unconfigured/unqualified because no desktop executable, reviewed distribution setup, packagers or signing evidence exists. The workflow therefore blocks at preflight; the [release contract](../tools/release/README.md) documents remaining activation work. This is no WU-12/G5 runtime pass.

Local checks at preparation: actionlint 1.7.12 (official Windows amd64 archive checked against the published SHA-256 file) accepted both workflows with shellcheck/pyflakes integrations disabled; this validates YAML/expressions, not native signing scripts. `node --test tools/release.test.mjs` passed eight cases, including actual package checksum/collection, tampering, missing native outputs and failed signing/install evidence. `node tools/release.mjs check` accepted disabled configuration; `RELEASE_TAG=v0.0.0 node tools/release.mjs plan` correctly exited nonzero with missing-executable/qualification diagnostics. Workflow execution, cache performance and native compilation results are recorded on the PR's final head; no actual release build/sign/publish is claimed.

Rust/cargo remains 1.99.1, MSRV 1.97.1, Node 24.18.0; the workspace still contains only `orange-model`, without declared features. Application Rust, manifests, selected features, toolchain and Cargo.lock are unchanged. Lockfile SHA-256 remains `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`.

Parent integration validation on Windows x64: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and `cargo test --workspace --locked --offline` passed, with 97 current core tests (after the preceding model test/module refactor). Metadata guard/10 fixtures and MSRV derivation passed. The `workflow-contracts` CI job runs actionlint 1.7.12 against CI/CD YAML plus the release helper tests/configuration; the Linux validator archive is pinned by its published SHA-256. Neither release publication nor native packagers were executed locally.

During implementation, a concurrent repository update introduced the `dev` integration / `main` promotion policy (`26cdfe4`, PR #3). These pipeline changes were integrated in a separate checkout while preserving that policy and leaving the shared checkout clean. PR validation includes dev/main pushes; application releases still require promoted main ancestry. The CI/CD PR tracks its integration dependency and final remote head/results separately.

Remote implementation evidence: [run 39534720986](https://github.com/erinmel/OrangeEditor/actions/runs/39534720986) on `805e16b` passed all 11 jobs. Step readback confirms Linux executed tests, Windows/macOS used native compile/link without execution, and MSRV used all-target compilation. Workflow lint, eight release-helper tests/configuration, architecture, format/Clippy/rustdoc, security and the aggregate passed. This documentation-only follow-up leaves both workflows/tooling unchanged and exercises cache reuse; the PR records the final head/run and observed cache outcome. CD's real build/sign/install/publish path remains unexecuted because release readiness is intentionally false.

## Additional AI development integrations

**No new MCP/plugin is required to finish this plan.** Web research, filesystem tools and an architecture subagent are available. These development integrations are unrelated to OrangeEditor expansion packages.

Context7 was found in the available plugin directory and suggested as an optional documentation integration. It was **not installed or connected** in this session. It may help retrieve dependency documentation, but Bevy 0.20 coverage/version selection must be verified before using its answers. Release-tag source and versioned local Rustdoc remain authoritative. Installation is optional and must not block planning or justify substituting 0.19 APIs.

OBS/Twitch account connectors are not required to design Orange's SDK. Actual integration testing later requires local OBS, an explicitly authorized Twitch test application/account and synthetic fixtures; credentials belong in the vault, never agent prompts or source files. Do not introduce an unrelated cloud project-management system as a prerequisite.

For implementation, verify the Rust toolchain, target linker/native SDKs, language server, formatting/lint tools and platform libraries. These are development prerequisites, not claims that they are installed. Any broker/container, audio diagnostic or security tool is added only for a work unit that uses it.

## UI framework change (2026-10-21)

Bevy was dropped from the project (ADR-02, ADR-21); the editor UI will use egui hosted by eframe, with versions pinned when P-01 runs. The former Bevy 0.20 source register and dossier are in Git history (doc 09 before this change). `tools/check-cargo-metadata.mjs` now rejects UI, window, renderer and engine frameworks (Bevy, egui, eframe, epaint, winit, wgpu, Slint, iced and their subcrates) under graph-core crates instead of pinning Bevy; the Bevy release-feature check left `tools/release.mjs`. Local results on Windows x64, Node 24: `node tools/check-cargo-metadata.mjs --self-test` PASS (8 cases), `node tools/check-cargo-metadata.mjs` PASS, `node --test tools/release.test.mjs` PASS (7 tests).

## Dependency selection ledger

No framework pin is fixed yet; egui/eframe are pinned in P-01. All implementation dependencies must acquire an exact version, minimum compiler, platform/feature matrix, license, maintenance/security review and contract test at their adopting work unit. A proposed family below is a research candidate, not a dependency approval or compatibility claim.

| Capability | Candidate / decision boundary | Required evidence before adoption |
|---|---|---|
| Presentation | egui + eframe (ADR-02), Slint fallback | P-01/P-02; minimal feature graph, text/accessibility/RTL evidence |
| Async services | Dedicated Tokio host as proposed in architecture | Exact crate/features/MSRV, shutdown/reactor contract, bounded worker bridge |
| Hot expansion execution | Wasmtime/component-model candidate | Exact supported component interface/runtime version, fuel/epoch interruption, memory limit, host-call cancellation and boundary benchmarks |
| Definitions and persistence | Serde-family serialization candidate | Versioned schema contract, malformed input bounds, deterministic output and migration tests |
| HTTP/TLS | Async HTTP client with explicit TLS backend | HTTP/2, proxies, certificates, streaming, cancellation, ALPN and licenses |
| WebSocket/MQTT/AMQP/gRPC | Separate protocol adapters, not a universal network library | Protocol version, authentication/stream behavior, async runtime compatibility, bounded payloads and fixtures |
| Audio | Dedicated endpoint/decoder service plus OS adapters | Actual per-player routing, decoder format/license inventory, device identity, clock/rate behavior and callback audit |
| Linux virtual source | PipeWire adapter | Versioned API/permissions/session policy and app-owned cleanup on a real Linux session |
| Credentials | OS-specific secure stores behind one host interface | Windows/macOS/Linux availability and locked/headless behavior; no plaintext fallback |
| Localization | Fluent FTL parser/formatter candidate, Unicode/CLDR data and host font fallback | Exact versions/features/MSRV/licenses, bounded reference/argument handling and locale/plural capabilities; L-suite, G-39 and P-02 platform evidence before adoption; sources in [13](13-internationalization.md) |

### Adopted: `orange-model` (WU-00/00b.1, 2026-10-20)

Serde-family candidate adopted for the graph document codec only. Versions are exact `Cargo.lock` entries; licenses and MSRV are from `cargo metadata --locked`. All declared MSRVs are at or below the workspace's 1.97.1.

| Crate | Version / features | License | Declared MSRV | Role |
|---|---|---|---|---|
| serde (+ serde_core, serde_derive) | 1.0.229, `derive` | MIT OR Apache-2.0 | 1.56 / 1.71 | Codec derives |
| serde_json | 1.0.154, `preserve_order` | MIT OR Apache-2.0 | 1.71 | JSON load/save, ordered opaque objects |
| indexmap (+ hashbrown, foldhash, equivalent) | 2.14.2, `serde` | Apache-2.0 OR MIT (foldhash: Zlib) | 1.85 | Creation-ordered maps |
| uuid | 1.28.0, `serde` | Apache-2.0 OR MIT | 1.89.0 | v4/v7 document IDs |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | 1.77 | Error enums |
| sha2 (+ digest, block-buffer, crypto-common, hybrid-array, typenum, const-oid, cpufeatures, cfg-if, libc) | 0.11.1, default features | MIT OR Apache-2.0 (all) | 1.88 (others 1.32–1.85) | `sha256:` signature keys (added in 00b.3) |
| proptest (dev only) | 1.12.0 | MIT OR Apache-2.0 | 1.88 | Round-trip property tests |

Transitive normal dependencies also include itoa 1.0.18, memchr 2.8.3 (Unlicense OR MIT), zmij 1.0.23 (MIT), proc-macro2 1.0.107, quote 1.0.47, syn 3.0.9 and unicode-ident 1.0.27 ((MIT OR Apache-2.0) AND Unicode-3.0). Security advisory review (for example `cargo audit`) has not been run. Malformed-input size bounds and migrations remain open for later 00b slices.

Do not select a crate solely because an ecosystem page lists it; UI integrations must explicitly support the pinned egui version. Record implementation results and the final lockfile in the work unit rather than retroactively presenting candidates as verified.

## Planning review evidence

Orchestrator review on 2026-10-20 checked the delegated architecture documents against the assignment and resolved consistency issues concerning numeric rounding, selected-provider preservation, atomic directory-bundle saves, OBS auth-provider ownership, transport versus method authority, and broad work-package decomposition.

### Graph-core review (2026-10-20)

A second review compared the plan against the goal of an expandable, performant node editor with events and triggers. No product code exists, so "code" findings refer to the planned contracts. The findings were resolved by adding [11](11-graph-language.md) and [12](12-graph-data-structures.md), FR-44–50, NFR-15, ADR-13–16, OPEN-10–12 and WU-00:

| # | Gap or contradiction found | Resolution |
|---|---|---|
| C1 | WU-03 froze plugin node/type contracts before WU-04 defined the graph language, while WU-04 depended on "WU-03 contracts" (circular) | WU-00 graph core precedes WU-03; G2 requires the G-suite |
| C2 | "Reject arbitrary/unbounded cycles" did not say which loops are legal; perpetual event/timer automation looked forbidden | Cycle classes and formal checks, [11 §8](11-graph-language.md) |
| C3 | Random, time and Get were not pure, but no node class described them; the reread rule was ambiguous | `volatile` kind, epoch prologue, stability classes |
| C4 | `Outcome = Complete/Suspend/Failed` could not select a Branch arm or an outcome group | Pure/action/iterator kernel contracts; control intrinsics compiled, [12 §10](12-graph-data-structures.md) |
| C5 | Reading outputs of nodes that did not execute (other branch, zero-iteration loop, failed path) was undefined | Region + availability (dominance) rule, [11 §7.2](11-graph-language.md) |
| C6 | Ports lacked cardinality, defaults, optionality, variadic, laziness and event-port semantics | Port descriptor, connection matrix and cardinality, [11 §4](11-graph-language.md) |
| C7 | Saved-graph example had empty edges/variables, an undefined `entrypoints` kind, and two type-reference spellings | Complete JSON formats; canonical type expressions; `exports` |
| C8 | Requirement asked for explicit integer/float types and protobuf/audio need int32/float, but only i64/u64/f64 existed; timers had no duration type | i32/u32/f32/duration/timestamp/map/result added |
| C9 | "Generics specialize at compile time" had no type-parameter syntax; the example was per-type `add-i64` | Constrained generics, persisted `type_args`, constraint implementations |
| C10 | "No implicit conversion" without any conversion mechanism, editor behaviour or plugin extension point | Converter registry, classes, auto-insert rule, ownership |
| C11 | Reentrancy referenced entrypoints, but no entry node kinds or modes beyond serial/parallel existed | Entry catalog; queue/drop/replace/parallel |
| C12 | "Concurrent run-local writes fail compilation" with no concurrency construct or defined execution model inside a run | Fork/join modes, one actor per run, write-conflict rule |
| C13 | Sequence behaviour with async steps was undefined | O2: each step completes before the next |
| C14 | Live editing versus pinned running plans was undefined | Stale-plan rule and preview semantics, [11 §15](11-graph-language.md) |
| C15 | Dynamic ports would require running provider code when a document opens, contradicting "no code on open" | Declarative port templates and persisted signature snapshots |
| C16 | Subgraphs had no interface, kind, recursion or scope rules | Functions, [11 §14](11-graph-language.md) |
| C17 | No custom events (emit/on-event) and no runaway feedback protection | Custom events, causal depth, handler order |
| C18 | Services used stream handles that the graph language could not represent | Affine `stream<T>` and its nodes, [11 §12](11-graph-language.md) |
| C19 | Memoization "keyed by input slots and state-read version" contradicted NFR-05 zero allocations | Compile-time stability classes with bitsets |
| C20 | Break/continue "nearest declared loop" was not computable for arbitrary wiring | Region tree and placement rules |
| C21 | Workspace shared Get looked pure, but shared state lives in a host actor | Published snapshot reads; async writes with read-your-writes |
| C22 | Deterministic replay was promised without recording async completion order | Ingestion log, [11 §7.6](11-graph-language.md) |
| C23 | FR-08 required Error to stay distinguishable as a value, but errors were control-only | `core/try` → `result<T>`; value layout makes None/missing/null/Error distinct |

Structural checks re-executed after this review with a Node.js script over all Markdown files (`node check-docs.mjs <repo>`, run from the session scratchpad):

- 15 Markdown files; code fences balanced.
- 11 JSON blocks parse as JSON. This verifies syntax, not schema conformance or executable SDK compatibility.
- 156 local Markdown links resolve to files.
- 50 unique FR IDs and 15 unique NFR IDs; each appears in the roadmap coverage matrix (numeric ranges expanded), and every referenced FR/NFR is defined.
- 16 ADR definitions; no referenced ADR ID is undefined.
- 38 G-suite cases; every referenced G-ID is defined. 40 diagnostic codes in the catalog; every referenced code is in it.
- 52 executable subunit rows under 14 work packages.

These checks validate document structure and traceability coverage. They do not establish runtime correctness, API compilation, security isolation, platform support or NFR achievement. Implementation/feasibility gates remain NOT RUN. No production code, dependency installation or toolchain update was performed.

### Node editor structure and i18n review (2026-10-20)

The architecture review found that Unicode/IME requirements and isolated references to localizable labels did not define a translatable product. This documentation work adds [13](13-internationalization.md), FR-51, NFR-16, ADR-17, OPEN-13, G-39 and L-01–08. The initial graph rules remain the core-first plan; catalogue/schema work precedes SDK freeze and real text/layout/IME evidence remains a UI feasibility gate.

| Finding | Resolution |
|---|---|
| No end-to-end i18n requirement, fallback, translation coverage or release owner | 13 text/catalogue/locale policy, FR-51/NFR-16 and L-suite traced through WU-00–05/12 |
| Ports mentioned translation keys but node/category/member examples used bare English strings | Shared TextRef/MessageRef and updated 03/12 examples; category IDs separate from captions |
| Structured errors could not retain typed arguments or the exact originating catalogue version | MessageOrigin, canonical tagged arguments and flattened error-field mapping in 12 §6.2; redaction before rendering; safe fallback after removal |
| Translation changes could leak into signature identity or graph `display` output | Explicit semantic port projection; invariant type/member IDs for derived display; G-39 core fixture and separate real L-05/L-08 integration |
| WU-00 risked depending on host localization after adding G-39 | Pure fixture projections in 00b/00e/00f; real formatter/catalogue/UI integration belongs to WU-03/05 |
| UUIDs described as preserved across paste despite required remapping | 12 representation table now distinguishes save/load identity from pasted content |
| G0 selected only WU-01 despite core-first next steps | G0 now agrees with priority 00a alongside 01a |
| ADR-04 described activation-only memoization despite stability classes in 11 | ADR-04 now states const/activation/epoch scopes |
| NFR-15 promised never visiting the whole graph even when the affected region spans it | Explicit connected worst-case reporting; unrelated components remain excluded; proposed timing targets retained |
| Document ownership/reading path and assignment file list omitted newer contracts | README reading order/authority map and AGENT_PROMPT/AGENTS include 11/12/13 |

Executed document audit: Node.js **v24.18.0**, `node - .` with the session checker piped on stdin, from the project root. The checker validates Markdown fences/table continuity, JSON syntax, local file links, ID/range references, FR/NFR roadmap coverage, TextRef/category example shapes, equality of repeated math definitions and graph diagnostic catalogue references. Result: **PASS** — 16 Markdown files, 13 JSON blocks, 183 local file links; 51 FR, 16 NFR, 17 ADR, 13 OPEN, 39 G and 8 L definitions; 40 graph diagnostic codes. Counts are structural checks, not proof of complete semantics or fixture execution. The audit script was supplied on stdin and is session-only tooling. A repeat attempt to load a temporary script by path was unavailable in a later command sandbox; rerunning the complete checker on stdin passed.

Primary i18n sources and their limits are recorded in 13 §8. No UI compilation, package loading, Fluent parsing, translation review, renderer/accessibility/platform probe, benchmark or implementation fixture ran. Exact formatter/data pins and additional shipped languages remain open before their named gates. No gate is declared passed by this review; next work remains 00a and 01a under a later implementation instruction.

### Git workflow review (2026-10-20)

Current observed directory: `C:\Users\eriar\OneDrive\Documentos\Projects\Programming\rust\OrangeEditor`. The original `OraneEditor` working directory is unavailable; earlier path records are historical. No folder was renamed by this review. `Test-Path .git` returned `False`; `git --version` returned `git version 2.55.0.windows.1`.

Added `docs/14-git-workflow.md`, `CONTRIBUTING.md` and `.github/PULL_REQUEST_TEMPLATE.md`; linked the policy to README, the assignment, delivery dependencies, quality checks and ADR-18/NFR-14. Selected one integration branch plus short-lived subunit branches, PR/squash integration, solo/team review modes, protected history, scoped checks, release/hotfix rules and explicit bootstrap/enforcement evidence. Corrected the previous universal PR-test wording so the documentation baseline does not require nonexistent Rust CI. No repository initialization, configuration, commit, branch, remote operation, CI workflow or release action was performed.

Executed validation: Git 2.55.0.windows.1 `git check-ref-format --branch <example>` accepted all eight branch examples in 14. Node.js v24.18.0 `node -` with the document checker on stdin passed for 19 Markdown files, 13 JSON blocks and 197 local file links; unique defined/reference IDs include 51 FR, 16 NFR, 18 ADR, 13 OPEN, 39 G and 8 L. Checks cover fences, JSON syntax, local links, catalogue table continuity, ID references, FR/NFR roadmap coverage and the repeated math-definition consistency. The checker is session tooling; these results do not establish remote enforcement. Hosting/visibility, maintainer identities, CI statuses/runners and signing setup remain pending. Primary Git/hosting references are recorded in 14; only document checks and branch-name validation ran.

### Public monorepo bootstrap (2026-10-20)

The user subsequently authorized creation/publication of `erinmel/OrangeEditor` with separate application and authored-plugin areas. GitHub profile/CLI identity agreed on `erinmel`; the repository lookup returned 404 before creation. Added the source areas and ownership in 15/ADR-19, ignore/line-ending policy, read-only CI and committed validation scripts. The current folders are documented reservations, not product scaffolding or implemented expansion packages. Local IDE state remains ignored and unmodified. The commit email uses the account's GitHub noreply address.

Tools: Git `2.55.0.windows.1`, GitHub CLI `2.96.0`, Node.js `v24.18.0`. Commands executed include `git init -b main`, explicit-path staging, both `node tools/check-*.mjs` scripts, `git diff --cached --check`, the initial commit, and `gh repo create erinmel/OrangeEditor --public --source . --remote origin --push` (with description/wiki options). Initial commit: `42efbe0f856a6e7ff956307d456ea7af5f7db4fd`; 32 tracked files. Local structural checks passed for 27 Markdown files and 13 JSON blocks; tracked-path/credential-pattern/whitespace checks passed. These scans are limited and do not prove complete secret absence or product correctness.

Public/default-branch metadata and SSH origin were read back successfully. The initial repository-check workflow run `39493589359` completed both `docs` and `policy` successfully on the configured Ubuntu runner. Action refs were resolved through the GitHub API and pinned to full commits: checkout v4 `11d5960a326750d5838078e36cf38b85af677262`, setup-node v4 `49933ea5288caeca8642d1e84afbd3f7d6820020`; Node is pinned to 24.18.0. After the first passing run, the main protection API was applied and read back: strict required checks bound to Actions app 15368, PR/conversation/linear-history requirements, zero solo-mode independent approvals, admin enforcement and no force pushes/deletion. Repository settings were verified squash-only with automatic topic-branch deletion.

The first attempt to use unsupported `gh repo edit --disable-merge-commit` flags made no change; supported boolean `--enable-merge-commit=false`/`--enable-rebase-merge=false` then succeeded. No rejected main push/deletion or other destructive protection test was attempted. Release/tag enforcement, license/signing choices and Rust/SDK/plugin/platform checks remain unverified future work. Follow-up evidence is submitted through a topic PR under the active workflow.
