# orange-model: code reading guide

This crate implements the document/model slices of WU-00/00b. It has no Bevy dependency. Graph validation, type inference, compilation and scheduling belong to later slices; successful JSON loading does not mean a graph can execute.

## Follow a document edit

1. `from_json` in `document/codec.rs` checks the format/version and structural IDs, then loads `GraphDocument` from `document/mod.rs`. Unknown fields and orphan edges survive loading.
2. A caller creates `DocCommand` values and calls `GraphDocument::apply` in `edit/transaction.rs`. The private `Journal` applies commands, records inverse `Patch` values and rolls back on failure.
3. A successful edit returns `Committed`: one `ChangeSet` for consumers and an `Inverse` for history. Revision advances once. `GraphDocument::revert` applies an inverse and returns the inverse needed for redo. Inverses bind to content state, so intervening undo/redo is distinct from applying an inverse to unrelated content.
4. The caller updates its separate `DocIndex` with the new document and that change set. The index provides node/body and edge lookups; it is neither persisted nor part of undo. Full type/region validation is not performed by this index.
5. `to_json` writes deterministic document JSON. It does not save runtime state or execute providers.

## Other paths

| Module | Responsibility and boundary |
|---|---|
| `id`, `type_term` | Typed identity wrappers and structural type-expression syntax. Type existence, constraints and inference are future 00c work |
| `signature` | Projects definition ports plus `type_args` into semantic snapshots; hashes canonical content and compares drift. Presentation is excluded |
| `paste` | Copies a selected fragment, resolves declaration dependencies and builds remapped edit commands; the caller commits them as one transaction. Cross-document package-lock merging remains a package-flow responsibility |
| `document/literal`, `value/literal_codec` | Persisted tagged literals and their typed conversion to/from runtime `Value`; package schemas enter through the schema registry |
| `value` | Compact runtime representation, immutable aggregate storage and structured errors. Rust equality/hash here mean storage identity; they do not implement the graph language's `eq`/`hash` constraints |
| `error` | Errors owned by the codec, model, edit and literal boundaries |

`lib.rs` is the public entrypoint. Storage owned by transactions is private; callers should use commands rather than mutate those fields. Locks and exports are still public because their command flows are not implemented. Avoid adding validator or UI behavior to the codec to close those future gaps.

## Tests and contracts

The authority is [graph semantics](../../../docs/11-graph-language.md) and [data layout](../../../docs/12-graph-data-structures.md). Tests are grouped by capability: `codec_*`/`roundtrip_props`, `tx_*`, `doc_index`, `paste`, `signature_vectors` and `literal_*`. Property tests cover rollback/history, ordering, remapping, index rebuilding and codec round trips.

`conformance_fixtures.rs` checks the [G-suite data](../../../conformance/README.md), including signature and script consistency. It does not execute the expected graph behavior. See the [review](../../../conformance/REVIEW.md) for remaining conformance gaps and evidence. Run `cargo test --workspace --locked` from the repository root; use the pinned toolchain and lockfile.
