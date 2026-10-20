# Plan: WU-00/00b.3 — derived indexes, paste remap, signature projection and key

Status: **implemented locally**, 2026-10-20, branch `feat/wu-00b-document-model` (local, on top of [00b.2](wu-00b-2-transactions.md)). Contracts: [12 §2, §3.1–3.2](../../docs/12-graph-data-structures.md), [11 §4.1, §4.5, §5.1](../../docs/11-graph-language.md), FR-02. Acceptance from [07](../../docs/07-delivery-plan.md): a paste remap property test, plus G-29 and G-39 semantic snapshot vectors.

## 1. Scope

In:

- `DocIndex`: node location, edge endpoints, incoming/outgoing edges per port and per-node incidence (12 §3.2 rows that need no language rules). Built from a document, updated from a `ChangeSet`.
- `signature` module: semantic projection of resolved ports (presentation removed, `$T` substituted from `type_args`), canonical JSON, the `sha256:` `SignatureKey`, implicit kind ports (11 §4.1), drift comparison, and orphan edges for G-29.
- `PutSignature` command. Signatures become transaction-owned (private) so paste can add the snapshots its nodes need (12 I-4).
- `Fragment` copy/paste with full ID remapping (FR-02).

Out: dense `DocIx`, topological order, type components, region index and diagnostics. Those need port-kind and type rules or have no consumer yet (00c/00d), so they are deferred rather than built speculatively. Also out: the `E-SIGNATURE-DRIFT` diagnostic itself (00d emits it from the comparison here), package-lock merging for cross-document paste of package nodes (packages flow, ADR-07), layout of pasted nodes (`orange-editor`), and declarative port templates (host evaluation).

## 2. Design

| Module | Responsibility |
|---|---|
| `index.rs` | `DocIndex::build(&doc)`, `update(&doc, &ChangeSet)`, and queries: location, incoming, outgoing, incident, orphan edges |
| `signature.rs` | `project`, `substitute_params`, `canonical_json`, `semantic_key`, `SignatureSnapshot::declares_port`, `compare` → `SignatureDrift` |
| `paste.rs` | `Fragment::copy(&doc, nodes)`, `Fragment::paste(&target, body, new_id)` → `Paste { commands, nodes }` |

- **Index separate from the document.** It is derived, so equality, serialization and undo of the document never involve it. Editor and compiler keep one beside the document. Edge sets are `BTreeSet` so an incrementally updated index equals a fresh build. A replaced or removed function body is re-scanned, because change sets report body changes through the function's `DeclId`.
- **Canonical JSON** for hashing: object keys sorted by byte order (the same as RFC 8785 for the ASCII keys used here), no whitespace, serde_json string escaping. Snapshot numbers are integers. Non-integer numbers inside extensions use serde_json's shortest form, which can differ from RFC 8785. That is acceptable until WU-03 publishes cross-language digest vectors (07 03a).
- **Projection:** drops `label`, `description`, `group` and `editor` (12 §3.1, 11 §4.1) from ports and the snapshot, and replaces `$ident` tokens in type expressions with `type_args` values. Grammar 11 §5.1 allows `$` only before a parameter name, so token substitution is exact without the 00c parser. Unresolved parameters stay visible for the validator.
- **Paste remap rule:** every pasted node, edge and declaration gets a fresh ID. A config string equal to a remapped source ID is rewritten, at any depth. Matching on exact random 122-bit UUIDs needs no per-definition knowledge of which config keys are references, and accidental matches are negligible. Literals are user data and are never rewritten.
  - Declarations referenced by copied nodes are captured, transitively through function bodies. On paste, a declaration whose ID the target already has is reused, so pasting a Get Variable into the same document reads the same variable. Otherwise it is pasted under a new ID.
  - Signatures are re-keyed to their `sha256:` key on paste, so keys from different documents never collide with different content.
  - New IDs come from a caller-supplied generator, keeping the model free of OS randomness and making tests deterministic.

## 3. Tests

| File | Checks |
|---|---|
| `src/signature.rs` unit tests | Parameter substitution (nested, unknown left as is); canonical JSON key order; known sha256 vector for a fixed snapshot |
| `tests/signature_vectors.rs` | G-39: English/Spanish/RTL pseudo labels give identical projection and key, and document bytes and revision are unchanged; missing presentation gives the same key. G-29: a missing package keeps the snapshot and edges; drift against a resolved snapshot lists removed ports and orphan edges, with no data loss. Implicit ports per kind |
| `tests/index.rs` | Queries on the 12 §6 fixture; property: after random transactions, the incremental index equals a full rebuild, undo included |
| `tests/paste.rs` | Internal edges kept and external edges dropped; selection across bodies rejected; same-document paste reuses declarations; paste into an empty document copies them under new IDs with references rewritten; function bodies remapped; property over random documents and selections: no aliasing, edges preserved, apply succeeds, undo restores |

Done when fmt, clippy (`-D warnings`), tests and both repo checks pass. Same-change documentation: 07 evidence, 10 for the `sha2` dependency, and 12 §3.2/§3.3 notes (`PutSignature`, separate `DocIndex`, deferred indexes).

## 4. Implementation notes

- Implemented as planned; all tests passed on the first run. The sha256 vector in `tests/fixtures/s-add-i64.sha256` was computed outside Rust (`printf '%s' '<canonical json>' | sha256sum`), so the key test does not compare the code against itself.
- Random edits for property tests moved to `tests/common/edits.rs`, which now also creates, replaces and removes a function. The index property therefore covers function-body rescans, and the undo property covers function edits.
- When a copied function body has orphan edges, paste drops them, because `UpsertFunction` requires a consistent body. Orphans in the source document stay untouched.
- Coverage limits: the paste property uses random documents without declarations or signatures; those paths are covered by the unit tests. Cross-document paste of package nodes does not merge package locks yet (ADR-07 flow).
