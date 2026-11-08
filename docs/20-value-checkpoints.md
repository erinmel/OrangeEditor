# Persistent-value checkpoint contract

C13, format version1 (2026-11-08). This contract persists explicitly selected
values and bootstraps fresh core actors. Durable files/bundle generations belong
to U05; checkpoint capture is independent of editor undo and C05 event replay.

## Capture and restoration

The host explicitly selects variable IDs whose current declarations are persistent
and have the requested run/workspace scope. Local variables are ineligible.
`Run::checkpoint` requires a clean Stopped, undisposed, uninterrupted actor and
its current pinned document. `Workspace::checkpoint` reads an immutable publication
at its exclusive host boundary. Capture never runs a graph or provider callback.
There is no automatic checkpoint-on-stop policy in this increment.

`ValueCheckpoint::from_bytes` performs bounded structural admission. Restore
rechecks current declaration/type/schema/codec/provider identities and values;
parsing alone grants no execution authority. `Run::start_from_checkpoint` and
its workspace-aware form require a fresh checked plan. `Workspace::from_checkpoint`
creates a fresh host actor. Failure never mutates an existing actor or the borrowed
source bytes. A restored workspace has a new process identity/version0; old runs,
requests and snapshots remain bound to their original workspace. There is no hot
schema migration, in-place workspace replacement or historical format reader.

Selected values replace initializers before state quotas, frame construction and
initial Watch evaluation. Other declarations use ordinary initializers. Run IDs,
clock, seed/counters, node state, resource/stream tables, effect commands, execution
stacks, event queues and bus leases are fresh. A restored Run starts its normal
On Start workflow only when explicitly pumped. Capture, parsing and startup never
replay old effects; a later explicit pump can execute fresh startup actions.

The supported profile uses the existing structural literal payload codec for
bool/numeric scalars (including exact float bits), string/bytes, duration/timestamp,
JSON text, lists/maps/options and registered nonrecursive record/enum schemas.
Uninitialized cells, Option None, nested Some(None), missing record fields and
JSON null remain distinct. JSON text stays verbatim, including number spelling,
whitespace and duplicate keys inside that opaque text. Result/error/resource/
stream/unknown or unsupported recursive shapes are refused recursively. Host
selection is the sensitivity boundary: this is not a credential store, and it
does not classify arbitrary strings as secrets. The host owns storage/integrity
policy and approves which eligible values may leave memory.

## Wire document and limits

The closed JSON envelope follows [checkpoint-v1.schema.json](schemas/checkpoint-v1.schema.json).
Every stored value has a required nullable `value`. Null means uninitialized;
an initialized literal has `{"encoding":"value","data":...}` or the float
form `{"encoding":"bits","data":"..."}` with8/16 lowercase hex digits.
The `data` is the existing literal codec payload, not another graph document.
Core JSON text is consequently a string payload, not a parsed object. For example,
one persistent workspace u64 with the identity vector described below is:

```json
{"format":"orange.checkpoint","format_version":1,"scope":"workspace","graph_id":"20000000-0000-4000-8000-000000000001","compatibility":"666040c65d852ec6410c6cbbc1822a677b83f5fec305e3a7476da5ef48f3912e","values":[{"id":"10000000-0000-4000-8000-000000000001","type":"core/u64@1","value":{"encoding":"value","data":"1"}}]}
```

Unknown envelope/value fields, duplicate object keys or variable IDs, malformed
types/bit encodings, unsupported format/version and missing fields are refused.
Before allocating literal trees, a structural pass enforces raw byte and variable
count bounds and the JSON reader's nesting bound. Capture publishes only artifacts
accepted by that same reader. Encoding grows a capped buffer; it never requests
growth beyond its configured byte ceiling. Typed decoding uses the lesser of the
per-value limit and remaining aggregate state allowance before allocating values.

Default CheckpointLimits:4096 selected variables,32MiB encoded input/output and
identity framing,16MiB per decoded value,32MiB combined nominal values and65536
identity items (selected declarations, locks and potential graph nodes). The
native model's type/JSON/codec bounds still apply. The artifact object retains
the caller's policy, not an artifact-supplied policy; limits are not wire fields.
Run/Workspace startup additionally enforces its ordinary state/resource budgets.
Artifact trees, temporary staging and caller-made copies are host-owned storage;
these are nominal limits, not allocator/RSS or physical latency qualification.

## Compatibility identity

`compatibility` is lowercase SHA-256 over the following versioned length framing.
It is a compatibility check, not a signature or execution grant. Each field is
u64 little-endian byte length followed by its bytes. Text is UTF-8; a number is
itself one field containing its u64 little-endian representation. Canonical IDs
use their model string representation. Order is:

1. Text `orange.checkpoint.contract@1`, text `orange.value.v1`, graph ID, scope.
2. Selected count; declarations sorted by variable ID: ID, exact type expression,
   then resolved shape. Every selected declaration must be persistent and match
   scope; those are admission predicates rather than additional framed fields.
3. Package-lock count and entries sorted by(id,version,content_digest), each as
   three text fields. Type-lock count and entries sorted by(type,codec), each as
   two text fields; every explicit codec must be the registry's actual codec.
4. Count of distinct non-core provider tuples from root and function bodies,
   including potential unused nodes, sorted by(definition ID,definition version,
   config version,package,package version,digest,provider ID). Fields follow that
   order, using numbers for definition/config versions and text for the rest.
   Provider lookup must confirm the exact locked implementation/config version.

Shapes start with a text tag: `bool`, `i32`, `i64`, `u32`, `u64`, `f32`, `f64`,
`json`, `string`, `duration`, `timestamp`, `bytes`, `list`, `map`, `option`,
`record` or `enum`. List/Option append one child; Map appends key then value.
Record appends nominal type, field count, a field containing the two native
structural Eq/key derive bytes(0/1), then each schema-ordered field ID, one optional
flag byte and child shape. Enum similarly appends nominal type, variant count,
the two derive bytes, then each schema-ordered variant ID, numeric payload-present
flag and optional child shape. Every flag byte array is length-framed too.

Names, editor layout, revision, declaration initializers and process-local identity
are excluded. Codec/type/field/variant order and exact declared package/provider
locks remain semantic facts. Changing the checked program while retaining the
same selected value contract is allowed; this restores values, not an instruction
position. Live executable contexts remain the host's pinned-provider responsibility.

The example vector uses the displayed graph/variable IDs, one selected workspace
`core/u64@1`, package lock `org.orange.math` version `0.0.0` with digest
`sha256:` followed by64 `a` characters, no explicit type locks and no non-core
nodes. An independent .NET SHA256 framing calculation agrees with the Rust test.

Executed behavior/environment and remaining qualification gates are recorded in
[10](10-tooling-and-sources.md). No disk crash-recovery or full external embedding
qualification is implied by the public API integration regression.
