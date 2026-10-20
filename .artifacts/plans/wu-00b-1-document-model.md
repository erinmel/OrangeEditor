# Plan: WU-00/00b.1 — workspace setup and first testable document model

Status: **implemented locally**, 2026-10-20, on branch `feat/wu-00b-document-model`. Local fmt/clippy/test evidence is recorded in [10](../../docs/10-tooling-and-sources.md). Remote CI has not run. See §9 for where the implementation differs from this plan.

Contracts: [12 §2–3, §6](../../docs/12-graph-data-structures.md) (normative layout and JSON), [11](../../docs/11-graph-language.md) (semantics), [02](../../docs/02-architecture.md) (crate boundaries), [07](../../docs/07-delivery-plan.md) (00b acceptance), [14](../../docs/14-git-workflow.md) (branch/commit/check rules).

## 1. Why this slice, and the gate caveat

07 lists 00b (`orange-model`) as the first code subunit. Its full acceptance is broad: IDs, document structs, literal codec, signature snapshots, transactions/undo, derived indexes, paste remap and the `Value` size assertion. One PR for all of it breaks the "small reviewable slice" rule in 14, so 00b is split (§7) and this plan covers only **00b.1: setup + IDs + document structs + JSON codec**.

Caveat: 07 orders 00a (fixtures/semantic review) and G0 before product code. Neither is recorded as passed. Starting 00b.1 now is a user-directed choice. It is chosen because the persisted JSON shape in 12 §6 is the part least likely to change in 00a. If 00a revises 12, this slice's fixture and codec change with it. The PR must say G0/00a are open; merging it passes no gate.

## 2. Scope

In:

1. Root Cargo workspace, toolchain pin, lint policy and a `rust` CI job.
2. Crate `app/crates/orange-model` with: typed identifiers; document structs for every field in the 12 §6 example; a JSON codec (load/save) enforcing structural invariants only.
3. Unit, golden, error and property tests.
4. Same-PR doc updates (§6).

Out (later 00b slices or other subunits): transactions/ChangeSet/undo, derived indexes, paste remap, signature hashing, `Value` and typed literal decoding, the TypeExpr parser (00c), any semantic validation (00d: unknown nodes/ports, fan-in, cycles), registry lookups. No Bevy, Tokio, network or filesystem dependency (NFR-14).

## 3. Setup

| File | Content |
|---|---|
| `rust-toolchain.toml` | `channel = "1.99.1"`, components `rustfmt`, `clippy` (matches the local toolchain recorded in [10](../../docs/10-tooling-and-sources.md)) |
| `Cargo.toml` (root) | `[workspace]` `resolver = "3"`, `members = ["app/crates/orange-model"]`, `exclude = ["plugins"]` (plugins build independently, 15). `[workspace.package]` `edition = "2024"`, `rust-version = "1.97.1"`, `publish = false`; no license field (license not chosen, 15). `[workspace.dependencies]` holds every version once (DRY). `[workspace.lints]` `rust.unsafe_code = "forbid"`, `clippy.all = "warn"` |
| `Cargo.lock` | Tracked |
| `.github/workflows/checks.yml` | New `rust` job on `ubuntu-latest`: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`. No third-party action: rustup on the runner reads `rust-toolchain.toml`. Keep `permissions: contents: read`. Make `rust` a required status only after it has passed once remotely (14 §7) |

Dependencies (crate-local; add with `cargo add`, then copy exact resolved versions and licenses from `Cargo.lock` into 10):

| Crate | Use | Kind |
|---|---|---|
| `serde` (derive) | Codec derives | normal |
| `serde_json` (`preserve_order`) | JSON in/out; opaque config/editor/extension values keep key order | normal |
| `uuid` (`serde`) | UUID-backed IDs | normal |
| `indexmap` (`serde`) | Creation-ordered node/edge/decl maps (12 §3.1) | normal |
| `thiserror` | Error enums | normal |
| `proptest` | Round-trip property tests | dev |

Before adding each, check current docs and MSRV compatibility with ≤1.97.1 (`ctx7`, per user rules). Anything else needs a reason in the PR.

## 4. Design

Principles applied:

- **KISS**: opaque newtypes where the semantics belong to a later subunit (`TypeExpr` is a validated-nonempty canonical string until 00c; `config`, `editor` and extension maps are ordered JSON). No traits without a second implementation.
- **DRY**: one macro defines all UUID ID newtypes; one generic serde adapter turns `IndexMap<K, T>` into a JSON array of objects carrying their own `id` and rejects duplicates; versions live once in the workspace manifest.
- **SOLID / high cohesion**: one module per concern (below). The codec checks *structure* (format, version, ID syntax, uniqueness); it never checks *meaning* (unknown definitions, edges to unknown ports are legal per 12 I-4). Meaning is the validator's job (00d).
- **Low coupling**: domain structs know nothing about the top-level wire envelope; `codec` alone knows `"format": "orange.graph"`, the flattened root body and `format_version`. Public API is a handful of types plus `from_json`/`to_json`; fields that later need invariants (revision, maps) are private with accessors, so 00b.2 transactions can own mutation.

Modules in `app/crates/orange-model/src/`:

| Module | Owns |
|---|---|
| `id.rs` | `GraphId`, `NodeId`, `EdgeId`, `VariableId`, `EventDeclId`, `FunctionId` (UUID, via macro); `PortId` (`[a-z][a-z0-9_]{0,63}`); `PortKey` (`port_id` or `port_id#k`, `k: u16`); `DefinitionId` (`namespace/name`, `is_core()`). Each has `FromStr`/`Display` and serde through them, so invalid text cannot be constructed |
| `type_expr.rs` | `TypeExpr` opaque canonical string; the parser/interner arrives in 00c behind the same type |
| `literal.rs` | `Literal` wire shape from 12 §6.1: `type`, then exactly one of `value` (JSON) or `bits` (lowercase hex, 8 or 16 digits), optional `display`. Typed decoding is 00b.4 |
| `document.rs` | `GraphDocument`, `GraphBody`, `NodeInstance`, `DefinitionRef`, `ProviderBinding`, `Edge`, `PortRef`, `VariableDecl`, `EventDecl`, `FunctionDecl`, `ActivationPolicy`, `PackageLockEntry`, `TypeLockEntry`, `SignatureSnapshot`, `ResolvedPort`, `GraphExport`, closed enums (`VarScope`, `ConflictPolicy`, `NodeKind`, reentrancy/overflow/fault policies). Enum tokens are copied from 11/12; an unknown token is an error, never a silent default |
| `keyed.rs` | `Keyed` trait (`fn key(&self) -> K`) + serde adapter for `IndexMap<K, T>` ↔ array |
| `codec.rs` | `from_json(&str) -> Result<GraphDocument, CodecError>`, `to_json(&GraphDocument) -> String`; envelope check; node/edge ID uniqueness across root and function bodies (12 §6) |
| `error.rs` | `IdError`, `CodecError` (thiserror); messages carry the JSON path and offending ID, never document contents |

Unknown fields: `GraphDocument`, `NodeInstance` and `Edge` keep a flattened `extensions` map so unknown keys survive load→save (FR-05). Test serde `flatten` interaction with the root-body flatten early; if it misbehaves, the codec uses a private wire DTO for the top level only. The "bounded" limit on extensions in 12 §3.1 has no number yet: record it as an open question in the PR instead of inventing one.

Determinism: maps use `IndexMap` (creation order) or `BTreeMap` (`type_args`, `inputs`, `signatures`), so the same document always produces the same bytes. Output is compact JSON plus a trailing newline; pretty formatting is not part of the contract.

## 5. Tests (acceptance for 00b.1)

| Test file | Checks |
|---|---|
| `src/id.rs` unit tests | PortId: 1 and 64 chars accepted, 65 rejected, uppercase/leading digit/empty rejected. PortKey: `a`, `a#0`, `a#65535` accepted; `a#`, `a#65536`, `a#-1`, `#1` rejected; display round-trips. DefinitionId: exactly one `/`, nonempty sides; `core/x` reports `is_core()`. UUID IDs reject `n1` |
| `tests/fixtures/threshold-v1.json` | The 12 §6 example with short IDs replaced by fixed UUIDs (12 §3 says real documents use UUIDs) |
| `tests/codec_golden.rs` | Load fixture → every field reachable through the API (spot-check node `n3` provider, `type_args`, edge `e7`, variable init literal, export); `to_json(from_json(x))` equals the canonical fixture bytes; saving twice gives identical bytes |
| `tests/codec_unknown.rs` | Unknown top-level, node and edge keys survive round trip unchanged and in order; a node with an unknown `definition_id` and its signature snapshot survive (structural basis for G-29) |
| `tests/codec_errors.rs` | Wrong `format`, `format_version: 2`, missing required field, duplicate node ID, duplicate edge ID, same node ID in root and a function body, invalid PortKey, `value` and `bits` both present, unknown enum token: each returns its specific `CodecError` variant |
| `tests/roundtrip_props.rs` | proptest strategies for IDs, PortKeys, literals, nodes and edges: `from_json(to_json(doc)) == doc` for generated documents |

Commands (record exact output in the PR and 07/10 evidence):

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node tools/check-docs.mjs
node tools/check-repo-policy.mjs
```

Done when all five pass locally and the remote `docs`, `policy` and `rust` jobs pass on the PR.

## 6. Same-PR documentation

- [10](../../docs/10-tooling-and-sources.md): `rust-toolchain.toml` selection, exact dependency versions/features/licenses from `Cargo.lock`, commands run.
- [07](../../docs/07-delivery-plan.md): split 00b into 00b.1–00b.4 (§7) with the same overall acceptance; record 00b.1 evidence. No gate marked passed.
- [app/crates/README.md](../../app/crates/README.md) and [15](../../docs/15-monorepo-layout.md): `orange-model` and the root workspace now exist.
- [14 §5](../../docs/14-git-workflow.md): `rust` status exists; note when it becomes required.
- No ADR: this slice implements 12 as written. If implementation forces a JSON/wire change, stop and update 12 + ADR first (AGENTS.md rule).

## 7. Remaining 00b slices (one PR each)

| Slice | Output | Acceptance from 07 / 11 |
|---|---|---|
| 00b.1 | This plan | Structural round trip, unknown fields, determinism |
| 00b.2 | `DocCommand` transactions, `ChangeSet`, inverse/undo, single revision bump | G-30, G-31, G-36; FR-03 document undo |
| 00b.3 | Derived indexes, paste remap, signature snapshot projection + `sha256:` canonical key | FR-02 remap; G-29, G-39 semantic snapshot vectors |
| 00b.4 | `Value` enum with `≤16 bytes` size assertion; typed literal codec per 12 §6.1 | FR-08 round trips; `Value` assertion |

## 8. Commit sequence on this branch

1. `docs(tooling): record rustup 1.99.1 toolchain selection` — the pre-existing AGENTS/09/10 edits.
2. `chore(repo): add .artifacts AI context folder and ignore rules`.
3. `docs(plan): plan WU-00b.1 document model slice` — this file.
4. Implementation commits per §3–§6, then PR per the template with `Work: WU-00/00b.1`.

## 9. Implementation notes and deviations

- **`TypeExpr` location.** `TypeExpr` and `SignatureKey` live in `id.rs` and share one `opaque_text!` macro (DRY). There is no `type_expr.rs` yet. 00c can move `TypeExpr` without changing callers.
- **Public fields.** All document fields are `pub`, including `revision` and the maps. Private fields would have needed a builder API just for tests. 00b.2 narrows this surface when transactions become the only mutation path. Until then, the map key always equalling the item's `id` is a convention (`keyed::collect` keeps it).
- **Unknown keys.** Every extensible object (document, node, edge, provider, lock entries, declarations, signature and port descriptors, activation, export, type parameter) has a flattened `extensions` map. `Literal` rejects unknown keys: it is a closed value encoding, so an unknown key is an error rather than silent loss.
- **Error granularity.** Format, version and cross-body duplicates are their own `CodecError` variants. Errors raised during parsing (ID syntax, duplicates within one body, literal shape, unknown enum tokens) arrive as `CodecError::Json`; the tests match on their message text.
- **Saved form.** Every top-level collection is always written, even when empty, and loading accepts documents that omit them. `persistent` and `conflict` are always written (defaults `false` and `reject`).
- **Golden file.** `tests/fixtures/threshold-v1.canonical.json` is generated output, created with `ORANGE_BLESS=1` and then reviewed. The independent check is value equality with the hand-derived source fixture.
- **Open questions.** There is no numeric bound on extension maps or input size, and `max_parallel` presence is not checked (validator work).
