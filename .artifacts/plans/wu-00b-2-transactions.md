# Plan: WU-00/00b.2 — transactions, change sets and undo

Status: **implemented locally**, 2026-10-20, branch `feat/wu-00b-document-model` (local, on top of [00b.1](wu-00b-1-document-model.md)). Contract: [12 §3.3](../../docs/12-graph-data-structures.md). Acceptance from [07](../../docs/07-delivery-plan.md): G-30, G-31, G-36 ([11 §18](../../docs/11-graph-language.md)); FR-03 for document state.

## 1. Scope

In: `DocCommand` (the 12 §3.3 list), atomic `GraphDocument::apply`, `ChangeSet`, an inverse for undo plus redo through the same call, and one revision increment per committed transaction. Transaction-owned document state becomes private.

Out: the undo/redo history stack (02 puts command history in `orange-editor`; the model supplies inverses), derived indexes and `affected_roots` (00b.3 and compiler), and every semantic rule: fan-in, port kinds, types, regions, and references from config to variables. Those belong to the validator (00d). The model enforces only what its storage cannot represent: unique IDs, an edge's endpoints in one body, and existing targets.

## 2. Design

Two layers, each with one job:

| Layer | Module | Job |
|---|---|---|
| `DocCommand` | `transaction.rs` | User intent. Checks structural preconditions against the current state and lowers itself to patches. Later commands in a transaction see earlier ones. |
| `Patch` (crate-private) | `patch.rs` | Primitive reversible edit. Each one sets a single slot: a node, edge, variable, event or function entry (with its position), or an editor path. Applying it returns its exact inverse. |
| `ChangeSet` | `change_set.rs` | Net per-ID effect (added, removed or changed) of the applied patches, plus `layout_only`. |

- **DRY:** one `set_slot` function serves every ordered collection (`IndexMap` and the editor JSON map) through a two-method `Slots` trait. Position is restored on undo, so undo is byte-exact even though removal compacts order (12 §3.1).
- **Atomic:** if any command fails, the patches applied so far are reverted in reverse order. The document is untouched and the revision unchanged.
- **Undo:** `apply` returns `Committed { change_set, inverse }`. `revert(inverse)` applies those patches as a new transaction and returns the redo inverse. An inverse records the revision it applies to; a stale one is rejected, never applied to a different state.
- **Revision:** +1 per committed transaction, undo/redo and layout-only included. `layout_only` exists so the compiler and preview can skip work (G-36).
- **Positions:** replacing an item keeps its position; a new item is appended.
- **Encapsulation:** `revision`, the root body, variables, events, functions and editor become private, with read accessors. Package/type locks, signatures, exports and extensions have no 12 §3.3 command, so they stay public until their flows exist (package install, rebind, 00b.3 signature projection).

Command rules beyond 12 §3.3 wording:

- `AddNode` names its target body (`BodyRef::Root` or `BodyRef::Function(id)`). The 12 struct omits it, but nodes are stored per body.
- `RemoveNodes` removes incident edges in the node's body. `RemoveVariadic` removes the instance port's edges and literal. Both are recorded for undo.
- `InsertVariadic` allocates `k = next`, inserts `k` at `at` in `order` and increments `next` (G-31). If `next` cannot advance, it fails instead of reusing a counter.
- `Connect` keeps any literal on the target port. It stays shadowed and is active again after `Disconnect` (G-30, 11 §4.4). `GraphDocument::active_literal` exposes the effective literal.
- `Layout(LayoutCommand::Set { path, value })` edits the opaque editor map. Typed layout commands arrive with `orange-editor`. Layout entries of removed nodes are left to that editor, which issues `Layout` commands in the same transaction.

## 3. Tests

| File | Checks |
|---|---|
| `tests/tx_literals.rs` | G-30: a literal persists while shadowed; disconnecting makes it active again; undo of each step |
| `tests/tx_variadic.rs` | G-31: inserting in the middle keeps existing `#k` keys and edges; counters are never reused; removing deletes the port's edges and literal; undo |
| `tests/tx_layout.rs` | G-36: a layout-only transaction yields `layout_only` with empty deltas; a semantic edit does not; undo of a created editor path removes it exactly |
| `tests/tx_structure.rs` | Each structural error is rejected with no state or revision change; atomic rollback after a partial transaction; stale inverse rejected; `RemoveNodes` removes incident edges and undo restores order |
| `tests/tx_props.rs` | Random command sequences: undoing all restores the original bytes; redo reproduces the edited bytes |

Done when fmt, clippy (`-D warnings`), tests and both repo checks pass locally. 07 records evidence and 12 §3.3 notes the `AddNode` body and the patch-based inverse (a Rust spelling change; the JSON format is unchanged).

## 4. Implementation notes and deviations

- **Inverse binding.** The first version stamped each inverse with the revision it applied to. The undo-all/redo-all property test showed that this breaks the second redo, because undo and redo advance the revision. Inverses bind to `GraphDocument::state_id`. The subsequent review found that revision-derived state IDs collide across divergent clones. IDs are now process-unique transient tokens: a clone preserves its current token, a new edit allocates a fresh one, undo/redo restore the prior token, and a load gets a fresh one. Tokens are excluded from persistence and document equality. `tx_structure.rs` covers redo; `tests/review_undo_invariants.rs` covers clone/reload isolation and function-body key integrity. Evidence is in [10](../../docs/10-tooling-and-sources.md#model-review-fixes-and-slice-4-analysis-2026-10-20).
- **Boxing.** `Patch` holds nodes and functions boxed so undo history stays compact. `DocCommand` keeps values inline (`clippy::large_enum_variant` allowed with a reason) because commands are short-lived.
- **Shared test strategies.** Proptest strategies moved to `tests/common/strategies.rs`. The save/load property test now builds its documents through transactions, so its edges always reference existing nodes. Orphan edges remain covered by `codec_unknown.rs`.
- **Function bodies.** A function body's node and edge IDs may not be used by another body, and its edges must stay inside it. Removing or replacing a function reports the function in `decls` rather than each body node.
