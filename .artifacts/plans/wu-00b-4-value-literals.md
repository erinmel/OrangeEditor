# Plan: WU-00/00b.4 — `Value` and the typed literal codec

Status: **implemented locally**, 2026-10-20, branch `feat/wu-00b-document-model` (local, on top of [00b.3](wu-00b-3-index-paste-signatures.md)). Contracts: [12 §6.1, §7](../../docs/12-graph-data-structures.md), [11 §5.1–5.3](../../docs/11-graph-language.md), [03 value model](../../docs/03-data-and-execution.md). Acceptance from [07](../../docs/07-delivery-plan.md): a `Value` ≤16 bytes assertion and scalar/aggregate literal round trips (FR-08). With this slice, 00b is complete.

## 1. Scope

In:

- `TypeTerm`: a structural parse of canonical type expressions (11 §5.1 grammar). 00c builds the interner, constraints and unification on it.
- `Value` per 12 §7, with a compile-time size assertion.
- `decode(&Literal, &impl Schemas) -> Value` and `encode(&TypeExpr, &Value, &impl Schemas) -> Literal` for every type that has a literal form. Record and enum layouts come through a small `Schemas` trait, which the registry will implement later; tests use an in-memory schema set.
- Base64 (standard alphabet, padding, strict) implemented locally. It is about 50 lines, so no dependency.

Out: arithmetic, `eq`/`hash` constraint semantics, conversions, the binary codec (04a), the structured JSON tree used by JSON nodes (05d), full `NodeError` fields (12 §6.2, WU-03 contracts), and TypeKey interning (00c).

## 2. Design

| Module | Responsibility |
|---|---|
| `type_term.rs` | `TypeTerm::parse(&TypeExpr)`: named `namespace/name@major` with arguments, or `$param`. Canonical only: re-display must equal the input |
| `value.rs` | `Value`, `ListRepr`, `MapRepr`, `RecordRepr`, `JsonText`, `HandleId`, `NodeError`, size assertion, structural equality (floats by bit pattern) |
| `literal_codec.rs` | Typed decode/encode, the `Schemas` trait, a depth budget, and `LiteralError` cases for codec failures |
| `base64.rs` | Strict standard base64 |

Decisions:

- **16 bytes:** `Str`/`Bytes` hold `Arc<String>`/`Arc<Vec<u8>>`, thin pointers, because a fat `Arc<str>` would make `Value` 24 bytes. The extra indirection is accepted until profiling. A thin-string crate is a candidate for 04a.
- **Typed lists:** `list<bool|i32|i64|u32|u64|f32|f64|string>` decode to typed `ListRepr` arrays (12 §7). Other element types use `Values`. `Bool` uses `Box<[bool]>` rather than a bit vector, avoiding a dependency.
- **`json`:** the literal is JSON text, which must parse and is kept verbatim. That preserves exact number text and JSON `null` (03) with no `arbitrary_precision` feature, which breaks serde `flatten`.
- **Canonical scalars:** decimal strings and `bits` must already be in canonical form (no `+`, no leading zeros, lowercase hex), so load→save stays byte-stable.
- **Distinctness (FR-08):** `option` is `{"none":true}` or `{"some":…}` and never JSON `null`. A missing record field exists only as an absent key (`None` in the record's field slot). JSON `null` exists only inside `json`. Errors have no literal form.
- **Unknown fields and variants:** they survive storage because the `Literal` stays raw JSON in the document (03). Decoding one to a `Value` fails with a specific error (run-time fallback is a language rule).
- **No literal form:** result, error, handle and stream are rejected with `NoLiteralForm`, which the validator maps to `E-HANDLE-LITERAL`. Wrong shapes are `TypeMismatch`, which maps to `E-LITERAL-TYPE`.
- **Depth budget:** 64 nested levels (03 requires codec depth budgets; the number is proposed and recorded as an open budget).
- **Enum tag:** the variant's index in schema order. Persisted literals always use the stable `variant_id`.

## 3. Tests

| File | Checks |
|---|---|
| `src/value.rs`, `src/type_term.rs`, `src/base64.rs` unit tests | Size ≤16; parse/display of nested terms and parameters, with non-canonical forms rejected; RFC 4648 base64 vectors plus strict-padding rejections |
| `tests/literal_scalars.rs` | Every core scalar round trip, including boundary values, −0.0, NaN bit patterns and empty strings/bytes; canonical-form rejections; range errors; `secret-ref` prefix |
| `tests/literal_aggregates.rs` | list (typed and nested), map (order, duplicate key, float key rejected), option (none / some / nested), record (optional absent vs present None, unknown and missing field), enum (payload / none, unknown variant), json (null, exact number text); FR-08 distinctness: None, missing, JSON null and no-error-literal |
| `tests/literal_props.rs` | Property: for generated `(type, value)` pairs, `decode(encode(v)) == v` and encode is stable |
| Existing golden fixture | Every literal in the 12 §6 example decodes |

Done when fmt, clippy (`-D warnings`), tests and both repo checks pass. Same-change documentation: 07 evidence (00b complete locally), 12 §6.1/§7 implementation notes, 10 evidence.

## 4. Implementation notes

- Implemented as planned; all tests passed on the first run apart from a lib.rs tidy-up.
- `TypeTerm::parse` enforces the nesting budget, so a too-deep literal type fails as `InvalidType` before any recursive decoding. The codec's own depth check backs it up.
- `decode`/`encode` are reached through `orange_model::literal_codec`, not re-exported at the crate root, because the bare names would be ambiguous next to `from_json`/`to_json`.
- 00b status: all four slices are implemented locally on this branch. The 00b acceptance in 07 stays open until G0/00a pass and remote CI runs on a PR.
- Post-implementation review: corrected top-level float encoding to require the exact f32/f64 value variant, and made map encoding reject duplicate and floating-point keys consistently with decoding. Added `tests/review_literal_encoding.rs`; the combined model suite passes 100 tests. [The review](../notes/wu-00b-4-review.md) records that JSON text nesting/size bounds, quadratic map checks and broader schema/property coverage remain open; [10](../../docs/10-tooling-and-sources.md#model-review-fixes-and-slice-4-analysis-2026-10-20) owns the executed evidence. No performance or Bevy gate passed.
