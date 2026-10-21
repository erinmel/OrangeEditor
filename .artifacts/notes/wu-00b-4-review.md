# WU-00b.4 implementation review

Reviewed 2026-10-20 against implementation commit `8c1f42f` and the subsequent local review fixes. This note is non-normative; [12](../../docs/12-graph-data-structures.md) owns layouts and wire formats, [11](../../docs/11-graph-language.md) owns graph semantics, and [10](../../docs/10-tooling-and-sources.md#model-review-fixes-and-slice-4-analysis-2026-10-20) records executed evidence.

## Assessment

Slice 4 supplies the payload layer for typed node ports. It is implemented, and its core representation is suitable for continuing with 00c/00d. It does not supply the node definition registry, generic inference, port connection validation, graph compiler or interpreter. Those remain the work units in [07](../../docs/07-delivery-plan.md).

| Area | Implemented behavior | Review assessment |
|---|---|---|
| Values | Inline scalars; shared aggregate storage; compile-time `Value` size assertion | The 16-byte bound compiles on the tested Windows x64 target. `Arc<String>`/`Arc<Vec<u8>>` deliberately use thin pointers. Extra allocations/indirection are a profiling question, not evidence of obsolete Rust. |
| Types | Canonical structural parser, bounded nesting, nominal names and generic arguments | A syntax tree is available. Registry lookup, constructor/constraint checking, interning and unification remain 00c; parsing alone does not establish that a type is valid in a graph. |
| Scalars | Integer ranges and canonical forms, exact float bits, strings/bytes, duration/timestamp and credential references | Appropriate wire separation. The review found and fixed a float-width mismatch in encoding. Secret references remain identifiers; no secret material or vault access belongs here. |
| Lists/maps | Typed scalar arrays, nested lists and insertion-ordered maps | Typed storage is appropriate. Encoder map validation was incomplete and is now symmetric with the decoder's existing duplicate/float-key policy. |
| Options/records/enums | Explicit absence, schema-ordered record fields, stable wire field/variant IDs | Missing fields, present None and JSON null remain distinct in the tests. Enum tags are schema-local indices, so callers must retain the resolved static type/schema. Registry validation is still required. |
| JSON | Validates syntax and preserves source text, including number spelling and null | Useful for lossless storage. It is not the future structured JSON-node representation, and its internal nesting is outside the current typed-container budget. |
| Errors/resources | Rejects literal forms for errors/results/handles/streams | These are intentionally incomplete runtime contracts; full structured errors and resource ownership are later work. |

The Rust design uses enums, fallible APIs, a small schema-provider trait and explicit shared ownership without unsafe code or a Bevy dependency. Structural equality compares floats by bits; it must not be reused as the language's `eq`/`hash` implementation. Static type context is essential because option payloads and credential references share representations with other values.

## Confirmed defects fixed during this review

1. **Float width (P2).** `encode(core/f32@1, Value::F64(1.0))` previously returned an f32 literal carrying 16 hex digits. Decoding it failed. The encoder now accepts only the matching f32/f64 runtime variant. Regressions include both wrong-width directions, negative zero and a NaN payload.
2. **Map encode/decode asymmetry (P2).** Public `MapRepr` could contain duplicate keys, and encoding accepted floating-point key types. Encoding succeeded while decoding rejected the result. Encoding now reports the existing `DuplicateMapKey`/`FloatMapKey` errors, including an empty map with a float key type. Valid insertion order survives encode/decode.

The permanent regressions are in [review_literal_encoding.rs](../../app/crates/orange-model/tests/review_literal_encoding.rs). These fixes implement existing FR-08 and [11 §5.2](../../docs/11-graph-language.md) contracts; they add no conversion, wire format or graph-language rule.

## Remaining limitations and next checks

- **JSON nesting/size policy remains open.** A probe using 84 nested JSON arrays succeeded through both `JsonText::new` and literal decoding despite `MAX_NESTING = 64`. That limit currently protects type expressions and typed aggregate traversal, not JSON text contents. Add explicit JSON depth/size limits and boundary tests before treating this codec as a bounded ingress for untrusted inputs. This is recorded as a follow-up, not a newly passed or failed release gate.
- **Map duplicate checking is quadratic.** Both directions scan prior keys. The fix restores correctness; it does not establish large-map scalability. Choose entry/byte limits and a schema-aware indexing strategy, then measure it. General `hash`/`eq` eligibility belongs to 00c; it is not implemented by rejecting direct float keys alone.
- **Property coverage is narrower than the aggregate API.** The existing generator covers core scalars, lists, string-keyed maps and options, with shallow recursion. Records/enums/JSON have example-based tests; randomized schemas, malformed encode inputs and depth/size boundaries need broader coverage when registry contracts settle.
- **Cold codec work is not the execution hot path.** Type parsing allocates strings; scalar-list decoding stages a `Vec<Value>` before packing into typed storage. The current slice has no allocation or latency benchmark. Keep type resolution outside runtime kernels and measure peak decode memory separately from warm graph execution.

The next semantic work remains 00c (resolved types/constraints/converters), then 00d (connection and graph validation). This review does not turn the storage model into a fully validated node editor.

## Validation and limits

Parent integration ran on Windows x64 with Rust/cargo 1.99.1, edition 2024, the unchanged tracked lockfile and unchanged dependency features. `cargo test --workspace --locked --offline` passed 100 tests; `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` passed. The extra tests include fixes to the earlier document/transaction/signature review as well as slice 4.

The initial temporary probe ran five checks: four assertions reproduced the float/map failures, and one observation confirmed the JSON nesting gap. The probe was removed from the permanent integration tests after preserving its source under ignored local review scratch space. No deliberately failing test remains in the normal suite. Windows reported incremental-cache hard-link failures with a copy fallback. There is no Bevy build, MSRV 1.97.1 build, cross-OS execution, benchmark or remote-CI claim here. G0/00a and the existing acceptance gates remain open.
