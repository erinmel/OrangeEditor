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

- Bevy's v0.20.0 manifest requires Rust 1.97.1 (S-B02). The installed 1.99.1 satisfies that minimum. B-P01 has not compiled Bevy with it. MSRV (1.97.1) CI coverage remains a WU-01 decision.
- WU-00/00b.1 (2026-10-20) added `rust-toolchain.toml` (channel `1.99.1`, components rustfmt/clippy, profile minimal), the root Cargo workspace (`resolver = "3"`, edition 2024, `rust-version = "1.97.1"`, member `app/crates/orange-model`, `plugins` excluded) and a tracked `Cargo.lock`. Cargo resolved dependencies to the highest versions compatible with Rust 1.97.1; see the 00b.1 row in the dependency ledger.
- Superseded planning observation: the environment originally reported `rustc`/`cargo` 1.96.1 from a standalone MSI install (`C:\Program Files\Rust stable MSVC 1.96`) that shadowed rustup on PATH, plus rustup toolchains `stable` and `nightly-2026-09-07`. That MSI, the previous rustup install and its toolchains were removed on 2026-10-20 before the reinstall above.
- 00b.1 local evidence (Windows 11, Rust 1.99.1, 2026-10-20): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked` passed (22 tests: 8 unit, 13 codec integration, 1 property test at proptest's default 256 cases). Remote `rust` CI evidence is recorded when it first runs.
- 00b.2 local evidence (same environment, 2026-10-20): the same three commands passed with 46 tests in total. The 24 new tests are 4 for G-36 layout, 2 for G-30 literals, 5 for G-31 variadic ports, 12 for transaction structure and 1 property test of undo-all/redo-all over random edit sequences. No new dependency.
- Apart from `orange-model`, no application implementation, integration test, audio routing check, broker/container setup or OS packaging verification has run. Local availability of Rust is not proof of a working Bevy build toolchain.

## Additional AI development integrations

**No new MCP/plugin is required to finish this plan.** Web research, filesystem tools and an architecture subagent are available. These development integrations are unrelated to OrangeEditor expansion packages.

Context7 was found in the available plugin directory and suggested as an optional documentation integration. It was **not installed or connected** in this session. It may help retrieve dependency documentation, but Bevy 0.20 coverage/version selection must be verified before using its answers. Release-tag source and versioned local Rustdoc remain authoritative. Installation is optional and must not block planning or justify substituting 0.19 APIs.

OBS/Twitch account connectors are not required to design Orange's SDK. Actual integration testing later requires local OBS, an explicitly authorized Twitch test application/account and synthetic fixtures; credentials belong in the vault, never agent prompts or source files. Do not introduce an unrelated cloud project-management system as a prerequisite.

For implementation, verify the Rust toolchain, target linker/native SDKs, language server, formatting/lint tools and platform libraries. These are development prerequisites, not claims that they are installed. Any broker/container, audio diagnostic or security tool is added only for a work unit that uses it.

## Bevy source register

All rows retrieved 2026-10-20. Mutable `latest` URLs below were opened and their visible version recorded; replace them with immutable/local evidence before implementation. Source evidence does not establish that Orange's proposed design has been tested.

| ID | Primary source | Resolved version | Evidence / use | Limitation |
|---|---|---|---|---|
| S-B01 | [Release announcement](https://bevy.org/news/bevy-0-20/) | 0.20 release, 2026-10-08 | Release existence; UI and graphics feature overview | Overview, not complete API contract |
| S-B02 | [Release manifest](https://github.com/bevyengine/bevy/blob/v0.20.0/Cargo.toml) | v0.20.0 | Edition, MSRV, version, license, feature declarations | Transitive dependencies not audited |
| S-B03 | [Plugin Rustdoc](https://docs.rs/bevy/latest/bevy/app/trait.Plugin.html), [tag source](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_app/src/plugin.rs) | Header 0.20.0 / v0.20.0 | Developer-plugin startup lifecycle | Does not implement Orange hot package lifecycle |
| S-B04 | [Schedule Rustdoc](https://docs.rs/bevy/latest/bevy/ecs/schedule/index.html) | Header 0.20.0 | System ordering/execution concepts | Graph semantics remain Orange's responsibility |
| S-B05 | [Message Rustdoc](https://docs.rs/bevy/latest/bevy/ecs/message/index.html) | Header 0.20.0 | Buffered messages and update retention | Not persistent/durable queue evidence |
| S-B06 | [Event module source](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_ecs/src/event/mod.rs) | v0.20.0 | Event/observer infrastructure | No Orange delivery guarantee follows |
| S-B07 | [Task-pool source](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_tasks/src/usages.rs) | v0.20.0 | Compute/async-compute/I/O distinction | Network-client reactor integration untested |
| S-B08 | [Feathers Rustdoc](https://docs.rs/bevy/latest/bevy/feathers/index.html) | Header 0.20.0 | Feature gate, editor widget role | Orange interaction/accessibility not verified |
| S-B09 | [Release examples index](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/README.md) | v0.20.0 | Specific UI/input/rendering test leads | Example existence does not prove requirement coverage |
| S-B10 | [Picking event source](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_picking/src/events.rs) | v0.20.0 | Input event implementation reference | Large graph hit testing unmeasured |
| S-B11 | [AudioPlugin Rustdoc](https://docs.rs/bevy/latest/bevy/audio/struct.AudioPlugin.html) | Header 0.20.0 | Public playback integration surface | No per-player device-routing proof |
| S-B12 | [Audio output source](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_audio/src/audio_output.rs) | v0.20.0 | Default sink opening and internal output ownership | Alternative routing requires separate validation |
| S-B13 | [Headless example](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/app/headless.rs) | v0.20.0 | Feature selection and runner reference | Orange core must work independently |
| S-B14 | [Linux dependencies](https://github.com/bevyengine/bevy/blob/v0.20.0/docs/linux_dependencies.md) | v0.20.0 | Distribution build prerequisites | Selected features/package runtime still need checking |
| S-B15 | [Migration guide](https://bevy.org/learn/migration-guides/0-19-to-0-20/) | 0.19 to 0.20 | Relevant incompatible API/behavior changes | Not a substitute for release API compilation |
| S-B16 | [Published manifest](https://docs.rs/crate/bevy/latest/source/Cargo.toml) | Header/package 0.20.0 | Corroborates release metadata | Mutable URL; exact-version fetch failed through research tool |

Excluded evidence: `https://docs.rs/bevy/latest/bevy/prelude/` opened as 0.19.1; conflicting search snippets were not accepted. Exact-version docs.rs requests failed through the research tool, so no claim is made that all versioned API pages were retrieved. Release-tag sources provide a verified alternative.

## Dependency selection ledger

Bevy is the sole requested framework pin. All other implementation dependencies must acquire an exact version, minimum compiler, platform/feature matrix, license, maintenance/security review and contract test at their adopting work unit. A proposed family below is a research candidate, not a dependency approval or compatibility claim.

| Capability | Candidate / decision boundary | Required evidence before adoption |
|---|---|---|
| Presentation/graphics | Bevy =0.20.0, Bevy UI/Feathers candidate | B-P01/B-P02; minimal feature graph, text/accessibility evidence |
| Async services | Dedicated Tokio host as proposed in architecture | Exact crate/features/MSRV, shutdown/reactor contract, bounded worker bridge |
| Hot expansion execution | Wasmtime/component-model candidate | Exact supported component interface/runtime version, fuel/epoch interruption, memory limit, host-call cancellation and boundary benchmarks |
| Definitions and persistence | Serde-family serialization candidate | Versioned schema contract, malformed input bounds, deterministic output and migration tests |
| HTTP/TLS | Async HTTP client with explicit TLS backend | HTTP/2, proxies, certificates, streaming, cancellation, ALPN and licenses |
| WebSocket/MQTT/AMQP/gRPC | Separate protocol adapters, not a universal network library | Protocol version, authentication/stream behavior, async runtime compatibility, bounded payloads and fixtures |
| Audio | Dedicated endpoint/decoder service plus OS adapters | Actual per-player routing, decoder format/license inventory, device identity, clock/rate behavior and callback audit |
| Linux virtual source | PipeWire adapter | Versioned API/permissions/session policy and app-owned cleanup on a real Linux session |
| Credentials | OS-specific secure stores behind one host interface | Windows/macOS/Linux availability and locked/headless behavior; no plaintext fallback |
| Localization | Fluent FTL parser/formatter candidate, Unicode/CLDR data and host font fallback | Exact versions/features/MSRV/licenses, bounded reference/argument handling and locale/plural capabilities; L-suite, G-39 and B-P02 platform evidence before adoption; sources in [13](13-internationalization.md) |

### Adopted: `orange-model` (WU-00/00b.1, 2026-10-20)

Serde-family candidate adopted for the graph document codec only. Versions are exact `Cargo.lock` entries; licenses and MSRV are from `cargo metadata --locked`. All declared MSRVs are at or below the workspace's 1.97.1.

| Crate | Version / features | License | Declared MSRV | Role |
|---|---|---|---|---|
| serde (+ serde_core, serde_derive) | 1.0.229, `derive` | MIT OR Apache-2.0 | 1.56 / 1.71 | Codec derives |
| serde_json | 1.0.154, `preserve_order` | MIT OR Apache-2.0 | 1.71 | JSON load/save, ordered opaque objects |
| indexmap (+ hashbrown, foldhash, equivalent) | 2.14.2, `serde` | Apache-2.0 OR MIT (foldhash: Zlib) | 1.85 | Creation-ordered maps |
| uuid | 1.28.0, `serde` | Apache-2.0 OR MIT | 1.89.0 | v4/v7 document IDs |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | 1.77 | Error enums |
| proptest (dev only) | 1.12.0 | MIT OR Apache-2.0 | 1.88 | Round-trip property tests |

Transitive normal dependencies also include itoa 1.0.18, memchr 2.8.3 (Unlicense OR MIT), zmij 1.0.23 (MIT), proc-macro2 1.0.107, quote 1.0.47, syn 3.0.9 and unicode-ident 1.0.27 ((MIT OR Apache-2.0) AND Unicode-3.0). Security advisory review (for example `cargo audit`) has not been run. Malformed-input size bounds and migrations remain open for later 00b slices.

Do not select a crate solely because a Bevy ecosystem page lists it. Independently versioned service crates need not depend on Bevy; UI/render integrations must explicitly support 0.20.0. Record implementation results and the final lockfile in the work unit rather than retroactively presenting candidates as verified.

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

Primary i18n sources and their limits are recorded in 13 §8. No Rust/Bevy compilation, package loading, Fluent parsing, translation review, renderer/accessibility/platform probe, benchmark or implementation fixture ran. Exact formatter/data pins and additional shipped languages remain open before their named gates. No gate is declared passed by this review; next work remains 00a and 01a under a later implementation instruction.

### Git workflow review (2026-10-20)

Current observed directory: `C:\Users\eriar\OneDrive\Documentos\Projects\Programming\rust\OrangeEditor`. The original `OraneEditor` working directory is unavailable; earlier path records are historical. No folder was renamed by this review. `Test-Path .git` returned `False`; `git --version` returned `git version 2.55.0.windows.1`.

Added `docs/14-git-workflow.md`, `CONTRIBUTING.md` and `.github/PULL_REQUEST_TEMPLATE.md`; linked the policy to README, the assignment, delivery dependencies, quality checks and ADR-18/NFR-14. Selected one integration branch plus short-lived subunit branches, PR/squash integration, solo/team review modes, protected history, scoped checks, release/hotfix rules and explicit bootstrap/enforcement evidence. Corrected the previous universal PR-test wording so the documentation baseline does not require nonexistent Rust CI. No repository initialization, configuration, commit, branch, remote operation, CI workflow or release action was performed.

Executed validation: Git 2.55.0.windows.1 `git check-ref-format --branch <example>` accepted all eight branch examples in 14. Node.js v24.18.0 `node -` with the document checker on stdin passed for 19 Markdown files, 13 JSON blocks and 197 local file links; unique defined/reference IDs include 51 FR, 16 NFR, 18 ADR, 13 OPEN, 39 G and 8 L. Checks cover fences, JSON syntax, local links, catalogue table continuity, ID references, FR/NFR roadmap coverage and the repeated math-definition consistency. The checker is session tooling; these results do not establish remote enforcement. Hosting/visibility, maintainer identities, CI statuses/runners and signing setup remain pending. Primary Git/hosting references are recorded in 14; only document checks and branch-name validation ran.

### Public monorepo bootstrap (2026-10-20)

The user subsequently authorized creation/publication of `erinmel/OrangeEditor` with separate application and authored-plugin areas. GitHub profile/CLI identity agreed on `erinmel`; the repository lookup returned 404 before creation. Added the source areas and ownership in 15/ADR-19, ignore/line-ending policy, read-only CI and committed validation scripts. The current folders are documented reservations, not product scaffolding or implemented expansion packages. Local IDE state remains ignored and unmodified. The commit email uses the account's GitHub noreply address.

Tools: Git `2.55.0.windows.1`, GitHub CLI `2.96.0`, Node.js `v24.18.0`. Commands executed include `git init -b main`, explicit-path staging, both `node tools/check-*.mjs` scripts, `git diff --cached --check`, the initial commit, and `gh repo create erinmel/OrangeEditor --public --source . --remote origin --push` (with description/wiki options). Initial commit: `42efbe0f856a6e7ff956307d456ea7af5f7db4fd`; 32 tracked files. Local structural checks passed for 27 Markdown files and 13 JSON blocks; tracked-path/credential-pattern/whitespace checks passed. These scans are limited and do not prove complete secret absence or product correctness.

Public/default-branch metadata and SSH origin were read back successfully. The initial repository-check workflow run `39493589359` completed both `docs` and `policy` successfully on the configured Ubuntu runner. Action refs were resolved through the GitHub API and pinned to full commits: checkout v4 `11d5960a326750d5838078e36cf38b85af677262`, setup-node v4 `49933ea5288caeca8642d1e84afbd3f7d6820020`; Node is pinned to 24.18.0. After the first passing run, the main protection API was applied and read back: strict required checks bound to Actions app 15368, PR/conversation/linear-history requirements, zero solo-mode independent approvals, admin enforcement and no force pushes/deletion. Repository settings were verified squash-only with automatic topic-branch deletion.

The first attempt to use unsupported `gh repo edit --disable-merge-commit` flags made no change; supported boolean `--enable-merge-commit=false`/`--enable-rebase-merge=false` then succeeded. No rejected main push/deletion or other destructive protection test was attempted. Release/tag enforcement, license/signing choices and Rust/SDK/plugin/platform checks remain unverified future work. Follow-up evidence is submitted through a topic PR under the active workflow.
