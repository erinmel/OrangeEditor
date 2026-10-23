# Tooling, environment and research register

## Foundation review reassessment (2026-10-23)

Reviewed the complete user-provided foundation review against source `b287dab` on `refactor/typed-port-descriptor`, current contracts and ADRs. Its Get/Set edge, static connection-resolution, duplicated intrinsic-rule and unsupported-intrinsic diagnostic findings are addressed by the later language-rules slice recorded below. The original review remains unchanged. Its single-run full-validation timing observations were not reproduced and are not NFR-15 evidence: W9 requires semantic-edit distributions and affected-scope counts.

Three temporary Rust probes reproduced gaps before their implementation slices:

- **Converter resolution (FR-44/45/46; ADR-16):** i32→i64 connection planning/application inserts `core/convert`, but combined validation marks the inserted converter incomplete. Core selection reports `E-PROVIDER-MISSING`; package selection reports `E-PROVIDER-UNAVAILABLE`. The node catalogue has no converter-derived contract resolution. Assertions target the converter's identity; the endpoint fixtures do not prove a wholly valid graph.
- **Declaration invalidation (FR-03/12/44/45; ADR-15/16):** after Get(i64)→generic Add persists `T=i64`, `apply_inferred(UpsertVariable(f64))` returned no issues and left the saved generic binding at i64; fresh component inference derived f64. Contract lookup itself was fresh. C2 below addresses persistence with typed-reference discovery in both document states; this probe is historical, not evidence of an executing incorrect type.
- **Definition legality (FR-44; ADR-13/16):** an available pure definition with an exec output passed registry construction and combined validation with no diagnostics. C3 now rejects that declaration at registry publication; broader node-kind/port legality remains open for later bounded checks.

Probes asserted the observed defects and passed (3 tests); they are not correctness acceptance tests. Their temporary test target was removed after reproduction, with source retained in ignored working artifacts. C1, C2 and the first bounded C3 publication rule now have desired-behavior regressions; broader definition legality remains future work. No production Rust, public API, dependency, lockfile or original review was changed by this reassessment. Documentation corrections cover current crate/validator status, fresh connection resolution, type-expression storage, existing explicit-migration policy and the unimplemented ADR-21 domain seam. Trace: FR-03/12/44–47, ADR-06/07/13/15/16/21; no change to selected semantics or delivery scope.

Environment: Windows x86_64 MSVC, active rustup toolchain `1.99.1-x86_64-pc-windows-msvc`; `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, LLVM 23.1.1; `cargo 1.99.1 (5f94df478 2026-08-27)`; Node.js `v24.18.0`. Default workspace features; direct feature selection from `cargo tree --locked -e features --depth 1`: indexmap default/serde, serde default/derive, serde_json default/preserve_order, sha2 default, thiserror default, uuid default/serde; development dependency proptest default. Cargo.lock SHA-256: `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`.

| Executed command | Result |
|---|---|
| `cargo test -p orange-model --test foundation_review_probe --locked --offline -- --nocapture` | 3 observed-defect probes passed; temporary target subsequently removed |
| `cargo test --workspace --locked --offline` | 311 tests passed, 0 failed, after removing the probe target; 0 doctests |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | pass |
| `$env:RUSTDOCFLAGS = '-D warnings'; cargo doc --workspace --no-deps --locked --offline` | pass |
| `node tools/check-cargo-metadata.mjs` | pass; core dependency boundary |
| `git diff --check` | pass |

These checks do not exercise a compiler, runtime, UI, package loader, migration engine or domain extension. No new benchmark, MSRV 1.97.1 run, other-platform run or remote CI verification was performed; no gate is passed. Cargo manifest requirements remain semver ranges, while the recorded lock and locked commands fix this tested resolution.

Warudo was compared using the official [blueprint overview](https://docs.warudo.app/docs/blueprints/overview), [first-blueprint guide](https://docs.warudo.app/docs/blueprints/understanding-blueprints), [node API](https://docs.warudo.app/docs/scripting/api/nodes) and [ports/triggers API](https://docs.warudo.app/docs/scripting/api/ports-and-triggers), retrieved 2026-10-23. This supplies an interaction reference for FR-01/02/04/44/51/52/53 and P-02, not measured usability, performance or version-pinned SDK evidence. Candidate editor refinements remain proposals in ignored working notes; Warudo's scene/integration scope does not expand ADR-22.

Checked 2026-10-20. Distinguish verified environment facts, verified source facts, design decisions and untested dependency candidates. Protocol/platform sources are also linked at the relevant contracts in [05](05-services-and-platforms.md); ADRs identify the decisions they support.

### Declaration-driven inference persistence (C2, 2026-10-23)

`plan_edit` now discovers changed variable declarations through definition-declared typed references in both before and after documents, including nested function bodies. It seeds each consumer and its incident edge endpoints into existing component inference; this catches a generic neighbor even when removal means the referenced Get no longer resolves to a contextual contract. `type_args` remain cache only. Inference updates stay in the declaration edit's one transaction; explicit pins are retained, while missing evidence clears inferred arguments and leaves the generic signature ambiguous. No registry-specific index is persisted. The current whole-document reference scan is a correctness-first implementation cost; no NFR-15 measurement or performance gate is claimed.

Trace: FR-03/12/44/45; ADR-15/16; [11 §5.5](11-graph-language.md) and [12 §3.2](12-graph-data-structures.md). Regression tests cover Get(i64)→generic Add followed by an f64 declaration edit with fresh signature agreement, undo/redo, removal/re-add without stale bindings, function-body references, incompatible explicit pins and an unrelated pinned component that remains unchanged. The unchanged document format, dependencies and lockfile remain as recorded above.

Toolchain: Rust/Cargo 1.99.1, Windows x86_64 MSVC, default workspace features and locked `Cargo.lock`. Commands run from the repository root:

| Executed command | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo test -p orange-model --test inference_persist --locked` | 16 tests passed |
| `cargo test --workspace --locked` | pass; all workspace tests and doctests completed successfully |
| `git diff --check` | pass |

The build emitted Windows incremental-cache hard-link warnings and used its copy fallback. No MSRV 1.97.1 run, other-platform run, remote CI, benchmark or NFR-15 evidence was performed.

## Foundation scope revision (2026-10-22)

ADR-22 records the user-selected usable foundation application, reusable core/UI and general language/trigger/file/network/audio scope. FR-52–55 add configurable input triggers, saved callable graphs, selected-device capture and level/hold events; FR-56 and MQTT remain conditional. OBS/Twitch, browser-canvas products and broader protocol suites are outside delivery. Requirements, architecture, service/package contracts, authoring/data notes, feasibility probes, FR/NFR verification and the ignored local plan were reviewed together. Existing format v1 fields, Rust APIs, dependencies and lockfile are unchanged. New editor/library/analysis encodings and algorithms require contracts and behavior tests at their implementation slice; no schema or runtime compatibility is claimed from this revision.

Local documentation verification on Windows, Node.js **v24.18.0**: `node .artifacts/check-foundation-scope.mjs` **PASS** (temporary local helper: 12 changed Markdown files, 12 parsed JSON examples, 194 existing local link targets, 56 unique FR IDs with priorities/traces, continuous requirement/probe/decision tables, eight probe rows and foundation plan/G5 scope checks). `git diff --check` **PASS**. Self-review removed remaining vendor-specific normative examples, retained retired FR IDs, distinguished callable graphs from event-containing templates, and kept capture/playback separate from virtual-device creation. Temporary editing/check helpers are not product artifacts or committed tooling.

Limits: file targets and JSON syntax were checked, not Markdown anchor resolution or rendered layout. No Rust builds/tests, hook registrations, microphone access, audio analysis/playback, broker probes, plugin installs, UI tests or performance measurements were run for this documentation-only change. Existing implementation evidence below remains historical; no feasibility/runtime/API-freeze or release gate is passed.

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

## Contextual Get/Set validation (2026-10-22)

WU-00/00d.3b projects Get/Set ports from referenced variable declarations through the existing node-scoped template overlay, inference and input validation. Distinct nodes retain their declaration types; workspace synchronous Set reports `E-VARIABLE-SCOPE`. The variable-reference parser is shared with function analysis. No format, dependency, feature or runtime change is introduced. Trace: FR-12/44/45/51, ADR-15/16/17/21/22.

Selected Windows toolchain: Rust `1.99.1 (58cf8f9c3 2026-10-13)`, Cargo `1.99.1 (5f94df478 2026-08-27)`, edition 2024; declared MSRV 1.97.1 was not tested. Cargo.lock SHA-256 remains `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`. `cargo tree -e features --locked --offline` passed; the dependency feature selection is unchanged.

Four initial public-entrypoint tests were written before implementation: the focused test executable compiled and failed on the intended missing contextual behavior, then passed after projection. Four additional integration/freshness/authority tests were written afterwards. Temporarily disabling contextual routing made the Get→generic Add→Set integration regression fail with provider-unavailable diagnostics; restoring routing made all eight focused tests pass. Coverage includes declaration-owned types, required/wrong input, malformed/missing references, unknown types, exact intrinsic authority, workspace scope, stale evidence across declaration edit/undo/redo, unrelated declaration preservation, deterministic diagnostics and read-only bytes/revision/state identity.

Orchestrator review independently inspected the integrated Rust changes and reran `cargo test --workspace --locked --offline` successfully, plus formatting and whitespace checks. No actionable correctness defect was found. A separate static negative-test audit mapped plausible scope/type/freshness regressions to existing assertions; it did not execute additional mutations. Review strengthened Set's existing test to assert its input has neither an output-source marker nor a default value, and reran the eight focused tests. These extra assertions were added after implementation; they are contract checks, not additional red-first evidence. Remote CI and MSRV execution remain unverified for this slice.

| Command | Local result |
|---|---|
| `cargo test -p orange-model --locked --offline --test validation_variables` | PASS: 8 tests |
| `cargo test --workspace --locked --offline` | PASS: 280 tests, 0 failures; 0 doctests |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --no-deps --locked --offline` | PASS; temporary override removed |
| `node tools/check-cargo-metadata.mjs` | PASS |
| `git diff --check` | PASS |

Cargo's incremental-cache hard-link warnings used its copy fallback. No runtime, cross-body visibility, region/cycle/availability, UI, hardware, performance, remote CI, SDK-freeze or release gate is established. Offline connection validation retains its stored/fixed metadata path. Contextual calls and Update/shared-variable operations remain follow-ups.

## Core integrity fixes (2026-10-22)

WU-00/00d integrity fixes address reproduced paste-data corruption, ambiguous provider locks, stale node/function evidence and import resource policy. Paste discovers/remaps only host-declared typed config paths, retaining unknown package data; exact intrinsic identities share one core list. Package/type lock duplicates are structural errors, and provider-key lookup fails closed even for a document created through direct Serde. Imports default to a pre-parse 64 MiB bound and post-parse 100k-node/500k-edge limits, configurable by the embedding host. Node template lookups compare actual current node metadata, provider/digest and contextual declarations; transitive function dependencies invalidate call overlays while unrelated functions and ordinary connection edits remain usable. Function evidence is bound to a content state. Stale evidence produces typed errors and incomplete analysis. Public inference/analysis signatures changed in unreleased v1; no stored-document migration is performed. Trace: FR-02/03/05/14/44/45/47; ADR-15/20/21; I-2/I-4.

Final local verification on Windows x86_64-pc-windows-msvc, Rust `1.99.1 (58cf8f9c3 2026-10-13)`, Cargo `1.99.1 (5f94df478 2026-08-27)`: `cargo test --workspace --locked --offline --quiet` PASS (272 tests, no failures); `cargo fmt --all --check` PASS; `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` PASS; `$env:RUSTDOCFLAGS = '-D warnings'; cargo doc --workspace --no-deps --locked --offline` PASS; `node tools/check-cargo-metadata.mjs` PASS; `git diff --check` PASS. Regression coverage includes original paste corruption, typed/transitive function references, nested literal text, duplicate locks in either order, import boundary errors, stale old node clones, transitive versus unrelated function edits, per-node validator projections and atomic wire insertion with undo/rollback. Windows hard-link cache warnings used Cargo's existing copy fallback.

MSRV remains 1.97.1. No features, dependency, manifest, toolchain or workspace lock changes; Cargo.lock SHA256 remains `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`. No new remote CI/platform/MSRV or benchmark gate is claimed. Removed one redundant incident-edge sort; full scratch-document cloning, index rebuilds, catalogue copy costs and incremental-validator performance remain open. Fingerprint validation scans installed overlays and serializes contextual evidence, so correctness checks are not a performance qualification. Searchable reusable functions, collapse-to-function authoring, contextual call-port generation and complete execution remain subsequent work; structural wire insertion is not type/region approval.

## Node-scoped inference contracts (2026-10-22)

WU-00/00d contextual-projection prerequisite: `TemplateCatalog::insert_node` and `get_for_node` permit distinct contracts for nodes sharing one definition/version. Inference, edit persistence and connection endpoint planning use node-specific lookup; shared lookup and exact-provider specialization keys remain unchanged. Overlays must be rebuilt for a new document/config/provider/lock/body snapshot. Definition/version matching alone is not a freshness proof. Trace: FR-14/44/45/47, ADR-15/20/21.

Local checks on the unchanged Rust 1.99.1/Cargo 1.99.1 Windows toolchain, features and lockfile recorded below: workspace tests (256, zero failures), formatting, warning-denied Clippy and rustdoc, and whitespace checks pass with the same locked/offline commands. Focused regressions exercise two same-definition nodes with independent i64/i32 inference, definition/version fallback and generation changes. Contextual validator projection, function interface conformance and execution are still pending. This prerequisite is based on the function-analysis slice in PR #11; remote qualification of the new changes is not claimed.

## Function analysis and suspension (2026-10-21)

WU-00/00d.3 adds graph-local function analysis to `orange-model`: inferred execution kind, conservative suspension and transitive run-variable write operations. Iterative strongly connected components reject recursion with an actual closed call path; callee-first dependency ordering avoids enumerating all call paths. Missing native/package semantics keep a function and dependent callers incomplete; declared kinds are never evidence for inferred behavior. Exact host intrinsic identities are separate from replaceable definitions such as `core/add`. Typed config references and node/function diagnostic subjects preserve redaction. Trace: FR-14/44/46/47, ADR-15/20/21, I-2/I-4.

`Suspends::{Never, Maybe, Always}` now flows through templates, snapshots, projection, hashing, inference persistence and drift. Absent historical async/iterator metadata stays unknown. Current async/iterator declarations require Maybe or Always; synchronous Never and omission have the same semantic key. A Wait in a function yields a conservative Maybe summary, never an every-path guarantee. Existing unreleased v1 async keys change on explicit re-projection; validators do not migrate stored snapshots.

The user selected core execution before concrete host services and editor integration. This changes sequencing, not broader release requirements. Contextual per-node call/variable ports are the next integration prerequisite: distinct call targets cannot share a definition-only template entry. Regions, data/control cycles, availability, return coverage, state conflicts, incremental invalidation, compiler and runtime remain unimplemented. This analyzer alone does not approve execution. Resolved signature/effect maps are fresh, call-scoped host evidence for the exact immutable document/config/provider/lock snapshot; they are not reusable after edits. External graph-body traversal remains package-host integration work.

Environment and dependency features are unchanged from the validator record below: Rust `1.99.1 (58cf8f9c3 2026-10-13)`, Cargo `1.99.1 (5f94df478 2026-08-27)`, Windows x86_64-pc-windows-msvc, MSRV 1.97.1, no crate features declared, no dependency/manifest/toolchain/lock changes. Cargo.lock SHA-256 remains `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`.

| Command | Final local result |
|---|---|
| `cargo test --workspace --locked --offline --quiet` | PASS: 254 tests, 0 failures; 0 doctests |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `$env:RUSTDOCFLAGS = '-D warnings'; cargo doc --workspace --no-deps --locked --offline` | PASS |
| `node tools/check-cargo-metadata.mjs` | PASS: no UI/engine dependency in core |
| `git diff --check` | PASS |

Regressions cover indirect recursion, a 400-function shared-callee DAG, exact intrinsic versions versus ordinary provider-backed core definitions, incomplete facts without misleading kind errors, typed variable operations, local/workspace exclusions, suspension identity/drift and historical snapshot preservation. Root review corrected traversal and intrinsic-classification errors before final verification. Windows hard-link cache warnings used the existing copy fallback. No runtime, performance target, MSRV, other-OS or remote CI result is claimed for this slice.

## Initial validator passes (2026-10-21)

Implemented WU-00/00d.1 and the node/input foundations of 00d.2 in `orange-model::validation`: `validate_connections`, `validate_nodes`, `validate_inputs`, exact host-supplied `ProviderCatalog` availability and typed diagnostics with immutable host catalogue origins. Trace: FR-05/08/44–46/51, I-1/I-4/I-5, NFR-14. These are read-only analysis passes, not a complete validator or compile approval. Provider metadata is declarative host evidence, not package authentication or installation.

Connection checks cover endpoint/scope/direction/kind errors, cardinality, affine stream fan-out, stable variadic instances and concrete snapshot types. Unavailable metadata is explicit. Node resolution checks exact definition/config/provider versions and locked digest/implementation IDs; it never substitutes a core implementation for a missing selected package. Fresh generic inference uses current host contracts, including non-generic and implicit-port evidence, without seeding from cached `type_args`. Implicit ports are expanded for inference separately from signature projection, preserving semantic snapshot keys. Unresolved neighbours defer dependent generic resolution without suppressing unrelated diagnostics. Input checks reuse the literal codec and preserve shadowed values without decoding them.

Focused regressions cover provider identity and core-list isolation, immutable documents, insertion-order invariance, missing metadata, implicit `error` output inference, signature drift, malformed option metadata and combined template/argument depth exhaustion (`E-TYPE-COMPLEXITY`). New resolution codes and I-4 clarification are in 11 §17 and 12 §3.1/§11. Existing inference specialization fallback is unchanged; validation separately rejects a missing lock. Plans and working handoffs are local ignored artifacts, not committed documentation.

Local environment: Rust `1.99.1 (58cf8f9c3 2026-10-13)`, Cargo `1.99.1 (5f94df478 2026-08-27)`, `x86_64-pc-windows-msvc`, Node.js `v24.18.0`. MSRV remains 1.97.1. `orange-model` has no declared Cargo features; dependency defaults and workspace selections (indexmap/serde, serde/derive, serde_json/preserve_order, uuid/serde) are unchanged. No dependency, manifest, toolchain or lockfile change. Cargo.lock SHA-256: `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`.

| Command | Local result |
|---|---|
| `cargo test --workspace --locked --offline --quiet` | PASS: 230 tests, 0 failures; 0 doctests |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `$env:RUSTDOCFLAGS = '-D warnings'; cargo doc --workspace --no-deps --locked --offline` | PASS |
| `node tools/check-cargo-metadata.mjs` | PASS: core dependency boundary |
| `git diff --check` | PASS |

Windows incremental-cache hard-link failures used Cargo's copy fallback. No new remote CI, MSRV execution, Linux/macOS execution, NFR-15 timing or gate passage is claimed. Host intrinsic projection currently covers definition/config version 1 of Branch, Sequence, Fork, Count, While, Yield, Wait, Break and Continue; other recognized intrinsics remain explicitly unresolved until their consuming slices add metadata. Config-dependent generic intrinsic projection, functions/suspension, regions/zones, cycles, availability, state conflicts, incremental invalidation, authoring integration and edit-cost measurements remain later 00d work. `W-CONST-ERROR` needs 00e folding/evaluation; the W9 runtime storm needs 00f. Source snapshots still support offline edge checks even when exact provider resolution fails.

## Inference replacement review fixes (2026-10-21)

Work: WU-00/00c review corrections, FR-44/45/46. Connection planning now infers generic endpoint types from surviving evidence before selecting a converter: replaced wires are removed and the newly shadowed target literal is excluded on a scratch copy only. Pins and other active evidence still constrain selection. Changed edge deltas contribute both old and new endpoints to inference, including a disconnect/connect that reuses the same ID. Bindings/signatures remain in the originating transaction. Public API shapes, JSON format, dependencies and lockfile are unchanged; 11 §5.5/§6, 12 §3.3 and ADR-16 clarify these existing invariants.

Selected local environment: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, `x86_64-pc-windows-msvc`, Node.js `v24.18.0`. `orange-model` declares no Cargo features; workspace selections remain indexmap/serde, serde/derive, serde_json/preserve_order and uuid/serde with dependency defaults, with no additional command-line features. `rust-toolchain.toml` and `Cargo.lock` are unchanged; lockfile SHA-256 is `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`.

Four added Rust tests cover replacement with a preserved shadowed literal, explicit pins versus surviving edges, a generic source with a stale cache, and reused edge IDs at the same or different targets with exact undo/redo restoration and unrelated-node preservation. An existing rejection test now checks empty, correct and stale caches produce the same no-direct-converter outcome. The connection regressions failed against the pre-fix planner and pass after correction; the earlier review also reproduced the reused-ID failure.

| Executed command | Local result |
|---|---|
| `cargo test --locked -p orange-model --test connection_apply --test inference_persist --test connection_plan --test connection_flow` | PASS: 31 tests |
| `cargo test --workspace --locked` | PASS: 195 tests, 0 failures; 0 doctests |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `$env:RUSTDOCFLAGS = '-D warnings'; cargo doc --workspace --no-deps --locked` | PASS (PowerShell) |
| `node tools/check-cargo-metadata.mjs` | PASS |
| `git diff --check` | PASS |

Windows emitted incremental-cache hard-link warnings and used its copy fallback. Selection adds a scratch document/index and endpoint-component inference before final edit inference; NFR-15 time/memory costs remain unmeasured. No MSRV 1.97.1 run, remote CI, cross-OS run, validator implementation or gate passage is claimed. Work is local on `codex/fix-inference-replacements`, based on `659986d` to preserve the existing documentation/test-policy changes; `origin/dev` was `bafd183` when fetched.

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

## Core quality fixes (2026-10-21)

A code review of `orange-model` reproduced five problems; all are fixed with regression tests:

- Revision exhaustion is checked before any patch (`TxError::RevisionExhausted`), so a loaded `u64::MAX` revision can no longer panic in debug or wrap in release.
- `format_version`, `graph_id`, `type_lock`, `exports` and document `extensions` are read-only accessors, so no caller can change content without a new `state_id` (a previously made connection plan was accepted after such a change).
- Structural derivation memoizes each (type, constraint, depth) proof; 16 records that each hold the previous one twice dropped from about 220 ms to 0.1 ms in a debug sample, and the `MAX_NESTING` budget fires where it did before.
- A type expression nested past the parser budget reports `E-TYPE-COMPLEXITY` instead of `E-TYPE-MISMATCH` (`IdError::TypeTooDeep`).

Specialization metadata moved from `GenericDefinition` to `TemplateCatalog::insert_specializations`, keyed by definition version and exact `ProviderKey` (package, version, locked content digest, implementation ID; `None` is the core implementation), so two providers of one definition can support different arguments. A binding without a lock entry delegates to constraints; reporting it is the validator's job (I-4). Not measured yet: whole-document cloning behind edits (NFR-15), the retention policy for unused signatures and lock entries, and the rule that treats config strings matching an ID as references. Local checks on Rust 1.99.1, Windows x64: fmt, Clippy `-D warnings`, rustdoc `-D warnings`, 199 tests after the provider change; `Cargo.lock` unchanged.

## Additional AI development integrations

**No new MCP/plugin is required to finish this plan.** Web research, filesystem tools and an architecture subagent are available. These development integrations are unrelated to OrangeEditor expansion packages.

Context7 was found in the available plugin directory and suggested as an optional documentation integration. It was **not installed or connected** in this session. It may help retrieve dependency documentation, but Bevy 0.20 coverage/version selection must be verified before using its answers. Release-tag source and versioned local Rustdoc remain authoritative. Installation is optional and must not block planning or justify substituting 0.19 APIs.

ADR-22 removes OBS/Twitch from foundation delivery, including account connectors and live integration tests. Synthetic local fixtures qualify the SDK; any independently scoped integration owns its own later account/fixture requirements. Credentials belong in the vault, never agent prompts or source files. Do not introduce an unrelated cloud project-management system as a prerequisite.

For implementation, verify the Rust toolchain, target linker/native SDKs, language server, formatting/lint tools and platform libraries. These are development prerequisites, not claims that they are installed. Any broker/container, audio diagnostic or security tool is added only for a work unit that uses it.

## UI framework change (2026-10-21)

Bevy was dropped from the project (ADR-02, ADR-21); the editor UI will use egui hosted by eframe, with versions pinned when P-01 runs. The former Bevy 0.20 source register and dossier are in Git history (doc 09 before this change). `tools/check-cargo-metadata.mjs` now rejects UI, window, renderer and engine frameworks (Bevy, egui, eframe, epaint, winit, wgpu, Slint, iced and their subcrates) under graph-core crates instead of pinning Bevy; the Bevy release-feature check left `tools/release.mjs`. Local results on Windows x64, Node 24: `node tools/check-cargo-metadata.mjs --self-test` PASS (8 cases), `node tools/check-cargo-metadata.mjs` PASS, `node --test tools/release.test.mjs` PASS (7 tests).

## Dependency selection ledger

ADR-22 defines the current foundation scope; historical research below is not a release requirement. No framework pin is fixed yet; egui/eframe are pinned in P-01. All implementation dependencies must acquire an exact version, minimum compiler, platform/feature matrix, license, maintenance/security review and contract test at their adopting work unit. A proposed family below is a research candidate, not a dependency approval or compatibility claim.

| Capability | Candidate / decision boundary | Required evidence before adoption |
|---|---|---|
| Presentation | egui + eframe (ADR-02), Slint fallback | P-01/P-02; minimal feature graph, text/accessibility/RTL evidence |
| Async services | Dedicated Tokio host as proposed in architecture | Exact crate/features/MSRV, shutdown/reactor contract, bounded worker bridge |
| Hot expansion execution | Wasmtime/component-model candidate | Exact supported component interface/runtime version, fuel/epoch interruption, memory limit, host-call cancellation and boundary benchmarks |
| Definitions and persistence | Serde-family serialization candidate | Versioned schema contract, malformed input bounds, deterministic output and migration tests |
| HTTP/TLS | Async HTTP client with explicit TLS backend | HTTP/2, proxies, certificates, streaming, cancellation, ALPN and licenses |
| WebSocket; conditional MQTT; deferred AMQP/gRPC | Separate protocol adapters; only WebSocket is foundation baseline (ADR-22) | Protocol version, authentication/stream behavior, async runtime compatibility, bounded payloads and fixtures |
| Audio | Endpoint/decoder/capture services and off-callback level workers | Actual input/output/cable routing, capture permission, decoder formats/licenses, RMS/peak/hold fixtures, device/clock/hotplug and callback audit |
| Input triggers | Native focused/global input adapter behind host event ingress | P-07 registration/conflict/permission/focus/repeat/release/cleanup on each advertised OS/session |
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

### G0 check for 00d (2026-10-21)

Scope: the contracts the 00d validator implements, read against each other and against the 00b/00c code they build on: 11 §3–4 (kinds, connection matrix, cardinality), §7.1–7.2 (regions, availability), §8.1–8.2 (cycles), §10.4 (subscriptions), §11.2 (write conflicts), §14 (functions), §17 (catalog), 12 §3.1 (`SignatureSnapshot`) and §11 (invariants I-1–I-4). The FRs involved (FR-14, FR-44–47) name their owning documents and verification families. This is a documentation check; no validator code or test ran.

| Finding | Resolution |
|---|---|
| 11 §4.2 rejected cells had no code apart from data→exec | All rejected cells are `E-KIND-MISMATCH`; reversed direction is `E-PORT-DIRECTION`; non-identical data types are `E-TYPE-MISMATCH` |
| 11 §7.1 "every exec node in exactly one region" contradicted unreachable nodes | Applies to reachable nodes; unreachable ones get `W-UNREACHABLE` and no region |
| 11 §7.2 treating Fork `all` as a sequential composition made branch A dominate sibling B, contradicting its own table | The composition applies only to sites reached through `completed`; sibling branches never dominate each other |
| 11 §11.2 did not say whether calls count as writes, and listed registers, which cannot be shared across branches | A call writes every run variable its body writes, transitively; callee locals never conflict; a shared register is already `E-REGION-AMBIGUOUS` |
| 11 §14 gave subscriptions in functions `E-ENTRY-IN-FUNCTION`; §10.4 gives `E-SUBSCRIPTION-IN-FUNCTION` | Subscriptions use `E-SUBSCRIPTION-IN-FUNCTION`; entries and Watch keep `E-ENTRY-IN-FUNCTION` |
| 11 §14 Return inside a Fork branch had no code; kind inference had no precedence | `E-REGION-ESCAPE`; the strongest kind present wins and a call counts as its callee's kind |
| 12 §3.1 `SignatureSnapshot` had no `suspends`, so control cycles and function kinds could not be checked for nodes whose package is missing | `suspends` is added to the contract; it lands with the 00d cycle checks and changes async/iterator snapshot keys (format v1 is unreleased) |

Open limits: the call graph through package-provided graph bodies (G-24 "through packages") needs those bodies from the registry, so 00d checks document-local functions first. Control and zone intrinsics have no stored signature; 00d needs a host-owned intrinsic table for their ports and `suspends`. Result: the 00d scope is consistent enough to start its first slice. This passes no G1/G2 or runtime gate, and 00b/00c's own scopes were not rechecked.

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

### Typed ports and registry snapshot (2026-10-23)

Slice 1 replaced the loose port flags with a typed `PortDescriptor` (renamed from `ResolvedPort`): optional, variadic and lazy are typed fields and the variadic bounds are explicit. Slice 2 added `orange_model::catalog` (12 §4, §8 subset): `NodeContract`, `NodeDefinition`, `ProviderDescriptor`, `RegistryBuilder` and the immutable `RegistrySnapshot`, with the built-in control nodes registered as data. Node resolution, inference, validation and connection planning now take one `&RegistrySnapshot` (plus `NodeOverlays` where per-node contracts are used) instead of separate template, provider, type and converter metadata. The former template catalog and provider catalog are removed, and plans record the document state, the snapshot generation and the overlay generation, so overlays changed after planning reject the plan (`StaleOverlays`). The document format and signature keys are unchanged.

Strictness change: a non-object variadic value or a non-boolean optional/lazy value now fails load. A legacy variadic port without `max` is read and saved with `max: null`.

Toolchain: Rust 1.99.1 (MSRV 1.97.1), no dependency or lockfile change. Commands run from the repository root with these results:

| Command | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | pass |

These checks cover the model crate only. They do not establish runtime, UI, package-loading or NFR evidence.

### Converter-node resolution C1 (2026-10-23)

Implemented exact resolution for the `core/convert` host-owned special form. The resolver requires definition/config version 1 and a saved signature, selects only Core when the provider field is absent, or derives an exact package implementation from the unique matching document lock and the existing explicit-choice validator. It requires saved policy maps to equal the normalized complete map; it derives ports from the selected entry and reports signature drift while retaining the fresh resolved projection and marking the node incomplete. Focused tests cover available source/sink providers through plan/apply/combined validation for Core and a package implementation, checked/lossy ports and options, malformed entries/options/versions, missing signature/lock, wrong digest and provider, read-only validation, undo/redo, and pure function summaries. This validates metadata resolution only; it does not execute a converter kernel.

Toolchain: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, active `1.99.1-x86_64-pc-windows-msvc`; declared MSRV 1.97.1; default workspace features and checked-in `Cargo.lock`; no dependency or lockfile change. Commands run from the repository root:

| Command | Result |
|---|---|
| `cargo test -p orange-model --test converter_resolution --locked` | pass (4 tests) |
| `cargo fmt --all --check` | pass |
| `cargo test --workspace --locked` | pass |

Cargo emitted Windows incremental-cache hard-link warnings and copied the affected cache files; builds and tests completed successfully. This work does not establish runtime execution, package loading, performance or NFR evidence.

### Computed contextual contracts (2026-10-23)

Slice 3 declares identity references on the definition: a `NodeDefinition` has `kind`, `suspends`, `type_params`, `ports` (declared, or derived from a variable) and `references` (an RFC 6901 pointer and a variable, event or function kind). `RegistrySnapshot::references` is the single parser of those config fields, with a fixed table for core controls that are not registered yet, and `build` rejects invalid or duplicate pointers. Only definition-declared fields are references, which closes the earlier open question about config strings that merely match an ID.

Get and Set Variable are registered with ports derived from the referenced declaration, and `RegistrySnapshot::contract` computes their contracts on every lookup. `NodeOverlays`, metadata fingerprints and the stale-overlay checks are removed, along with the overlay parameter of inference, connection planning and node validation and `PlanApplyError::StaleOverlays`. A declaration edit or undo takes effect on the next lookup. Paste discovers references from definitions instead of a host schema. Connection planning now resolves implicit ports (`error` is a typed data output, `then` is exec) through the same lookup as inference. `E-METADATA-STALE` remains for function analysis evidence built for another document state. The document format and signature keys are unchanged.

Tests that only exercised overlay freshness were deleted. Tests that injected a contract through an overlay now use Get/Set declarations or registered definitions with the same assertions, and new tests cover the computed Get/Set contracts and the paste gating of reference fields.

Toolchain: Rust 1.99.1 (MSRV 1.97.1), no dependency or lockfile change. Commands run from the repository root:

| Command | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | pass |

These checks cover the model crate only. They do not establish runtime, UI, package-loading or NFR evidence.

### Single validation entry point (2026-10-23)

Slice 4 adds `orange_model::validate(document, registry)`, which returns one `ValidationReport`: sorted, de-duplicated diagnostics, resolved signatures, incomplete nodes, function summaries and incomplete functions. It runs node resolution once and gives that resolution to the input and function passes; connection validation still works from stored signatures. It never changes document content, revision or state identity. The individual passes remain public, and `FunctionEvidence::from_nodes` binds a node resolution to the document for hosts that run passes separately. Diagnostic ordering now covers the whole record, so removing duplicates is deterministic.

Decision: function leaf effects are not derived from the registry. `validate` supplies none, so a package action leaf that is not pure or volatile leaves its function incomplete (`E-FUNCTION-METADATA-UNAVAILABLE`) instead of being treated as effect-free; core Get/Set writes are still summarized. `validate_with_effects` takes host-supplied facts for hosts that have fresh ones. A report is valid only with no error diagnostics and nothing incomplete.

Toolchain: Rust 1.99.1 (MSRV 1.97.1), no dependency or lockfile change. Commands run from the repository root:

| Command | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | pass |

These checks cover the model crate only. They do not establish runtime, UI, package-loading or NFR evidence.

### Language rules and connection resolution (2026-10-23)

Connection validation now checks edges against the node resolution from `validate_nodes` instead of a static copy of the intrinsics; `validate_connections` takes the resolved map, like `validate_inputs`. Get and Set edges are therefore checked against the variable declaration, and a mistyped variable edge reports `E-TYPE-MISMATCH`. A package node that did not resolve still falls back to its stored signature (11 §4.5).

Every incomplete node in a report now has a diagnostic that names it. `E-DEPENDENCY-UNAVAILABLE` marks a node whose check is blocked by an unresolved connected node: a generic node whose component has an unresolved member, or a resolved endpoint of an edge whose other type is unknown. `RegistryBuilder::build` rejects a data or event port without a type and an exec port with one, so a resolved node never has a data port that cannot be compared.

`catalog::language` is the one table of language rules. It lists every intrinsic ID with the kind and suspension function analysis counts it as, the declaration its config references and whether it ends its region, plus the implicit ports each kind gets. It replaces the intrinsic ID list in `document`, the core reference table, the function kind table, the reserved port list and the Break/Continue special case. Inference now also sees that Break and Continue have no `then`. An intrinsic the table lists but this core version does not register (Call, On Event, Switch and others) reports `E-INTRINSIC-UNSUPPORTED` instead of `E-PROVIDER-UNAVAILABLE`.

Toolchain: Rust 1.99.1 (MSRV 1.97.1), no dependency or lockfile change. Commands run from the repository root:

| Command | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | pass |

These checks cover the model crate only. They do not establish runtime, UI, package-loading or NFR evidence.

Root C1 integration verification (2026-10-23): Rust 1.99.1, default features and unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Independently ran cargo test --workspace --locked --offline (315 tests passed), cargo fmt --all --check, cargo clippy --workspace --all-targets --locked --offline -- -D warnings, Rustdoc with warnings denied, dependency-boundary guard and git diff --check; all completed successfully. Windows incremental cache hard-link warnings fell back to copying. No remote CI/MSRV/native hardware or runtime gate result is implied.

Root C2 integrated verification (2026-10-23): independently reviewed invalidation and new tests; cargo test --workspace --locked --offline passed 319 tests. Formatting, Clippy with warnings denied, Rustdoc with warnings denied, dependency-boundary guard and whitespace checks passed. Rust 1.99.1/default features/unchanged lock as C1. No remote/platform/NFR result claimed.

### Pure-definition exec-port publication rule (C3, 2026-10-23)

`RegistryBuilder::build` now rejects an explicitly declared exec port on a `pure` definition with `RegistryError::PureExecutionPort`, carrying the definition ID and offending port ID. This is the first narrow definition-shape rule only: valid pure data definitions, package action outcome ports and host-owned control intrinsics still publish; saved authoring documents remain loadable under their existing codec rules. Remaining kind/source/direction/default/optional/variadic/lazy/suspension legality is not claimed here. Trace: FR-44; ADR-13/16; 11 §3–4 and 12 §4/§8.

The registry already resolves and checks constraint proof metadata; arbitrary package constraint kernels are a separate host/runtime integration and are not executed by `orange-model`. No dependency or lockfile change.

Toolchain: Rust/Cargo 1.99.1, Windows x86_64 MSVC, default workspace features and locked `Cargo.lock`. Commands run from the repository root:

| Command | Result |
|---|---|
| `cargo test -p orange-model catalog::snapshot::tests --locked` | pass (18 tests) |
| `cargo fmt --all --check` | pass |
| `cargo test --workspace --locked` | pass |

No MSRV 1.97.1, other-platform, remote CI, runtime or NFR evidence is claimed.

Root C3 integrated verification (2026-10-23): reviewed registry rule and valid/invalid regressions; independently ran cargo test --workspace --locked --offline (321 tests passed), formatting, Clippy with warnings denied, Rustdoc with warnings denied, dependency-boundary guard and git diff --check successfully. Rust 1.99.1/default features/unchanged lock. This closes the reproduced pure/exec publication defect; other definition legality groups and full validation remain planned.

### C3 registry port publication rules (2026-10-23)

Registry publication now rejects event input ports and package definitions with `lazy: true`, using typed `RegistryError` variants that identify the definition and port. Host intrinsics registered through the crate-private path may declare lazy ports. Missing and explicit `lazy: false` are both non-lazy. Passive `PortDescriptor` serde remains unchanged. The focused snapshot unit fixtures cover accepted event outputs, rejected event inputs, host lazy acceptance, package lazy rejection and package explicit-false acceptance. No suspension/source/default/optional redesign is included.

Toolchain: Rust 1.99.1 via rustup (MSRV 1.97.1), checked-in lockfile unchanged. Validation: `cargo test -p orange-model catalog::snapshot::tests --locked` passed; `cargo fmt --all --check` passed; `cargo test --workspace --locked` passed. Cargo emitted existing Windows hard-link fallback warnings for incremental compilation and one existing `orange-model` warning; no test failed. No Clippy/Rustdoc/MSRV-matrix, runtime, or performance claim is made for this slice.

Root C3 event/lazy integration verification (2026-10-23): reviewed rule branches and corrected the event-output acceptance fixture to a valid action with no data-source annotation. Independently ran cargo test --workspace --locked --offline (323 tests), fmt, Clippy with warnings denied, Rustdoc with warnings denied, dependency-boundary guard and git diff --check successfully. Rust 1.99.1/default features/unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. No full definition or execution gate passed.

### C4 validation coverage and freshness (2026-10-23)

`ValidationAnalysis` retains a private ordinary report, document `state_id`, registry generation and immutable actual coverage. Only `analyze`/`analyze_with_effects` construct it; legacy validate functions extract the same report. Current coverage is exactly node resolution, input values, connections and function summaries. Required checks also include definition/config, body domains, contextual contracts, function restrictions, regions, data/control cycles, availability, state, streams and entry/export policies. Readiness requires current inputs, a pass-local valid report and zero missing checks, so remains false for current code, including empty graphs and unknown domains. Host effects are not authenticated. Positive V11 compile-input issuance remains NOT RUN.

Toolchain: Rust 1.99.1 via rustup (MSRV 1.97.1), checked-in lockfile unchanged. `cargo test -p orange-model --test validation_report --locked` passed (13 tests); `cargo test -p orange-model --doc --locked` passed (the private-field compile-fail example). Full workspace tests, formatting, Clippy and Rustdoc are delegated to root integration and are not claimed here. Existing Windows incremental hard-link fallback warnings appeared; no test failed.

Root C4 integrated verification (2026-10-23): reviewed all source/test/contract changes and incorporated independent review of freshness, coverage and compatibility. Rust 1.99.1 (58cf8f9c3), Cargo 1.99.1, Windows x86_64 MSVC, default workspace features, unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Independently ran `cargo test --workspace --locked --offline`: 327 tests passed, including the compile-fail construction test. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Windows incremental-cache hard-link warnings fell back to copying; no code lint failed. MSRV, remote CI, other OS/native hardware, compiler/runtime and NFR gates were not run or passed by this slice. V11 positive compiler-input acceptance remains NOT RUN.

### C3 declared data-output source requirement (2026-10-23)

At registry publication, every declared data output now requires an explicit `source`; missing values fail with `RegistryError::MissingDataOutputSource` naming its definition and port. All five declared source categories remain accepted, with representative kinds; state-read uses the host intrinsic registration path and iterator declares suspension metadata. Inputs, event outputs and exec outcomes without a source remain accepted by this rule. Persisted signature deserialization is unchanged; source-versus-kind compatibility is not validated by this rule. Contextual generated ports already provide sources. The first root workspace run found seven failing targets because the shared generic-add registry fixture omitted its pure computed output source; the shared definition was corrected to explicit `Computed`, as required by 11 §§3/8.2. A subsequent run correctly reported signature drift in two fixtures claiming available valid add nodes; their saved output descriptors were aligned with that definition. Offline/repair snapshots remain unchanged.

Toolchain: Rust 1.99.1 via rustup (MSRV 1.97.1), checked-in Cargo.lock unchanged. `cargo test -p orange-model --lib catalog::snapshot::tests --locked` passed (22 tests); `cargo fmt --all` and `cargo fmt --all --check` passed. Existing Windows incremental-cache hard-link warnings fell back to copying. Workspace tests, Clippy and Rustdoc are left for root integration; no broad gate is claimed here.

Root C3 source-presence integration verification (2026-10-23): independently reviewed the complete implementation, representative fixtures and contracts, with a separate source review. `cargo test --workspace --locked --offline` passed 329 tests after the three targeted fixture corrections above. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Rust/Cargo 1.99.1, default features and unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4, Windows x86_64 MSVC. Incremental hard-link warnings fell back to copying. Other source-position, suspension, default/optional/variadic checks and full validation remain incomplete; no MSRV, remote CI, native-platform, runtime or NFR gate is claimed.

### C3 suspension metadata publication (2026-10-23)

Registry publication now applies the suspension kind/value matrix before declared or contextual variable ports: async and iterator definitions require `maybe` or `always`; pure, volatile, action and entry definitions accept omission or `never`. Typed `RegistryError` variants distinguish missing and incompatible metadata. A crate-private classifier is shared with node and function validation, preserving the existing `E-SUSPENDS-MISSING` and `E-SUSPENDS-INVALID` diagnostics for passive/legacy snapshots. Entry-in-function diagnostic precedence remains unchanged. Tests cover valid and rejected metadata, declared and contextual definitions, built-in registry publication, full valid node resolution and read-only document bytes/revision/state; no passive DTO serde changes were made.

Toolchain: Rust 1.99.1 via rustup (MSRV 1.97.1), checked-in Cargo.lock unchanged. `cargo test -p orange-model --lib catalog::snapshot::tests --locked` passed (23 tests); `cargo test -p orange-model --test suspension_validation --locked` passed (2 tests); `cargo test -p orange-model --test function_analysis --locked` passed (15 tests); `cargo fmt --all --check` passed. Windows incremental hard-link warnings fell back to copying. Workspace tests, Clippy and Rustdoc are left for root integration; no broad gate is claimed here.

### C3 variadic bound publication (2026-10-23)

Registry publication rejects declared variadic descriptors with a finite `max` below `min`, returning `RegistryError::InvalidVariadicBounds` with definition, port and both limits. Equal and zero bounds remain valid; `max: None` remains unbounded. The rule runs for package and host intrinsic definitions. Existing built-in sequence/fork definitions still publish. Passive descriptor serde and dynamic instance-count behavior are unchanged.

Toolchain: Rust 1.99.1 via rustup (MSRV 1.97.1), checked-in Cargo.lock unchanged. `cargo test -p orange-model --lib catalog::snapshot::tests --locked` passed (24 tests); `cargo fmt --all --check` passed. Windows incremental hard-link warning fell back to copying. Workspace tests, Clippy and Rustdoc are left for root integration; no broad gate is claimed here.

Root C3 suspension integration verification (2026-10-23): preserved the newer config/check-group/iterator-fixture/converter refactors through f123c65. Root and an independent reviewer checked the shared classifier, contextual-port bypass prevention, legacy function diagnostics and registration-boundary test rewrite. `cargo test --workspace --locked --offline` passed 331 tests including the existing compile-fail evidence test. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Toolchain: Rust 1.99.1 (58cf8f9c3), Cargo 1.99.1, Windows x86_64 MSVC, default features, unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Incremental-cache hard-link warnings fell back to copying. No new dependency, MSRV/remote CI/other-OS/native hardware, compiler/runtime or NFR result is claimed. Validation coverage remains partial.

Root C3 variadic-bounds integration verification (2026-10-23): root and independent review confirmed the finite-range rule, preservation of existing shape-error precedence, typed error context and unchanged passive deserialization. `cargo test --workspace --locked --offline` passed 332 tests. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Rust/Cargo 1.99.1, Windows x86_64 MSVC, default features and unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Existing incremental hard-link fallback warnings did not fail checks. This does not establish dynamic variadic-count enforcement, full definition/config validation, MSRV/other-platform/native hardware, compiler/runtime or NFR evidence. C4 coverage remains partial.

### V1a contextual variable Update Add/Sub (2026-10-23)

`core/update-variable@1` now resolves fresh local/run variable contracts for Add/Sub, requires exact numeric-operation capability evidence, and projects the required value input and execution-sourced old/new outputs on `then`. Scope, malformed/unknown operation, recognized unsupported operation, missing capability proof, and invalid type remain distinct diagnostics. The shared operation parser preserves the public `WriteOperation` paths while accepting the nine stable update tokens and excluding Set. Function summaries recheck the contextual contract before recording an Update effect; package preflight retains typed contract diagnostics. This does not implement other Update operations, runtime mutation, shared workspace writes, or source availability/region analysis.

Focused evidence on Rust 1.99.1 via rustup, default features and unchanged lockfile: `cargo test -p orange-model --test validation_variables --locked` passed (12 tests); `cargo test -p orange-model --test function_intrinsic_effects --locked` passed (6); `cargo test -p orange-model --lib update_parser_recognizes_all_v1_update_tokens_and_rejects_set --locked` and `cargo test -p orange-model --lib unsupported_update_diagnostic_has_stable_empty_argument_envelope --locked` each passed (1). `cargo fmt --all --check` was run during integration and reported formatting differences in the concurrently edited inference-persistence regression; this slice does not claim formatting passed. Root owns that test and the workspace integration checks. Windows incremental compilation warned that hard links were unavailable and copied cache files instead. No workspace, Clippy, Rustdoc, MSRV, runtime or NFR gate is claimed here.

Root V1a integration verification (2026-10-23): reviewed the full core diff and disjoint package tests with an independent reviewer. The root-owned Update-output inference regression preserves the existing Get regressions and checks declaration-only edits in root/function bodies, atomic undo/redo, removal/re-add, workspace-scope invalidation/undo, unrelated components and absence of persisted intrinsic signatures. `cargo test --workspace --locked --offline` passed 345 tests, including the compile-fail analysis-evidence test. After strengthening raw function-evidence rejection to include a type with no Add proof, `cargo test -p orange-model --test function_intrinsic_effects --locked --offline` passed all 6 tests. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Package-focused tests passed 4; final variable-validation target contains 13 passing tests. Rust 1.99.1 (58cf8f9c3), Cargo 1.99.1, Windows x86_64 MSVC, default features, unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Existing incremental-cache hard-link warnings fell back to copying. No package kernel was executed and no runtime mutation, other Update operations, complete region/availability/conflict validation, MSRV/remote CI/native-platform or NFR gate is claimed. Compiler readiness remains false with partial coverage.
