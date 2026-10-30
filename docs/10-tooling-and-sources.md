# Tooling, environment and research register

Current implementation checkpoint (2026-10-23): supported control documents run six covered categories (body domains, node resolution, input values, connections, function summaries and data cycles); nine required categories remain missing. Unsupported domains stop combined control analysis after the domain prerequisite, so that invocation records one covered category and fourteen unexecuted checks. Compiler readiness remains false. Earlier coverage counts below are historical results at their recorded commits; the latest verification entries supersede their status statements.

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

### V1b contextual variable Update Min/Max (2026-10-23)

Min/Max now share the V1a fresh local/run Update port contract and require an exact `Ord` proof. Core integer, bool, string, duration and timestamp types are accepted; raw f32/f64 are rejected because no total-order float policy is adopted. Exact nominal package Ord evidence is accepted through immutable registry lookup, without executing package code or a comparator. The normative future execution rule is lower/higher selection with ties retaining the old value; this slice only validates metadata and does not execute it. Add/Sub proofs remain exact numeric-operation requests; other Update families and all runtime, region, availability and compiler-readiness work remain incomplete.

Focused checks on Rust 1.99.1 via rustup, default features and unchanged lockfile: `cargo test -p orange-model --test validation_variables --locked` passed (14 tests), and `cargo test -p orange-model --test function_intrinsic_effects --locked` passed (6 tests). Package Ord-only/broad-Num proof tests are maintained separately. Formatting is left for root integration because another writer owns those package tests; no workspace, Clippy or Rustdoc claim is made here. Windows incremental compilation reported hard-link fallback to file copying.

Root V1b integration verification (2026-10-23): root reviewed the source, core and package tests with an independent reviewer. Review found that retaining old on comparator ties does not establish package Min/Max commutativity; 11 §11.2 and the ADR clarification now limit that Fork exception to language-owned ordered scalar types. Conflict analysis itself remains unimplemented. Root also added raw function-evidence rejection for Min on f64 and preserved the earlier Num-only Add rejection. `cargo test --workspace --locked --offline` passed 347 tests; after a test-only enum variant rename, `cargo test -p orange-model --test validation_update_package --locked --offline` passed 5. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. The first integrated Clippy run rejected redundant enum variant suffixes in the package fixture; the final run passed after removing them. Rust 1.99.1 (58cf8f9c3), Cargo 1.99.1, Windows x86_64 MSVC, default features, unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Incremental hard-link warnings fell back to copying. No comparator/kernel execution, runtime mutation, complete validation, compiler readiness, MSRV, other-OS/native or NFR gate is claimed.

### V1c contextual variable Update Compare-and-set (2026-10-23)

`compare-and-set` now resolves local/run variable contracts only with exact Eq capability evidence. Its required inputs are expected/value of the declaration type; old/new retain that type and exchanged is always core/bool@1. All results are execution-sourced on then. This metadata slice does not execute equality, compare/exchange mutation, notification, initialization, availability, conflict or atomicity rules; match/mismatch behavior is specified for future execution in 11 §11.1. Other Update families (And/Or/Xor/Append), shared writes and compiler/runtime remain incomplete. No dependencies, toolchain, feature selection or lockfile changed.

Root integration reviewed source, independently assigned package/inference tests and an independent read-only contract review. Regressions cover scalar/aggregate Eq acceptance and float/nested-float/stream rejection, exact package Eq versus Num/Add/Ord evidence, required operands, literal and connection types, fixed boolean result, transitive function write summaries and forged raw-evidence rejection. Declaration edits re-infer old/new consumers in root/function bodies, preserve the boolean result for eligible types, remove the entire contract when Eq is lost, clear stale generic arguments and undo atomically. Validation does not mutate documents or persist intrinsic signatures. Reviewer-found wording issues in README and the earlier ADR clarification were corrected.

`cargo test --workspace --locked --offline` passed 353 tests including the compile-fail analysis-evidence check. Root focused checks `cargo test -p orange-model --test validation_variables --test function_intrinsic_effects --locked --offline` passed 17 and 6 tests; after strengthening connection and capability-loss cases, `cargo test -p orange-model --test inference_persist --test validation_variables --locked --offline` passed 18 and 17 tests. Independent developers ran `cargo test -p orange-model --test validation_update_package` (7 passed) and `cargo test -p orange-model --test inference_persist` (18 passed); the initial package run preceded the implementation and its two CAS tests failed with UpdateUnsupported, then passed after implementation. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Rust 1.99.1 (58cf8f9c3), Cargo 1.99.1, Windows x86_64 MSVC, default features, unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Incremental hard-link warnings fell back to copying. No MSRV 1.97.1, remote CI, Linux/macOS, native hardware, performance/NFR or full compiler-readiness gate is claimed; C4 coverage remains four implemented categories and eleven missing categories.

### V1d logical/bitwise Update metadata (2026-10-23)

And/Or/Xor now accept exactly core bool/i32/i64/u32/u64 with required value and execution-sourced old/new ports on then. Bool uses logical and integers fixed-width bitwise semantics in the future execution contract; no kernel is executed. Root review corrected the developer's Boolean XOR exclusion and preserved unsupported Append precedence before type validation; an independent reviewer confirmed the corrected source and tests. Package Num/Add/Ord/Eq declarations never authorize these operations on nominal package types. Root `cargo test -p orange-model --test validation_variables --test validation_update_package --test function_intrinsic_effects --locked --offline` passed 18, 8 and 6 tests. `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` passed on the integrated checkout, which also contained the separately reviewed variadic-count slice. Formatting applied; complete workspace/Rustdoc integration evidence follows with that slice. Rust/Cargo 1.99.1, Windows x86_64 MSVC, default features and unchanged lockfile SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Existing hard-link warnings fell back to copying. No runtime, native-platform, MSRV, performance or compiler-readiness gate is claimed.

### Dynamic variadic instance counts (2026-10-23)

Resolved variadic ports now enforce inclusive declared instance bounds using persisted order length, including output/control ports; absent state is zero. Edges, literal usage, sparse IDs and next counters do not change cardinality. Each violation reports E-VARIADIC-COUNT at node/base port with a safe namespaced empty-argument message. No authoring state is rewritten; unresolved nodes await primary metadata resolution. This extends input-validation behavior without completing a missing C4 category. Root reviewed the implementation and independent review; added edge/literal independence, absent zero-width exec and exec overflow cases, and removed a sort-order assumption from the test. `cargo test -p orange-model --test validation_variadics --locked --offline` passed 4 tests and `cargo test -p orange-model --lib variadic_count_diagnostic_has_stable_empty_argument_envelope --locked --offline` passed 1. Complete workspace integration evidence is recorded after the concurrent Append unit. Rust/Cargo 1.99.1, Windows x86_64 MSVC, default features, unchanged lockfile; existing incremental hard-link warnings fell back to copying. No runtime, native, MSRV, performance or complete validation gate is claimed.

### V1e Append and input metadata publication (2026-10-23)

Append now validates exact string/bytes/list same-type concatenation contracts; registered nominal package element types inside lists are accepted without Eq/Num/Ord, while package nominal outer types gain no concatenation kernel from unrelated proofs. Scalar/map/nested-unknown rejections, required literals/ports and transitive Append function writes are tested. All nine Update tokens now have metadata contracts; no operation is executed. Publication additionally confines defaults/true optional flags to data inputs and requires a well-formed exact Option wrapper for true optional. False flags remain inert, malformed syntax/name resolution/default payload validation remain separate, and invalid passive snapshots still round-trip for repair. Both units received root review and independent review; root added package-element/outer-boundary, transitive-write, valid-default, false-output, unknown-inner and passive-invalid-snapshot cases.

Combined integration of V1d, variadic counts, V1e and input metadata: `cargo test --workspace --locked --offline` passed 367 tests including the compile-fail analysis-evidence check. After strengthening the four input-metadata cases, `cargo test -p orange-model --test catalog_input_metadata --locked --offline` passed 4 tests. Root `cargo test -p orange-model --test validation_variables --test validation_update_package --test function_intrinsic_effects --locked --offline` passed 19, 9 and 7 tests. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed on the integrated checkout. Initial root V1d focused-test redirection used a nonexistent log parent and did not start Cargo; it was corrected and rerun with an explicit exit-code capture before evidence was claimed. Rust/Cargo 1.99.1, Windows x86_64 MSVC, default features, unchanged lockfile SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4; incremental hard-link warnings fell back to copying. This does not establish general storage/stream restrictions, default payload correctness at publication, initialization, allocation/runtime semantics, compiler readiness, MSRV/other-platform/native or NFR gates. C4 remains four categories covered and eleven missing.

### V2 shared variable metadata (2026-10-23)

Exact core/set-shared and core/update-shared now derive workspace-only Async/Maybe contracts. Set requires the declaration-typed value; Update shares the same nine-operation proof and port projector as synchronous Update. Local/run shared targets get a separate redacted E-SHARED-VARIABLE-SCOPE; sync workspace writes retain their existing diagnostic. The central language table owns typed variable-access facts, tested against the registered definitions. Function analysis rechecks fresh contracts, validates SetShared type evidence, propagates Async/Maybe through calls, ignores forged core leaf facts and excludes workspace writes from run_writes. Package definitions may reuse the metadata shape when Async but require explicit trusted leaf effects; metadata shape grants no core execution authority. Root review corrected the initial unrequested host-only shape restriction; independent cross-review of catalog and function sides found no remaining issues.

`cargo test --workspace --locked --offline` passed 377 tests including the compile-fail analysis-evidence check. Root `cargo test -p orange-model --test validation_shared_variables --test function_intrinsic_effects --test validation_update_package --locked --offline` passed 6, 9 and 10 tests; `cargo test -p orange-model --lib shared_variable_scope_has_stable_redacted_message --locked --offline` passed 1. Independent review also ran `cargo test -p orange-model --lib registered_intrinsics_agree_with_the_table --locked --offline` (1 passed). `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. An earlier root diagnostic run overlapped incomplete catalog edits and failed on an array length/nonexhaustive match; the completed source fixed those before final checks. An incompatible-operation negative fixture initially used valid Add/i64 and was corrected to Add/bool before acceptance. Rust/Cargo 1.99.1, Windows x86_64 MSVC, default features, unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Incremental hard-link warnings fell back to copying. This does not implement a workspace host actor, atomic mutation, publish-before-then/cancellation/persistence/storage guarantees, compiler/runtime, MSRV, other-platform/native or performance qualification. C4 remains four covered categories and eleven missing categories.

### Port-level data-dependency cycles (2026-10-23)

Combined validation now constructs the formal D graph separately in every body, using fresh resolved metadata, valid same-body data edges and concrete persisted variadic instances. Every data input depends into computed outputs only; state/execution/payload/iteration outputs break feedback. Iterative SCC and witness traversals return one deterministic closed PortRef path per cyclic component. E-CYCLE-DATA has a safe namespaced empty-argument message; paths are structured in the pass and ValidationReport.data_cycle_paths for host/UI presentation. Unresolved dependency metadata remains incomplete, existing primary errors explain it without duplicate derivative messages, and no stored signature fallback supplies dependency facts. DataCycles is now the fifth covered C4 category, with ten still missing and compilation readiness false.

Root/independent review caught an initial implicit-port lookup bug and replaced a duplicate lookup with the shared fresh-port resolver. Root strengthened the regression so implicit in/then truly remain absent from declared ports; added a mixed state/computed sibling accept/reject pair, multiple SCC witnesses, real orphan sparse-instance connection rejection, real-registry self-cycle rejection and legal Get→computed→Set feedback. Edge-error and diagnosed-node sets avoid repeated diagnostic scans; this is not a performance measurement. `cargo test --workspace --locked --offline` passed 391 tests including compile-fail. After final sparse-orphan test strengthening, `cargo test -p orange-model --test validation_data_cycles --locked --offline` passed 11 tests; combined focused `cargo test -p orange-model --test validation_data_cycles --test validation_report --locked --offline` passed 11 and 15. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Rust/Cargo 1.99.1, Windows x86_64 MSVC, default features, unchanged lockfile SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4; existing hard-link warnings fell back to copying. No control-cycle/region/availability/state/stream, compiler/runtime, MSRV, other-OS/native or NFR gate is claimed by this check.

### Default literal publication validation (2026-10-23)

Registry publication now validates defaults after established descriptor/input checks and only for concrete declared types validated by the current TypeRegistry. Exact type mismatch yields DefaultType; canonical codec/schema rejection yields DefaultLiteral, both carrying definition/port IDs without payload text. Generic, malformed and unresolved declarations defer checking; passive metadata still round-trips. Root and an independent reviewer inspected source and tests; root strengthened schema regressions and reused the same immutable schema registry across cases. Nine focused cases cover canonical scalar values, mismatch/payload precedence and redaction, record/enum schemas, unsupported handle/stream/result/error literals, generic/unknown deferral and descriptor precedence.

Root `cargo test --workspace --locked --offline` passed 396 tests, including the compile-fail evidence test. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Ignored logs: default-workspace.log, default-clippy.log, default-doc.log. Rust 1.99.1 (58cf8f9c3), Cargo 1.99.1, Windows x86_64 MSVC, default features; Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4 unchanged. Incremental hard-link warnings fell back to copying. No dependency, runtime, complete DefinitionAndConfig, compiler-readiness, MSRV 1.97.1, other-OS/native or NFR gate is claimed. The next availability helper was unreferenced and its tests remained ignored during this acceptance run.

### Execution-output availability publication (2026-10-23)

Publication now restricts available_on to execution-sourced data outputs, checks nonempty distinct lists against declared or implicit exec outcomes, and validates omitted metadata as then. Custom outcome declarations remove implicit then. Inputs and data/error/event outputs do not become outcomes. No source-to-kind exclusivity matrix or value-dominance analysis is implied. Root reviewed the developer helper and caught the original omitted-default gap and accidental implicit in/error inclusion before integration; both were corrected. An independent read-only reviewer found no remaining semantic blocker. Root replaced an unnecessary provider fixture, activated and strengthened eight focused tests for failed/cancelled/body/completed outcomes, source categories, unavailable default then, reserved-port precedence and passive repairability.

`cargo test --workspace --locked --offline` passed 404 tests, including compile-fail. `cargo test -p orange-model --test catalog_availability --locked --offline` passed 8. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Ignored logs: availability-focused-final.log, availability-workspace.log, availability-clippy.log, availability-doc.log. Rust/Cargo 1.99.1, Windows x86_64 MSVC, default features; Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4 unchanged. Existing incremental hard-link warnings fell back to copying. No dependency change, runtime execution, complete DefinitionAndConfig/ValueAvailability, compiler readiness, MSRV, Linux/macOS/native or NFR gate is claimed.

### Body-domain prerequisite and invocation coverage (2026-10-23)

The core checks domain metadata in graph/function extension maps without adding a second domain or typed registration framework. Absence or exact control is accepted; all other JSON is preserved and diagnosed per body, including empty bodies. Combined analysis stops before control passes if any body is unsupported: no projections, cycle witnesses or function summaries are produced, all nodes/functions remain incomplete with safe blocking subjects, and the private coverage evidence records only BodyDomains. Supported control documents run six categories with nine required categories missing. Coverage and freshness remain immutable and bound to document state plus registry generation. Standalone structural inspection remains available; compilation readiness remains false.

Root orchestrated a domain developer and independent reviewer, reviewed the flattened wire storage, and added combined known-intrinsic gating, wrapper parity, redaction, no-mutation and domain edit/undo coverage/freshness tests. Review corrected a prior foreign-domain assertion that expected an unimplemented check rather than an executed failed prerequisite. The first focused run exposed only a fixture comparison missing defaulted interface arrays; root fixed the fixture without changing codec behavior. A later added undo test needed an explicit FunctionId annotation; the final focused run passed 123 library tests, 6 domain tests and 15 report tests (`cargo test -p orange-model --test validation_domains --test validation_report --lib --locked --offline`). No unknown-field data loss was found: function body metadata is flattened into FunctionDecl.extensions.

`cargo test --workspace --locked --offline` passed 411 tests including compile-fail. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Ignored logs: domains-integration-final.log, domains-workspace.log, domains-clippy.log, domains-doc.log. Rust/Cargo 1.99.1, Windows x86_64 MSVC, default features; Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4 unchanged. Existing incremental hard-link warnings fell back to copying. No dependency change, foreign-domain execution support, complete validation, compiler/runtime, MSRV 1.97.1, other-OS/native, UI or NFR gate is claimed.

### V3a call-site type interfaces and invalidation (2026-10-23)

`catalog::call_interface` projects exact graph-local call targets into declared parameters/constraints and data ports, without reading the declared function kind or producing kind, suspension, effects, provenance or an executable signature. The resident call and target require supported control domains; typed references, descriptor direction/kind/type, unique non-reserved IDs and declared parameters are checked. Input provenance and invalid output initialization metadata fail closed; output provenance is removed from the type-only result. Unknown concrete type validity stays with inference; this DTO is not accepted execution evidence. Validation reports specific missing/unknown targets and leaves even a valid type-only call unresolved pending body/interface conformance and execution contracts.

Inference instantiates each call independently in its node scope and shares repeated/nested parameters within that call. Pins, literals, input/output edge context and constraints use the existing solver; persisted arguments never supply evidence. Persistence writes call arguments only, preserving historical signature data and pins. One typed before/after reference scan handles variable/function declaration edits and removal, re-inferring callers and generic neighbours in root and function bodies. Root strengthened the stale-signature regression and reproduced a real gap: direct SetConfig target loss cleared the call but left a generic consumer cache stale. The shared neighbour-seeding helper now also visits changed nodes in both document states, and the regression passed after the fix. No fake call NodeContract or per-marker universal-parameter inference was introduced.

Root orchestrated separate catalog and inference developers plus independent read-only review, integrated validation/error mappings and public exports, and updated FR-45, contracts and ADR-15/16 clarification. Review rejected silently clearing provenance from malformed input declarations; root corrected that rule and the positive fixture's optional/constraint metadata. Root added nested/output-only inference, conflict, pure-plan and unrelated-call preservation checks. Initial developer test fixtures needed typed signature/config values and state-correct undo expectations; they were corrected before acceptance. Final root focused suites passed catalog interface6, call inference7, validation calls3 and combined report15. Existing persistence18 and component inference9 also passed.

`cargo test --workspace --locked --offline` passed 427 tests including compile-fail. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline`, `node tools/check-cargo-metadata.mjs` and `git diff --check` passed. Ignored logs include call-target-loss-repro.log (expected failing pre-fix regression), call-inference-root-final.log, call-focused-final.log, call-workspace.log, call-clippy.log and call-doc.log. Rust 1.99.1 (58cf8f9c3), Cargo 1.99.1, Windows x86_64 MSVC, default features; unchanged Cargo.lock SHA-256 39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4. Existing incremental hard-link warnings fell back to copying. Normal validation coverage remains six categories with nine missing; no ContextualContracts completion, accepted executable calls, body/return conformance, compiler/runtime, native/other-OS/MSRV, UI, performance or NFR gate is claimed.

### Edit cost by document size (2026-10-23)

`plan_edit` cloned the document and rebuilt `DocIndex` per edit. `apply_inferred_indexed` now plans by applying the edit to the live document, reading the result and undoing it, and updates a caller-kept index; the registry returns declared contracts by borrow and looks definitions up without cloning ids. `apply_inferred` and `plan_edit` keep their results but still build an index per call.

Measured once with `cargo test --release -p orange-model --test perf_edit_baseline -- --ignored --nocapture` (ignored test, best of 5, chain of `org.test/add` nodes, one `Connect`; Windows x86_64, Rust 1.99.1). This is not NFR-15 qualification.

| Case (10,000 nodes) | Before | After |
|---|---|---|
| `apply_inferred`, edit joins a small isolated component | 16.6 ms | 9.5 ms (index build) |
| `apply_inferred_indexed`, same edit | n/a | 0.10 ms |
| `apply_inferred` / `_indexed`, edit extends the 10,000-node component | 305 ms | 196 / 186 ms |
| `validate` | 220 ms | 119 ms |

At 1,000 nodes the isolated edit is 0.04 ms indexed. An edit that touches a 10,000-node generic component still re-solves the whole component; incremental solving and the O(document) declaration-consumer scan remain open, so NFR-15 is not met for such edits.

### Validation cost by call-chain depth (2026-10-23)

Review of PR 14 found that call acceptance reran every validation pass over the whole document once per call-chain level, and that `GraphDocument::locate_node` and `node` scanned every body, so passes cost O(nodes × functions). A depth-200 chain took 2.7 s in a one-off probe at `a843f88`. Functions are now accepted callee first, each body checked once, before one whole-document round (12 §3.2 coverage contract), and the document keeps a node-to-body lookup current through patches.

Measured with `cargo test --release -p orange-model --test perf_edit_baseline -- --ignored --nocapture --test-threads=1` (best of 5; `depth` pure functions, each calling the next; Windows x86_64, Rust 1.99.1). This is not NFR-15 qualification.

| Depth (nodes) | Callee-first only | Plus node lookup |
|---|---|---|
| 200 (599) | 31 ms | 11 ms |
| 1,000 (2,999) | 543 ms | 61 ms |
| 4,000 (11,999) | 8.1 s | 289 ms |

The same run measured the 10,000-node `validate` at 142 ms with the old lookup and 148 ms with the new one, which builds the lookup on first use; the indexed isolated edit stays at 0.10 ms once a document has been looked up. `locate_edge` and `edge` still scan bodies, and `DocIndex::update` uses them per changed edge.

### Security and performance verification (2026-10-23)

Repository review at `65747d2880d2a34056077b299e88c38f763e0ca8`, branch `feat/foundation-core-readiness`, [PR 14](https://github.com/erinmel/OrangeEditor/pull/14). Scope included the model's import/codec, types/constraints, registry/provider identities, inference, transactions/indexes, validation passes and tests, plus dependency inputs, CI/release helpers and product contracts beyond the PR diff. No compiler, executor, concrete host services, package loader, editor or audio implementation exists to qualify.

Corrected existing behavior (FR-12/44/46, NFR-05/08/15; 08/11/12 updated together):

- Sequence availability used counter-sorted ports instead of saved instance order; sparse reordered steps could both reject available values and accept unavailable ones. A regression checks both directions.
- Config numeric bounds rounded integers to f64. For example, 9007199254740993 passed a maximum of 9007199254740992. Comparisons now retain integer precision and order mixed finite binary64/integer values correctly; regressions include signed/unsigned extremes and fractions. Diagnostics expose no config values.
- Export analysis followed a register's write inputs when exporting its stored `current` value. State outputs now stop pull dependency traversal, including behind a computed node; activation-dependent computed exports still fail. Root incoming data adjacency replaces a full edge scan per visited node.
- Availability retained one large reachable-vertex set per producer/outcome group, with quadratic memory even on an action/data chain. Uses are grouped and each set is released after answering its consumers. The control-flow algorithm and Fork join semantics are unchanged; CPU and query-count limits below remain open.
- Resource storage checks inspected only textual type arguments, so nominal record fields and enum payloads could hide streams or handles. Cycle-safe schema traversal now covers those paths and register checks use the fresh resolved cell type instead of persisted type arguments. Regressions cover recursive records, enum payloads, resource-free cycles and a stale register cache.
- Direct workspace dependencies now use exact `=version` requirements, enforcing their existing selected pins without upgrading or changing Cargo.lock. Corrected stale config-schema Rustdoc and the unqualified claim that callee-first acceptance made every validation pass linear.

Toolchain and inputs: Rust 1.99.1 (`58cf8f9c3`, LLVM 23.1.1), Cargo 1.99.1 (`5f94df478`), Windows x86_64 MSVC; Node 24.18.0. Workspace default features, no extra feature flags; dependency features include indexmap/serde, serde/derive, serde_json/preserve_order and uuid/serde. Cargo.lock SHA-256 remains `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`.

Dependency security: built the CI-selected cargo-deny 0.20.2 with `cargo install --locked --root .artifacts/audit-tools cargo-deny --version 0.20.2`, then ran `.artifacts/audit-tools/bin/cargo-deny.exe --locked check advisories bans licenses sources`. All four checks passed after refreshing [RustSec](https://github.com/RustSec/advisory-db) online to `84d943a6deeb8cc6c6f632531aaf4f1c62275239` (database commit 2026-10-23 16:42:16 +02:00). This is evidence for that snapshot, not future advisories or application security certification. Initial sandbox DNS access failed; the authorized network run succeeded. Targeted private-key, GitHub/OpenAI-token and AWS-access-key pattern scans found no matches in tracked files or the 107 commits reachable through local refs. This was a pattern scan, not credential validation or an exhaustive entropy scan. The model forbids unsafe Rust and currently has no production file/network/process-execution sink; CI/release permission and artifact paths were reviewed without changing them. OS vault, network capability, worker isolation and release-signing enforcement remain later executable evidence.

The Sequence, numeric-bound and register-export regressions failed against the original behavior before fixes. Final checks:

- `cargo test --workspace --locked`: 514 passed (including compile-fail evidence), zero failed; four timing tests ignored by default.
- `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --no-deps --locked`: passed. Existing OneDrive incremental-cache hard-link warnings fell back to copying; they were not lint failures.
- `node tools/check-cargo-metadata.mjs --self-test` (8 cases), `node tools/check-cargo-metadata.mjs`, `node --test tools/release.test.mjs` (7 tests), `node tools/release.mjs check`, `git diff --check`: passed. Release configuration remains disabled.
- `cargo test --release -p orange-model --locked --test perf_edit_baseline -- --ignored --nocapture --test-threads=1`: all four timing probes passed. These report best of five, not p95/p99 qualification.

Performance comparison, same machine and corrected fixtures:

| Workload | Before correction | After correction |
|---|---|---|
| Export one 10,000-node pure chain, standalone pass | 43.633 ms | 5.452 ms |
| Availability, 2,000-action/data chain, standalone pass | 262.947 ms | 203.697 ms |
| Peak process working set for the availability probe (200/1,000/2,000 actions) | 241.1 MiB | 35.4 MiB |

The availability baseline binary used HEAD's original `availability.rs` with the new identical probe; the edited source was backed up and restored in a `finally` block before subsequent builds. Both probe processes ran the same filtered test with `--ignored --nocapture --test-threads=1`. Peak working set was read from Windows process counters during each run at 15 ms intervals; it includes fixture setup/cloning and the complete probe process, not isolated allocator bytes. Time is best of five; peak memory is one process run per variant. A later full timing run measured availability at 198.942 ms; this is ordinary run variation, not an additional optimization.

Benchmark evidence correction: the older edit baseline supplied no persisted package signatures or required input values and never asserted successful resolution. Its historical `validate` numbers therefore exercised an incomplete graph and cannot establish healthy whole-document validation speed. The probe now supplies signatures, resolved type arguments, pins and required values, and asserts validity and resolved-node count before timing. On that corrected 10,000-node chain: full `validate` 408.622 ms; an indexed isolated connect 0.1164 ms; an indexed connect touching the full generic component 207.952 ms. Callee-chain validation remains separately measured at 11.312/57.293/228.631 ms for depths 200/1,000/4,000. The export probe likewise asserts complete resolution and a diagnostic-free result.

Remaining objective/performance gaps:

- NFR-15 is **not met** by large affected components. Incremental inference still solves the full touched component; declaration-consumer discovery scans the document, connection planning clones it, and changed-edge lookup scans function bodies. The small indexed-edit result alone does not qualify W4/W9.
- Availability still traverses control flow once per producer/outcome group, so CPU can be quadratic. Shared lazy cones can also grow the discovered query set. Data-cycle construction materializes every input-to-computed-output dependency, region overlap analysis can revisit shared nodes, and transitive function-write summaries can grow with reachable effects. Callee-first acceptance removes repeated whole-document rounds, not those costs.
- Import has a 64 MiB pre-parse byte cap; 100,000-node/500,000-edge defaults are checked after structural allocation. Type/literal nesting and unification expansion limits exist, but there is no aggregate CPU/allocation/cancellation budget across all validation passes, declarations, metadata and diagnostics. The default import sizes do not promise bounded interactive validation or adversarial-input NFR-08 qualification. A host-facing analysis-budget/cancellation contract and focused stress evidence remain necessary.
- `StateRules` still lacks compiler constant-folding warnings, so compile readiness remains false. No compiler/runtime, package hot-installation, OS-service, editor/accessibility/audio or release gate is passed. MSRV 1.97.1 and Linux/macOS were not run locally; only 1.99.1 is installed. New local edits have no remote CI evidence until published and checked.

Ignored `.artifacts/security-performance-*.log` files retain local commands/results.

### Readiness revalidation (2026-10-23)

Rechecked committed `4f9866d8e83131edd139d0977f3a7772040ff57a` after the security/performance corrections above. Rust 1.99.1 (`58cf8f9c3`), Cargo 1.99.1, Windows x86_64 MSVC, default workspace features; unchanged Cargo.lock SHA-256 `39279fc3c0349b56f062bce50ca6d0d81033e13f9ca47c5f65869adb022a43b4`. `cargo test --workspace --locked` passed 514 tests with four timing tests ignored; formatting, Clippy with `-D warnings`, Rustdoc with `-D warnings`, and the Cargo dependency-boundary guard passed. This revalidation changed no production algorithm or readiness flag. MSRV, other operating systems and current-head remote CI remain unverified; PR 14 still pointed to the preceding `65747d2` when inspected.

Additional release probes composed `tests/perf_edit_baseline.rs` with ignored `.artifacts/readiness-probe-extra.rs` into a temporary `tests/_readiness_probe.rs`, then ran `cargo test --release -p orange-model --locked --test _readiness_probe readiness_probe_ -- --ignored --nocapture --test-threads=1`. The temporary test file was removed in `finally`; four exploratory probes completed. Logs are `.artifacts/readiness-revalidation-{tests,probes,clippy,rustdoc}.log`. Exploratory probe success means observations were collected, not that the discovered gaps satisfy acceptance.

| Same-machine workload | Samples | Observed median | Observed p95 |
|---|---|---|---|
| Indexed isolated connect in a valid 10,000-node generic chain plus two isolated nodes | 31 | 0.1115 ms | 0.1897 ms |
| Indexed connect extending that entire generic component | 31 | 222.740 ms | 237.232 ms |
| Complete validation of the same original graph | 7 | 426.640 ms | 428.976 ms |
| Standalone node resolution and fresh type inference | 7 | 287.154 ms | 293.930 ms |
| Standalone data-cycle pass over that graph | 7 | 74.614 ms | 81.645 ms |
| One valid node with 64 inputs and 64 computed outputs, data-cycle pass | 3 | 3.473 ms | 3.837 ms |
| One valid node with 256 inputs and 256 computed outputs, data-cycle pass | 3 | 55.220 ms | 64.138 ms |
| One valid node with 512 inputs and 512 computed outputs, data-cycle pass | 3 | 244.356 ms | 246.626 ms |

Edit preparation (document clone, index construction and first node lookup) is outside the timed indexed API call; undo-result disposal is also outside it. Every edit sample must return no inference issues. Standalone pass measurements reuse fresh prerequisite evidence; connection-pass timing includes its own public-API index construction, unlike the combined pipeline. Individual pass times are therefore not additive accounting. On the pure graph, median connections/inputs were 19.531/16.069 ms and regions/control cycles/availability were 4.978/3.275/5.270 ms. The wide-node fixture has real registered provider/signature metadata, concrete variadic instances and initialized inputs; resolution, connections, inputs and cycle checks pass before timing. It exposes the cost of materializing input×computed-output dependency edges even with zero persisted graph edges. No large-width extrapolation was measured.

These are repeated measurements of a few deterministic shapes, not the mixed seeded W4/W9 workload or statistical NFR qualification. In particular, a 0.1897 ms inference/edit result does **not** include updated full diagnostics: there is no incremental equivalent of combined validation yet. A consumer running full validation after that edit would still incur the approximately 427 ms validation cost. Hardware-model/RAM enumeration through CIM was unavailable, so these are same-machine observations rather than a named-reference-machine qualification.

**New completeness evidence.** Eight malformed or unresolved unused-declaration cases (each of `not-a-type`, `org.missing/unknown@1`, `$T`, and `core/list@1<>` as either a run-variable type without an initializer or an event payload type) all returned `report.is_valid() == true`, no diagnostics, `missing_checks() == [StateRules]`, and compilation readiness false. Variable declaration validation currently reaches type parsing only when an initializer exists; event payload type resolution happens through consuming nodes rather than a declaration-wide pass. Passive import preserving these declarations is expected; silently certifying their semantic types is not sufficient for the compiler's resolve-all-identities precondition. No executable bad plan exists today because readiness remains false and no compiler exists. Before enabling it, declaration-wide concrete type resolution, registry identity/constraint checks and redacted declaration subjects need focused regressions. The earlier statement that constant-folding warnings are the only remaining behavior must not be read as proof that toggling the `StateRules` coverage bit would complete validation.

**Readiness ordering issue.** The local roadmap makes V11 complete validation a prerequisite of N1/E1 kernels/compiler, while `StateRules` is held incomplete for `W-CONST-ERROR`, which the compiler's folding phase produces (03, compiler step 6). Resolve this phase ordering in the affected contracts before implementation: the compiler may need an internal analysis/lowering phase that cannot publish a runnable plan until folding diagnostics and all checks finish. This is a boundary to design and test, not authority to mark missing checks covered or remove them.

Code-quality assessment: immutable registry snapshots, document/registry freshness tokens, atomic inferred edits, stable diagnostic subjects, iterative traversal and UI-independent core remain useful foundations. The current evidence favors finishing the validation/compiler boundary over a broad refactor, blanket clone removal, replacement collection types or a speculative incremental framework. The measured type-inference and cycle costs identify useful later targets; function lengths or raw clone counts alone do not establish defects. Catalog/validation origin ownership and related module coupling should be addressed when the compiler/runtime split gives them an actual consumer.

The next readiness work should close declaration-type coverage and phase ordering, then add aggregate work/allocation/metadata limits with explicit incomplete diagnostics and cancellation handling before exposing unrestricted imports or provider metadata to consumers. Complete a small typed compiler/reference-evaluator slice with known-answer semantics, stable source maps and stale-input rejection. Keep the timing probes, and qualify incremental/full equivalence before wiring full diagnostics into interactive editing. NFR-15 and G2/G3 performance gates remain required; readiness sequencing does not waive them. Further optimization should be attached to those concrete consumers or a reproduced pathological case, rather than treated as an open-ended prerequisite to all progress.

### Readiness fixes verification (2026-10-23)

This checkpoint supersedes the resolved gaps in the preceding read-only revalidation. Applied declaration-wide variable/event type validation (including unused declarations), linear dependency edges for wide computations, combined-analysis admission quotas and cooperative cancellation, and distinct semantic-lowering versus completed-plan readiness. `DeclarationTypes` and metadata `StateRules` are covered; `ConstantEvaluation` remains missing. Constant errors still require compiler folding and preservation before publishing a completed plan. No compiler, evaluator, runtime, package execution or platform gate is claimed.

Environment: selected rustup `1.99.1-x86_64-pc-windows-msvc`, rustc `1.99.1 (58cf8f9c3 2026-10-13)`, Cargo `1.99.1 (5f94df478 2026-08-27)`, LLVM `23.1.1`, workspace default features, no dependency or feature changes. Cargo.lock SHA-256 remains `39279FC3C0349B56F062BCE50CA6D0D81033E13F9CA47C5F65869ADB022A43B4`. Rust 1.97.1 MSRV and other operating systems were not executed. The preceding refreshed cargo-deny/RustSec scan applies to the unchanged dependency graph; it was not rerun for these source-only fixes.

Checks from the repository root: `cargo test --workspace --locked` passed **521 tests**, including the compile-fail evidence test, with five timing tests ignored. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS=-D warnings`, `node tools/check-cargo-metadata.mjs` (Node 24.18.0), and `git diff --check` passed. OneDrive still reports incremental-cache hard-link fallback to copying; no code lint failure remained. Focused regressions cover malformed/unknown/unresolved/incorrect-arity and constrained declaration types, nesting bounds, event/variable subjects, redaction, read-only behavior, exact aggregate admission boundaries, cancellation signal reuse, large literal rejection, freshness, coverage and wide-node closed port witnesses.

The retained timing test is reproducible with `cargo test --release -p orange-model --locked --test perf_edit_baseline data_cycles_cost_by_port_width -- --ignored --nocapture`. Fresh registered metadata, valid connections and initialized inputs are established before timing. Five samples per width gave data-cycle medians **0.2276/1.062/2.110 ms** for 64/256/512 inputs and computed outputs (maxima 0.2365/1.653/2.954 ms). The previous three-sample medians were 3.473/55.220/244.356 ms. Different sample counts and local scheduling make these descriptive measurements, not a qualified speedup ratio or a general NFR result. The implementation avoids the input×output relation's quadratic edge materialization; narrow computations retain direct edges.

The four exploratory probes were also rerun against the final source with the earlier ignored helper, temporarily appended to the retained timing source as `_readiness_probe.rs`: `cargo test --release -p orange-model --locked --test _readiness_probe readiness_probe_ -- --ignored --nocapture --test-threads=1`. The temporary file was removed in `finally`. Both 10,000-node edit/combined fixtures are explicitly asserted valid under the default admission quotas before timing. An initial stricter quota rejected this existing fixture and was corrected; rejection timings are excluded. Final observations:

| Same-machine workload | Samples | Median | Observed p95 |
|---|---|---|---|
| Indexed isolated connect | 31 | 0.1152 ms | 0.2032 ms |
| Indexed connect extending the entire 10,000-node generic component | 31 | 229.390 ms | 235.040 ms |
| Complete valid combined analysis | 7 | 450.551 ms | 457.999 ms |
| Standalone node resolution/type inference | 7 | 302.320 ms | 308.466 ms |
| Standalone data-cycle pass | 7 | 80.128 ms | 88.522 ms |

The regular combined graph measured about 5.6% higher than the earlier 426.640 ms median; these runs do not isolate admission overhead from machine noise and do not claim a general improvement. All eight previously accepted malformed unused-declaration probes now return `E-DECLARATION-TYPE`, invalid reports and no lowering/compilation readiness. Logs: `.artifacts/readiness-fixes-{tests-final,focused,clippy,rustdoc,wide-final,probes-final}.log` (ignored).

Remaining work: compiler-owned constant evaluation/error preservation; hard interruption or isolated workers for expensive individual passes; separate registry/type-schema lifecycle budgets; mixed W4/W9 NFR qualification; incremental combined diagnostics; and large-component inference (still roughly 229 ms here, beyond NFR-15). Admission estimates bound inspected input size, not elapsed time, heap/RSS or instruction counts. Existing import limits remain necessary. Next development should prioritize the bounded compiler/reference-evaluator readiness slice, with further optimization tied to measured failures and real consumers.
### Bundled scalar kernels verification (2026-10-23)

N1a adds app/crates/orange-bundled as the second root workspace member, under the existing statically linked native-kernel exception. It publishes real org.orange.math Add/Subtract/Multiply/Divide/Not contracts, explicit scalar specialization metadata and safe typed failures. Execution binds exact definition/config/provider/type/policy once, pins the Arc registry snapshot, and performs scalar operations without runtime name lookup, I/O or success-path buffer construction. Code inspection supports this narrow allocation statement; no allocator instrumentation, throughput, audio-callback or NFR qualification is claimed. This is not graph compilation/evaluation: the product-graph integration test validates a saved graph, resolves its binding from fresh analysis and the exact lock, then calls the scalar kernel directly to obtain 42. ConstantEvaluation stays missing and final compilation readiness stays false.

Toolchain: selected rustup 1.99.1-x86_64-pc-windows-msvc, rustc 1.99.1 (58cf8f9c3 2026-10-13), Cargo 1.99.1 (5f94df478 2026-08-27), LLVM 23.1.1, workspace defaults. New crate has no optional features; its existing exact-pinned serde_json/thiserror/proptest dependencies and local orange-model dependency add no new external package versions or feature choices. Initial offline member check added only its local Cargo.lock package entry; external checksums/versions did not change. Selected post-change lock SHA-256: `956C82119B5D2545847C61978348A2C172B8CCB5690CB21135CC6403B9CA0392` (previous `39279FC3C0349B56F062BCE50CA6D0D81033E13F9CA47C5F65869ADB022A43B4`). Arithmetic APIs were checked in the selected local Rust 1.99.1 standard-library source (`core/src/num/int_macros.rs` and `uint_macros.rs`), including wrapping/saturating division and its stable-version annotations. No external crate API/version was adopted. Rust 1.97.1 MSRV and other OSs remain unexecuted.

Verification commands from repository root:

- `cargo test --workspace --locked`: **532 passed**, including eleven new bundled integration/property tests and the existing compile-fail evidence test; five timing tests ignored.
- `cargo test -p orange-bundled --locked`: eleven passed. `cargo test --release -p orange-bundled --locked`: eleven passed in optimized builds, with no debug-only overflow dependency.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS=-D warnings`, and `git diff --check`: passed. OneDrive incremental hard-link fallback warnings remain environmental; no code lint failure remained.
- Node 24.18.0: `node tools/check-cargo-metadata.mjs --self-test` passed nine policy fixtures, including a forbidden bundled-kernel UI dependency; `node tools/check-cargo-metadata.mjs` passed the real direct/transitive boundary; `--msrv` reported declared 1.97.1, which is metadata, not an MSRV build.

Known-answer checks cover all six numeric specializations and four arithmetic operations, boolean Not, checked/wrapping/saturating overflow and underflow at all integer widths, zero divisors, signed MIN/-1, signed truncation, f32/f64 finite inputs/results versus IEEE NaN/infinity, negative-zero bits and f32 precision. A 64-case proptest compares checked i64 add/subtract/multiply/divide against an independent i128 oracle. Additional checks reject mixed Values/arity/config/types/versions/providers/digests and missing metadata; compare native config acceptance with published schemas; validate a saved product graph without test-only definitions; retain old metadata and exact provider identity after replacement; and reject unlocked or differently selected providers without native fallback. The host still authenticates the build digest assertion and supplies fresh graph/type evidence; the resolver does not authenticate executable bytes or certify a graph.

Logs are ignored `.artifacts/native-kernels-{focused,workspace,release,clippy,rustdoc}.log`. Initial fixture/import/type-annotation mistakes were corrected before the final passing runs. The foundation review remains untracked and byte-identical (SHA-256 D8F25A2F3DF04A88578972340B3CA3B591DC42126083B107A60344B00125D553). No push, new package loader, compiler/constant-folding proof, run actor, platform/UI or SDK-freeze gate is claimed. Remaining N1 includes executable conversions and broader kernel families; E1/E2 still require the checked dense plan/source-map and reference-evaluation boundaries. The preceding unchanged-external-dependency RustSec scan was not refreshed for this local-member addition.
### Core converter execution verification (2026-10-23)

N1b adds executable implementations of the four existing Core numeric conversion entries to orange-bundled: i32→i64, checked i64→i32, lossless f32→f64 and lossy f64→f32 with explicit nearest-even/reject options. No entries, dependency versions, feature flags or graph format changed. Binding rejects missing metadata, wrong entry/version/provider, unknown options and incomplete saved policies rather than adopting defaults. A registered package alternative never falls back to Core; an existing Core binding ignores later new-binding defaults and retains its Arc snapshot/entry. This scalar API does not certify graph freshness or construct a compiler plan.

Selected environment remains Rust 1.99.1 (58cf8f9c3 2026-10-13), Cargo 1.99.1 (5f94df478 2026-08-27), LLVM 23.1.1, x86_64-pc-windows-msvc, workspace default features. Cargo.lock SHA-256 remains `956C82119B5D2545847C61978348A2C172B8CCB5690CB21135CC6403B9CA0392`. The numeric float-cast contract was checked in the selected toolchain's local versioned Rust Reference, `share/doc/rust/html/reference/expressions/operator-expr.html` (float widening/narrowing: closest target value and roundTiesToEven). NaN payload/signaling preservation uses explicit integer bit mapping, with exact known-answer and round-trip tests rather than assumptions about native NaN casts. Rust 1.97.1 and other OSs were not executed; no new external API/version was adopted.

Commands/results from the repository root: `cargo test --workspace --locked` passed **542 tests**, including ten new converter tests and the existing compile-fail evidence test, with five timing tests ignored. `cargo test -p orange-bundled --locked --test conversions` and `cargo test --release -p orange-bundled --locked --test conversions` each passed ten. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS=-D warnings`, `node tools/check-cargo-metadata.mjs`, `node tools/check-cargo-metadata.mjs --self-test` (nine cases) and `git diff --check` passed. OneDrive incremental-cache hard-link fallback warnings remain environmental; no code lint failure remained. Logs: ignored `.artifacts/core-conversions-{focused,workspace,release,clippy,rustdoc}.log`.

Known answers cover integer bounds/overflow without saturation, finite values just above f32::MAX, ties-to-even, subnormal underflow, positive/negative zero, infinities and quiet/signaling NaN payloads. Two 256-case property tests sample all-bit-pattern f32 round trips (including sign/NaN bits) with no loss and exact i32 round trips. This is focused invariant testing, not exhaustive floating-point qualification. Loss metadata is always Some(JSON {lost:bool}) on f64→f32 success and absent for the other three entries; the flag is true exactly when widening the narrowed result does not recover the original bits. The two metadata Values are validated/prebuilt at binding and shared across evaluations, verified by Arc identity. No input/output values enter error messages or loss JSON. Range and wrong-variant failures return safe typed OutOfRange/InternalFault with namespaced message keys and no partial result.

A saved Core converter graph with value/result/loss ports, initialized f64 input and result export passes semantic lowering readiness. The test reads its persisted config, binds the exact Core entry, decodes its literal and directly evaluates the converter to f32 1.0 with lost=true. It also checks that document bytes/state/revision do not change and final compilation readiness remains false. This is not execution of a compiled graph. Constant-error folding, dense plan/source maps, compiler freshness enforcement, graph reference evaluation, package execution and NFR/per-platform qualification remain open. Success-path buffer/JSON parsing absence follows code construction; no allocator/throughput or audio-callback claim is made. The external dependency graph is unchanged, so no new RustSec scan was run. The foundation review remains untracked and byte-identical.
### Checked pure-export plans verification (E1a, 2026-10-23)

Continuation base c949847. Added orange-compiler -> orange-model and the orange-bundled -> orange-compiler trusted resolver adapter. External dependency versions, checksums and feature selections are unchanged; Cargo.lock adds the local compiler package/dependency edge only. Selected toolchain: 1.99.1-x86_64-pc-windows-msvc, rustc 1.99.1 (58cf8f9c3, 2026-10-13), Cargo 1.99.1 (5f94df478, 2026-08-27), LLVM 23.1.1; no optional features enabled in these commands. MSRV metadata remains 1.97.1. Root lock SHA-256: 2F82E8EFEE2BC8D72A49E79E8D812D5DEDD5F032D74EDAEB9826C27DBAF9DC17. No new third-party API or dependency version was adopted; implementation uses the existing pinned model/value/catalogue interfaces and local selected-toolchain Rust APIs.

Commands from the repository root: cargo test --workspace --locked passed 554 tests, with five timing tests ignored, including a new compile-fail check for private plan construction. cargo test --release -p orange-bundled --test compile_pure --locked passed all 11 compiler integration tests. cargo fmt --all --check, cargo clippy --workspace --all-targets --locked -- -D warnings, cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs, node tools/check-cargo-metadata.mjs --self-test (nine fixtures), node tools/check-cargo-metadata.mjs --msrv and git diff --check passed. Node 24.18.0. OneDrive incremental-cache hard-link fallback to copying persists; there are no remaining code lint failures. Ignored raw logs: .artifacts/e1a-{tests,release,clippy,rustdoc}-final.log.

Focused acceptance: product graph diamond and 512-node chain, shared-producer once-only counters, dense source/edge mapping, persisted export order, first fault by declared input order despite UUID/storage order, unobservable dead pure cones, observable divide-by-zero and checked conversion faults, typed JSON loss output, fresh descriptor defaults and instance overrides, incoming-edge shadowing, stale cached type_args ignored for binding, exact registry Arc retention, immutable document bytes, stale document/registry and invalid semantic evidence rejection before callbacks, correct matching-evidence reuse after undo, unsupported unused volatile root rejection, operation/slot/graph/byte quotas and pre-cancellation, missing backend, wrong executable port order, wrong output type and malformed fault envelope. Failures publish no partial plan. Folded failures carry original producer IDs and safe W-CONST-ERROR diagnostics.

These tests qualify the restricted eager pure scalar/JSON root-export boundary only. No full ExecutionPlan hash/encoding, generic function/control/lazy/volatile scheduler, independent graph reference evaluator, package loader or final runtime ABI is implemented. Global model ConstantEvaluation remains missing; ValidationAnalysis::is_ready_for_compilation remains false. CompileOptions limits bound admitted graph/storage work and retained values, not callback CPU/allocations, full process RSS or hard interruption. Callbacks are trusted effect-free host code; there is no guest execution or new sandbox claim. No latency, allocation instrumentation, NFR, Linux/macOS or MSRV 1.97.1 execution claim. The unchanged external dependency graph was not rescanned for RustSec in this unit. Next readiness work is independent reference evaluation/equivalence, then broader execution semantics; optimize against measured workloads. The foundation review remains untracked and byte-identical, and no push was performed.

### JSON versus binary evaluation and import improvement (2026-10-23)

Base a9d84ed. This evaluates document persistence, not execution layout: 12 §1 already separates GraphDocument, RegistrySnapshot, ExecutionPlan and run state. Typed values/dense slots are native in-memory data; a binary file encoding is an independent persistence choice. The product remains format-v1 compact JSON, with no binary importer or serialized PurePlan/cache. Selected local toolchain/features are unchanged: Rust/Cargo 1.99.1, x86_64-pc-windows-msvc, LLVM 23.1.1, default workspace features; MSRV remains 1.97.1 and was not executed. Root lock SHA-256 remains 2F82E8EFEE2BC8D72A49E79E8D812D5DEDD5F032D74EDAEB9826C27DBAF9DC17, with no product dependency or lock changes.

The retained ignored benchmark app/crates/orange-model/tests/perf_codec.rs generates 1,000/10,000 nodes with 999/9,999 edges, a shared signature, i64 decimal literals, nested configuration and unknown extensions including u64::MAX. These are structurally loadable codec fixtures, not semantically executable graphs. It checks document equality and canonical save/load/save bytes before timing. Command: cargo test --release -p orange-model --test perf_codec --locked -- --ignored --nocapture. One warmup and nine samples per operation; medians below exclude fixture setup, destruction, disk I/O, semantic validation and compilation. The ordinary full importer still performs header/version, byte/count and duplicate-ID/lock checks.

| Nodes | Edges | Saved JSON bytes | Baseline load | Final load | Baseline save | Final save |
|---|---|---|---|---|---|---|
| 1,000 | 999 | 444,158 | 6.092 ms | 5.135 ms | 0.394 ms | 0.379 ms |
| 10,000 | 9,999 | 4,458,160 | 63.926 ms | 52.353 ms | 6.461 ms | 6.167 ms |

The fix removes EnvelopeIn's extra whole-document flattened Serde buffer: after the existing header gate, deserialize GraphDocument directly and shift-remove only the reserved format key. This preserves the relative order of unknown extensions. The 10k median load decreased about 18% in these runs; an earlier final-source run measured 51.096 ms. Export code is unchanged, so save-time differences are machine variation, not an export improvement claim. No allocator/RSS, disk startup, worst-case input, NFR or cross-platform performance qualification is implied; full structural deserialization and other flatten buffers still exist, and import read/byte limits remain necessary.

A separate ignored .artifacts/codec-probe workspace used exact rmp-serde 1.3.0, rmp 0.8.15, num-traits 0.2.19, serde 1.0.229 and serde_json 1.0.154 with preserve_order. Command: cargo run --release --manifest-path .artifacts/codec-probe/Cargo.toml --locked. The isolated binary dependency and its lockfile were not added to the product workspace. Its API source was verified from [versioned rmp-serde 1.3.0 Rustdoc](https://docs.rs/rmp-serde/1.3.0/rmp_serde/). The experiment serializes the same GraphDocument with serde_json::to_vec and rmp_serde::to_vec_named, deserializes both directly to GraphDocument and checks full document equality plus identical product JSON re-export, including unknown extensions and exact integer values. Named fields are retained to compare the existing extensible document shape. Both raw paths omit the product envelope and importer checks; these numbers must not be compared directly to the full-import table above as production end-to-end results.

| Nodes | Raw JSON bytes | Named MessagePack bytes | Raw JSON decode | Raw MessagePack decode | JSON encode | MessagePack encode |
|---|---|---|---|---|---|---|
| 1,000 | 444,133 | 370,758 | 4.948 ms | 3.936 ms | 0.477 ms | 0.718 ms |
| 10,000 | 4,458,135 | 3,718,759 | 46.623 ms | 39.414 ms | 6.537 ms | 8.905 ms |

At 10k, named MessagePack was about 16.6% smaller and 15.5% faster to decode, while encoding took about 36% longer. Existing UUID/type/literal strings and repeated field names remain in this schema-preserving binary encoding. This single fixture does not qualify other binary codecs, positional schemas, compressed JSON, blob-heavy assets or zero-copy caches. The temporary probe is exploratory, not a maintained benchmark or supported format; its source/lock/log remain ignored locally. The retained JSON benchmark provides reproducible product timing. Recommendation: retain readable/diffable interoperable JSON source now, take the measured importer improvement, and revisit versioned binary plan caching only after the complete plan identity/ABI and real startup bottlenecks are established. No persisted Rust memory image or executable pointer is allowed.

Verification: cargo test --workspace --locked passed 555 tests, six timing tests ignored (including the new codec measurement). Added envelope-position regression checks reserved-format removal, unknown key order, nested format data, exact u64::MAX and identical output across header positions; existing golden, import-limit/lock/duplicate, unknown-field and generated round-trip tests pass. cargo fmt --all --check, cargo clippy --workspace --all-targets --locked -- -D warnings, cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs and its --self-test (nine fixtures), and git diff --check pass. Node 24.18.0. OneDrive's incremental-cache hard-link fallback persists. Logs: .artifacts/codec-{json-before,json-direct,json-final,binary-probe,regressions,tests-final,clippy-final,rustdoc-final}.log. Root manifest/lock unchanged; no new product dependency scan was needed. The binary probe did not receive production dependency/security qualification. Foundation review remains untouched/untracked; local commit only, no push.

### Independent integer pure oracle verification (E2a, 2026-10-23)

Base 86ab4ab. Added test-only support/reference.rs and reference_equivalence.rs in orange-bundled, plus oracle assertions in the existing fresh default/instance override test. Product source, public APIs, schemas, manifests and Cargo.lock are unchanged. Selected toolchain is still Rust 1.99.1 (58cf8f9c3, 2026-10-13), Cargo 1.99.1 (5f94df478, 2026-08-27), LLVM 23.1.1, x86_64-pc-windows-msvc; default workspace features. Root lock SHA-256 remains 2F82E8EFEE2BC8D72A49E79E8D812D5DEDD5F032D74EDAEB9826C27DBAF9DC17. No new third-party APIs, dependencies or product binary encodings were adopted. The user confirmed retaining JSON.

The independent oracle reads source graph ports and directly scans edges, recursively demands inputs in fresh declaration order, memoizes by source node and preserves the first fault producer/code. Recursion is explicitly bounded to 64; generated graphs are 1–24 nodes. It never reads the compiler plan/slot/constant pool or calls native kernels/resolvers. i128/u128 intermediates provide a separate integer implementation for i32/i64/u32/u64 checked/wrapping/saturating Add/Subtract/Multiply/Divide; bool Not and i32↔i64 are also implemented independently. It shares model validation, metadata and canonical literal decoding, so this is independent traversal/arithmetic evidence, not independent validation/codec proof. Extra compiler work/warnings on later pure inputs after a fault are intentionally excluded from equivalence: the oracle short-circuits demand. Unsupported test vocabulary fails rather than silently approximating it.

Commands: cargo test --workspace --locked passed 560 tests, with six timing tests ignored. cargo test --release -p orange-bundled --test reference_equivalence --test compile_pure --locked passed 16 tests (five new reference tests and 11 compiler integration tests). The generated-DAG property runs 128 cases per invocation with shrinking/failure persistence enabled and is paired with explicit known-answer checks. Cases include all four integer widths/operations, three overflow policies, sharing and memo counters, dead cones, shadow literals, reversed storage, UUIDs unrelated to dependency order, export order, exact JSON round trip and original fault provenance. Named cases verify diamond=35, competing faults where input a has a higher UUID, signed MIN/-1, unsigned 0-1, boolean values and integer converter range boundaries. Existing default/override assertions also pass against the oracle. No production discrepancy was found in this bounded scope and no corrective production change was necessary.

cargo fmt --all --check, cargo clippy --workspace --all-targets --locked -- -D warnings, cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs, its --self-test (nine fixtures) and git diff --check passed. Node 24.18.0. OneDrive incremental-cache hard-link fallback persists, without code lint failures. Ignored logs: .artifacts/e2a-{focused,tests-final,release-final,clippy-final,rustdoc-final}.log. No failed property seed was generated. This unit adds readiness evidence only: general ConstantEvaluation coverage, float/JSON/lossy-conversion reference equivalence, runtime epochs, state/control/lazy/functions, public evaluation APIs and scheduled execution remain open. No MSRV 1.97.1 execution, other OS, hard CPU/RSS, allocation or NFR claim; unchanged product dependencies were not rescanned. Foundation review remains byte-identical/untracked, local commit only, no push.

### Checked pure result API (E2b, 2026-10-23)

Base 5f78b14. Added PurePlan::evaluate and private-construction PureEvaluation, EvaluationOptions, EvaluationError and borrowed EvaluatedExport in orange-compiler. No new workspace member/dependency/feature or lock change. Selected local Rust/Cargo 1.99.1, x86_64-pc-windows-msvc, LLVM 23.1.1; default features. Root lock SHA-256 remains 2F82E8EFEE2BC8D72A49E79E8D812D5DEDD5F032D74EDAEB9826C27DBAF9DC17. This slice reads fully folded constants; it does not execute a scheduler or re-enter kernels. Code inspection shows O(1) indexed reads, O(exports) iteration and no new frame/value/metadata allocations; no allocator, throughput or NFR qualification is claimed.

cargo test --workspace --locked passed 565 tests, six timing tests ignored, including a new compile-fail check for view construction. cargo test --release -p orange-bundled --test compile_pure --test reference_equivalence --locked passed 20 tests. Four new integration tests check borrowed pointer identity, type/source/id metadata, zero additional resolver/kernel calls, frozen results and pinned Arc after editing/catalogue publication, stale new-start rejection, exact undo and distinct compile/entry revisions, export count boundary, pre/mid-read cancellation, empty exports and missing indices, and graph fault versus entry-error distinction. The 128-case integer-DAG property now consumes the public evaluation API and compares original error producer/code or values with the independent oracle before/after JSON round trip.

cargo fmt --all --check, cargo clippy --workspace --all-targets --locked -- -D warnings, cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs, its --self-test (nine fixtures) and git diff --check passed. Node 24.18.0; OneDrive cache hard-link fallback persists. Ignored logs: .artifacts/e2b-{focused,tests-final,release-final,clippy-final,rustdoc-final}.log. No scheduled runtime, public activation/epoch machinery, final ABI, global compilation-readiness coverage, binary persistence, MSRV/other-OS or NFR gate is implied. Product dependencies unchanged; no new dependency scan. Foundation review unchanged/untracked, local commit only, no push.

### Floating and multi-output reference coverage (E2c, 2026-10-24)

Base f0baf86. Test-only extension of the demand oracle: f32/f64 arithmetic and finite/IEEE policy checks, f32↔f64 conversion and distinct per-port result/loss memoization. Integer widened arithmetic remains unchanged. The interpreter still reads original graph edges/signatures and never calls native kernels/resolvers or compiler folding helpers. It shares Rust floating primitives, model validation and literal decoding; floating agreement is traversal/policy evidence, not an independent hardware/soft-float or validation proof. Ordinary arithmetic NaNs compare classification; converters compare exact payload/sign/signaling/quiet bits, and other floats compare Value's exact bit identity. The provisional public result API itself is unchanged in this commit.

cargo test --workspace --locked passed 569 tests, six timing tests ignored. cargo test --release -p orange-bundled --test compile_pure --test reference_equivalence --locked passed 24 tests (15 compiler/API and nine reference tests). Three proptests each run 128 cases per invocation: integer DAGs (1–24 nodes), float DAGs (1–16 nodes) with finite source literals under both policies, and arbitrary f32/f64 conversion bit patterns, all compared before/after canonical JSON reload through PurePlan::evaluate. Named cases independently check finite input faults before zero-divisor faults, signed zero/infinity, overflow to nonfinite, IEEE NaN classification, positive/negative converter NaNs, ties-to-even, lost=true/false JSON and out-of-range faults on both result/loss with original producer IDs. No production mismatch or failing property seed was found in this scope.

cargo fmt --all --check, cargo clippy --workspace --all-targets --locked -- -D warnings, cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs, --self-test (nine fixtures) and git diff --check passed. Selected toolchain/features remain local Rust/Cargo 1.99.1, LLVM 23.1.1, x86_64-pc-windows-msvc, default workspace features; Node 24.18.0. OneDrive cache hard-link fallback persists; no remaining code lint failure. No manifests/dependencies/lock changes; root lock SHA-256 2F82E8EFEE2BC8D72A49E79E8D812D5DEDD5F032D74EDAEB9826C27DBAF9DC17. Ignored logs: .artifacts/e2c-{focused,tests-final,release-final,clippy-final,rustdoc-final}.log. No new dependency scan, allocator/latency/NFR, MSRV 1.97.1 execution, other OS or general runtime/JSON-family/control/state/lazy/function gate claim. JSON remains the graph format. Foundation review remains byte-identical/untracked; local commit only, no push.

### Constant Select/And/Or execution (E1/E2d, 2026-10-24)

Base 834fdef. Added private host-intrinsic validation/projection code in orange-compiler/lazy.rs, an iterative actual-demand lowering stack and a shared input-materialization helper. Whole-graph semantic readiness and conservative potential-cone resource checks still precede callbacks. Only core/select/core/and/core/or version/config 1 without a package provider are admitted as lazy; their exact fresh port directions, order, flags and types are checked. Package definitions receive no lazy extension authority. Only the existing bool/i32/i64/u32/u64/f32/f64/JSON plan types are supported. Selected deciding inputs/arms materialize once per producer; inactive alternatives are absent from executable bindings/constant slots unless independently exported. PureStep bindings for these intrinsics are immutable selected-port projections, not a general runtime branch ABI. No public method/field signature, schema, manifest, dependency, feature or lock changes; JSON retained.

cargo test --workspace --locked passed 579 tests, six timing tests ignored. cargo test --release -p orange-bundled --test compile_pure --test reference_equivalence --locked passed 34 tests (23 compiler/API and 11 reference). Ten new tests include an extra 128-case nested-selection property and truth tables; the four proptests now run 128 cases each per invocation. Existing eager, floating/converter and checked result API tests remain green. Focused regressions prove no inactive executable callback/binding/warning, selected fault provenance, selector fault suppression, faulty RHS short circuit, large inactive JSON compiler-decode budget suppression, a separately exported fault, 512 nested Select levels with shared results and one native producer fold, conservative admission and pre-cancellation before external resolution, explicit unsupported Try, and mid-selector cancellation before arm binding. No failing property seed or production discrepancy was found after implementation.

cargo fmt --all --check, cargo clippy --workspace --all-targets --locked -- -D warnings, cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs, --self-test (nine fixtures) and git diff --check passed. Selected toolchain remains Rust 1.99.1 (58cf8f9c3, 2026-10-13), Cargo 1.99.1 (5f94df478, 2026-08-27), LLVM 23.1.1, x86_64-pc-windows-msvc, default workspace features; Node 24.18.0. OneDrive incremental hard-link fallback persists, with no code lint failure. Root lock SHA-256 remains 2F82E8EFEE2BC8D72A49E79E8D812D5DEDD5F032D74EDAEB9826C27DBAF9DC17. Ignored logs: .artifacts/e2d-{eager-regressions,lazy-focused,focused,tests-final,release-final,clippy-final,rustdoc-final}.log.

Limits: potential-cone admission still visits inactive source metadata and can reject a graph whose selected work would fit a smaller budget. Source semantic validation may decode inactive literals before compilation; only additional compiler materialization is suppressed. Trusted callbacks cannot be interrupted mid-call; there is no hard CPU/RSS, allocator/latency/NFR, MSRV 1.97.1 execution or other-OS qualification. General Option/Result/coalesce/try, seeded entropy/volatile epochs, functions, control/actors and global compilation-readiness gates remain open. This does not complete parent E2, E3 or R1. Product dependencies unchanged, no new dependency scan; original foundation review byte-identical/untracked, local commit only, no push.

### Constant options and Coalesce (E1/E2e, 2026-10-24)

Base 7f390d2. Extended checked PlanType with recursive options over its eight existing scalar/JSON leaves and implemented host-owned core/coalesce. The selector demands fallback only for outer None; faults and present JSON null/nested Some(None) retain their distinct meanings. Model codecs and persisted values are unchanged. Pre-decode admission inspects the supported option envelope by reference; retained charges include boxed children/JSON text and export IDs. Potential-cone admission, source freshness/provider snapshots, cooperative cancellation and trusted-callback limitations remain. No new dependencies, features, manifests, schemas or lock changes. PlanType loses Copy; EvaluatedExport.ty is now borrowed. ADR implementation note in 08 records the unfrozen 0.0.0 Rust source migration.

cargo test --workspace --locked passed 588 tests, six timing tests ignored. cargo test --release -p orange-bundled --test compile_pure --test reference_equivalence -p orange-compiler --locked passed 41 integration tests (30 compiler/API, 11 reference); cargo test --release -p orange-compiler --lib --locked passed two type-contract tests. Seven new compiler tests cover all scalar/JSON leaf types including exact NaN bits, nested Some(None), faults/unavailable backends on fallback, selector-fault suppression, shared exports, large optional JSON and exact retained boxed-value budget boundaries; one is a 128-case nested-option property against the independent demand oracle after JSON reload. Existing numeric/lazy/API properties remain green. Two private type tests cover wrong leaf/box/error shapes and type-depth/unsupported-leaf admission. Early fixture-only failures (moved JSON setup and omitted export-ID charge in expected budget) were corrected; no production discrepancy was concealed or failing property seed found.

cargo fmt --all --check; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings; node tools/check-cargo-metadata.mjs; node tools/check-cargo-metadata.mjs --self-test (nine fixtures); and git diff --check passed. Selected Rust 1.99.1 (58cf8f9c3, 2026-10-13), Cargo 1.99.1 (5f94df478, 2026-08-27), LLVM 23.1.1, x86_64-pc-windows-msvc, default workspace features; Node 24.18.0. OneDrive incremental hard-link fallback persists; no code lint failure. Root Cargo.lock SHA-256 2F82E8EFEE2BC8D72A49E79E8D812D5DEDD5F032D74EDAEB9826C27DBAF9DC17 unchanged. Ignored evidence: .artifacts/e2e-{focused,tests-final,release-final,release-types,clippy-final,rustdoc-final}.log.

Limitations: borrowing export type metadata avoids a recursive clone on reads by source inspection; no allocator or timing improvement is measured here. Compiler Option types remain subject to existing import/analysis and depth budgets, and source analysis may decode inactive literals before compilation. General collections/Result/Try, seeded entropy/volatile epochs, control/functions/actors, final global compilation readiness, NFR, MSRV execution and other-OS qualification stay open. No new dependency/security scan or release-gate claim. JSON retained; original foundation review unchanged/untracked; local commits only, no push.

### Constant Result/Try capture (E1/E2f, 2026-10-24)

Base 3b88da3. Implemented host core/try for recursively composed Option/Result over the existing scalar/JSON leaves. It catches only graph input-cone faults, retaining original code/node in NodeError data while direct consumers and W-CONST-ERROR remain intact. Added Value::Ok(Arc<Value>) only for statically ambiguous successful payloads (nested Result/Option<Result>); ordinary success remains inline and Value still fits 16 bytes. NodeError now includes optional producer identity. Result/error retain no literal form. Existing JSON persistence is unchanged. Source compatibility and representation decisions are recorded in 08 and 12.

cargo test --workspace --locked passed 595 tests, six timing tests ignored. cargo test --release -p orange-bundled --test compile_pure --test reference_equivalence --locked passed 46 tests (35 compiler/API, 11 reference). Additional release commands cargo test --release -p orange-compiler --lib --locked and cargo test --release -p orange-model --lib nested_result_success_retains_its_own_tag_and_error_identity --locked passed three type tests and one Value representation test. Five new compiler tests cover captured/direct exports, original code/node and warning counts, nested Ok(Err), successful None and Option/Result composition, inactive Try, cancellation/resource rejection and four broken backend modes. One is a 128-case nested-Try graph property compared with the independent source oracle after JSON reload and reversed storage/exports. Private tests cover type/tag/code acceptance and Value size/equality/hash. Initial missing Debug and malformed test-placeholder failures were fixed; the former unsupported-Try regression now tests its still-unsupported string leaf. No failing property seed or concealed production discrepancy.

Formatting, workspace Clippy/all targets with warnings denied, workspace Rustdoc/no deps with warnings denied, dependency-boundary guard/nine fixtures and whitespace checks passed. Selected Rust 1.99.1 (58cf8f9c3, 2026-10-13), Cargo 1.99.1 (5f94df478, 2026-08-27), LLVM 23.1.1, x86_64-pc-windows-msvc, default features; Node 24.18.0. OneDrive incremental hard-link fallback persists, with no code lint failure. No manifests/features/dependencies/lock changes; Cargo.lock SHA-256 remains 2F82E8EFEE2BC8D72A49E79E8D812D5DEDD5F032D74EDAEB9826C27DBAF9DC17. Ignored evidence: .artifacts/e2f-{initial,focused,try-tests,tests-final,clippy-final,rustdoc-final,release-final,release-types,release-values}.log.

Limits: NodeError data currently retains code and producer only; full safe message/activation/detail/cause envelopes remain runtime/WU-03 work. No general core/error leaf, collection/control/function/actor execution, seeded entropy/epochs, final global compilation-readiness, allocator/latency/NFR, new dependency scan, MSRV execution or other-OS qualification. Parent E2 remains partial. Local commits only, no push; original foundation review unchanged/untracked.

### Versioned entropy/epoch primitives (E2g1, 2026-10-24)

Base 8a14b0e. Added orange-runtime with EntropyStream, immutable bounded EntropyLayout/private dense slots and activation-owned counters/epoch memoization. Algorithm orange.entropy.sha256-counter@1 has the exact field encoding in 03 and no mutable global RNG. begin_epoch uses generation stamps; sampling a bound node reuses that epoch's sample, advances invocation only on later actual demand and rejects foreign owners. All counters fail before reuse. These are prerequisites, not graph Random lowering, a complete G-02 or R1 actor.

The independent vector computation used PowerShell 7.6.5/.NET 10.0.11: System.Security.Cryptography.SHA256.HashData over ASCII algorithm bytes plus zero, BitConverter.GetBytes(u64) on the recorded little-endian Windows host for each integer, and Convert.FromHexString for network-order UUID bytes. With seed/activation/invocation zero and node 10000000-0000-4000-8000-000000000001, block 0 digest is 444ED5EFE961A7FBC0E40C78EBC838354BAF25DA2CFD29D1F828303FF99787F2 and block 1 is AC48411FC539F5000E1991792714BFE198B219E72913A3A3D6BE72B81F7F6FEA. With seed/activation/invocation/block u64::MAX and node 10000000-0000-4000-8000-000000000002, digest is 2639B8755EC374553D16ABF1BED543320A9408E84928B358048191A68650824F. Rust tests pin all four LE words of each digest and verify no final-block wrap.

cargo check -p orange-runtime --offline added only the local runtime entry/edges to Cargo.lock; no external version changed. Product dependencies are orange-model and existing exact sha2 0.11.1/thiserror 2.0.21. cargo tree -p orange-runtime --locked -e features --depth 1 confirms default features for all three. New lock SHA-256: 3D7948198E468B5F196956E2EEB1F748E515A8B33AF6B2864B3FAE696E537BE2. No new external dependency adoption or scan.

cargo test --workspace --locked passed 603 tests, six timing tests ignored. cargo test --release -p orange-runtime --locked passed eight tests: fixed vectors across blocks/maximum fields, replay/scope fields, layout/demand-order invariance, same-epoch sharing/skipped nodes, layout replacement lifetime, admission/duplicate/unknown identity and counter-exhaustion boundaries. cargo fmt --all --check; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings; node tools/check-cargo-metadata.mjs; --self-test (nine fixtures); git diff --check passed. Initial constant-chunk/collapsible-if lint failures were corrected. Ignored logs .artifacts/e2g1-{build,focused,tests-final,clippy-final,rustdoc-final,release-final}.log.

Selected Rust/Cargo 1.99.1, LLVM 23.1.1, x86_64-pc-windows-msvc, default workspace features and Node 24.18.0 remain unchanged. Known OneDrive hard-link fallback only; no code lint failure. Source inspection shows no UUID/name lookup on bound sample reads and no whole-layout clear at epoch start; no allocator, timing or NFR qualification was executed. Hosts still own unique activation sequences and correct consumption scopes. Entropy-node metadata/provider binding, compiled graph consumption and replay/run metadata integration remain next; E2/E3/R1 not complete. JSON retained, original review unchanged/untracked, local commits only/no push.

### Fresh volatility/determinism metadata (N1/E2g2, 2026-10-24)

Base a6bf183. NodeDefinition now explicitly supplies Option<Volatility> and Determinism. Registry publication requires a subclass exactly for volatile nodes, limits package volatile authority to Entropy, requires Seeded entropy and Deterministic pure contracts, rejects volatile exec ports and rejects input-bearing declared/template state/clock readers. Core Get derives StateRead from typed VariableAccess::Read; bundled arithmetic declares None/Deterministic. Existing host/test construction sites explicitly supply the new fields. Stored NodeContract/signature JSON projections, graph formats, dependency pins/features and lockfile are unchanged. These are provider obligations, not authentication or behavioral conformance.

cargo test --workspace --locked passed 610 tests, six timing tests ignored. cargo test --release -p orange-model --test catalog_execution --lib --locked passed 134 tests (128 model unit tests and six focused public metadata tests). New evidence covers positive seeded entropy/data inputs and Core Get, subclass placement, package authority, pure/entropy determinism combinations, no volatile exec ports, host state/clock declared/template input rejection and immutable metadata across replacement. Existing suspension/descriptor/availability checks retain their earlier diagnostic precedence. Initial whole-workspace construction migration passed before focused tests were added; no remaining failing test or property seed.

cargo fmt --all --check; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings; node tools/check-cargo-metadata.mjs; --self-test (nine fixtures); git diff --check passed. Rust/Cargo 1.99.1, LLVM 23.1.1, x86_64-pc-windows-msvc, default workspace features and Node 24.18.0 unchanged; known OneDrive hard-link fallback only. Cargo.lock SHA-256 3D7948198E468B5F196956E2EEB1F748E515A8B33AF6B2864B3FAE696E537BE2 unchanged. Ignored logs .artifacts/e2g2-{initial,focused,tests-final,clippy-final,rustdoc-final,release-final}.log.

Unfrozen source migration: NodeDefinition literals must choose volatility/determinism explicitly; exhaustive RegistryError matches handle new admission errors. No package manifest/wire schema or graph migration is claimed. Entropy kernel/provider binding and compiled consumption are next; this does not complete E2, G-02, R1, runtime error envelopes or provider conformance. No new dependency scan, MSRV/other-OS run or NFR measurement. Review unchanged/untracked; local commits only/no push.

### Exact native entropy binding (N1/E2g3, 2026-10-24)

Base bcbfdf3. Added separate trusted EntropySource/EntropyKernel/EntropyBinding/EntropyKernelResolver APIs in orange-compiler. MathPackage registers random_u64_definition alongside its existing scalar definitions: org.orange.math/random-u64@1/config 1 is seeded entropy, no inputs, computed value: u64 and empty configuration. Native binding verifies exact provider/digest/version, empty specialization/config, fresh class and ordered semantic ports. ResolvedEntropyKernel retains the exact registry/provider. The callback validates arity before drawing, forwards one host u64 and never requests an OS source or reads config/document/registry during execution. No pure fold adapter exists.

cargo test --workspace --locked passed 615 tests, six timing tests ignored. cargo test --release -p orange-bundled --test entropy --locked passed five focused tests. A saved product graph passes fresh semantic lowering analysis but is explicitly rejected by the pure compiler/resolver. Tests cover 0/u64::MAX forwarding, zero draws for malformed ABI, source fault/no partial output, provider/digest/version/config/signature rejection, presentation-invariant versus semantic-drift binding and pinned old-snapshot execution after replacement. Review identified that full definition equality would wrongly reject localized port labels; the implementation compares semantic projections and the regression now passes. Initial test-only NodeId inference errors were corrected. A prior custom-catalogue default fixture now registers the newly added product definition before reusing the complete provider descriptor.

cargo fmt --all --check; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings; node tools/check-cargo-metadata.mjs; --self-test (nine fixtures); and git diff --check passed. Selected Rust/Cargo 1.99.1, LLVM 23.1.1, x86_64-pc-windows-msvc, default workspace features and Node 24.18.0 unchanged. Known OneDrive hard-link fallback only; no code lint failure. No manifest/dependency/feature/lock changes; Cargo.lock SHA-256 remains 3D7948198E468B5F196956E2EEB1F748E515A8B33AF6B2864B3FAE696E537BE2. Ignored logs .artifacts/e2g3-{initial,focused,tests-final,clippy-final,rustdoc-final,release-final}.log.

Limitations: the test host source checks callback behavior, not a seeded compiled graph. Actual frame/epoch integration and independent seeded graph oracle are still required for G-02/E2. Trusted callbacks do not become sandboxed, authenticated or interruptible through this API. No general actor/control execution, final global compilation readiness, NFR/allocator/timing, new dependency scan, MSRV or other-OS qualification. Graph JSON and original review unchanged; local commits only/no push.

### Reusable private constant demand folder (E1/E2g4a, 2026-10-24)

Base c50b00e. Extracted the existing demand work loop into a private ConstantFolder session for reuse by forthcoming entropy/data lowering. The public pure compiler still performs fresh semantic and whole-potential-cone admission first, charges export identities before callbacks, visits export roots in persisted order and retains shared producers, faults, warnings, byte/slot limits, cancellation and source/snapshot identity. The session reuses its scratch/work buffers across roots and rejects any non-pure node before resolver invocation. No public signature, language, schema, dependency, feature or lock change. This is a prerequisite refactor, not dynamic graph execution.

cargo test --workspace --locked passed 615 tests, six timing tests ignored. cargo test --release -p orange-bundled --test compile_pure --test reference_equivalence --locked passed 46 compiler/API/reference tests, including all existing numeric/option/Try properties, shared exports and deep lazy chains. No redundant tests of private structure were added; existing behavior regressions are the acceptance checks. Guarded extraction attempts left source unchanged until the exact final-export marker was found; initial moved-reference/needless-borrow issues were corrected and tests rerun.

Formatting, full Clippy/all targets and Rustdoc/no deps with warnings denied, dependency metadata guard/nine fixtures and whitespace checks passed. Selected Rust/Cargo 1.99.1, Windows x86_64 MSVC, default features, Node 24.18.0 and lock SHA-256 3D7948198E468B5F196956E2EEB1F748E515A8B33AF6B2864B3FAE696E537BE2 unchanged. Known OneDrive hard-link fallback only; no code lint failure. Ignored logs .artifacts/e2g4a-{initial,focused,tests-final,clippy-final,rustdoc-final,release-final}.log. No new timing/allocator/NFR, dependency scan, MSRV or other-OS evidence. Dynamic entropy graph integration remains next; E2/R1 incomplete. Review unchanged/untracked, local commit only/no push.

### Checked dynamic root data lowering (E1/E2g4b, 2026-10-24)

Base 3bbce4d. Added compile_data/DataPlan for the supported pure/seeded-entropy root computed-data subset. Shared admission checks fresh document/registry evidence, all root types/kinds/ports and whole potential-cone quotas before callbacks. DataPlan privately owns typed dense nodes/slots, original sources, initial constant outcomes versus dynamic absence, ordered exports/diagnostics, resource requirements and a pinned snapshot. The existing demand folder folds constant islands once across roots; static lazy selectors prune inactive cones before binding/folding. Epoch-dependent selectors retain both possible arms, whose eligible constant islands may fold during compilation. Entropy callbacks are bound separately and never run as constants. No frame, actor, control or function execution is implied.

Admission now explicitly rejects unsupported host intrinsics (including core/call) before generic kernel resolution. Review found that a valid pure function call could otherwise fall through to a generic resolver despite the documented unsupported function scope; the new test proves both entry points stop before callbacks. Ordinary package pure/entropy bindings remain open and ABI checked. Internal folder metadata maps are borrowed rather than cloned. No graph schema, dependency, feature or lock change; PurePlan API remains available.

cargo test --workspace --locked passed 623 tests, six timing tests ignored. cargo test --release -p orange-bundled --test compile_data --test compile_pure --test reference_equivalence --locked passed 53 tests (seven data-plan, 35 pure/API, 11 reference). New checks cover native Random plus a shared constant island with zero entropy execution, stable dense sources/types/resources, static true/false pruning including no inactive constant callbacks, constant Try/direct error provenance, stale/quota/cancellation/wrong entropy ABI, state/function rejection before callbacks and a reversed-storage 512-node epoch chain with shared exports/no rebind. The DataPlan constructor/default compile-fail test passes. Existing integer/float/selection/Option/Try properties stay green.

cargo fmt --all --check; workspace/all-target Clippy and workspace/no-deps Rustdoc with warnings denied; metadata dependency guard/nine fixtures; git diff --check passed. Selected Rust/Cargo 1.99.1, LLVM 23.1.1, x86_64-pc-windows-msvc, default features and Node 24.18.0 unchanged. OneDrive hard-link fallback only; no code lint failure. Cargo.lock SHA-256 remains 3D7948198E468B5F196956E2EEB1F748E515A8B33AF6B2864B3FAE696E537BE2. Ignored logs .artifacts/e2g4b-{check,focused,tests-final,clippy,clippy-final,rustdoc-final,release-final}.log.

Limits: DataPlan publication covers this checked lowering subset; dynamic output/type/fault publication and epoch memoization still require the runtime frame. Value/fault charge helpers are nominal payload budgets, not process-memory guarantees. Epoch-dependent possible arms may bind/precompute constant work; only statically inactive arms are eliminated. No seeded graph/G-02/E2 closure, final global compilation readiness, runtime error-envelope completion, actors/functions/control/services, NFR/allocator/timing, dependency scan, MSRV or other-OS qualification. JSON unchanged; original review unchanged/untracked, local commits only/no push.

### Checked root data consumption (E2g4c, 2026-10-24)

Base d1ee82a. DataActivation consumes checked DataPlan with explicit epochs, typed atomic publication, shared producer memoization, first-error demand, dynamic Select/And/Or/Coalesce/Try and scoped native entropy. Entry rejects stale document/registry evidence and enforces resource limits before frame allocation. Old epoch payloads remain charged until replaced. Observed cancellation aborts the epoch even if a callback clears the host flag; graph draw exhaustion remains catchable while infrastructure limits do not. Partial callback outputs are released on failure. Replay metadata identifies the algorithm, seed and activation sequence; host sequence allocation remains future work. Public unfrozen DataOperation::Try now records boxing and DataPlan exposes checked initial charges/export producers.

cargo test --workspace --locked passed 638 tests, six timing tests ignored. cargo test --release -p orange-bundled --test data_epochs --test compile_data --locked passed 20 tests. cargo test --release -p orange-runtime --locked passed nine unit tests and one compile-fail Rustdoc. The 13 frame regressions cover native seeded diamonds/replay/epochs, first-error demand and nested Try, retained-byte limits, cancellation and draw budgets, freshness/pinned execution, malformed callback output and scratch release, all supported dynamic lazy forms, a 512-node chain, entry limits and multi-output conversion. Test-only seeded bool/option/float providers exercise the open ABI and are not shipped random families.

Formatting, workspace/all-target Clippy and no-deps Rustdoc with warnings denied passed; dependency metadata guard/nine fixtures passed. Selected Rust/Cargo 1.99.1, LLVM 23.1.1, x86_64-pc-windows-msvc, default workspace features and Node 24.18.0 unchanged. Known OneDrive hard-link fallback only. Cargo.lock SHA-256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152: only local runtime-to-compiler and bundled dev-to-runtime edges added, no external versions/features changed. Ignored logs .artifacts/e2g4c-{tests-final,clippy-final,rustdoc-final,release-final,runtime-release}.log.

Limits: independent seeded source-graph equivalence remains next; no full G-02/E2 closure. This synchronous root-data frame does not implement actors, control/functions, variable/clock/event/service execution or the complete runtime error envelope. Nominal payload quotas are not allocator/process-memory guarantees; trusted callbacks cannot be preempted inside arbitrary native work. No new timing/allocator/NFR, dependency scan, MSRV or other-OS qualification. JSON retained, review unchanged/untracked, local commit only/no push.

### Independent seeded root graph equivalence (E2g4d, 2026-10-24)

Base81c169c. Extended the existing direct-source reference evaluator with UUID-keyed invocation counters and per-epoch caches. No compiler plans, dense slots/entropy scope objects or native arithmetic callbacks are read. The versioned EntropyStream primitive is shared deliberately; previous independent .NET SHA vectors anchor its encoding. Oracle constant recomputation is intentional and separate callback-counter tests establish production constant reuse. Extracted the existing downstream seeded ABI fixture into shared test support without changing shipped providers.

Two proptest properties run 128 cases each: bounded U64 DAGs with four math operations/random leaves/constants, literal zero divisors, reversed storage/export demand, random demand masks, three epochs, repeated reads, JSON import and fresh activation replay; and dynamic Select/And/Or/Coalesce, nested options and Try with faults across epochs and reversed demand. A focused regression verifies declared first-fault provenance, two idle epochs before first entropy demand and activation-sequence separation. Corrected an initially invalid option None test literal before admission; no production mismatch or unresolved failing seed. The temporary invalid-fixture regression seed was removed.

cargo test --workspace --locked passed 641 tests, six timing tests ignored. cargo test --release -p orange-bundled --test seeded_equivalence --test reference_equivalence --test data_epochs --locked passed 27 tests. cargo fmt --all --check; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings; dependency metadata guard/nine fixtures and whitespace checks passed. Rust/Cargo1.99.1, LLVM23.1.1, Windows x86_64 MSVC, default features and Node24.18.0 unchanged; known OneDrive hard-link fallback only. Lock SHA-256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152 unchanged. Logs .artifacts/e2g4d-{focused,tests-final,clippy-final,rustdoc-final,release-final}.log.

Supported root-data E2 prerequisite evidence now exists for diamonds, first errors, lazy forms, seeded scopes and nested error capture. This is not every graph family, an independent SHA/IEEE implementation, performance/NFR/allocator qualification, full runtime error envelopes or global G-02 readiness. E3 control/functions and R1–R7 remain open. No production/public API/schema/dependency change, JSON retained, original review unchanged/untracked; local commit only/no push.

### Root-data local timing baseline (E2g4e, 2026-10-24)

Base99386af. Added a manual ignored release-only probe to data_epochs; ordinary CI has no time threshold. Real JSON/registry/native plans contain one Random-u64 followed by zero/63/511 dependent U64 additions. The measured plan uses MathPackage directly, without test counting wrappers. Timed operations are cached export, begin_epoch plus a leaf export, begin_epoch plus the full chain, and frame construction/destruction. Analysis/compilation are outside execution samples. Every size warms actual full execution for ten seconds, followed by30 separate batches per operation. Results below are ns/op quantiles of batch means, not individual event-latency percentiles or throughput confidence intervals. Work/results pass through black_box; raw sorted batch means are in ignored .artifacts/e2g4e-timing-final.log.

Command: cargo test --release -p orange-bundled --test data_epochs root_data_epoch_timing_baseline --locked -- --ignored --exact --nocapture --test-threads=1. Passed1 manual timing test in30.18s. Rust1.99.1/Cargo1.99.1/LLVM23.1.1, x86_64-pc-windows-msvc, default features, optimized Cargo release profile, no debugger. Local Windows NT10.0.26300.0, AMD Ryzen9 9900X3D12-core/24 logical processors. CPU name read from the local hardware registry; CIM access unavailable. No affinity/power/background-load/reference-machine qualification. Earlier exploratory runs used counters or omitted raw samples and are not the baseline below.

| Nodes | Operation | Batch size | Median ns/op | p95 batch mean | p99/max batch mean |
|---|---|---:|---:|---:|---:|
|1|Cached export|10000|5.0|6.5|6.5|
|64|Cached export|10000|6.0|9.5|15.7|
|512|Cached export|10000|6.0|12.2|13.0|
|1|Sparse leaf epoch|1000|108.4|169.3|243.5|
|64|Sparse leaf epoch|1000|114.8|124.4|135.6|
|512|Sparse leaf epoch|1000|111.8|116.0|132.2|
|1|Full epoch|100|112.5|1332.0|1426.0|
|64|Full epoch|100|3060.0|4521.0|5787.0|
|512|Full epoch|100|25026.5|28535.0|28734.0|
|1|Frame entry/drop|100|247.0|370.0|396.0|
|64|Frame entry/drop|100|1532.0|1826.0|1873.0|
|512|Frame entry/drop|100|7838.5|10494.0|11462.0|

Cached and sparse reads show no material dependence on total chain size in this probe; full traversal and frame initialization increase with demanded/materialized size. No production optimization is justified from these results. In particular, do not extrapolate these small chains to W1/W2 or claim NFR01/04/05 passed. Allocation instrumentation was not executed: source inspection finds preallocated frame work/input/output vectors, but nested Try intentionally allocates error/success boxes and trusted kernels may allocate. A source review is not allocator evidence. The workspace forbids unsafe code, so no custom GlobalAlloc implementation was added to its tests; WPR/xperf are available but no system tracing or instrumentation was started.

cargo test --workspace --locked passed641 tests, seven timing tests ignored (one newly added). Workspace/all-target Clippy with warnings denied, fmt check, no-deps Rustdoc with warnings denied, metadata guard/nine fixtures and whitespace checks passed. Normal runtime regressions remain unchanged. Lock SHA-256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152 unchanged; original review unchanged/untracked. No product behavior/API/schema/dependency change or push. Core control/functions/actors/services and NFR qualification remain open.

### Bounded default plan diagnostics (E2g4f, 2026-10-24)

Baseec94702. Focused data-flow audit found derived DataPlan/PurePlan Debug traversing retained values and pinned registry state, EvaluatedExport Debug traversing its outcome, and KernelBinding/EntropyBinding Debug calling arbitrary downstream kernel formatters. Formatting a large JSON plan could therefore copy payloads into diagnostic logs and expand with graph/catalogue size. This is a default diagnostic exposure risk, not evidence that this repository stored or leaked credentials.

Default plan Debug now reports fixed metadata/counts only; export Debug reports source and value/fault category; binding Debug reports port counts without calling kernel Debug. PureEvaluation inherits its plan's summary. All existing explicit typed value/kernel/registry access and Debug trait bounds remain available. Formatting strings are not a stable wire interface. No blanket Value/GraphDocument redaction is claimed: hosts must deliberately authorize and bound explicit value inspection.

A regression compiles a large private-marker JSON value through both plan kinds, proves direct reads preserve it and verifies pretty Debug for DataPlan/PurePlan/PureEvaluation/EvaluatedExport omits it with output under1024 bytes. Another uses callbacks whose Debug/execution panic, proving binding formatting invokes neither. cargo test --workspace --locked passed643 tests, seven timing tests ignored. cargo test --release -p orange-bundled --test compile_data --test data_epochs --test seeded_equivalence --locked passed24 tests with one manual timing test ignored; cargo test --release -p orange-compiler binding_debug_does_not_call_downstream_formatters --locked passed1 focused unit test. Formatting, workspace/all-target Clippy and no-deps Rustdoc with warnings denied, metadata guard/nine fixtures and whitespace checks passed. Known OneDrive fallback only.

Rust/Cargo1.99.1, LLVM23.1.1, Windows x86_64 MSVC/default features unchanged. Lock SHA-256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152 unchanged. Logs .artifacts/e2g4f-{focused,tests-final,clippy-final,rustdoc-final,release-final,binding-release}.log. The focused audit consulted the online [RustSec advisory index](https://rustsec.org/advisories/) on2026-10-24 (search result crawled today); this is not a pinned/downloaded advisory database or full lockfile scan. cargo-audit is not installed, and no dependency vulnerability-clear claim is made. No dependency changes, actor/control/core completion, allocator/NFR/MSRV/other-OS qualification or push. Original review unchanged/untracked.

### Recent-commit review corrections (2026-10-24)

Base29fb6ba. Reviewed recent checked data lowering/runtime, entropy scopes/native binding, metadata admission, source-oracle and diagnostic changes. Corrected two reproduced runtime boundary gaps: begin_epoch cancellation previously left an old active epoch readable after flag clearing; and a downstream callback could discard a host entropy-budget error and publish success or replace its fault. The focused cancellation review probe and new budget regression both failed before their corrections. No additional confirmed correction was needed in the inspected binding/admission/boxing/source-map paths; this is a focused review, not security or whole-core clearance.

Activation checkpoints now consistently invalidate an active scope when observing cancellation. Rejected initial starts still leave NoEpoch and consume no entropy identity; successful explicit starts alone recover evaluation. Epoch counter failure also latches the infrastructure error rather than leaving old results readable. ScopedSource retains its first host exhaustion fault; callback success/replacement error cannot hide it. Cancellation takes infrastructure precedence. Tests cover zero requests at zero budget, exact/exceeded draws, direct/Try producer faults, repeated epochs, rejected initial/active starts, missing-export reads, explicit recovery and invocation continuity. The existing cancellation callback test now exercises both propagated and swallowed source errors after flag clearing.

Reconciled stale current-scope statements in README and03, and added adopted volatility/determinism fields to12's implementation note. Public signatures, dependencies/features, graph JSON and typed result representation are unchanged. Scratch work remains preallocated; this does not establish measured allocation counts or eliminate intentional nested Try boxes. No speculative optimization or new allocator dependency was introduced. Allocator/NFR, actor/control/functions/services and full runtime envelopes remain open.

cargo test --workspace --locked passed645 tests, seven timing tests ignored. cargo test --release -p orange-bundled --test compile_data --test data_epochs --test seeded_equivalence --test compile_pure --locked passed61 tests, one manual timing probe ignored. cargo fmt --all --check; cargo clippy --workspace --all-targets --locked -- -D warnings; cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings; node tools/check-cargo-metadata.mjs and --self-test (nine fixtures); git diff --check passed. Selected rustc1.99.1(58cf8f9c3), Cargo1.99.1(5f94df478), LLVM23.1.1, x86_64-pc-windows-msvc/default workspace features unchanged. Known OneDrive hard-link fallback only. No new dependency-advisory/MSRV/other-OS run or timing/allocator measurement. Cargo.lock SHA-256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152 unchanged. Logs .artifacts/review-fix-{budget-before,focused,tests-final,clippy-final,rustdoc-final,release-final}.log and earlier .artifacts/small-review-cancel-proof.log. Original review SHA-256 D8F25A2F3DF04A88578972340B3CA3B591DC42126083B107A60344B00125D553 unchanged/untracked. Local commit only/no push.

### E3 runtime prerequisites (2026-10-24)

Base 6e2d56b. Four slices close gaps that E3 control and function lowering would otherwise turn into bugs. DataActivation holds an owned CancelToken instead of borrowing an AtomicBool. DataLimits::max_activation_operations adds non-refilling fuel across epochs, with the sticky FuelExhausted error. EntropyKey gains a FrameId call-path scope, ActivationEntropy keeps bounded per-frame counters, and the algorithm moves to orange.entropy.sha256-counter@2. Known-answer vectors for two blocks, the final block, a derived callee frame and its first word were recomputed with .NET SHA-256 through PowerShell; the same script first reproduced the retired @1 vector. DataOperation::Input with DataActivation::set_input lets the host supply core/get-variable values per epoch; compile_pure still rejects state reads.

Breaking public changes: DataOptions and DataActivation lose their lifetime parameter; ActivationEntropy::new takes max_frames; EntropyKey has a frame field; DataError and DataOperation gain variants. Graph JSON, dependencies, features and Cargo.lock are unchanged.

cargo test --workspace --locked --offline passed 650 tests, seven timing tests ignored. Release runs of data_epochs, seeded_equivalence and compile_data passed 28 tests, one timing probe ignored. cargo fmt --all --check, cargo clippy --workspace --all-targets --locked --offline -- -D warnings, cargo doc --workspace --no-deps --locked --offline with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs and its nine-fixture self-test, and git diff --check passed. rustc 1.99.1 (58cf8f9c3), Cargo 1.99.1 (5f94df478), x86_64-pc-windows-msvc, default features. Cargo.lock SHA-256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152 unchanged. No MSRV, other-OS or timing run. Not yet built: control lowering, Execution/Payload/Iteration sources, function frames in plans, the run actor and a cancellation tree. Local commits only, no push.

### Deep-analysis fixes (2026-10-24)

Base e577291. Four confirmed defects were fixed:

- A runtime publication rejected by the retained-byte limit no longer keeps its value alive in scratch storage.
- `Fragment::paste` drops top-level edges whose endpoints the fragment lacks, instead of panicking (paste.rs remap index).
- Graph loading rejects repeated JSON object keys at any depth with `CodecError::DuplicateKey`, after the version gate. Before this, serde silently kept the last value.
- `GraphDocument::scratch` rolls back during unwinding, and inference planning then resets its kept DocIndex.

Each regression test was confirmed to fail without its fix, except the paste test: the panic was reproduced by an earlier probe that has since been removed.

The duplicate-key scan runs inside the existing envelope pass. Objects of up to 16 keys use a short list before a hash set. One local release run of perf_codec measured 10k-node load at 53.6 ms before and 58.9 ms after; save was unchanged. That is a single run, not a qualified measurement.

cargo test --workspace --locked --offline passed 656 tests, seven timing tests ignored. cargo fmt --all --check, cargo clippy --workspace --all-targets --locked --offline -- -D warnings, cargo doc --workspace --no-deps --locked --offline with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs with its nine-fixture self-test, and git diff --check passed. Environment: rustc 1.99.1 (58cf8f9c3), Cargo 1.99.1 (5f94df478), x86_64-pc-windows-msvc, default features. Cargo.lock SHA-256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152 is unchanged. Public change: the new CodecError::DuplicateKey variant. Local commits only, no push.

### Dominator-based availability (2026-10-24)

Base 6d45a07. validate_availability now answers reads from one Cooper–Harvey–Kennedy dominator tree per activation root ([11 §7.2](11-graph-language.md)) instead of one reachability walk per producer and outcome set. The walk remains the reference and the fallback. A property test (512 random control-flow graphs with all/any forks, foreign labels and overlapping outcome sets) checks that the two agree. The new DomTree is checked against brute-force vertex-removal reachability. Two deliberate mutants were rejected by the property test: inverted dominance, and skipping the join-all recheck. Their shrunk seeds are kept in proptest-regressions.

New document-level tests cover a 100-action chain (all available, then one premature read) and a premature read after a `join: all` Fork. The ignored perf_edit_baseline timing now goes up to 16,000 actions. One local release run of validate_availability on the action/data chain:

| Actions | Before | After |
|---|---|---|
| 1,000 | 60.5 ms | 6.2 ms |
| 2,000 | 256.9 ms | 13.5 ms |
| 4,000 | 1,139 ms | 29.2 ms |
| 16,000 | not run | 128.5 ms |

The 4,000 "before" value comes from the deep-analysis probe, not this test. These are single runs, not NFR evidence.

cargo test --workspace --locked --offline passed 663 tests, seven timing tests ignored. cargo fmt --all --check, cargo clippy --workspace --all-targets --locked --offline -- -D warnings, cargo doc --workspace --no-deps --locked --offline with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs with its nine-fixture self-test, and git diff --check passed. Environment: rustc 1.99.1 (58cf8f9c3), Cargo 1.99.1 (5f94df478), x86_64-pc-windows-msvc, default features. Cargo.lock is unchanged. Diagnostics and public signatures are unchanged.

Remaining limits: the control-flow graph still uses BTree vertices with string port keys (about 8 µs per action at 16,000), validation is still whole-document, and NFR-15 is not met. Local commits only, no push.

### Projection caching (2026-10-24)

Base f4036f6. The node pass and edit persistence now share projections among nodes of one registry-declared definition with equal solved arguments (and, for validation, the same body) ([12](12-graph-data-structures.md), projection caching). The node pass also decides drift by comparing semantic keys, hashing each stored snapshot once per document key instead of twice per node.

Profiling (temporary instrumentation, since removed) showed that the projection loop spent most of its time in `signature::compare`, which computes two canonical-JSON SHA-256 keys per node. Caching the projection alone gave about 10%.

New regressions:

- Three nodes sharing a projection report drift only on the node with a stale snapshot, and a shared failed projection is reported on each node.
- Two nodes of one two-parameter definition in one component keep distinct signatures.

Removing the arguments from either cache key fails tests. The edit-path mutant survived the existing suite until the second test was added.

One local release run of perf_edit_baseline::edit_cost_by_document_size on the generic add chain:

| Step | Size | Before | After |
|---|---|---|---|
| validate_nodes | 1,000 | 29.1 ms | 14.7 ms |
| validate_nodes | 10,000 | 312.5 ms | 167.2 ms |
| validate | 1,000 | 42.2 ms | 26.4 ms |
| validate | 10,000 | 486.3 ms | 340.6 ms |
| apply_inferred_indexed connect, tail of chain | 10,000 | 251.9 ms | 176.6 ms |
| apply_inferred_indexed connect, isolated pair | 10,000 | 120 µs | 105 µs |

These are single runs, not NFR evidence. The remaining node-pass cost is generic inference (about 14 µs per node at 10k) and per-node work for contextual contracts.

cargo test --workspace --locked --offline passed 665 tests, seven timing tests ignored. cargo fmt --all --check, cargo clippy --workspace --all-targets --locked --offline -- -D warnings, cargo doc --workspace --no-deps --locked --offline with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs with its nine-fixture self-test, and git diff --check passed. Environment: rustc 1.99.1 (58cf8f9c3), Cargo 1.99.1 (5f94df478), x86_64-pc-windows-msvc, default features. Cargo.lock is unchanged. Diagnostics, plans and public signatures are unchanged. Local commits only, no push.

### Inference speedups (2026-10-24)

Base 619b2dc. Temporary phase timers (since removed) on the 10,000-node generic add chain:

| Phase | Time |
|---|---|
| Component search | 25.5 ms |
| Equation building | 40.8 ms |
| Unification | 26.1 ms |
| Argument resolution | 17.8 ms |

Inside those phases, `data_edges`, `port_term`, `declared_port` and `generic` dominated, because every data edge was resolved from both ends in the search and again in equation building. A first attempt that memoized port descriptors and terms per port was reverted: cloning the cached values cost as much as recomputing them.

The kept changes:

- Each data edge, with its endpoint terms and variable-link flag, is resolved once per call and keyed by EdgeId, and node templates are shared through Rc (e6ab6b6).
- The unifier borrows equation context instead of cloning it per applied term and argument (01ff474).
- Solved-argument checks are shared by parameter declaration, solved type and body, and a failure is retargeted to each node (f8079a9).

Mutation checks against the full model suite:

- Skipping the retarget: caught.
- Dropping the solved type from the check key: caught.
- Letting every edge link variables: survived the existing suite. A new test now pins that concrete edges at a generic node stay with the validator.

One local release run of perf_edit_baseline::edit_cost_by_document_size:

| Step | Size | Before | After |
|---|---|---|---|
| validate_nodes | 1,000 | 14.7 ms | 7.8 ms |
| validate_nodes | 10,000 | 167.2 ms | 98.7 ms |
| validate | 1,000 | 26.4 ms | 17.4 ms |
| validate | 10,000 | 340.6 ms | 230.8 ms |
| apply_inferred_indexed connect, tail of chain | 10,000 | 176.6 ms | 110.2 ms |
| apply_inferred_indexed connect, isolated pair | 10,000 | 105 µs | 97 µs |

These are single runs, not NFR evidence. Whole-document validation and whole-component re-inference on edits remain, so NFR-15 is still unmet.

cargo test --workspace --locked --offline passed 667 tests, seven timing tests ignored. cargo fmt --all --check, cargo clippy --workspace --all-targets --locked --offline -- -D warnings, cargo doc --workspace --no-deps --locked --offline with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs with its nine-fixture self-test, and git diff --check passed. Environment: rustc 1.99.1 (58cf8f9c3), Cargo 1.99.1 (5f94df478), x86_64-pc-windows-msvc, default features. Cargo.lock is unchanged. Inference results, issues and public signatures are unchanged. Local commits only, no push.

### Analysis session skeleton (V12a, 2026-10-24)

Base b612f3f. `validation::AnalysisSession` keeps a document's combined analysis across committed edits and states what each update did (`EditStats`, `SessionWork`, `RebuildReason`). In this slice every semantic change recomputes the whole analysis, so visit counts equal the document size; dirty-unit recompute follows in V12b–V12e. Layout-only changes rebind without work. Mismatched revisions, registry, limits or effects changes, and cancelled results rebuild with a reason.

Evidence:

- Known-answer tests cover creation, recompute, undo, layout rebind (still ready for lowering), skipped changes, registry and limits changes (including the full pass's over-quota rejection), equal and changed leaf effects, and cancellation.
- Two property tests (64 cases each, up to 15 random edits from an empty document and from the saved example) compare the session with `analyze_with_options` after every commit, rollback, undo and redo. They also assert that no update was a state-mismatch rebuild.
- Mutants that skip the layout rebind, or ignore cancellation, fail these tests.

cargo test --workspace --locked --offline passed 674 tests, seven timing tests ignored. cargo fmt --all --check, cargo clippy --workspace --all-targets --locked --offline -- -D warnings, cargo doc --workspace --no-deps --locked --offline with RUSTDOCFLAGS=-D warnings, node tools/check-cargo-metadata.mjs with its nine-fixture self-test, and git diff --check passed. Environment: rustc 1.99.1 (58cf8f9c3), Cargo 1.99.1 (5f94df478), x86_64-pc-windows-msvc, default features. Cargo.lock is unchanged.

Public additions: AnalysisSession, EditStats, SessionWork and RebuildReason (the last three `#[non_exhaustive]`), plus `PartialEq` on ValidationAnalysis. No timing claim: V12a does not change edit cost.

## Structured headless control verification — 2026-10-24

Integrated work on `core/headless-control-runtime`, based directly on fetched dev `1a901281f6bdf8fb274a6064a35335d4822800ef`. The selected E3/R1 cohort implements checked Start/Stop/Branch/Sequence/Count/While/Break/Continue/Wait/Yield and local/run Get/Set execution, bounded resumable data/control work, virtual time, staged drain/cancel/dispose and narrow timer-continuation ownership. General events, Fork/functions, affine resources and service adapters are not implemented by this cohort. V12a implementation remains on its own branch; no topic commits were copied.

Environment: rustc 1.99.1 (58cf8f9c3, LLVM 23.1.1), Cargo 1.99.1 (5f94df478), Windows x86_64 MSVC, default features, Node.js 24.18.0. Reported OS kernel: Microsoft Windows NT 10.0.26300.0; 24 available logical processors. CPU model enumeration through CIM was unavailable, so this is not named-hardware qualification. Toolchain/features/external dependency versions are unchanged. Cargo.lock SHA-256 remains `E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152`. The configured human Git author was retained; no generated attribution trailers were added.

| Executed command/check | Result |
|---|---|
| `cargo test --workspace --locked` | PASS: 699 tests, 8 manual timing tests ignored; includes checked-construction compile-fail tests |
| `cargo test -p orange-bundled -p orange-compiler -p orange-runtime --release --locked` | PASS: 153 tests, 2 manual timing tests ignored |
| `cargo test -p orange-bundled --release --locked --test control headless_control_timing_baseline -- --ignored --nocapture` | PASS: one explicitly invoked local timing probe |
| `cargo run -p orange-bundled --example headless_control --locked` | PASS: `total=3; virtual_time_ns=10`; Count accumulation, virtual Wait, Branch, graceful stop and dispose |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo doc --workspace --no-deps --locked`, with `RUSTDOCFLAGS=-D warnings` | PASS; caller environment restored |
| `node tools/check-cargo-metadata.mjs --self-test` | PASS: nine policy fixtures |
| `node tools/check-cargo-metadata.mjs` | PASS: core dependency boundary |
| `node tools/check-cargo-metadata.mjs --msrv` | PASS: declares 1.97.1; this is metadata, not an MSRV build |
| `git diff --check` | PASS |

Only Rust 1.99.1 is installed locally. Rust 1.97.1 compilation, Linux/macOS execution and full dependency/security CI are not claimed by local checks; repository CI owns those checks on the PR. The Windows incremental cache emitted hard-link warnings and successfully copied files instead; the recorded commands exited successfully. No dependency, feature, lockfile, graph JSON or schema-version changes were needed.

### Behavior and review evidence

Thirty focused control tests pass. A test-only independent source interpreter scans saved edges and uses lexical regions; it never reads compiler blocks/slots, runtime cursors or native kernels. Ninety-six generated nested Count/While/Sequence/Branch/Wait/state/seeded cases use proptest 1.12.0 with fixed runner seed 20261024 at work budgets 1, 3 and 64. The fixed-seed API was checked in the installed versioned `proptest-1.12.0/src/test_runner/config.rs` release source. The oracle shares fresh validation metadata, the model literal codec and the versioned entropy stream primitive anchored by prior external vectors. It is not an independent validator, IEEE implementation or cryptographic proof.

Known answers cover zero/negative Count, fresh indices, nested Break/Continue, initial-false While, exact iteration bounds, suspended Sequence/loop bodies, source/storage/presentation invariance, stale entry versus pinned active snapshots, original Get fault provenance, Error as data versus successful Try, shared-state snapshots across interleaved data safe points, wait-completion/stop ordering at the same time, cleanup entropy continuity, stop-run fault policy, cleanup rejection/fuel/deadline/second cancellation, zero-time wait-cycle fuel, 512 nested iterative regions and fail-closed depth limits. Explicit disposal releases the plan; late time/execution operations are rejected. Segment inspection reports written IDs only; Eq/change notifications and Watch dispatch remain R2b work.

Self-review followed compiler admission and original-document demand roots, implicit/variadic outcome resolution, source writers, input-fault publication, actor ownership, epoch resume, loop cursor lifetimes, timer retirement, every stop path and default Debug/inspection. A producer index replaces repeated edge scans in control demand admission. The example uses public consumer APIs and actual bundled provider metadata. Independent oracle tests supplement self-review and are not independent human approval.

The embedding host serializes commands through exclusive `&mut Run` methods and drives pump/time advancement; no redundant command queue, worker thread or OS clock was introduced. Idle Running remains live; virtual deadlines require the host to advance time. Native callbacks cannot be preempted, and configured retained-value limits are not hard process RSS guarantees. Start still conservatively requires exact document/registry state; semantic plan reuse after layout-only edits belongs to later incremental work. Public Rust enums/types are unfrozen; PlanType gains Duration/Error, DataError gains PullInProgress, and the new control/run surfaces require downstream source review before API freeze.

### Local control timing baseline

One native release probe, default features, seed 42, a 1,000-iteration Count accumulating a u64 through Get/Add/Set. Compilation is excluded; start/frame preparation, run, stop and dispose are included. Tracing is disabled. Ten-second warmup, 30 samples, 8 whole runs per sample; each run uses 8,004 counted work items. Batch-mean ns/run: median 213118.8 (about 213 microseconds), p95 215812.5, maximum 216437.5. This is not a tail-latency, allocation, cross-OS, NFR-02/05/15 or throughput-concurrency qualification.

Sorted batch means (ns/run): 210150, 210525, 210875, 211250, 211475, 211912.5, 212312.5, 212325, 212337.5, 212587.5, 212700, 212700, 212775, 212862.5, 212962.5, 213275, 213325, 213350, 213650, 213812.5, 213950, 214375, 214387.5, 214450, 214500, 214512.5, 214825, 215075, 215812.5, 216437.5.

The supported headless cohort is qualified locally, not the complete E3/R1/R3 rows, general core completion, G2 API freeze, installed-package lifecycle, concrete services, editor or release gates.

## PR #18 review corrections and verification (2026-10-24)

The deep review at `62a115c` reproduced five defects/policy gaps in debug and release. Corrections at `9b958af` and `c035fa2` retain the supported subset and leave the PR unmerged:

| Finding | Correction and durable regression |
|---|---|
| Unrelated host inputs could exhaust data retention in an inactive path; execution scanned every input per consumer | Shared per-consumer dependency projections, bounded metadata and demand-only publication. Regressions cover an inactive control arm, an inactive dynamic Select arm at pump budgets 1/1000, projection admission, existing snapshot interleaving and original fault/index provenance. |
| Control-only Yield cycles ignored cancellation until run fuel exhaustion | Check the cancellation signal at every control/timer safe point. A concurrent cancellation during a two-million-item pump now stops successfully; existing cleanup/second-stop tests remain green. |
| Two Get/Add/Set activations lost an increment at pump(1) | A pending Set retains scheduling ownership across internal data slices without retrying entropy. Both pump(1) and pump(1024) produce total 2. Read-only snapshot interleaving remains supported. |
| Initializers and constants each spent the entire configured retention pool | One combined compile budget, with remaining allowance passed to data lowering. A 356-byte plan rejects a 256-byte limit and accepts 512. |
| Saturated ordinary activation capacity rejected graceful stop | Separate bounded shutdown capacity, frames and ready storage reserved at startup. Tests cover a waiting run at normal capacity, actual On Stop mutation/drain, insufficient shutdown reserve rejection at start, and no On Stop execution on immediate stop. |

Selected local environment: Windows x86_64 MSVC, rustc 1.99.1 (58cf8f9c3 2026-10-13), cargo 1.99.1 (5f94df478 2026-08-27), Node.js 24.18.0, default features. MSRV remains 1.97.1. Cargo.lock is unchanged, SHA-256 `E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152`; no dependencies or features added.

| Executed command | Result |
|---|---|
| `cargo test --workspace --locked` | PASS: 707 passed, 9 manual timing tests ignored |
| `cargo test -p orange-bundled -p orange-compiler -p orange-runtime --release --locked` | PASS: 161 passed, 3 manual timing tests ignored |
| `cargo test -p orange-bundled --test control --locked` | PASS: 38 passed, 2 manual timing tests ignored; includes eight added behavior regressions |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS=-D warnings` | PASS; caller environment restored |
| `node tools/check-cargo-metadata.mjs --self-test` | PASS: 9 fixtures |
| `node tools/check-cargo-metadata.mjs` / `--msrv` | PASS: boundary / declared MSRV metadata |
| `cargo run -p orange-bundled --example headless_control --locked` | PASS: `total=3; virtual_time_ns=10` |
| `git diff --check` | PASS |

The incremental filesystem cache emitted hard-link warnings and successfully copied files; these commands passed. Local Rust 1.97.1, Linux/macOS execution and security/dependency CI require remote checks on the new candidate. Earlier green CI on `62a115c` does not qualify these corrections.

### Repeated local scaling and control probes

Executed `cargo test -p orange-bundled --release --locked --test control review_unrelated_host_input_scaling -- --ignored --nocapture`. Count performs 1,000 literal bool writes, with 0/64/512 unrelated Get/Branch pairs. Compilation excluded; start/frame preparation, drive and drop included; explicit stop/dispose excluded; tracing disabled. Each size warms up one second and records 30 individual runs. All sizes now consume exactly 3,003 work items; the probe asserts this.

| Unrelated inputs | Before median µs | After median µs | After p95 µs |
|---|---:|---:|---:|
| 0 | 103.6 | 107.8 | 109.8 |
| 64 | 1797.5 | 111.4 | 112.4 |
| 512 | 13633.5 | 123.7 | 139.8 |

The remaining size difference includes startup/frame preparation; this does not establish zero allocation, fixed RSS, tail latency or an NFR/platform gate.

Also repeated `cargo test -p orange-bundled --release --locked --test control headless_control_timing_baseline -- --ignored --nocapture`: the existing Count-1000 Get/Add/Set probe includes start/run/stop/dispose, tracing disabled, 10-second warmup, 30 samples of eight runs. Updated counted work is 9,003 items/run because actual host demand/resume is explicit. Batch means: median 213443.8 ns/run, p95 229200.0, maximum 230287.5. The previous section's 8,004-item figure describes the earlier candidate only. Both explicitly invoked probes passed.

Public source compatibility: unfrozen RunLimits adds max_stop_activations (default 256); exhaustive struct literals need the new field. Public DataPoll and missing-input errors keep their behavior. No persisted graph/schema changes. Full core, host/package/service, UI and API freeze gates remain open.

## Whole-codebase reuse qualification — 2026-10-24

The review of `3203195` expanded beyond PR #18 to the model, registry/type/edit/inference/validation paths, compiler, runtime, bundled-provider boundaries and repository tooling. The eight identified changes are implemented on the same topic branch:

| Area | Implemented change and retained contract |
|---|---|
| Connection planning | Additive `plan_edit_indexed` and `plan_connection_indexed` reuse a retained index and journaled trial edits. Existing read-only APIs remain available. Equivalence cases cover direct/converter/selection outcomes; rejection and a panicking identity source preserve content/index. Freshness, shadowed literals, imported conflicts and exact provider selection remain unchanged. |
| Validation projections | Scoped memoization shares successful descriptors and lookup failures through body passes and final analysis checks. The settled resolution hands off its cache; new rounds/invocations create fresh scopes. Private Rc sharing avoids locks/atomic reference counts. Public document, registry and analysis types still satisfy Send + Sync. |
| Graph algorithms | Data, control and function analysis use one private ordered iterative SCC primitive. Each caller keeps its own legality and witness rules. A 64-case bounded graph property compares partitions against independent mutual reachability; existing deep/cycle/function cases remain green. |
| Diagnostics | One declaration generates enum/serde identity, stable codes, translation keys, severity and safe fallback text. Existing order, wire values and redacted argument behavior are preserved. |
| Activation cells | Separate dense run/local layouts remove cross-owner placeholders. Only Index/Error projections allocate execution-output cells. Stable plan indices map to these private layouts; disposal releases them. |
| Runtime preparation | One checked immutable prepared data layout/initial charge is shared by a run's frames. Mutable values, memos, entropy counters, fuel, limits and cancellation remain activation-owned. Frame admission and pinned running providers remain intact. |
| Compiler bookkeeping | Output slots and producers share one entry keyed by original node/port; traversal storage is reused across roots. A small helper constructs control input roots. Implicit Error and loop-index projections retain distinct writers. |
| Executor structure | Explicit private HostPoll input demand replaces the side channel; PullState groups consumer/frozen-input/pending-publication state and reuses capacity. Cursor resume and action completion have separate helpers. Public DataPoll, pump(1), Set atomicity, lazy demand and original fault provenance remain unchanged. |

Environment: Windows x86_64 MSVC, rustc 1.99.1 (58cf8f9c3 2026-10-13), Cargo 1.99.1 (5f94df478 2026-08-27), Node.js 24.18.0, default features. MSRV remains 1.97.1. No dependencies/features/graph format/schema versions changed. Cargo.lock SHA-256 remains `E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152`.

| Executed command | Result |
|---|---|
| `cargo test --workspace --locked` | PASS: 712 tests, 11 manual probes ignored |
| `cargo test --workspace --release --locked` | PASS: 712 tests, 11 manual probes ignored |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS=-D warnings` | PASS; environment restored |
| `node --test tools/release.test.mjs` | PASS: 7 tests |
| `node tools/check-cargo-metadata.mjs --self-test` | PASS: 9 fixtures |
| `node tools/check-cargo-metadata.mjs` / `--msrv` | PASS: core boundary / declared MSRV metadata |
| `cargo run -p orange-bundled --example headless_control --locked` | PASS: `total=3; virtual_time_ns=10` |
| `git diff --check` | PASS |

Windows incremental-cache hard-link warnings fell back to copying; the recorded commands exited successfully. Local metadata success is not an MSRV build. Rust 1.97.1, Linux/macOS and dependency/security qualification require remote CI on the final head. Earlier green checks do not qualify these changes. No merge is included.

### Measured reuse and limits

Explicit release probes, with compilation/fixture construction excluded:

| Probe | Previous candidate | Current candidate |
|---|---:|---:|
| Plan one wire between isolated nodes beside 10,000 unrelated chain nodes, read-only API median | 63.625 ms | 35.653 ms |
| Same request, new retained-index API median | unavailable | 36.2 µs |
| Start/drop 128 activations over 512 data nodes, median | 2.427 ms | 0.372 ms |
| Whole validation of 10,000-node chain, best of five | 332.014 ms | 322.913 ms |

Connection command: `cargo test -p orange-model --release --locked --test perf_edit_baseline isolated_connection_planning_by_document_size -- --ignored --nocapture`. Three warmups and 15 samples per mode/size. For 100/1,000/10,000 unrelated nodes, read-only medians are 348.6/3037.2/35652.5 µs and indexed medians 63.0/36.0/36.2 µs. Current p95 at 10,000 is 40569.3/45.6 µs respectively. Old read-only review medians were 600.1/5412.7/63624.9 µs under the same construction/timing method. Plans are Ready and the final document/index equal their original/fresh state.

Startup command: `cargo test -p orange-bundled --release --locked --test control activation_preparation_by_owner_count -- --ignored --nocapture`. Start/frame preparation and drop are timed; drive/explicit stop/dispose are excluded. Each size warms up 500 ms and records 30 runs. Medians for 1/32/128 owners are 2.4/50.1/372.3 µs, p95 2.6/484.4/1776.4 µs. Previous medians were 2.8/104.4/2427.1 µs. In the 128×512 shape, sharing two immutable lookup arrays and removing unused execution cells eliminates approximately 6.5 MiB of repeated/unused vector payload before additional layout overhead. This is a representation calculation using measured Rust type sizes, not an allocator or process RSS measurement.

To avoid comparing historical validation numbers from a different run, the existing `edit_cost_by_document_size` probe was also built from an isolated tracked-source archive of `3203195`, with the same toolchain/lockfile and a separate target directory. Old/new whole-validation best-of-five times are 20.855/22.426 ms at 1,000 and 332.014/322.913 ms at 10,000 nodes. These small differences and other local run variation establish no general validation speedup. Whole-component inference, whole-document validation and V12/NFR-15 work remain open.

Repeated existing release probes also pass: Count-1000 Get/Add/Set uses 9,003 work items, with 10-second warmup and 30 batches of eight start/run/stop/dispose operations; median 215237.5 ns/run, p95 236337.5, max 251262.5. The unrelated-host-input probe remains exactly 3,003 work items at 0/64/512 unrelated inputs, with medians 111.2/103.9/105.95 µs. Its one-second warmup/30 runs include start/drive/drop and exclude explicit stop/dispose. These local timings do not qualify NFR, allocation, tail-latency or platform gates.

The independent source interpreters remain separate from compiler/runtime helpers. Four crate boundaries remain deliberate: model, compiler, runtime and bundled providers. No full core, SDK/package/service, UI, release or public API freeze gate is claimed.


## Structured Fork concurrency qualification — 2026-10-25

Base: squash-merged PR18, 104bbadac0ecc973036dcd2a2959f88bde026349. Behavior source after review: 05866c1 on core/structured-concurrency; subsequent delivery documentation/example does not change runtime behavior. Environment: rustc 1.99.1 (58cf8f9c3), Cargo 1.99.1 (5f94df478), x86_64-pc-windows-msvc, default workspace features; Node 24.18.0. MSRV remains 1.97.1. Cargo.lock SHA256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152 is unchanged; no dependency, feature, crate or persisted graph schema changes.

### Executed local verification

| Command / check | Result |
|---|---|
| cargo test --workspace --locked --offline | PASS: 728 tests, 12 manual probes ignored |
| cargo test --workspace --release --locked --offline | PASS: 728 tests, 12 manual probes ignored |
| cargo fmt --all --check | PASS |
| cargo clippy --workspace --all-targets --locked --offline -- -D warnings | PASS |
| cargo doc --workspace --no-deps --locked --offline, RUSTDOCFLAGS=-Dwarnings | PASS; prior environment restored |
| node tools/check-cargo-metadata.mjs | PASS: core dependency boundary |
| node tools/check-cargo-metadata.mjs --self-test | PASS: 9 policy fixtures |
| node tools/check-cargo-metadata.mjs --msrv | PASS: declared 1.97.1 metadata; not a local MSRV build |
| node --test tools/release.test.mjs | PASS: 7 tests |
| cargo run -p orange-bundled --example headless_control --locked --offline | PASS: total=3; virtual_time_ns=10 |
| cargo run -p orange-bundled --example headless_fork --locked --offline | PASS: join notification / final owner retirement at all 10/10 ns, any 5/10 ns, race 5/5 ns, detach 0/10 ns |
| git diff --check | PASS |

Windows incremental-cache hard-link warnings fell back to copying. Rust 1.97.1, Linux/macOS and dependency-security checks require final-head remote CI; local metadata/lint success does not substitute. The PR records that head-specific result. No merge or full core/API/NFR gate is included.

The control suite has 54 ordinary tests and four ignored timing probes. Sixteen new ordinary tests cover checked Fork order/admission, all four joins, nested region lifetime, shared activation locals, fail-fast, loop cancellation, equal-time wait/stop races, fuel preservation, owner retirement and per-operation cleanup deadlines. A 10,000-iteration Fork test stays within three live strand/continuation frames and four regions. The new independent source interpreter uses ordinary Rust futures with a small bounded executor, scans saved edges and never reads production IR, cursors or ownership/join helpers. Its 96 fixed-seed cases (20261025) run at budgets 1/7/1000, comparing final state, timestamped observable outcomes and handled-fault provenance for nested Fork/Sequence/Count/Wait, seeded data and reversed storage. The earlier independent control/data oracles remain intact. Generated competing deadlines are deliberately separated; equal-time and stop ordering have focused known answers. Shared model validation/literal codecs and externally anchored entropy primitives are explicit independence limitations.

Review added regressions proving that completed cleanup cannot replenish the surviving activation's ordinary data fuel, and that a later race does not inherit an expired prior cleanup deadline. Cleanup has a non-refilling activation work reserve; each cancelled operation owns a deadline capped by the run drain deadline. Epochs/entropy remain activation-owned. One pending data pull serializes sibling consumption while other activations can interleave; this avoids copying mutable evaluator state and preserves frozen prologues.

### Measured costs and optimizations

Commands explicitly invoked with --ignored --nocapture: cargo test -p orange-bundled --test control --release --locked --offline headless_control_timing_baseline, and the same command selecting fork::fork_owner_scaling. Fixture construction/compilation are excluded. These are local probe distributions, not allocator/RSS or wall-time acceptance gates.

The existing Count-1000 Get/Add/Set probe includes start, execution, stop and disposal, with trace off, 10-second warmup and 30 batches of eight. Both base and current execute 9,003 work items. A tracked-source archive of 104bbad built into a separate target directory with the same toolchain/lock yields median 214212.5 ns/run, p95 225550.0, max 228700.0. The initial concurrency implementation measured median 431275.0 ns/run. Replacing per-item map removal/reinsertion with indexed boxed shared activation frames reduced the cost; cancelling a race subtree in one pass also avoids per-sibling rescans. A tested alternative boxing each region continuation did not help and was not retained. Final reviewed source measures median 292562.5 ns/run, p95 311775.0, max 313025.0. The final median is about 37% above the merged base, reflecting additional ownership bookkeeping. No general speedup or NFR success is claimed; further optimization must preserve the new equivalence/ownership evidence.

Fork scaling uses 500 ms warmup per shape, 30 batches of eight full lifecycles, trace off, and virtual waits at 1 ns for the first branch / 2 ns for others:

| Join / width | Median ns/run | p95 ns/run | Work items | Retained owners at suspension: activations / frames / regions / timers |
|---|---:|---:|---:|---|
| all / 1 | 937.5 | 950.0 | 7 | 1 / 2 / 3 / 1 |
| all / 16 | 5112.5 | 5987.5 | 67 | 1 / 17 / 18 / 16 |
| all / 128 | 45150.0 | 56037.5 | 515 | 1 / 129 / 130 / 128 |
| race / 1 | 1537.5 | 1900.0 | 7 | 1 / 2 / 3 / 1 |
| race / 16 | 6025.0 | 6987.5 | 37 | 1 / 17 / 18 / 16 |
| race / 128 | 34025.0 | 42512.5 | 261 | 1 / 129 / 130 / 128 |

These measurements confirm one retained evaluator/activation, rather than one per branch, and complete owner retirement; they do not measure process memory or guarantee tail latency. Fan-out, region traversal and trusted native callbacks are bounded by admission/ownership limits rather than wall-clock preemption. Final PR CI and self-review remain required. Functions/other controls, events/reactivity, general resources/services, shared state/checkpoints, domains/API evolution and incremental/NFR work remain open.

## Whole-codebase review fixes (2026-10-25)

Implementation `d6041a1` fixes all seven reproduced findings: duplicate/inconsistent variadic counters, ignored explicit codec locks, transaction-iterator unwind, paste generator collisions, cleanup-scope entry ownership loss, layout-only executable invalidation, and recursive literal type-argument cloning. Contracts and FR/ADR trace are updated in 01/03/08/11/12. Unknown saved state remains round-trippable; invalid semantic state cannot publish an executable plan. Explicit codec locks, including unused ones, must resolve; absent format-v1 locks retain the existing structural codec default. No new dependency, crate, feature or saved schema field is introduced. `Cargo.lock` SHA-256 remains `e20eacb56cec10f874aa8c3869d5ab93b34831229a74b91332433db5f5267152`.

Environment: Windows x86_64-pc-windows-msvc, selected rustup Rust/Cargo 1.99.1, default features, Node 24.18.0, unchanged lockfile. Declared MSRV is 1.97.1; local results are not an MSRV/cross-platform build claim. Commands executed on the corrected source:

| Command/check | Result |
|---|---|
| `cargo test --workspace --locked --offline` | PASS: 738 passed, 13 ignored, zero failures, including doctests |
| `cargo test --workspace --release --locked --offline` | PASS: 738 passed, 13 ignored, zero failures, including doctests |
| `cargo test -p orange-bundled --test core_integrity --locked --offline` | PASS: eight behavior regressions; one manual timing probe ignored |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS; only Cargo's existing OneDrive hard-link fallback notices |
| `cargo fmt --all --check`; `git diff --check` | PASS |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline` (PowerShell environment equivalent) | PASS |
| `node tools/check-cargo-metadata.mjs --self-test`; `node tools/check-cargo-metadata.mjs` | PASS: nine policy fixtures and core dependency boundary |
| `node --test tools/release.test.mjs`; `node tools/release.mjs check` | PASS: seven tests; packaging remains intentionally disabled |
| `cargo run -p orange-bundled --example headless_control --locked --offline`; corresponding `headless_fork` example | PASS: total 3 at 10 ns; all 10/10, any 5/10, race 5/5, detach 0/10 ns |

Ten added ordinary regressions cover malformed Sequence/Fork identities and atomic insertion rejection, supported/unsupported codec locks and exact saved-payload preservation, ordinary/scratch iterator unwind, generated function-body/target collisions, cleanup entry fuel overflow with zero retained activation/strand/region owners, and semantic/layout freshness through undo/redo and pure evaluation. Existing scalar/aggregate codec properties and independent source/runtime oracles also pass. The stopped-run pump rejection is retained. The cleanup overflow path records one truncated cleanup rather than losing its checked-out activation.

Explicit descriptive timing: `cargo test -p orange-bundled --test core_integrity --release --locked --offline measure_nested_none_literal_decoding -- --ignored --nocapture`. The probe decodes and drops 10,000 outer None values in a list of nested option types, with three warmups and five timed samples per type depth. Before the fix, recorded medians at depths 1/32/63 were 1,066/25,631/50,646 µs. Borrowing type arguments instead of cloning them yields 151/157/161 µs (sample maxima 152/157/163 µs). Type parsing remains in each decode; no timing assertion, allocator count, cross-OS comparison, general runtime speedup or NFR pass is claimed.

Self-review checked every semantic-state consumer in compiler/evaluation entry, preserved whole-content binding for undo/index/edit proposals, checked type-lock validation executes once in combined analysis, and traced cleanup entry failures through activation restoration and normal region retirement. Paste occupancy uses binary UUID keys by typed namespace rather than allocating one UUID string per existing identity. Temporary review reproductions are promoted to durable behavior tests. Final-head remote CI is recorded separately after push; no merge is authorized.


## Resilient edit analysis and bounded local reuse — 2026-10-25

PR17's original V12a work was integrated with merged dev `bda93989233e799398a25eefa837c4c8afd9dae4` (PR19). This section supersedes the earlier V12a content-rebind/current-work claims; their test results remain historical evidence for their stated base.

Implemented V12a reliability plus bounded V12b configuration/declaration-type reuse. Committed changes carry journal-owned read-only before/after content and semantic identities, revision lineage, committed/trial status and explicit global invalidation. Session reuse checks every update path, including current cancellation and unchanged effects. Layout retains semantic compiler evidence. A retained optional index supports journaled inference trials; borrowed diagnostics, signature and changed-subject reads avoid per-read whole-report copies. A shared full-analysis pipeline consumes the same local validators as uncached analysis; inputs/resolution/connections and all global passes remain full. No dependency, feature, lockfile or graph/schema-format change occurred. Unfrozen ChangeSet field users must adopt getters; EditStats input cardinalities are distinct from actual LocalWork visits/computations/hits.

Verification environment: Windows `x86_64-pc-windows-msvc`; rustc `1.99.1 (58cf8f9c3 2026-10-13)`; Cargo `1.99.1 (5f94df478 2026-08-27)`; Node `24.18.0`; default features and locked/offline Cargo commands. Declared MSRV remains 1.97.1; local metadata checks are not an MSRV build. Cargo.lock SHA256 is unchanged: `E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152`.

| Executed command/check | Result |
|---|---|
| `cargo test --workspace --locked --offline` | PASS: 770 passed, 14 manual probes ignored, zero failures; 84 result groups including doctests |
| `cargo test --workspace --release --locked --offline` | PASS: same 770/14/zero failures |
| Original session properties | PASS: two bounded 64-case edit/rollback/undo/redo properties compare complete analysis with the full oracle; original seven V12a tests retained |
| New behavior coverage | PASS: immutable edit/undo/trial provenance, foreign/divergent/reloaded/skipped/replayed receipts, all update/cancellation/quota paths, global lock/signature changes, declarations inside functions, cache invalidation/undo/function replacement, borrowed subject views, Send+Sync and payload-safe Debug |
| `cargo test -p orange-bundled --test analysis_session_embedding --locked --offline` | PASS: two external API tests; already-running plan produces old result 5 after an edit, stale new-run/background compiler inputs are rejected, and new plan produces 23; partial failed editor trials preserve source and analysis/index identities |
| Cache invalidation mutation | Removing local invalidation makes the public config-edit/full-oracle test FAIL (exit101); restored source passes. This is a test-sensitivity result, not production code or a bypass |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `cargo doc --workspace --no-deps --locked --offline`, `RUSTDOCFLAGS=-Dwarnings` | PASS |
| `node tools/check-cargo-metadata.mjs --self-test` / normal metadata guard | PASS: nine policy fixtures; core remains UI/engine independent |
| `node --test tools/release.test.mjs` / `node tools/release.mjs check` | PASS: seven tests; release configuration valid and intentionally disabled |
| Both `headless_control` and `headless_fork` examples, `--locked --offline` | PASS: total3 at10ns; all10/10, any5/10, race5/5, detach0/10ns |
| `git diff --check` | PASS |

The complete suite is 32 tests above merged dev: the seven original V12a tests plus 25 additional tests/doctests. The extra ignored probe is run explicitly below. OneDrive incremental-cache hard-link warnings used Cargo's copy fallback; no Rust/Clippy warning was suppressed.

Independent bounded review found and corrected two additional integration issues: domain admission must precede index/local preparation, and a layout fast path must defer an invalid retained index rebuild rather than hide a scan. Regressions cover both. Four previously reproduced stale-session issues are now covered by passing public-consumer tests. Source/effects are not authenticated by analysis; host-owned effects and late worker-publication request identities remain host responsibilities. Native callbacks and individual analysis passes are not wall-clock preemptible.

### Descriptive release probe

Executed `cargo test -p orange-model --test analysis_session_units --release --locked --offline measure_config_unit_reuse -- --ignored --nocapture`. Two valid registered/provider-bound isolated-node fixtures, one boolean config edit per sample, five warmups and 40 measured samples. Full analysis, session update and owned materialization have separate timers; complete oracle equality checks are outside timed sections. No wall-clock assertions.

| Nodes | Full analysis p50 / p95 | Session update p50 / p95 | Owned materialize p50 / p95 |
|---|---|---|---|
| 128 | 161.4 / 185.5 microseconds | 125.1 / 151.2 microseconds | 2.8 / 2.9 microseconds |
| 1024 | 1406.8 / 1496.4 microseconds | 1162.0 / 1213.6 microseconds | 22.5 / 25.8 microseconds |

Each edit computes one configuration unit and reuses127/1023, but still visits128/1024 node IDs and runs full orchestration/global passes once. These are descriptive local fixture distributions, not W4/W9 or NFR-15 evidence, a universal speedup or an allocator/retained-memory measurement. Full admission, global validation, result/change publication scans, broader invalidation indexes, per-pass work instrumentation and V12c–f remain open. Compiler calls may borrow `session.analysis()` directly; owned materialization is needed only when a consumer needs a retained snapshot.

Final remote Linux/macOS/Windows/MSRV, workflow/security and aggregate CI must pass on the actual updated PR head; local checks do not substitute for them. No merge is authorized by this evidence.

## PR17 review corrections — 2026-10-25

The follow-up review reproduced two missed boundaries and four reuse/simplification opportunities at `ff2a0a6`. They are addressed in `71e1c76` and `9f83961`, with unchanged public signatures, dependencies/features, graph format and Cargo.lock.

- Fresh deterministic domain/quota errors are current reports, independently of coverage or execution readiness. Layout/undo/redo and unchanged-effect updates retain them without validation; changed inputs still rerun. Cancellation remains transient and requires recovery.
- Session analysis borrows retained effect maps instead of deep-cloning write vectors before cancellation/admission. Normal and cached analysis share one prerequisite boundary; preparation follows successful admission/domain checks.
- Configuration validation borrows JSON objects, shares root object/enum/member rules and caches schema-dialect verdicts by exact definition/registry generation. Unsupported config versions retain their previous handling.
- Indexed input checking reuses incoming edges, appends into one diagnostic buffer and sorts once per pass. The standalone fallback remains scoped and equivalent, including invalid-wire literal shadowing. Result-change views share a sorted-map merge that compares each surviving value once.

Windows x86_64 MSVC; unchanged Rust/Cargo1.99.1 toolchain, Node24.18.0, default features and locked/offline Cargo commands. `cargo test --workspace --locked --offline` and its `--release` equivalent both PASS: 775 tests, 14 manual probes ignored, zero failures, 85 result groups including doctests. Five added regressions cover deterministic-error freshness/history/changed inputs, large-effect rejection and cancellation recovery, object schema rules and registry replacement, single-comparison ordered-map changes, and indexed/standalone function-scope invalid-wire precedence. The last scope test was also rerun separately in debug/release after tightening its malformed-wire fixture to keep edge IDs unique.

`cargo fmt --all --check`, workspace/all-target Clippy with `-D warnings`, Rustdoc with `RUSTDOCFLAGS=-Dwarnings`, metadata architecture guard/nine policy fixtures, seven release-helper tests, disabled-release check, both headless examples and `git diff --check` PASS. OneDrive hard-link fallback warnings remain environmental. No local MSRV build or new NFR/allocator/RSS gate is claimed; required remote platform/MSRV/security checks apply to the final pushed head.

The prior public-API reproduction was rerun using the corrected model (`cargo run --manifest-path .artifacts/pr17-review-probe/Cargo.toml --release --locked --offline --target-dir target/pr17-review-probe`). The unknown-domain 10k-node case now reports analysis/session current=true and layout work=None/full_runs=0; a single sample was140us versus8216us previously, including retiring old changed-subject sets. The one-million-write cancelled update now borrows the retained17MB vector and a single sample was7us versus2230us previously. These are diagnostic samples; they establish the changed path and recorded work, not timing guarantees or measured allocations.

The explicit `measure_config_unit_reuse` release probe also PASSes: five warmups/40 samples at128/1024 nodes. Full analysis p50/p95=135.6/142.9us and1209.6/1270.9us; session p50/p95=118.5/124.7us and1046.8/1113.7us; owned materialization p50/p95=2.8/2.9us and22.7/24.3us. All node IDs/global passes are still visited/run on semantic edits; per edit one config unit recomputes and127/1023 are reused. Historical earlier measurements remain tied to their versions; this is not a controlled universal before/after speedup or W4/W9/NFR-15 qualification.

Contracts/ADR/trace are updated in01/08/12/16. Existing diagnostics/localization keys and provider/override interfaces are unchanged. Core completion and V12b remainder still require their later cohorts. The PR remains subject to user review; no agent merge is authorized.

## Pure graph-local call compilation — 2026-10-26 UTC

Base fetched `origin/dev` b7d32e4a7590a41591df8cf21b7ed405efe461d5, containing merged PR17; branch `core/function-execution`. This cohort implements constant Pure calls only; volatile/action/async frames, dynamic data/control calls, erased generic parameters, package bodies and Return cleanup remain pending. Contracts:03/11/12, ADR amendment:08, FR trace:01. No gate or performance target is declared complete.

Environment: Windows x86_64-pc-windows-msvc; selected Rust1.99.1 (`rustc 58cf8f9c3`, Cargo1.99.1 `5f94df478`); declared MSRV1.97.1, local MSRV not rerun. Node24.18.0. Workspace default features; checked-in Cargo.lock unchanged, SHA256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152. No dependency/toolchain/feature changes. The user foundation review remains untouched (SHA256 D8F25A2F3DF04A88578972340B3CA3B591DC42126083B107A60344B00125D553).

Executed local checks:

| Command | Result |
|---|---|
| `cargo test --workspace --locked --offline` |789 passed,14 ignored,0 failed,85 result groups including doctests|
| `cargo test --workspace --release --locked --offline` |789 passed,14 ignored,0 failed,85 result groups including doctests|
| `cargo fmt --all --check` |PASS|
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` |PASS|
| `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked --offline` |PASS|
| `node tools/check-cargo-metadata.mjs --self-test` and actual policy |9 fixtures and architecture guard PASS|
| `node --test tools/release.test.mjs` |7 passed|
| `node tools/release.mjs check` |valid, intentionally disabled|
| `cargo run -p orange-bundled --example headless_control --locked --offline` |total3 at10ns|
| `cargo run -p orange-bundled --example headless_fork --locked --offline` |all10/10,any5/10,race5/5,detach0/10ns|
| `git diff --check` |PASS|

Fourteen new tests: two model marker/inference regressions and twelve compiler tests, including a64-case independent source-demand property. Deterministic checks cover per-call generic arithmetic/cache isolation, three independent type instantiations, nested calls, typed result aliases, first-result fault choice, argument faults/Try, lazy skipped callbacks, function-edit freshness/frozen views,128-call depth, five quota limits, cancellation, expanding acyclic call graphs, exact concrete provider refusal and explicit erased-argument refusal despite saved inference. Existing dynamic/control suites still refuse calls in their compiler profiles.

A deliberate reversed-result-order mutation made the first-fault regression fail (actual NumericOverflow instead of original DivideByZero). Original compiler bytes were restored in a finally block; the focused regression passed again. Full debug/release runs include the final two-fault fixture; only documentation changed afterward. Raw logs live in ignored `.artifacts/functions-*.log`. The source oracle scans original body edges with independent invocation caches; its generated function vocabulary is nested identity plus integer division/Try. It shares the model codec/validation and arithmetic platform, not compiler call preparation, slots or callbacks. This is bounded behavioral equivalence, not proof of general function/runtime correctness.

Only environmental OneDrive incremental-cache hard-link fallback warnings appeared; no code warnings were suppressed. Potential call expansion is conservatively bounded, not optimized/fused or measured for allocations/RSS. Trusted callback time/allocations remain outside cooperative admission guarantees. Remote native platform, MSRV and security CI will qualify the published final head separately; no remote pass is claimed by this local record.

## PR18–20 review repairs — 2026-10-26 UTC

Review of combined head94764867b68a2c29448b8f490f9de1dcfb90888f found five issues still present from PR18/19/20. All five are repaired on the existing core/function-execution / PR20 branch: compiler92535e1 shares admitted call metadata, enforces root/callee port parity and matches ports in declaration order; runtime783c8c2 indexes owned cancellation/ready work and borrows immutable constants. Original commits remain; no merge is authorized or performed. Contracts/trace/ADR are updated in01/03/08/11/12. Dynamic/volatile/action/async calls, package bodies, erased arguments and Return cleanup remain pending; no core/NFR gate is declared complete.

Selected local environment: Windows x86_64-pc-windows-msvc; rustc1.99.1 (58cf8f9c3), Cargo1.99.1 (5f94df478), Node24.18.0, workspace default features. Declared MSRV1.97.1 was not rerun locally; final-head remote CI checks it separately. Product Cargo.lock and dependencies/features are unchanged, SHA256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152. The user's foundation review is untouched, SHA256 D8F25A2F3DF04A88578972340B3CA3B591DC42126083B107A60344B00125D553.

Executed qualification:

| Command/check | Result |
|---|---|
| `cargo test --workspace --locked --offline` |799 passed,14 ignored,0 failed;85 result groups including doctests|
| `cargo test --workspace --release --locked --offline` |799 passed,14 ignored,0 failed;85 result groups including doctests|
| `cargo fmt --all --check` |PASS|
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` |PASS|
| PowerShell `$env:RUSTDOCFLAGS='-D warnings'`; `cargo doc --workspace --no-deps --locked --offline` |PASS|
| metadata policy self-test / actual architecture guard |9 fixtures / actual check PASS|
| `node --test tools/release.test.mjs` |7 passed|
| `node tools/release.mjs check` |valid, intentionally disabled|
| existing headless_control and headless_fork examples, locked/offline |total3 at10ns; all10/10,any5/10,race5/5,detach0/10ns|
| `git diff --check` |PASS|

Ten new tests: four compiler regressions cover large borrowed defaults on rejected/successful calls, shared generic metadata plus admission before cloning, exact root/callee Computed-source parity before callbacks, and wide nonlexicographic interface keys/types. Runtime regressions cover32 waiting roots cancelled once, a race preserving an unrelated activation's timer, stopping a wide queued pending pull, and two activation frames borrowing the same constant values across epochs. Two private model tests compare20,000 mixed ready-queue operations with VecDeque and20,000 ownership-set operations with BTreeSet, including removal, spill, clear and identity/slot reuse. Existing source oracles, quota, lifecycle, cleanup/fuel and freshness regressions also pass. Final compact-layout assertion/lint annotation received a focused runtime rerun after the full runs; they do not change product behavior.

Two temporary mutations were detected: cloning instead of borrowing nongeneric defaults fails the default-identity regression; removing callee port admission fails the root/callee refusal regression. Original compiler bytes were restored in finally blocks and the focused four regressions passed afterward. These are executed mutation checks, not assertions about coverage percentage.

Ready-queue selection and region ownership remain private safe Rust implementations. One narrowly scoped `clippy::box_collection` expectation is intentional: only the multi-owner spill is boxed so the common IdSet representation stays at most16 bytes; the layout bound and spill behavior are tested. There is no crate-wide lint suppression. Environmental OneDrive hard-link fallback warnings remain.

### Release measurements and tradeoffs

The ignored local probe uses the real public APIs and assertions; no timing thresholds enter ordinary tests. Compiler measurements compare the original review log against repaired code. Runtime comparisons use the exact9476486 runtime sources in an isolated ignored crate, with the same current compiler/model and fixture source on both sides to isolate runtime changes. No product source was replaced for this baseline. The two variants were measured sequentially; final probes ran separately from workspace compilation/tests. Raw logs are retained in ignored `.artifacts/`.

| Fixture | Before | Repaired | Method |
|---|---:|---:|---|
|64 calls,256KiB default,4KiB compile budget rejection |6141us|246us|single descriptive samples; fresh analysis outside timing|
|wide identity call,256 /512 /1024 /2048 port pairs |1163 /2818 /6349 /17605us|478 /1168 /2079 /4238us|7-sample medians; zero provider callbacks|
|stop1 /16 /64 /256 independent waiting roots |0.5 /11.4 /155.3 /2784.3us|0.6 /4.8 /17.9 /152.8us|11-sample medians; startup/pumping outside timing,trace disabled|
|1000-node dynamic pull,Fork widths2 /128 /512 /2048 |317.2 /531.3 /1060.1 /4231.5us|423.4 /460.5 /572.8 /1057.3us|7-sample medians,pump(1),trace disabled;3008 /3134 /3518 /5054 pumps in both variants|
|Count1000 full lifecycle |median283.2us,p95 292.1us|median304.8us,p95 314.8us|10s warmup,30x8 runs,9003 operations/run,trace disabled|

Thus the wide-call and largest pending-pull fixtures improve about4x and256-root stop about18x in these samples. The ownership directory has a real small-workload cost: Count1000 is about8% slower and width2 pull about34% slower. Inline singleton ready/ownership paths reduced the initial repair's larger serial overhead; this is still a tradeoff, not a universal speedup or completed NFR acceptance. Structural cancellation no longer scans/reinserts unrelated work, but destroying retained dynamic frame state still costs work. No allocations/peak RSS were measured; pointer-identity tests establish metadata/constant sharing without claiming a process-memory bound. The dangerous multi-gigabyte default expansion discussed in review was not executed.

Reproduction commands used the ignored `.artifacts/pr18-20-review-probe` and `.artifacts/pr20-before-probe` Cargo manifests with `--release --locked --offline --target-dir target/pr18-20-review-probe`; Count used `cargo test ... headless_control_timing_baseline -- --ignored --nocapture`, other probes `cargo run`. Logs:pr18-20-review-probe.log,pr20-fixes-probe.log,pr20-before-runtime-timing.log,pr20-before-count-timing.log,pr20-fixes-count-timing.log,pr20-fixes-debug.log,pr20-fixes-release.log,pr20-fixes-clippy.log,pr20-fixes-rustdoc.log and the focused/mutation logs. Temporary review/harness/baseline/build sources are removed after the repair is qualified; durable behavior tests, contracts and this evidence remain.

Remote CI is qualified separately on the published final head; this local record does not claim remote success in advance. The PR description and ignored delivery checkpoint record the final CI run/head once verified.

### Runtime Pure-call qualification — 2026-10-26 (PR21)

Base: fetched `origin/dev` `4662b3ec71a623f4251127a24eab2fd8171648f9`, the verified PR20 squash merge. Branch: `core/pure-call-runtime`, following the user-selected `core/<focus>` convention. Local platform: Windows x86_64 MSVC, Rust `1.99.1 (58cf8f9c3 2026-10-13)`, Cargo `1.99.1 (5f94df478 2026-08-27)`, Node.js24.18.0, default workspace features. No dependency/features/lockfile change; root Cargo.lock SHA256 remains `E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152`.

The implemented scope is the bounded graph-local Pure vocabulary through DataPlan and supported control data demands, including epoch-dependent arguments, nested interface-visible generics, source scopes and ordinary runtime fuel/budget/cancellation. This is not general function-frame/Return execution. Contracts and remaining scope are in03/11/12 and the ADR amendment.

| Check | Result |
|---|---|
| `cargo test --workspace --locked --offline` | PASS:808 tests,0 failures,14 ignored,86 test-result groups including doctests |
| `cargo test --workspace --release --locked --offline` | PASS:same808/0/14 |
| `cargo test -p orange-bundled --test function_epochs --locked --offline` | PASS:9 new regressions; existing tests updated to assert equivalent constant-call outputs across compiler profiles |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline` | PASS; PowerShell environment variable assignment used locally |
| `node tools/check-cargo-metadata.mjs --self-test` and ordinary metadata check | PASS:9 policy fixtures; UI/engine boundary intact |
| `node --test tools/release.test.mjs`; `node tools/release.mjs check` | PASS:7 tests; release configuration valid and intentionally disabled |
| `cargo run -p orange-bundled --example headless_control --locked --offline`; same `headless_fork` | PASS:total3 at10ns; all/any/race/detach retain documented timing/retirement |
| `git diff --check` | PASS |
| Remote OS/MSRV/workflow/supply-chain checks | Separate exact-head CI qualification required on the PR; local results alone do not pass these gates |

The arithmetic regression uses an independent wrapping-add formula over32 varied epochs in two activations, two distinct call instances and duplicate exports; it checks actual callback counts, not just final values. Other tests cover nested u64/bool specialization, shared immutable constants, strict unused argument faults, two faulting results with nonlexical declaration order, original-body producer identity, lazy arms with both constant and dynamic inputs, latent constant warnings, Try capture, root seeded-input replay without extra draws, a statically skipped unavailable callee,128-deep iterative calls, pre-callback admission limits, cancellation during a suspended pull, operation/activation fuel limits, and control demands resumed through one-unit host pumps.

Two isolated mutations were detected by failing behavior tests: dropping the strict argument prefix loses the unused-argument fault; removing the scope from body output keys breaks the independent call-instance test. Sources were restored immediately and the focused suite rerun successfully. Final full debug/release and lint checks ran after restoration and the final test changes. OneDrive emitted hard-link-to-copy fallback warnings; there were no Rust/Clippy/Rustdoc diagnostic failures in qualification.

Self-review: entry aliases carry original producer dependencies, boundary gathering checks arguments before any body demand, result faults publish atomically across outputs, all expanded dynamic nodes use existing fuel and cancellation, and exact provider/snapshot bindings remain pinned. Constant islands reuse the folder rather than creating a second implementation. Potential expansion is conservatively admitted, and each call boundary has extra slots/work; no timing, allocation/RSS, scalability improvement or completed NFR/core gate is claimed. Public DataOperation adds Call, requiring downstream exhaustive matches to update; resolver traits and persisted formats remain unchanged. Volatile/action/async calls, stateful call locals, erased generic parameters, package bodies, full runtime error stacks and Return cleanup remain pending.

### Seeded function calls and PR21 merge review — 2026-10-26 (PR22)

PR21 review inspected its compiler/runtime diff for scoped completion/output keys, constant imports and accounting, strict argument gating, ordered atomic result faults, lazy/constant warning behavior, checked-plan construction, cancellation/fuel, and pinned provider/source ownership. No confirmed blocking gap was found within its documented Pure scope. The nine focused function regressions were rerun successfully. The exact reviewed head `3a23c82aa67dc31a843776cb4c745eab51110268` already had all checks passing in CI40351255674. Following explicit user authorization, PR21 was squash-merged as `0d0bcf405fe45f3d54ae54532d0ea6badc565f3c`, then fetched before starting `core/seeded-function-calls`. This review does not certify the explicitly pending core/function families.

PR22 implements the bounded seeded-volatile call profile described in03/11/12. Local verification used Windows x86_64 MSVC, Rust `1.99.1 (58cf8f9c3 2026-10-13)`, Cargo `1.99.1 (5f94df478 2026-08-27)`, Node.js24.18.0 and default workspace features. No dependency/feature/lockfile changes. Cargo.lock SHA256 remains `E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152`.

| Executed check | Result |
|---|---|
| `cargo test --workspace --locked --offline` | PASS:818 passed,0 failed,14 ignored,86 test-result groups |
| `cargo test --workspace --release --locked --offline` | PASS:same818/0/14 |
| `cargo test -p orange-bundled --test function_epochs --locked --offline` | PASS:18 tests, including9 new seeded-call regressions |
| `cargo test -p orange-runtime --lib --locked --offline` | PASS:15 tests, including1 new dense-counter/layout regression |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline` | PASS; environment set with PowerShell locally |
| `node tools/check-cargo-metadata.mjs --self-test`; ordinary metadata check | PASS:9 fixtures; UI/engine boundary intact |
| `node --test tools/release.test.mjs`; `node tools/release.mjs check` | PASS:7 tests; disabled release configuration valid |
| Both `cargo run -p orange-bundled --example headless_control/headless_fork --locked --offline` commands, run separately | PASS:existing total3/10ns and all/any/race/detach timing/retirement behavior |
| `git diff --check` | PASS |
| Remote platform/MSRV/workflow/security/supply-chain checks | Separate exact-head qualification required on PR22; local checks do not pass those gates |

Independent vector generation used Python's hashlib.sha256, UUID canonical network-order bytes, struct.pack little-endian u64 fields and the existing frame/stream domain strings. Seed42/activation7/invocations0and1 were computed for call paths[1],[2],[3,203] at body node103, and the existing root node4. Pinned expected u64 values are committed in the regression; no production compiler/runtime output generated them. Existing public stream and frame known vectors continue to pass. Other tests check two activations, repeated reads, independent counters under different skipped epochs, unused/unavailable body nodes, argument faults before entry, mixed generic Pure callees, original-body draw faults/Try, cancellation outside Try, one-unit control Set consumption, and128deep calls with pre-binding depth/operation rejection.

The private counter test compares128dense counters against public explicit-frame key semantics over64source nodes/twoframes/threeepochs, checks foreign-slot refusal and asserts no sparse frame arrays are allocated in the dense owner. This verifies counter cardinality/semantics, not RSS/allocator totals or timing. Preparation hashes only ancestry referenced by actual entropy instructions; Pure plans and unused potential calls do not hash call frames. The legacy standalone explicit-frame API remains unchanged.

Two isolated mutations were detected by behavior-test failures: replacing the bound frame with ROOT breaks the independent vectors, and aliasing every instruction to counter0 breaks sibling paths with different skipped epochs. Product sources were restored, focused tests rerun, then full debug/release and quality checks qualified the restored behavior. OneDrive hard-link fallback warnings remain environmental; Rust/Clippy/Rustdoc checks pass.

Self-review retained Pure-only compilation, rejected state-reading bodies before callbacks, checked exact registered seeded entropy metadata, and kept all call nodes on the existing ordered/fueled evaluator. No generic call binding, new scheduler, saved schema, replay-version change, provider override change or dependency was added. Other volatile/state/clock calls, mutable call locals, action/async functions, package bodies, erased parameters, full error stacks and Return cleanup remain pending. No completed E3/core/NFR or universal performance claim follows.

### Core-loop control-dispatch qualification — 2026-10-26 (PR23)

The user authorized repeating reviewed/fixed/CI-qualified squash-merge cycles toward headless core completion, with UI separate. PR22 was re-reviewed for exact seeded metadata, immutable source/call-path bindings, independent dense counters under skipped demand, shared invocation logic and unchanged standalone entropy APIs. No confirmed blocking gap was found in its documented subset;18 focused function tests passed, and final-head CI40388507637 was all-success. PR22 was squash-merged as `b6670b086604c9a3bac871d04ebe7a5b7dc268ba`. The next branch `core/control-dispatch` starts from that fetched dev.

Local verification: Windows x86_64 MSVC, Rust1.99.1 (`58cf8f9c3`,2026-10-13), Cargo1.99.1 (`5f94df478`,2026-08-27), Node24.18.0, default features. No dependencies/features/lockfile changes. Core string plan support and Switch/Match-result contracts are described in03/11/12; this does not complete E3/core/NFR gates.

| Executed check | Result |
|---|---|
| `cargo test --workspace --locked --offline` | PASS:826 passed,0 failed,14 ignored,86 test-result groups |
| `cargo test --workspace --release --locked --offline` | PASS:same826/0/14 |
| `cargo test -p orange-bundled --test control control_dispatch --locked --offline` | PASS:8 new regressions, including table-driven source comparisons at two pump budgets |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline` | PASS; environment assigned in PowerShell |
| Metadata policy/self-test, release-helper tests/check | PASS:9 policy fixtures/7 release tests; release remains intentionally disabled |
| Both headless_control and headless_fork examples via `cargo run -p orange-bundled --example <name> --locked --offline` | PASS:existing total3 at10ns and all/any/race/detach timing/retirement |
| `git diff --check` | PASS |
| Remote platform/MSRV/security/workflow/supply-chain CI | Requires separate exact-head PR23 qualification; local checks do not pass those gates |

Switch tests independently select expected arms by searching original JSON case values, not compiled keys or the runtime comparator. They cover all four integer widths including extrema, bool, Unicode/empty strings, sparse/nonlexical instance order, default and a matched unconnected arm that must not fall through, at pump budgets1and7. Invalid narrow/unsigned case values and large case-string quotas reject before an otherwise reachable provider is bound. Default operation Debug omits sensitive case text. String literal/payload charges flow through existing plan and variable publication.

Match regressions check typed success/error routing at two budgets, exactly one error projection, original DivideByZero producer identity, Ok(error) through nested Try/Match, present None, uncaptured upstream fault going to failed rather than err,128 replacements of the same retained string payload without charge accumulation, oversized matched publication, and the combined local/Match state limit rejecting a Set before commit. Both selected outcome projections and retained quota updates use checked immutable target metadata.

Three mutations were detected by behavior-test failures: substituting sorted array position for original case ID, retaining the outer Ok wrapper during Match unwrapping, and failing to subtract the old Match payload charge. Sources were restored and focused tests rerun, followed by full debug/release and quality qualification. OneDrive hard-link fallback warnings are environmental; no Rust/Clippy/Rustdoc errors remain.

Self-review: admission checks exact supported key families and case ranges before binding; case count/table bytes consume existing budgets; no generic comparator or provider fallback is introduced. Match projections suppress duplicate implicit error storage, preserve shared result boxing/provenance and commit typed retained values only after budget reservation. Existing data-pull ownership, snapshots, stop, cleanup and fuel remain in use. Public PlanType, ControlOperation, HostSource and ControlOutcome additions require exhaustive downstream Rust matches to update; no saved schema/provider trait change occurs. Enum/custom-key Switch, remaining state controls/iterators/zones, action/async function frames/Return, resources/events/services and other planned core acceptance remain open. Structural/admission tests are not timing, allocation/RSS or completed NFR evidence.

Post-publication PR23 review found a real authoring/performance gap: integer case JSON kinds were checked but not width/signedness bounds, and duplicate checks repeatedly scanned prior values. The correction reports E-SWITCH-CASE during semantic analysis and uses a borrowed HashSet with the pinned serde_json1.0.154 Value Hash/Eq implementation (verified from versioned local registry source). It retains original variadic iteration/diagnostic order and adds a1024-case acceptance/duplicate regression. A fourth mutation removing the i32 range check is detected. Final corrected full-suite qualification supersedes the initial825-test result; no timing/RSS or general incremental-validation claim is made.

Final corrected qualification: debug/release826passed/0failed/14ignored across86groups,8focused dispatch regressions, four detected/restored mutations, format/Clippy/Rustdoc warnings-denied checks all PASS. Final-head CI is recorded separately on PR23; the earlier published-head run cannot qualify this correction.

### Core-loop state-control qualification — 2026-10-26 (PR24)

PR23 final head6ffb73a1952a62329189b47d3ecf4bb5ba95da15 passed CI40425555209 before squash merge9acc3fa81f08c0bce322409e93c5686a049853bf. State work begins on core/state-controls from fetched dev at that merge. No dependency, feature, saved-schema or lockfile change. Selected local Rust1.99.1 (58cf8f9c3), Cargo1.99.1 (5f94df478), Windows x86_64 MSVC, default workspace features, Node24.18.0. Cargo.lock SHA256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152 unchanged. The unrelated foundation review remains untracked and byte-identical.

| Executed check | Result |
|---|---|
| cargo test --workspace --locked --offline | PASS:838 passed,0 failed,14 ignored,86 result groups |
| cargo test --workspace --release --locked --offline | PASS:same838/0/14 |
| cargo test -p orange-bundled --test control state_controls --locked --offline | PASS:12 focused regressions |
| cargo fmt --all --check | PASS |
| cargo clippy --workspace --all-targets --locked --offline -- -D warnings | PASS |
| cargo doc --workspace --no-deps --locked --offline, PowerShell RUSTDOCFLAGS=-D warnings | PASS |
| Metadata policy and --self-test | PASS:9 fixtures; core remains UI/engine independent |
| node --test tools/release.test.mjs; node tools/release.mjs check | PASS:7 tests; desktop release intentionally disabled |
| Both headless examples with cargo run -p orange-bundled --example NAME --locked --offline | PASS:total3 at10ns; existing Fork all/any/race/detach timing and retirement |
| git diff --check | PASS |
| Remote OS/MSRV/security/supply-chain CI | Requires final-head PR24 qualification; local checks do not substitute |

Focused source-command oracles exercise Gate open/close/toggle/in aliases at two pump budgets, DoOnce reset/suppression and FlipFlop alternation with state snapshots. Register regressions cover feedback across one-unit pumps, state-write inspection, reset without demanding a faulting next input, original uninitialized producer identity, independent runs and activation/run scoping, stop/disposal, transactional string quota refusal, pre-copy compile admission, pending mutation ordering against another activation, and typed Gate cancellation cleanup. Three deliberately introduced faults were detected: mapping Gate open to Enter, dropping pending Register segment ownership and executing Store during Reset. Sources were restored before full debug/release qualification. Ignored logs are .artifacts/state-controls-*.log.

Review: destination ports become typed commands before execution; no runtime string dispatch or package comparator/callback is added. Generic checked cell metadata avoids separate variable/state type validators. Initializer JSON copy admission precedes cloning; state replacement checks type and combined nominal budget before commit. Original region/loop/timer identities remain canonical. State belongs to Run/activation rather than immutable plans, and cleanup retains existing fuel/deadline constraints. Clippy's complex initialization tuple was replaced by a named private structure. Default diagnostics do not expose state payloads. Public routing fields now use ExecTarget and ControlVariable aliases ControlCell; downstream exhaustive Rust matches must update. No API-freeze, allocator/RSS, timing/NFR or complete core gate is claimed. OneDrive cache hard-link fallback warnings are environmental. Action/async frames/Return, state/clock function calls, iterators/zones, Update/shared state, broader value families, events/resources/service seams and other planned core gates remain open.

### Core-loop variable Update qualification — 2026-10-26 (PR25)

PR24 final head10f9d5f5161befb918fa1443975e9c6fae1f252b passed CI40428560864 before squash merge58d4aab45bb8a989479c9bb74a0e8db8af93bae1. Fetched dev starts core/variable-updates at that merge. Selected local Windows x86_64 MSVC, Rust1.99.1 (58cf8f9c3), Cargo1.99.1 (5f94df478), Node24.18.0, default workspace features. No dependency, feature, manifest or lock change; Cargo.lock SHA256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152. The untracked foundation review remains byte-identical.

| Executed check | Result |
|---|---|
| cargo test --workspace --locked --offline | PASS:849 passed,0 failed,14 ignored,86 result groups |
| cargo test --workspace --release --locked --offline | PASS:same849/0/14 |
| cargo test -p orange-bundled --test control variable_updates --locked --offline | PASS:11 focused regression groups (subsequently rerun in full suites) |
| cargo fmt --all --check | PASS |
| cargo clippy --workspace --all-targets --locked --offline -- -D warnings | PASS |
| cargo doc --workspace --no-deps --locked --offline with PowerShell RUSTDOCFLAGS=-D warnings | PASS |
| Metadata policy/self-test | PASS:9 fixtures |
| node --test tools/release.test.mjs; node tools/release.mjs check | PASS:7 helper tests; desktop release intentionally disabled |
| headless_control and headless_fork via cargo run -p orange-bundled --example NAME --locked --offline | PASS:total3 at10ns; existing all/any/race/detach timing/retirement |
| git diff --check | PASS |
| Remote OS/MSRV/security/supply-chain CI | Requires separate final-head PR25 qualification |

Table-driven independent expected values cover all nine operation tokens, the four integer widths, integer extrema and checked/wrapping/saturating modes, f32 rounding/f64 addition and finite/IEEE overflow, Unicode/empty append, ordered bool/string/duration, CAS match/mismatch/equal replacement and typed old/new/exchanged at multiple pump budgets. Other regressions exercise one entropy invocation for a shared operand node versus two for distinct nodes in one strict epoch, first-fault provenance without later entropy, pending mutation ordering, Fork-shared local cells with dynamic operands, nested Option distinctions, output charge replacement over64 iterations, transactional projection-quota rollback, malformed policy refusal before binding and uninitialized/stop/dispose paths.

Four mutations were detected by behavior failures: restarting the operand epoch, removing Update's pending segment hold, retaining old output charges and forcing CAS exchange on mismatch. Sources were restored before full final qualification. An initial mutation harness newline conversion failed compilation and was corrected; only the four subsequent behavior-test failures count as detected mutations. Logs remain ignored .artifacts/update-*.log.

Self-review: Core operation/type admission is closed and preserves fresh language proofs, never treating package Eq/Ord/numeric metadata as an executable callback. Ordered roots share one compiler-owned host-input union; fixed operand storage avoids per-action operand buffers and reuse of borrowed plan metadata avoids extra Arc cloning per poll. State/version and output quotas are reserved before publication; mismatch does not write, and failure/reentry releases old projections appropriately. Local variables and Match/Update projections use one retained-state budget. Shared scalar arithmetic preserves existing bundled policies/errors and eliminates a duplicate arithmetic implementation. Empty-string append reuses immutable storage while still recording a write intent; size admission precedes string allocation. This is not allocator/RSS or timing/NFR qualification. OneDrive incremental hard-link fallback warnings remain environmental; no code lint errors remain.

New Update operation/demand/projection enums and shared scalar types are unfrozen Rust surface changes; bundled KernelError retains its original import path as a ScalarError reexport. No saved-schema/provider ABI change occurs. Bytes/list/timestamp/JSON/result/package Update execution, workspace actors and change/Watch delivery remain open; action/async functions, iterators/zones, resources/services and other planned core gates also remain incomplete.

PR25 pre-merge review correction: UpdateDemand now reuses the existing immutable Arc host-input directory when operands have identical directories, including all single-input updates. Only differing inputs build and charge a new union. This removes per-update copied metadata and duplicate quota admission while retaining checked sorted indices. It is a structural allocation improvement, not a timing/NFR claim; final qualification must use the corrected head.

### Core-loop collection and iteration qualification — 2026-10-27 (PR26)

Based on PR25 squash merge8b9b98ac51afbb223b110ef871a5601bd14277f9 after its final head2ba7489b7b53eb09061e5e353168ef68214472a8 passed CI40431675230. Local Windows x86_64 MSVC, Rust1.99.1 (58cf8f9c3), Cargo1.99.1 (5f94df478), LLVM23.1.1 and Node24.18.0; default workspace features and checked-in lockfile. No dependency, feature or manifest change; Cargo.lock SHA256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152. The untracked foundation review remains byte-identical.

| Executed check | Result |
|---|---|
| cargo test --workspace --locked --offline | PASS:869 passed,0 failed,14 existing ignored,86 result groups; rerun after resuming checkpoint b81ea2b |
| cargo test --workspace --release --locked --offline | PASS:same869/0/14 |
| cargo test -p orange-bundled --test control collection_iteration --locked --offline | PASS:18 focused groups, including96 fixed-seed source-fold comparisons at two pump budgets and512 nested iteration contexts |
| Compiler checked-value units in full workspace suites | PASS:2 groups, including malformed/deep types, packed/generic charging and order-independent nested map equality |
| cargo fmt --all --check | PASS |
| cargo clippy --workspace --all-targets --locked --offline -- -D warnings | PASS |
| cargo doc --workspace --no-deps --locked --offline with RUSTDOCFLAGS=-D warnings | PASS |
| Metadata policy and self-test | PASS:9 fixtures |
| node --test tools/release.test.mjs; node tools/release.mjs check | PASS:7 helper tests; desktop release intentionally disabled |
| headless_control and headless_fork via cargo run -p orange-bundled --example NAME --locked --offline | PASS:total3 at10ns; existing all/any/race/detach timing and retirement |
| git diff --check | PASS |
| Remote platform/MSRV/security/supply-chain CI | Requires final-head PR26 qualification; local tests do not substitute |

Independent source folds check list/map insertion order at small and larger pump budgets. Focused regressions cover empty collections, nested indices, immutable collection capture across replacement/waits, Break/Continue quota reuse across50 invocations, cancellation cleanup in the original lexical frame, independent sibling scopes, race/stop/disposal, moving a suspended Run between host threads, quota refusal without advancing the body, local-state budget sharing, container Append/CAS and timestamp ordering, pre-binding literal admission and retryable host-input rejection. Count Break now preserves its index until canceled body cleanup retires.

Five deliberately introduced mutations produced behavior-test failures: reverse list order, omit the RAII charge release, omit child iteration context, make map equality order-sensitive, and restore premature Count index clearing. Sources were restored before full qualification. Ignored logs are .artifacts/collection-*.log.

Self-review: typed native checks reject malformed provider/host shapes, duplicate keys and excessive depth, including publicly constructed empty/None PlanTypes. Borrowed checks precede retained publication; packed scalar list charging is constant-time. Map membership uses native checked keys and borrowed hash tables; package comparator authority is not inferred from constraint metadata. Lexical frames retain only actual enclosing scopes through immutable Arc links, release parent chains iteratively, and share a private RAII byte ledger with local state and Match/Update outputs. Actor mutation is exclusive; relaxed atomic accounting provides ownership-safe release when Run moves between host threads without claiming parallel actor execution. Snapshot and per-item charges are conservative even when immutable storage is shared. Empty Append reuses storage; matching packed lists stay packed. No runtime saved-JSON parsing, UI dependency, generic iterator callback, unsafe code or new dependency is introduced.

PlanType/HostSource/ControlOperation additions, ValueCheckError and ListRepr::concat are unfrozen Rust surface changes; downstream exhaustive matches need updating. Saved JSON and provider ABI remain unchanged. Validation and equality are synchronous within configured depth/nominal byte bounds; these checks do not prove wall-time preemption, allocator/RSS or NFR targets. OneDrive incremental hard-link fallback warnings are environmental. Action/async functions and Return, zones/affine streams, broader kernels/types, events/Watch, workspace actors, resources/service seams, checkpoint recovery and applicable core/API-freeze gates remain incomplete. UI and concrete OS backend qualification remain separate.

### Core-loop function state-read qualification — 2026-10-27 (PR27)

PR26 head74ff0a0d8268407ece585066999dc9a0508de57d passed every job in CI40530585668 (Linux/macOS/Windows, MSRV1.97.1, lint, architecture/workflow security, supply chain and aggregate) before squash merge3de89b4. Fetched dev starts core/function-state-reads at that merge. Local Windows x86_64 MSVC, Rust/Cargo1.99.1 (58cf8f9c3/5f94df478), LLVM23.1.1, Node24.18.0, default workspace features. No dependencies/features/manifests changed; Cargo.lock SHA256 remains E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152. User foundation review remains untouched/untracked.

| Executed check | Result |
|---|---|
| cargo test --workspace --locked --offline | PASS:876 passed,0 failed,14 existing ignored,86 result groups |
| cargo test --workspace --release --locked --offline | PASS:same876/0/14 |
| cargo test -p orange-bundled --test function_epochs state_functions --locked --offline | PASS:7 new focused groups; full25-group function_epochs suite rerun in workspace tests |
| cargo fmt --all --check | PASS |
| cargo clippy --workspace --all-targets --locked --offline -- -D warnings | PASS |
| cargo doc --workspace --no-deps --locked --offline with RUSTDOCFLAGS=-D warnings | PASS |
| Metadata policy and self-test | PASS:9 fixtures |
| node --test tools/release.test.mjs; node tools/release.mjs check | PASS:7 helper tests; desktop release remains disabled |
| headless_control/headless_fork via cargo run -p orange-bundled --example NAME --locked --offline | PASS:existing calculation, clock and all/any/race/detach behavior |
| git diff --check | PASS |
| Remote OS/MSRV/security/supply-chain CI | Requires final-head PR27 qualification |

A known-answer recurrence exercises nested and repeated state calls across24 action epochs at three pump budgets and in independent runs. Other tests distinguish function-local initializers from caller-local writes, preserve separate host-input indices per expanded call and reject missing next-epoch inputs, preserve original uninitialized Get identity through Try for local/run scope, skip lazy state demands and faulting strict-argument bodies, and refuse workspace reads in potential callees before kernel binding. The former state-call rejection regression now checks PurePlan refusal plus successful explicit DataActivation host publication without resolver callbacks. Existing deep/seeded/generic/first-fault/cancellation call tests remain active.

Two restored mutations caused behavior failures: alias function locals to caller locals and mark a volatile call result as constant. Full suites ran after restoration. Ignored logs .artifacts/state-calls-*.log retain exact output. Initial test-helper mistakes (boolean pump status and missing entry activation policy) were corrected before qualification and are not mutation evidence.

Self-review: root/body single-output State admission shares one policy, with checked data provenance/types. Potential call preparation is reused by control/data lowering; called variable collection precedes binding, preserving workspace refusal and initializer admission. Body IDs are resolved from the declaration directory rather than indexed as root nodes. Run publishes frozen consumption-epoch inputs and read-only call-local initializers through dense sources; it does not call providers for state or search the document during execution. PurePlan remains restricted. The new HostSource::CallLocal variant is unfrozen public Rust surface; no saved schema/provider ABI changes occur. Call-local values share immutable initial data because volatile bodies cannot mutate them; mutable action/async call frames are not implemented by this representation. No allocator/RSS, timing/NFR, full-core or API-freeze claim follows. OneDrive incremental hard-link fallback warnings remain environmental. Clock reads, action/async frames/Return, zones, events/Watch, resources/services/workspace/checkpoints, broader kernels and original core gates remain open.

### Core-loop action/async function qualification — 2026-10-27

Based on fetched dev b44e7faa501cb75b47536567e2f29898fa15b201 after PR27 final headf013729771a404d5c2b9514be7a565bf6af305f0 passed CI40532749498. Local Windows x86_64 MSVC, Rust/Cargo1.99.1 (58cf8f9c3/5f94df478), LLVM23.1.1, Node24.18.0, default workspace features. No dependency, feature, manifest or lock changes; Cargo.lock SHA256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152. The untracked user foundation review remains byte-identical.

| Executed check on corrected source | Result |
|---|---|
| cargo test --workspace --locked --offline | PASS:895 passed,0 failed,14 existing ignored,86 result groups |
| cargo test --workspace --release --locked --offline | PASS:same895/0/14 |
| cargo test -p orange-bundled --test function_epochs action_functions --locked --offline | PASS:19 new focused groups; full function suite44 groups included in workspace tests |
| cargo fmt --all --check | PASS |
| cargo clippy --workspace --all-targets --locked --offline -- -D warnings | PASS |
| cargo doc --workspace --no-deps --locked --offline with RUSTDOCFLAGS=-D warnings | PASS |
| Metadata policy and self-test | PASS:9 fixtures |
| node --test tools/release.test.mjs; node tools/release.mjs check | PASS:7 helper tests; desktop release intentionally disabled |
| headless_control/headless_fork via cargo run -p orange-bundled --example NAME --locked --offline | PASS:total3 at10ns and existing all/any/race/detach timings/retirement |
| git diff --check | PASS |
| Remote Linux/macOS/Windows/MSRV/security/supply-chain CI | Requires exact final-head PR qualification before merge |

Known-answer regressions exercise generic call-site specialization, nested/repeated fresh locals, concurrent async instances, source-scoped state inspection, strict ordered argument/result collections, a shared entropy operand drawn once, original callee/Return faults and caller failed routing, virtual waits, stop after moving Run between threads, outputless fallthrough/no-entry behavior, silent-suppression refusal, sparse Sequence completion/early Return, and Fork join Return coverage. Fifty repeated normal and nested-detached calls prove actual quota reuse; a two-call string Update case distinguishes retired execution projections under a320-byte state limit. Frame-entry and result-capture refusals leave caller outputs unchanged. A second entry cannot interleave synchronous function work because a host uses one-unit pumps; source write intents remain stable across those observation boundaries. Root and called1024-step Sequences refuse a32-item compile limit before target allocation/binding.

Eight restored mutations caused behavior failures: alias call locals, drop function segment ownership, omit Return cancellation, omit nested-frame release, ignore DoOnce suppression in Return proof, clear write intents at each pump, omit retired projection charge release, and omit variadic step admission. Sources were restored before final suites. Early implementation/test-helper failures and intermediate builds are superseded by the corrected final logs .artifacts/action-calls-*-qualified.log. The recorded217-file Rust/toolchain/lock source digest36adec9a3c17b9a860f5f8063ba1dfeefff6d03a7f729f21e8e031a22c6e31c6 was rechecked unchanged after qualification; it is local evidence, not an artifact signing/provenance gate.

Self-review corrections include scoped control/DemandRoot identities, shared prepared call metadata, exact native reader admission inside effectful scopes, a shared RAII retained-value guard, actor-owned frame storage without locks, cancellation transfer of nested frame lifetimes, atomic local Update/result quotas, retired function projections, and source-intent deduplication. Rare pending calls/results/faults and retained frame sets are boxed to avoid growing common strand/region payloads. Return conformance reuses region ownership and precomputes Sequence continuation/Gate-write facts, avoiding repeated whole-body scans per path. Same-strand host polling no longer manufactures new write-intent segments. This is structural reuse/bounding evidence, not a measured general speedup.

Public unfrozen Rust additions include FunctionLayout, ControlNode::scope, Call/Return operations, FrameLocal/Argument/CallOutput sources and Run::control_state; source-only node_state refuses ambiguous repeated IDs. Downstream exhaustive matches need updating. Saved JSON, provider ABI/locks and dependencies remain unchanged. Cancellation cleanup Return terminates its canceled path without normal result evaluation/publication. Node state preserves declared run/activation lifetime per expanded call site; mutable variables initialize per invocation. Bounded DataActivation memo/slot caches retain their existing activation lifetime and separate quotas. No wall-time preemption, allocator/RSS, full-core, full call-stack error, NFR or API-freeze gate is claimed. OneDrive incremental hard-link fallback warnings are environmental. Clock/other volatile families, zones, enum/custom-key dispatch, package bodies, event/Watch queues, resources/inert services, workspace/checkpoints, broader kernels and original core acceptance gates remain open.


### Iteration-zone validation slice — 2026-10-27

Branch `core/iteration-zones`, based on fetched dev cab56e65e5157d90695cefe8c7dcf3be144a59c9 (PR28). Local Windows x86_64 MSVC, rustc1.99.1 (58cf8f9c3), cargo1.99.1 (5f94df478), selected rust-toolchain.toml and default features. No dependency/manifest/feature changes; Cargo.lock SHA256 E20EACB56CEC10F874AA8C3869D5AB93B34831229A74B91332433DB5F5267152. The protected untracked user review remains unchanged.

| Executed check | Result |
|---|---|
| cargo test --workspace --locked | PASS:903 passed,0 failed,14 existing ignored,86 result groups |
| cargo test -p orange-model --locked | PASS:598 passed,0 failed,8 existing ignored,67 result groups |
| cargo test -p orange-model --test validation_zones --test validation_data_cycles --locked, after mutation restoration | PASS:26 passed (8 new zone behavior groups),0 failed |
| cargo clippy -p orange-model --all-targets --locked -- -D warnings | PASS |
| cargo doc -p orange-model --no-deps --locked with RUSTDOCFLAGS=-D warnings | PASS |
| cargo fmt --all --check; git diff --check | PASS |

Two deliberate source mutations were detected by the focused regressions: omitting zone begin-prefix dependency edges, and assigning nested owners in the wrong order. Exact original source bytes were restored in finally blocks before the passing focused/workspace suites. Tests distinguish original begin parameters, body nodes, outer captures and nested end results; preserve function-local pairing and state feedback; cover all four same-id families and reject self-dependent/mutually dependent prefixes. One early test-fixture attempt used internal serialization with the saved-format decoder and was corrected to the canonical document codec before final qualification.

Self-review: pairing uses exact intrinsic identities and borrowed configuration strings; the cycle pass and region pass share it. Retained lexical ownership is one record per node, with immediate parents rather than duplicated nested body sets. Incomplete/type/cycle acceptance remains the responsibility of the complete analysis; the zone directory alone is not executable evidence. This slice does not yet admit zones into the compiler/runtime. Release, full-workspace lint/doc, source-oracle/runtime qualification and exact-head remote CI remain required for the finished cohort. No PR merge, core/NFR/API-freeze or execution-completion claim follows. Raw local logs remain ignored under .artifacts/iteration-zones-*. OneDrive hard-link fallback warnings remain environmental.


### Iteration-zone lowering slice — 2026-10-27

Continues `core/iteration-zones` after model validation commit020dc73. Same selected Windows MSVC/Rust1.99.1/Cargo1.99.1 toolchain, default features and unchanged Cargo.lock hash recorded above; no dependencies or manifest changes. Protected user review remains byte-identical.

| Executed check | Result |
|---|---|
| cargo test --workspace --locked | PASS:912 passed,0 failed,14 existing ignored,87 result groups |
| cargo test -p orange-bundled --test zone_epochs --test compile_data --test function_epochs --locked, after mutation restoration | PASS:61 passed,0 failed,3 groups, including9 new lowering regressions |
| cargo clippy --workspace --all-targets --locked -- -D warnings | PASS |
| cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings | PASS |
| cargo fmt --all --check; git diff --check | PASS |

Lowering tests inspect real plans built from fresh full analysis and exact MathPackage bindings. A million-iteration Repeat stays three instructions, binds its body once and executes no body fold. Other cases cover Map/Fold/Filter port order, nested lexical parents independent of UUID order, distinct generic call programs with constant u64/string arguments, outer entropy capture domains, interior-export rejection, exact carry/Filter element type matching, and metadata quota refusal before callbacks. Three restored source mutations were detected: removing zone metadata charges, allowing scalar constant folding of zone-containing calls, and removing the body memo domain. The metadata test's128-byte limit admits the original scalar slots/constants; its detected charge mutation proves it tests zone metadata admission rather than an earlier slot refusal.

Self-review found and fixed two prior model gaps: valid closed zone results were rejected as actor activation values, and marker pairs could disagree on carried or Filter element types. Combined export analysis reuses the checked zone directory; standalone export checking computes only lexical zone facts. Result cones include begin-prefix producers so actor-scoped prefix captures still fail. Interior exports are structure escapes, and only actual checked zone begin projections are exempted inside a closed result cone. Early lowering tests exposed these model failures; a generic test also incorrectly used a symbolic type pin and was corrected to infer the enclosing function's rigid parameter from its entry edge.

This remains an intermediate compile-time slice. At this commit DataActivation explicitly refused the new zone instructions; the runtime slice below supersedes that refusal. compile_pure explicitly refuses zones under its existing borrowed folded-result contract. Runtime slicing, per-iteration memo/entropy, capture snapshots, typed builders, cancellation/byte-limit reuse, source-reference equivalence, release qualification and exact-head CI remain pending for this cohort. No runtime, performance/NFR, complete-core or API-freeze claim follows. Raw logs are ignored .artifacts/iteration-zones-*; environmental OneDrive hard-link fallback warnings persist.

### Iteration-zone runtime slice — 2026-10-27

Continues `core/iteration-zones` after lowering commit 46413eb, on the same Windows MSVC/Rust 1.99.1/Cargo 1.99.1 toolchain with default features. Cargo.lock is unchanged from `dev` (SHA-256 e20eacb56cec10f874aa8c3869d5ab93b34831229a74b91332433db5f5267152). No dependencies or manifests changed. The protected user review remains untouched.

| Executed check | Result |
|---|---|
| cargo test --workspace --locked | PASS: 927 passed, 0 failed, 14 existing ignored, 88 result groups |
| cargo test -p orange-bundled --test zone_epochs --locked | PASS: 24 passed, 0 failed |
| cargo clippy --workspace --all-targets --locked -- -D warnings | PASS |
| cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings | PASS |
| cargo fmt --all --check; git diff --check | PASS |

DataActivation now executes the four zone families as resumable work items. A private ZoneMachine keeps the count, source handle, carry and typed list builder. Every iteration costs one operation and one activation fuel unit. Body nodes memoize against a per-zone generation that is never reset, and outer captures keep the consumption epoch. Body entropy draws a fresh invocation per iteration under the original node/call stream identity. A fault or error in the prefix or body retires the machine with the original begin identity. Cancellation, quota failure and a new epoch discard partial machines. Repeat with a negative count reuses the documented `core/count-out-of-range` fault of Count loops.

Review fixes in this slice:
- Fold/Map/Filter no longer charge the source list a second time; its memoized outer slot already retains it.
- `advance_zone` leaves the machine in its cell, so `zone_bytes` stays exact when an advance fails.
- One reset helper replaces five copies.
- One helper now maps value-check errors, and `KernelFault::NEGATIVE_COUNT` is shared.
- The entropy stamp no longer has a separate branch for nodes inside zones.
- Prefix arity, builder and zone-index charges have named constants.

A new regression folds a 1,000-item list (about 16,000 charged bytes) under a 24,000-byte limit; restoring the double charge made it fail, and the restored source passes. The existing Map quota test exceeded its limit only through that double charge, so its limit is now 3,000 bytes and the test still rejects mid-build, then succeeds with a smaller list in the next epoch.

Still open for this cohort: source-reference equivalence, release qualification, and exact-head remote CI. Last-iteration body values stay charged until they are overwritten. No performance/NFR, complete-core or API-freeze claim follows. OneDrive hard-link fallback warnings remain environmental.

Follow-up review (same day, same toolchain and lockfile):
- The zone work steps (prefix, next, body), zone cells and machine now live in the private `data::zone` child module. `data.rs` keeps the generic work loop and shrinks from 1,275 to 1,052 lines.
- Zone start no longer computes the source list's value charge, an O(n) result that was being discarded. The machine type-checks and charges only the initial carry against the end type.
- Discarded Filter items are no longer type-checked a second time; their producer already checked them when it published them.
- One `pulled_fault` helper replaces the operand fault check that was copied into Gather and both zone walks.
- A probe test for a nested zone whose prefix does not depend on the outer iteration confirmed that the model rejects the shape as `E-ZONE-STRUCTURE`. Deriving `parent` from the begin's owner is therefore sound, and the probe was removed.

Results: fmt check, clippy `-D warnings`, rustdoc `-D warnings` and `git diff --check` all pass. `cargo test --workspace --locked` passes with 927 passed, 0 failed, 14 existing ignored, in 88 result groups.

### Data shape slice — 2026-10-27

`core/data-shape` starts from `dev` after the zones merge (82952bc), on the same Windows MSVC/Rust 1.99.1/Cargo 1.99.1 toolchain with default features. Cargo.lock and the manifests are unchanged.

| Executed check | Result |
|---|---|
| cargo test --workspace --locked | PASS: 928 passed, 0 failed, 14 existing ignored, 88 result groups |
| cargo clippy --workspace --all-targets --locked -- -D warnings | PASS |
| cargo doc --workspace --no-deps --locked with RUSTDOCFLAGS=-D warnings | PASS |
| cargo fmt --all --check; git diff --check | PASS |

Changes:
- **Value shape:** the compiler's string-prefix type parser and its separate value checks are gone. `ValueShape` resolves from parsed `TypeTerm`s and owns:
  - the value check and its nominal charges (named constants);
  - checked equality;
  - the error-code rule.
- **List packing:** one model `ListBuilder` serves the literal codec and runtime zones. The codec's panicking `unreachable!` packing path is gone.
- **Charges:** compiler literal-size estimates and fault charges use named constants.
- **Intrinsic IDs:** about 165 intrinsic ID literals outside tests and the language table now use the `catalog::intrinsic_id` constants, which also serve as the language table's IDs. Tests keep wire literals so they still pin the format.
- **Non-exhaustive enums:** growing value and error enums are `#[non_exhaustive]`, and downstream matches gained wildcard arms.

Tests: the moved shape tests keep their assertions. They add parsed-term resolution cases (wrong arity, float map key, parameter, foreign namespace, major version) and equal charging of bare and result errors. A codec↔shape test checks that decoded literals satisfy their shape and that rebuilt lists keep the same packing.

Deliberately not changed:
- **`PortDescriptor`:** stays flat. Registry publication already rejects incoherent kind/type/source combinations, and `Option<bool>` flags keep byte-stable round-trips.
- **Literal codec dispatch:** stays a JSON-format layer. Its precise errors and secret references have no executable shape.
- **Converter error codes:** keep their own laxer rule.

Package record, enum and handle shapes; the domain seam; port templates; and redirects remain open. No NFR, API-freeze or core-completion claim follows.

Simplification follow-up (same slice, toolchain and lockfile). Targets were picked by measurement: `clippy::too_many_lines` and `clippy::cognitive_complexity` reports and redundant-clone lints, not taste. All changes are behavior-preserving:
- **Control lowering:** `compile_control` was one 908-line function with cognitive complexity 55. It is now a 221-line orchestrator over named phases: admission, variables, function layouts, state cells, exec edges, per-node lowering and host sources. Two copied dependency-union blocks became one `union_inputs`. Two identical activation-order sorts now share one key. Repeated control-index lookups use the known owner.
- **Constant charges:** these now come from the constant's `ValueShape`. The untyped `value_bytes` duplicate rule is gone, and one `FAULT_CHARGE` replaces two literal `64`s. Charges are byte-identical for every checked value.
- **Function summaries:** one fallible per-node step (`Result` with `?`) replaces about 15 copies of "emit, mark incomplete, continue", for 99 fewer lines. The diagnostics are unchanged, including one report per unknown variable in a leaf's effects.
- **Region checks:**
  - The two ways of testing whether an edge carries a region are now one predicate.
  - Per-node `BTreeSet`/`BTreeMap` clones used only for subset tests are gone.
  - Ancestor walks use one `lineage` iterator.
  - Diagnostics go through one reporter.
  - The file is 38 lines shorter.
- **Clones:** six redundant clones and three implicit `String` clones were removed.

Results: `cargo test --workspace --locked` passes with 928 passed, 0 failed, 14 existing ignored, in 88 result groups. Clippy `-D warnings` and fmt pass. Cognitive-complexity warnings fell from 10 to 8 across the libraries. The remaining long functions are compiler data/pure lowering, runtime run/pull steps and model transactions. They are left for later measured passes.

### Lowering simplification — 2026-10-27

`core/simplify-lowering` continues the measured pass on the same toolchain with an unchanged lockfile. All changes are behavior-preserving:
- **Data lowering:** the 465-line `Lowering::lower` loop is now a short task dispatcher over `enter`, `choose`, `finish`, `expand_call` and `return_call`.
  - Shared helpers replace four repeated patterns: the next instruction index, publishing a produced output, demanding a node's declared inputs, and the lazy selection-mode port filter (`SelectionMode::admits`).
  - The pure and entropy bindings share one ABI check.
- **Pure lowering:**
  - Three copies of the first-fault scan became `first_fault`.
  - The ABI comparison and resolve-error mapping were duplicated between pure and data lowering. They now exist once (`matches_abi`, `binding_error`).
- **Left unchanged:** the runtime control interpreter (`Run::execute`) is a flat dispatch over operations. Its repeated "consume, or fail with the fault" step differs per arm in what follows. Extracting it would need hidden control flow (a macro or labelled breaks), which is less clean than the current code.

Results: `cargo test --workspace --locked` passes with 928 passed, 0 failed, 14 existing ignored, in 88 result groups. Clippy `-D warnings`, rustdoc `-D warnings` and fmt pass. No public API, graph format, diagnostic or charge changes.

### Audit remediation — 2026-10-27

An external architecture audit (snapshot ddd675d) was validated finding by finding, then fixed in five parallel worktrees on separate files and integrated on `core/audit-remediation` from `dev` 873167e. Toolchain: Rust 1.99.1, x86_64-pc-windows-msvc, default features. Cargo.lock is unchanged (SHA-256 e20eacb56cec10f874aa8c3869d5ab93b34831229a74b91332433db5f5267152). The integrated branch passes `cargo test --workspace --locked` with 941 passed, 0 failed, 14 existing ignored, in 89 result groups, plus fmt, clippy `-D warnings` and rustdoc `-D warnings`.

**Confirmed and fixed** (allocation and layout figures are measured unless marked as counted from code):

| Finding | Before | After |
|---|---|---|
| Map check allocations, 4 / 1,000 entries | 2 / 10 | 0 / 1 |
| Map equality allocations, 4 / 1,000 entries | 5 / 21 | 0 / 3 |
| Bytes per dynamic data slot, all parallel arrays | 80 | 25 |
| Checks per `set_input` value | 2 | 1 |
| Node/edge quota | after full deserialization | during the streaming pre-pass, before building |
| Startup frame storage | no aggregate cap | `max_frame_storage_bytes`, default 64 MiB |
| Allocations per call + return (counted from code) | about 10 | 3 |
| Allocations per Count/While/For-each iteration (counted from code) | 1–2 | 0 (unless the frame is still captured) |
| Allocations per `cancel_region` (counted from code) | 2 Vecs + set | 0 |
| `Strand` / `Region` size | 360 / 464 B | 304 / 408 B |
| Dependency union, growing chain n=4,000 (release timing) | 332 ms | 137 ms |

Also fixed:
- The function-local access does one frame lookup instead of two.
- The bundled operation identity (ID, ports, numeric kind, scalar op) is declared once.
- The narrow binding request is additive.
- The admission estimate's data-side sizes come from the data module that owns the layout, not word-sized guesses in `run.rs`.

**Measurement method:** map allocations were counted by a throwaway crate with a counting `GlobalAlloc` outside the workspace, because the workspace forbids unsafe code. Sizes come from temporary `size_of` tests. The other allocation counts come from reading the code paths.

**Validated without code change:**
- No unsafe code: the workspace sets `unsafe_code = "forbid"`, and no unsafe blocks exist.
- No self-referential graph storage, and no JSON interpretation while pumping.
- Run variables use dense vectors.
- The scoped `RefCell` port cache is not an ownership cycle.
- `Value` keeps its 16-byte assertion.

**Confirmed but deferred, needing design or profiling first:**
- Availability analysis re-walks shared pure cones per consumer (`availability.rs`, consumer loop). Requirements depend on each consumer's region, so caching needs a careful key.
- Semantic edits still rerun the wider analysis pipeline (documented incremental limit).
- Type terms own their structural text; profile before interning further.
- Compare-and-set may re-validate already-checked values (`run/update.rs`); this is unproven.
- Compiler-resolved frame-local addresses.
- Resumable consumption snapshots and cancellation.
- Pooled generational ownership storage.
- A fuller `StrandPhase` covering yield/reenter/cleanup.

No NFR, allocator/RSS, MSRV rerun, exact-head CI or core-completion claim follows.

### Measured runtime slice — 2026-10-27

A second wave of three worktree agents continued the audit migration plan from `dev` 349b8e7, integrated on `core/measured-runtime`. Toolchain: Rust 1.99.1, x86_64-pc-windows-msvc. Cargo.lock gains only the local `orange-measure` entry, with no external version changes.

Integrated results:
- `cargo test --workspace --locked`: 944 passed, 0 failed, 15 ignored, in 90 result groups.
- fmt, clippy `-D warnings`, rustdoc `-D warnings` and `node tools/check-cargo-metadata.mjs` all pass.
- Not run locally, because the tools are not installed: the MSRV 1.97.1 build and cargo-deny. No external dependency was added.

**Measurement harness.** The development-only binary crate `orange-measure` runs with:

```
cargo run --release -p orange-measure --locked -- [--budget 64] [--warmup 3] [--runs 15] [FILTER]
```

It never runs under `cargo test`. Each workload checks its own result. Allocation counts cover start, pumps, stop and dispose.

This crate holds the repository's first `unsafe` code: one counting `unsafe impl GlobalAlloc` that forwards to `System`. Containment:
- The crate declares its own lints instead of inheriting the workspace's: `unsafe_code = "deny"`, `unsafe_op_in_unsafe_fn = "deny"` and clippy `undocumented_unsafe_blocks = "deny"`. The single allowance is on that impl, and every block has a `SAFETY:` comment.
- The four library crates still forbid unsafe code.
- Earlier "no unsafe code" statements in this file refer to the library crates.

**Integrated baseline.** Release build on an AMD Ryzen 9 9900X3D, Windows 11, budget 64, 3 warm-ups, 15 runs. Timings are single-machine and coarse below about 2 µs. p95 varied by up to about 40% between runs.

| Workload | Units | Allocs/run | Bytes/run | Pump p50 / p95 µs | Run p50 ms |
|---|---|---:|---:|---:|---:|
| Scalar chain (16 adds) | 1,000 iterations | 34 | 12,977 | 1.3 / 1.5 | 1.45 |
| Function call + return | 1,000 calls | 3,038 | 113,423 | 3.5 / 3.8 | 1.23 |
| Nested for-each 32×32 | 1,024 items | 130 | 17,046 | 4.5 / 4.7 | 0.37 |
| Fork race + cancel | 1,000 iterations | 3,032 | 747,944 | 10.6 / 11.1 | 1.18 |
| Map CAS, 16 entries | 1,000 updates | 30 | 6,857 | 18.8 / 21.4 | 1.82 |
| Map CAS, 1,000 entries | 1,000 updates | 3,032 | 55,387,753 | 499 / 583 | 47.9 |
| Host map input, 16 entries | 1,000 epochs | 13 | 585 | 0.6 / 0.9 | 0.60 |
| Host map input, 1,000 entries | 1,000 epochs | 1,013 | 18,448,585 | 15.4 / 15.8 | 15.8 |
| Map zone, 10,000 items | 100,000 items | 174 | 3,423,589 | 1.2 / 1.4 | 16.1 |

**Compare-and-set** re-validation was confirmed and removed. On the same machine, 1,000-entry map CAS went from 6,032 allocations, 127 MB and 106.9 ms per 1,000 updates to 3,032 allocations, 55 MB and 47.9 ms. 16-entry map CAS went from 3.27 ms to 1.82 ms.

**Availability cones.** The re-walk finding was confirmed and fixed. Times for the ignored `availability_cost_by_shared_pure_cone` test, best of 5, release:

| Cone × consumers | Before | After |
|---|---:|---:|
| 1,000 × 10 | 1.3 ms | 0.93 ms |
| 1,000 × 1,000 | 49–53 ms | 2.9–3.3 ms |
| 4,000 × 1,000 | 350–392 ms | 7.8–11.6 ms |

A temporary side-by-side check showed identical diagnostics on 11,824 suite bodies and 2,000 random graphs. It was not committed.

**Runtime handlers.** `execute` went from 525 lines to 75. Runtime long-function and complexity warnings went from 9 to 6.

**Next candidates, ranked by the baseline:**
1. Large-map validation on publish and CAS still allocates once per check above 16 entries. A host map of 1,000 entries costs about 18 KB per publish of the same shared value.
2. Calls and forks make about 3 allocations each, because invocation and region storage are not pooled.
3. The Map zone builder grows by doubling: about 342 KB allocated per epoch for an 80 KB result. Presizing it from the source length would help.
4. Pump budgets count fine-grained work items, so a 64-unit budget covers about one 16-add iteration.

Compiler-resolved frame-local addresses and per-body incremental analysis remain queued.

### Exact-length list results — 2026-10-27

Branch `core/zone-builder-capacity` from `dev` 75caaef (same toolchain; Cargo.lock unchanged).

- **Change:** `ListBuilder::reserve` makes an exact reservation for producers that know their result length. Map zones reserve the source length, so the result is built in one allocation and needs no shrink when finished. The literal codec reserves its JSON array's length.
- **Filter:** Filter keeps growing on demand, because its result length is unknown.
- **Memory bound:** the reservation is at most one more list the size of the already-retained source. Nominal retained-byte charging is unchanged and still applied per pushed element.
- **Measured** with the `orange-measure` "map zone (10000 items)" workload: release, same machine, 15 runs per workload. Results are byte-identical; all existing zone and codec tests pass unchanged.

| Metric per run | Before | After |
|---|---:|---:|
| Allocations | 174 | 44 |
| Bytes allocated | 3,423,589 | 802,469 |
| Peak live bytes | 212,245 | 161,213 |
| Run p50 | 16.1 ms | 16.3 ms (unchanged within noise) |

**Checks:** `cargo test --workspace --locked` passes with 944 passed, 0 failed, 15 ignored, in 90 result groups. fmt, clippy `-D warnings`, rustdoc `-D warnings` and the metadata policy pass.

**Frame-local addresses deferred.** The compiler-resolved frame-local addresses step was measured and not taken. It would remove only a binary search over one function's few local variables per access. The function-call workload is dominated by its 3 allocations per call. Encoding slot indices into public compiled plan types would add surface for no measurable gain, so it stays deferred until a workload shows local-access cost.

### Unique map keys by construction — 2026-10-27

Branch `core/map-check-alloc` from `dev` 4a13f48 (same toolchain; Cargo.lock unchanged).

- **Change:** `MapRepr`'s entries are private. `MapRepr::new` checks key uniqueness once, when a map is built, and returns `DuplicateKey` otherwise; `entries()` reads them. Shape checks no longer search for duplicates, and encoding drops its duplicate set. Decoding builds through `MapRepr::new`, so a literal's keys are checked once instead of twice.
- **Equality:** map equality returns at once for the same shared map, then walks both maps while their keys agree in order. Only the remaining, differently ordered tail is matched by key, pairwise up to 16 entries, otherwise through one presized index. Order-independent equality is unchanged.
- **Behavior:** a host can no longer build a map with a repeated key, so host-input admission cannot meet one. Codec errors are unchanged, except that a malformed later entry is now reported before a repeated key.
- **Measured** with `orange-measure` map workloads: release, same machine, 15 runs per workload. All existing map tests pass, with duplicate cases moved to `MapRepr::new`.

| Workload, per run | Allocations before → after | Bytes before → after | Run p50 before → after |
|---|---:|---:|---:|
| Map CAS, 1,000 entries, 1,000 updates | 3,032 → 30 | 55,387,753 → 6,857 | 47.4 → 10.8 ms |
| Map CAS, 16 entries, 1,000 updates | 30 → 30 | 6,857 → 6,857 | 1.80 → 0.53 ms |
| Host map input, 1,000 entries, 1,000 epochs | 1,013 → 13 | 18,448,585 → 585 | 15.8 → 3.8 ms |
| Host map input, 16 entries, 1,000 epochs | 13 → 13 | 585 → 585 | 0.59 → 0.15 ms |

**Checks:** `cargo test --workspace --locked` passes with 944 passed, 0 failed, 15 ignored, in 90 result groups. fmt, clippy `-D warnings`, rustdoc `-D warnings` and the metadata policy pass.

**Next candidates:** calls and forks still make about 3 allocations each, because invocation and region storage are not pooled, and pump budgets count fine-grained work items.

### Reused call frame storage — 2026-10-27

Branch `core/call-frame-alloc` from `dev` 7e284d1 (same toolchain; Cargo.lock unchanged).

- **Change:** a retired function frame returns its emptied argument and local buffers to the run, which keeps up to 16 for later calls. A call takes spare storage before falling back to a new allocation, so a steady call loop allocates nothing per call. Locals still start from their initial values on every call.
- **Return:** a function's result travels unboxed in the strand transfer and the function region. This reverses the earlier boxing of rare results: `Strand` grows from 304 to 336 bytes, which costs less than one allocation per call.
- **Measured** with `orange-measure`: release, same machine, 15 runs per workload. All existing function tests pass, including repeated calls that must see fresh locals.

| Workload, per run | Allocations before → after | Bytes before → after | Run p50 before → after |
|---|---:|---:|---:|
| Function call + return, 1,000 calls | 3,038 → 41 | 113,423 → 10,831 | 1.22 → 1.07 ms |
| Fork race + cancel region, 1,000 iterations | 3,032 → 3,032 | 747,944 → 813,392 | 1.18 → 1.17 ms |

Fork bytes rise only because each branch strand is 32 bytes larger.

**Checks:** `cargo test --workspace --locked` passes with 944 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings`, rustdoc `-D warnings` and the metadata policy pass.

**Next candidates:** a fork still makes 3 allocations: its branch list and the boxed ordered set of its two branch regions. Pump budgets still count fine-grained work items.

### Pump budget granularity measured — 2026-10-27

On `dev` b33a7d9 (same toolchain; Cargo.lock unchanged). No code change.

- **Measured** every `orange-measure` workload at budgets 16, 64, 1,024 and 65,536: release, same machine, 15 runs per workload.
- **Throughput:** total run time barely depends on the budget. The fixed cost per pump is about 0.13 µs, so coarser units would not speed up runs.

| Workload, per run | Budget 16 | Budget 64 | Budget 1,024 | Budget 65,536 |
|---|---:|---:|---:|---:|
| Scalar chain (16 adds), 1,000 iterations | 1.75 ms | 1.44 ms | 1.30 ms | 1.30 ms |
| Function call + return, 1,000 calls | 1.21 ms | 1.08 ms | 1.04 ms | 1.06 ms |
| Map CAS, 1,000 entries, 1,000 updates | 10.8 ms | 10.4 ms | 10.8 ms | 10.7 ms |

- **Slice latency:** one unit is not a steady amount of time. A whole-value operation counts as one unit regardless of size. At budget 64, a scalar-chain pump takes about 1.3 µs and a 1,000-entry map CAS pump about 109 µs. At the proposed 1,024-unit slice, that CAS pump takes 1.7 ms p50 and 2.1 ms p95, above the proposed 1 ms slice.
- **Decision:** budgets stay operation bounds, as [03](03-data-and-execution.md) already states, not wall-time guarantees. Size-weighted charges for value-proportional work, such as collection equality and CAS, are deferred until slice latency (NFR-02) is qualified. The candidate is closed.

**Next candidates:** a fork still makes 3 allocations, for its branch list and the boxed ordered set of its two branch regions.

### Option/Result composition — 2026-10-28

On `dev` fa9d983 (same toolchain; Cargo.lock unchanged). The change fixes audit finding A11, where Coalesce turned Ok(Some(None)) into Ok(None) for `option<result<option<option<i64>>>>`.

- **Executed:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` and `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked --offline` pass. `cargo test --workspace --locked --offline` reports 947 passed, 0 failed and 15 ignored.
- **New checks:** every option/result layering up to depth 6 over i64 is checked: each semantic value has one accepted compact encoding, and no two values share one. Known answers cover dynamic Coalesce over `option<result<option<option<i64>>>>` (None, Some(Ok(None)), Some(Ok(Some(None))), present payloads and Some(Err)), and constant Try over the new compositions against the tagged oracle.
- **Not covered:** this does not requalify any NFR. The 15 ignored tests were not run, and Linux/macOS/MSRV were not exercised locally.

### Checked document construction — 2026-10-28

Branch `core/checked-document-serde` on `dev` 458f9b0, after #38 (same toolchain; Cargo.lock unchanged). Closes audit finding A01.

- **Change:** `GraphDocument`'s Serde attributes move to a crate-private wire layout. The public `Deserialize` impl reads that layout, then runs the importer's admission: format tag if present, version, cross-body node/edge IDs and lock identities. `from_json` uses the same admission and still reports typed `CodecError`s in its documented order. `Serialize` output is unchanged.
- **Regressions:** Serde construction (`from_str` and `from_value`) refuses an unsupported version, a foreign format tag, a node ID repeated across bodies, and duplicate package and type locks. Valid documents read through Serde equal the imported document, keep unknown fields and save identical bytes. A test that built duplicate locks through raw Serde now asserts rejection; locks have no public mutator, so that state is no longer constructible from outside the crate.
- **Limitations:** duplicate JSON keys and node/edge quotas need the text and stay importer-only. Import error messages still echo rejected input (A02).

**Checks:** `cargo test --workspace --locked --offline` passes with 949 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Type locks on copy/paste — 2026-10-28

Branch `core/paste-type-locks` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A14 and the type-lock part of A05.

- **Commands:** `DocCommand::PutTypeLock` appends a lock, reuses a matching one and rejects another codec (`TxError::TypeLockConflict`). `DocCommand::RemoveTypeLock` removes an existing lock (`TxError::TypeLockNotFound` otherwise). Both are reversible patches and mark the change set globally dirty.
- **Paste:** `Fragment` gains `type_lock`. Copy keeps every source type lock; paste emits `PutTypeLock` for each before any node, in the same transaction.
- **Regressions:** a node whose source locks i64 to an unavailable codec still reports `E-TYPE-CODEC` after it is pasted into an empty document, and undo restores the empty lock list. A matching target lock is reused without a duplicate. A conflicting target lock rejects the paste and leaves the target unchanged. Transaction tests cover append order, reuse, conflict, missing removal, explicit codec change and undo.
- **Limitations:** the precise set of types a copy depends on is not computed, so unrelated source locks travel too. An unrelated unavailable or conflicting lock can therefore block a paste or make the target unready; this fails closed. Export and activation-policy commands from A05 remain open.

**Checks:** `cargo test --workspace --locked --offline` passes with 952 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Export and activation commands — 2026-10-28

Branch `core/export-policy-commands` on `dev` 3b58100, after #40 (same toolchain; Cargo.lock unchanged). Closes the remaining part of audit finding A05.

- **Change:** `DocCommand::UpsertExport`, `RemoveExport` and `SetActivation`. Exports are a reversible keyed slot like the lock lists and mark the change set globally dirty; activation edits go through the ordinary node patch. Import-only metadata is documented in 12 §3.3.
- **Regressions:** upsert appends, replaces in place and is undone exactly; removing a missing export is rejected without change; activation policies are set, cleared and undone, and a missing node is rejected. End to end, a bundled math graph gains its export through a command, compiles to the expected constant, reports a retargeted export through analysis, and undoes back to the compilable graph.

**Checks:** `cargo test --workspace --locked --offline` passes with 955 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Shared nesting budget — 2026-10-28

Branch `core/nesting-budget` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A13.

- **Change:** `document::nesting::MAX_NESTING` (64) bounds the JSON container nesting of a saved document, root included. A non-allocating serializer measures each transaction patch at its saved position, and the transaction rejects an over-budget patch before applying it. The importer's streaming pass and Serde construction apply the same budget.
- **Regressions:** for a layout value, a layout path, a root-body node config and a function-body node config, edits are accepted up to the budget, each accepted document saves and reloads equal, the deepest one nests exactly 64 levels, and the next level is rejected without changing the document. Loading a document one level over the budget fails on both construction paths.
- **Limitations:** byte and node/edge quotas remain import policy, not edit limits; `from_json_with_limits` reopens a larger edited document. Measuring a patch walks the patched entry once.

**Checks:** `cargo test --workspace --locked --offline` passes with 953 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Inference plan freshness — 2026-10-28

Branch `core/inference-plan-freshness` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A15.

- **Change:** `InferencePlan` records its preconditions and makes `commands` read-only (`commands()`). `inference::apply_plan` checks them and commits the plan as one transaction. Connection and inference plans share one `edit::PlanApplyError` and precondition check; `conversion::PlanApplyError` remains the same type.
- **Decision:** any content change makes a plan stale, a layout-only edit included. Plans are derived from exact content, so the host plans again instead of rebasing by semantic identity.
- **Regressions:** a proposal planned before a newer user config edit is rejected and the user's value stays. Plans are also rejected after a layout edit, on a divergent clone and on a reloaded copy, and with a replaced registry snapshot. A fresh plan commits and one undo restores the document.

### Instance-aware orphan edges — 2026-10-28

Branch `core/instance-orphan-edges` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A16.

- **Change:** one crate-private rule, `catalog::instance_port`, decides whether a port key names a port of a node under a signature, including variadic instances, bare variadic base ports and implicit ports. The validator's port lookup and `DocIndex::orphan_edges` both use it. `orphan_edges` now takes `&NodeInstance` instead of a `NodeId`; `DocIndex` stays structural and takes the resolved signature from the caller.
- **Regressions:** on a Fork with active `branch#0`, edges from `branch#9`, from bare `branch` and from an undeclared port are reported identically by `E-PORT-UNKNOWN` and by the projection, and the active instance's edge by neither. Removing the active instance and undoing it keeps both views in agreement.

### Redacted import errors — 2026-10-28

Branch `core/redacted-import-errors` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A02 (SEC-001).

- **Change:** `error::Rejected` holds text a check refused and formats only its character count. `IdError` variants, `CodecError::{UnsupportedFormat, DuplicateKey, DuplicateLock}` and the new `error::JsonError` (kind, line, column, `Rejected` parser detail) use it; `CodecError::Json` no longer wraps `serde_json::Error`. Both error enums gain `message_key()`.
- **Regressions:** a sentinel with control characters placed in the graph ID, format tag, a duplicated object key, a duplicated package lock, a definition ID, a type expression, an enum token and a port ID never appears in the error's `Display` or `Debug` output, neither does any control character, every error has an import message key, and JSON errors keep a non-zero line and column. Existing tests read rejected text through `reveal`.
- **Limitations:** deserializing `GraphDocument` with Serde directly returns the caller's `serde_json::Error`, whose own messages can quote input; hosts handling untrusted text use `from_json`.

### Map admission before reservation — 2026-10-28

Branch `core/zone-admission` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A03.

- **Change:** before reserving a Map result, the zone checks with overflow-checked arithmetic that one value charge per element plus the list's own charge fits the capacity left; otherwise it fails with `ResourceLimit`. The bound uses the minimum element charge, not the source element's size, so it holds for maps to wider outputs.
- **Measured** with `orange-measure` (release, same machine): a 10,000-item `u64` Map under a 161,024-byte retained limit peaked at 80,104 heap bytes before being refused; it now allocates nothing before refusing. The 10,000-item map-zone workload still succeeds with its preallocation. A unit test under the counting allocator fails above 16 KiB.
- **Behavior change:** the existing quota-failure test now sees the refusal before any element is computed, instead of after part of the result was built; the next smaller epoch still succeeds.

### Disposal releases run storage — 2026-10-28

Branch `core/dispose-releases-storage` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A04.

- **Change:** `Run::dispose` replaces its containers instead of clearing them, so their capacity is freed, and empties the spare frame pool and the cancellation traversal buffers. The terminal summary stays readable.
- **Measured** with `orange-measure` (release, same machine): after the function-call workload, stop and dispose, the run held 6,040 heap bytes that were released only on drop; it now holds 8. The function-call workload still makes 41 allocations per 1,000 calls, so the active frame pool is unchanged. `orange-measure` prints the disposed-run figure and a unit test under its counting allocator fails above 1 KiB.

### Provider panic boundary — 2026-10-28

Branch `core/run-panic-poison` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A17.

- **Change:** `Run::pump` marks itself open while it runs and closes on every return. A mark still open on the next call means a callback panicked through the pump, so the run faults with the new `RunError::Interrupted` (`core/run-interrupted`). `state()` and `last_error()` report the fault immediately; `pump` and `advance_to` refuse ordinary work; `stop` settles the fault and cancels the remaining work.
- **Regressions:** a control run whose only math kernel panics reports `Faulted` and `Interrupted` after the caught unwind, refuses pump and clock advance, then stops, drains to `Stopped` with no strands and disposes.
- **Measured** with `orange-measure` (release, same machine): the scalar-chain workload ran in 1.36 ms per run, against 1.44 ms recorded at the same budget before; the two flag writes per pump are not measurable.
- **Limitations:** a standalone `DataActivation` has no such mark; a panic there leaves the activation for the host to drop.

### Add write-conflict evidence — 2026-10-28

Branch `core/add-conflict-evidence` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A12 with the contract decision recorded in 11 §11.2 and 08.

- **Change:** state validation records, per write, whether it is a core `update-variable` Add with `int_overflow: "wrapping"`. The Add exemption needs that on every writer and a core `i32`, `i64`, `u32` or `u64` variable. And, Or, Xor and the existing Min/Max rule are unchanged.
- **Regressions:** saturating, checked, default (checked), mixed wrapping/saturating, float and duration Adds in two Fork branches report `E-WRITE-CONFLICT`; wrapping integer Adds and any pair under last-writer-wins do not. A call summary's Add no longer counts as evidence, while a summary's And still pairs with an And. The runtime test of concurrent Fork updates now declares wrapping overflow.
- **Behavior change:** graphs that relied on the old blanket Add exemption are rejected until they use wrapping overflow or last-writer-wins.

### Scalar contract cleanup — 2026-10-28

Branch `core/scalar-contract` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit findings A18 and A19.

- **Not:** `scalar::evaluate` now requires the `Bool` scalar for Not, as it already required a numeric scalar for arithmetic; `Not` with any numeric scalar is `InputContract`. The bundled catalog already resolves Not with `Bool`, so compiled graphs are unchanged. A unit test covers every scalar.
- **Messages:** `ScalarError::message` is the one static fallback text and also its `Display`. The bundled math adapter and runtime Update use it and keep only their own `InputContract` wording. Codes and message keys are unchanged.

**Checks:** `cargo test --workspace --locked --offline` passes with 950 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Crate documentation refresh — 2026-10-28

Branch `core/crate-docs` on `dev` c7fffe7. Closes audit finding A20.

- **Change:** the `orange-compiler` and `orange-runtime` crate docs now state the current profile, including iteration zones and graph-local Action/Async calls, and name what remains unsupported: events, package-provided function bodies, clock readers and host services. Two README sentences that still listed zones as open now match the implemented zone runtime, and the README gains one sentence on zone execution. No code changed.

**Checks:** rustdoc `-D warnings` passes.

### Metadata layering — 2026-10-28

Branch `core/metadata-layering` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A08.

- **Change:** `DeclId` moves to `id` and `HostOrigin`/`OriginError` to a new `origin` module, below the catalog. The catalog now imports neither `validation` nor `edit`. `edit::DeclId`, `validation::HostOrigin` and `validation::OriginError` remain as re-exports, so downstream paths, the wire format and the language are unchanged.
- **Scope:** actor-internal `impl Run` blocks and compiler/runtime module imports were left as they are; the audit advises against splitting the scheduler or adding traits only to restrict them.

**Checks:** `cargo test --workspace --locked --offline` passes with 949 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Narrow kernel resolvers — 2026-10-28

Branch `core/narrow-resolvers` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A10.

- **Change:** the resolver traits require only `resolve_binding` and `resolve_entropy_binding`. `BindingRequest` no longer stores the document; it extracts the locked provider when built. The whole-document `resolve` and `resolve_entropy` move to `ResolveInDocument` and `ResolveEntropyInDocument`, implemented for every resolver and not overridable. Exact provider-lock checks and retained snapshots are unchanged.
- **Breaking:** a downstream resolver that implemented only the old whole-document methods must implement the request methods. Every in-repository resolver was converted; the test of the old forwarding default was removed, and the narrow-only resolver test now needs no stub.

**Checks:** `cargo test --workspace --locked --offline` passes with 948 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### First-party dependency policy — 2026-10-28

Branch `core/dependency-policy` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Closes audit finding A09.

- **Change:** `tools/check-cargo-metadata.mjs` classifies every workspace member as core or host and enforces the first-party dependency matrix of 15, separately for normal and development dependencies. Core product graphs, following only normal and build edges, may not contain network, async-runtime, TLS, audio or OS-service crates; the existing UI/engine checks are unchanged. An unclassified member fails.
- **Fixtures:** 18 self-test cases, up from 9, including a core crate depending upward, a core crate depending on a host crate, an unclassified member, bundled shipping a runtime dependency, a transitive async runtime in core, and allowed cases for bundled's dev-only runtime use, a service crate only in core tests and services in a host.
- **Limitations:** Cargo metadata cannot see `std::fs` or other standard-library I/O.

**Checks:** `node tools/check-cargo-metadata.mjs --self-test` passes 18 fixtures; `node tools/check-cargo-metadata.mjs` passes on the workspace; `--msrv` derives 1.97.1.

### Background analysis worker — 2026-10-28

Branch `core/analysis-worker` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Addresses audit finding A06's first remediation step.

- **Change:** `validation::AnalysisWorker` owns a thread that runs `analyze_with_options` on submitted document copies. A new submission cancels the previous one through the existing cooperative flag; superseded queued jobs are skipped and cancelled results are never published. `current` filters by `ValidationAnalysis::is_current`, so results bind to the semantic state and registry generation. Dropping the worker cancels and joins it.
- **Regressions:** after 20 rapid edits each submitted, the published result equals a direct `analyze` of the newest content and is not current for any earlier one. A layout edit keeps the result current; a semantic edit, an undo to content not submitted since, and a replaced registry withhold it until resubmission. A worker with no submissions reports none and drops cleanly with a job in flight.
- **Measured** with `orange-measure` (release, same machine, 15 runs): full analysis of a 2,000-node add chain took 2.14 ms p50 from submission to result; a resubmission right after another took 2.12 ms p50.
- **Limitations:** cancellation is still observed only between passes, and semantic edits still rerun the combined analysis; per-body incremental analysis remains V12 work, and NFR-15 is not requalified. Submitting copies the document.

**Checks:** `cargo test --workspace --locked --offline` passes with 952 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Value-sized Update budget — 2026-10-28

Branch `core/size-weighted-budget` on `dev` c7fffe7 (same toolchain; Cargo.lock unchanged). Addresses audit finding A07 for Update and revises the deferral recorded with the pump budget measurements of 2026-10-27.

- **Change:** `publish_update` returns the update's value work, one unit per nominal value beyond the first in the old and the new value, and the Update step spends it from the pump quantum. Atomic Update, frozen consumption snapshots, Return and cleanup barriers are unchanged.
- **Harness:** `orange-measure` reports pump p99 and the slowest pump next to p50 and p95.
- **Measured** with `orange-measure` (release, same machine, 15 runs per workload). The audit measured a 2,106 µs p95 pump for a 1,000-entry map CAS at budget 1,024.

| Workload, budget 1,024 | Pumps/run | Pump p95 µs | Pump p99 µs | Pump max µs | Run p50 ms |
|---|---:|---:|---:|---:|---:|
| Map CAS, 1,000 entries | 1,001 | 11.0 | 11.4 | 37.8 | 9.95 |
| Map CAS, 16 entries | 77 | 6.6 | 6.9 | 10.1 | 0.50 |
| Scalar chain (16 adds) | 68 | 19.0 | 20.1 | 23.7 | 1.25 |
| Fork race + cancel region | 7 | 154.4 | 163.9 | 166.5 | 1.04 |
| Map zone, 10,000 items | 790 | 19.3 | 21.2 | 55.2 | 14.57 |

At budget 64 the slowest pump of any workload was 73.7 µs (1,000-entry map CAS). Each large CAS now takes its own pump; run totals are within noise of earlier baselines.

- **Limitations:** snapshotting a consumer's host inputs and traversing cancelled region trees are still single items, and Set publication of a large value is not weighted. Budgets remain operation bounds; NFR-02 slice latency is not qualified.

**Checks:** `cargo test --workspace --locked --offline` passes with 949 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Analysis request identity and worker lifecycle — 2026-10-28

Branch `core/analysis-request-identity` on `dev` c86b8a9 (same toolchain; Cargo.lock unchanged). Addresses findings F01–F03 of the final quality review.

- **Change:** every `AnalysisWorker::submit` returns a new `AnalysisRequestId`, and `current`/`wait` publish only the newest request's analysis, so effect-only changes on the same document and registry no longer return an earlier report. The two unbounded channels are replaced by a capacity-one mailbox: a newer submission replaces the pending one and a newer result the finished one, and a superseded result is never published. `submit` takes a host-built `Arc<GraphDocument>` snapshot instead of cloning on the caller's thread and fails with `AnalysisWorkerError` once the worker is shut down or stopped. `request_shutdown` returns without waiting, `join` waits and reports a panicked thread, and dropping no longer joins.
- **Regressions:** a worker submitted changed effects for the same document returns the report for those effects, and returning to the first effects is a new request; after 200 submissions at most one superseded snapshot is retained beside the newest, and none after the result; shutdown refuses new submissions and joins cleanly. The earlier rapid-edit, layout, undo and registry tests still pass.
- **Measured** with `orange-measure analysis-worker` (release, same machine, 9 runs, 2,000-node add chain, p50): snapshot 2.69 ms, submit 0.83 ms, full analysis 47.97 ms, resubmission 45.91 ms, shutdown request under 0.01 ms, join 1.43 ms. The 2.14 ms figures recorded with the first worker measured `wait` returning the cached result for the same document, the defect fixed here. Submit includes releasing the previous result; join here follows a submission the thread had not yet started.
- **Limitations:** one pass is still not interruptible, so a superseded or shut-down analysis runs to its next pass boundary; per-body incremental analysis remains V12 work and NFR-15 is not requalified. The snapshot is a full document copy that the host makes; structural sharing is not introduced. The worker stays in `orange-model` until a host composition crate exists.

**Checks:** `cargo test --workspace --locked` passes with 972 passed, 0 failed, 15 ignored. fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Canonical config pointer array indices — 2026-10-28

Branch `core/canonical-pointer-index` on `dev` c86b8a9 (same toolchain; Cargo.lock unchanged). Addresses finding F04 of the final quality review.

- **Change:** the config pointer helper shared by reference discovery and paste rewriting accepts an array token only as an RFC 6901 §4 index. `usize::from_str` had also accepted a leading `+`, so `/targets/+0` resolved and rewrote element zero while serde_json's pointer lookup rejects it.
- **Regressions:** `0` and `1` select elements; `01`, `+0`, `+1`, `-` and an overflowing index select nothing and are never rewritten; object keys literally named `+0` and `0` and escaped names still resolve; serde_json's `Value::pointer` agrees on every array token tested.
- **Limitations:** pointers are still parsed on every lookup rather than once at registry publication.

### Redacted literal decoding errors — 2026-10-28

Branch `core/redacted-literal-errors` on `dev` c86b8a9 (same toolchain; Cargo.lock unchanged). Addresses finding F05 of the final quality review.

- **Change:** `LiteralError::UnknownField` and `UnknownVariant` carry the rejected name as `Rejected`, so default Display and Debug no longer print arbitrary literal text; `reveal` is the controlled path. `LiteralError::message_key` adds stable `core/literal-*` keys.
- **Regressions:** a record literal with an unknown field and an enum literal with an unknown variant, both named by a synthetic marker, format without the marker, reveal it on request and report their message keys; the existing literal tests still match these errors by value.
- **Limitations:** `MissingField` and the `ty` fields still name schema-declared fields and registry types, which are not literal input.

### JSON text checked before copying — 2026-10-28

Branch `core/borrowed-json-text-check` on `dev` c86b8a9 (same toolchain; Cargo.lock unchanged). Addresses finding F06 of the final quality review.

- **Change:** `JsonText` checks size, nesting and syntax on borrowed text. `JsonText::new(String)` keeps owned input; the new `TryFrom<&str>` copies only accepted text. Literal decoding and the bundled conversion loss flag use the borrowed form, so a rejected `core/json@1` literal is no longer copied first. Valid text is still parsed once.
- **Regressions:** `orange-measure` asserts that refusing an 8 MiB JSON literal against the 1 MiB budget allocates under 64 KiB; it measured 8,618,812 bytes before the change and 56 bytes after. `TryFrom<&str>` and `new` agree on valid and malformed text.
- **Limitations:** list, map and record decoding still have no aggregate byte or element budget across a whole literal.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Public trait surfaces — 2026-10-28

Branch `core/public-trait-surfaces` on `dev` (same toolchain; Cargo.lock unchanged). Addresses finding F08 of the final quality review.

- **Change:** `ControlCell` gains a metadata-only Debug that shows whether an initial value exists but not the value; `DataPoll` derives Debug through the already payload-free `EvaluatedExport` formatter. `ListPushError`, `DuplicateKey` and `ResolveError` implement Display and `std::error::Error` with static text. The workspace enables `missing_debug_implementations`, so a new public type without Debug fails clippy `-D warnings`.
- **Unchanged on purpose:** `Run`, `DataActivation` and `TypeRegistry` still implement neither Clone nor Default.
- **Regressions:** cell Debug in a compiled plan with initial values shows `has_initial: true` and none of the values.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Same-storage list equality — 2026-10-28

Branch `core/list-eq-fast-path` on `dev` (same toolchain; Cargo.lock unchanged). Addresses finding F09 of the final quality review.

- **Change:** `ListRepr` equality compares two lists with the same packed storage slice to slice: integers and booleans directly, floats by bits, strings by pointer then content, generic values by `Value` equality. Mixed storage still compares element values, and hashing is unchanged, so Eq and Hash keep agreeing across storages.
- **Regressions:** NaN lists are equal and `0.0`/`-0.0` lists differ in both float widths; shared, separately allocated and different strings compare as expected; every same-storage result matches element-wise equality, and equal lists hash alike, also against a generic `Values` copy.
- **Measured** with `orange-measure string-list-eq` (release, same machine, 15 runs): comparing two 65,536-element string lists sharing one string 16 times took 32.58 ms p50 before and 0.21 ms after. This isolates reference-count work in a sharing-heavy case; it is not an end-to-end graph speedup.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### MapRepr construction contract — 2026-10-28

Branch `core/map-repr-contract` on `dev` (same toolchain; Cargo.lock unchanged). Addresses finding F10 of the final quality review.

- **Change:** `MapRepr` documentation promised non-float keys although `MapRepr::new` checks only structural key uniqueness. It now states that it is untyped storage and that key eligibility (no floats, language `hash` and `eq`) is enforced by the registry-aware literal codec and by value shape checks. No behavior changes: registered package key types stay possible.
- **Regressions:** a map built with a float key is refused by the shape check for both a `u64` and an `f64` key type.

### Typed run identities and timer ownership — 2026-10-28

Branch `core/typed-runtime-ids` on `dev` (same toolchain; Cargo.lock unchanged). Addresses the first slices of finding F07 of the final quality review.

- **Change:** strands, regions and function frames use `StrandId`, `RegionId` and `FrameId` instead of plain `u64`. A frame has no identity of its own: it takes its function region's identity only through `FrameId::of_region` and `RegionId::of_frame`. `OwnerIds` is the only place that mints strand and region identities after the activation roots. The ready queue, ownership sets, pull owner, segment owner and loop unwinding carry the typed identities. `Timers` owns the timer order and the strand-to-timer directory together, so stepping and cancellation call its operations instead of editing two maps.
- **Unchanged:** behavior, ownership counts and the single-owner actor model; no locks or shared state were introduced.
- **Measured** with `orange-measure` (release, same machine, 15 runs per workload) against `dev` 8bebb3f: allocations per run and peak live bytes are identical for every workload, and pump times stay within run-to-run noise.
- **Limitations:** sibling run modules still receive `&mut Run`; ownership, function and loop code still reach the scheduler, frames and stores directly. Grouping ownership state and narrowing step contexts beyond `Cells` remain F07 work.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Panic policy — 2026-10-28

Branch `core/unwrap-audit` on `dev` (same toolchain; Cargo.lock unchanged). Addresses the unwrap/expect criterion of the final quality review.

- **Rule:** `orange-model`, `orange-compiler`, `orange-runtime` and `orange-bundled` warn on `clippy::unwrap_used` and `clippy::expect_used` in library code, and CI runs clippy with `-D warnings`. Tests, examples and `orange-measure` are exempt. Input, lifecycle and other fallible paths return typed errors. A panic remains only for an internal invariant, written as `#[expect(clippy::expect_used, reason = "...")]` on the statement or function that states why it cannot fail.
- **Fixed names:** `port_id!("then")` checks a port literal at compile time. `id::fixed_id` is the one documented parse for identifiers spelled in source code; it accepts only `'static` text, so document and host input cannot reach it, and the identifier tables it parses are covered by tests. Call and return ports carry `PortId` instead of reparsing their names, and printed type terms and signature digests convert to their identifiers without a parse.
- **Runtime:** run and data stepping already report broken actor state as `RunError::InvalidState` and data kernel mismatches as `DataError::KernelContract`; their remaining `unwrap` and `expect` calls now use those errors, so the run faults instead of panicking. The output-publishing helpers and the step completion paths return `Result` for this. A collapsed ownership spill no longer needs a panic, and the data work loop ends when its stack is empty. Allocation counts and peak bytes in `orange-measure` are unchanged.
- **Invariants kept loud:** ready-queue links, the frame a prepared write just found, built-in registries, strongly connected components, dependency counts, rollback of patches that just applied, resolved converters, slot positions, list indexes and JSON number conversions keep their panic with a stated reason. Replacing them with defaults or skipped work would hide bugs. Trusted callback panics still propagate as before.

### Region tree ownership — 2026-10-29

Branch `core/run-regions` on `dev` 88d8727 (same toolchain; Cargo.lock unchanged). Continues finding F07 of the final quality review.

- **Change:** `Regions`, in its own module with private fields, owns the run's region map, the identity counter and the live strand count. Body entry, function calls, Fork, strand retirement, region completion, loop unwinding and cancellation use its operations (`open`, `retire_strand`, `take_if_done`, `detach`, `subtree`, `prune`, `has_room`, `root_of`, `loop_region`), so no other run module can insert or remove a region, mint an identity or change the live count directly. Opening a region lists it under its parent and counts its strands in one step; admission checks strands, regions and identities together.
- **Unchanged:** behavior, trace order, ownership counts and allocation. Fork now records the join region before its branches; the parent's child set and the ready order are the same. A function call at identity exhaustion now fails through the run's ordinary resource-limit path like body entry and Fork.
- **Regressions:** opening and retiring keep the parent's children and the live count in step, a region with live strands is not taken, a second retirement or detach is refused, and pruning a subtree leaves only the root with its cleanup strands.
- **Measured** with `orange-measure` (release, same machine, 15 runs per workload) against `dev` 88d8727: allocations per run and peak live bytes are identical for every workload; pump times stay within run-to-run noise.
- **Limitations:** run modules still receive `&mut Run` and reach the scheduler, frames, activations and trace directly; regions' own fields remain visible to them for continuations, cursors and join state. Narrower step contexts beyond `Cells` remain F07 work.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Loop step context — 2026-10-30

Branch `core/loop-step-context` on `dev` 04879d4 (same toolchain; Cargo.lock unchanged). Continues finding F07 of the final quality review.

- **Change:** the control trace becomes `Trace`, which only records events and reports what it kept and dropped. ForEach, While, Count, Sequence, Break and Continue steps, cursor resumption and iteration frames now run on `Loops`, which borrows only the state layout, the limits, the region tree (read-only) and the trace. Loop code can no longer reach the scheduler, timers, frames, cells or activations.
- **Step families now:** gate, latch, register and Set steps use `Cells`; Branch, Switch and Match Result use the layout alone; Fork, Entry, Yield and Wait use the clock alone; loops use `Loops`. Calls, Return and Update still run on `Run`: they pull data through the run's demand and epoch machinery, and calls open regions and frames. Update's writes already go through `Cells`.
- **Unchanged:** behavior and trace order. `orange-measure` against `dev` 04879d4 shows identical allocations per run and peak live bytes for every workload, with pump times within run-to-run noise.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Literal decode budget — 2026-10-30

Branch `core/literal-decode-budget` on `dev` 78d9097 (same toolchain; Cargo.lock unchanged). Follows the limitation recorded with finding F06.

- **Change:** literal decoding charges one byte budget across the whole literal: 16 bytes per decoded value and the length of every string, byte, secret-reference and JSON text payload, each before it is copied or decoded. `literal_codec::decode_with_budget` takes the budget; `decode` uses `MAX_LITERAL_BYTES`, 64 MiB, the default document import size, so every literal a default document can hold still decodes. Exceeding it fails with `LiteralError::TooLarge` for the type at the failing position. The compiler keeps its own constant preflight against `max_constant_bytes`.
- **Regressions:** a list of 100 strings of 1,000 bytes decodes with exactly its charge and fails one byte below it.
- **Limitations:** `MAX_LITERAL_BYTES` is a proposed default, not a qualified budget; model validation uses the default rather than a host-chosen limit.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Update step context — 2026-10-30

Branch `core/update-step-context` on `dev` 22ad0fc (same toolchain; Cargo.lock unchanged). Continues finding F07 of the final quality review.

- **Change:** data demand consumption (`consume`, `consume_inputs`) and the whole Update step now run on `Cells`, since they read only the layout, cells and frames and write cells. The epoch, pull ownership and host-input snapshot live in the activation and strand passed in, as before.
- **Step families now:** gates, latches, registers, Set and Update use `Cells`; Branch, Switch and Match Result use the layout; Fork, Entry, Yield and Wait use the clock; loops use `Loops`. Only Call and Return still run on `Run`, because a call opens a region and a frame and schedules its body, and a return completes regions and resumes the caller.
- **Unchanged:** behavior, trace order and allocation; `orange-measure` against `dev` 22ad0fc shows identical allocations per run and peak live bytes, with pump times within noise.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Ownership work in the pump budget — 2026-10-30

Branch `core/ownership-work-budget` on `dev` ec72b64 (same toolchain; Cargo.lock unchanged). Continues audit finding A07 after the value-sized Update budget.

- **Change:** `Regions` counts ownership work beyond one unit per step: each Fork branch beyond the first and each descendant region a cancellation retires. The pump spends that count from its budget after the step, so a wide Fork or race cancellation ends the pump instead of running further work behind it. Ownership work done outside a pump, such as a host stop, is not charged to the next one. Spawning and each cancellation traversal are still single steps.
- **Harness:** `orange-measure fork-fanout-256` runs a 256-branch Fork race 100 times; one branch finishes at once and 255 wait, so every iteration cancels 255 strands. The measurement tests now take a lock, because the counting allocator is process-wide and a long workload in a parallel test skewed the others.
- **Measured** with `orange-measure` (release, same machine, budget 64, 15 runs):

| Workload | Pumps/run | Pump p50 µs | Pump p95 µs | Pump max µs |
|---|---:|---:|---:|---:|
| fork race fan-out 256, before | 8 | 732.3 | 1,144.2 | 2,331.5 |
| fork race fan-out 256, after | 201 | 27.8 | 84.4 | 200.6 |
| fork race + cancel region, before | 110 | 10.8 | 15.2 | 46.4 |
| fork race + cancel region, after | 141 | 8.3 | 8.7 | 27.6 |

  Other workloads keep their pump counts, allocations and peak bytes.
- **Regressions:** the fan-out workload needs at least one pump per spawn and per cancellation at budget 64; before the change it needed 8 pumps for all 100 iterations.
- **Limitations:** one cancellation of 255 strands still takes about 200 µs in a single indivisible step, and host-input snapshots remain one loop per pull. Making either resumable would change cancellation order or frozen-snapshot semantics and needs a design decision. NFR-02 is not qualified.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.

### Session input check reuse (V12b) — 2026-10-30

Branch `core/session-input-reuse` on `dev` 95d7944 (same toolchain; Cargo.lock unchanged). Continues audit finding A06 and V12b (16).

- **Change:** `AnalysisSession` keeps each node's input diagnostics together with the resolved signature, the rigid type parameters and the ordered wire answers the check read. An analysis reuses them when the change journal left the node untouched and all of those are equal; the shared per-node checker computes every miss. Registry changes and unproven journals clear the cache. `LocalWork` gains `inputs_computed` and `input_hits`.
- **Regressions:** one configuration edit among 16 nodes recomputes one input check and reuses 15; a cached check is recomputed when its wire moves between bodies; the session-versus-full-analysis property tests, which connect, disconnect, edit literals and variadics and add and remove functions, pass, also with 512 cases in release.
- **Limitations:** the session still visits every node and runs every other pass in full, so V12b is not complete and NFR-15 is not requalified. Each entry clones its node's signature snapshot; retained memory is not measured. No release timing was recorded: the existing fixtures have only scalar input literals, so a timing would mostly measure orchestration.

**Checks:** `cargo test --workspace --locked` passes; fmt, clippy `-D warnings` and rustdoc `-D warnings` pass. Linux, macOS and the MSRV toolchain were not run locally.
