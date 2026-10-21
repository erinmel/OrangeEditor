# Graph data structures

Status: **normative proposal for graph format v1**, 2026-10-20. Rust items below are conceptual layouts, **not compiled code**. Field names and JSON shapes are contracts; Rust spelling, generic parameters and crate placement can change in WU-00 if the JSON/wire contract and the invariants in §11 are preserved. Semantics are defined in [11](11-graph-language.md); value rules in [03](03-data-and-execution.md). JSON examples are syntax-checked documentation, not installable artifacts; IDs are shortened (`n1`, `v1`) where real documents use UUIDs and digests are synthetic.

## 1. Four representations

"Know the nature of the data": each representation has one owner, one mutability rule and one identity scheme. Nothing is shared mutably across them.

| Representation | Owner / crate | Mutability | Identity | Lifetime | Thread |
|---|---|---|---|---|---|
| `GraphDocument` | Editor/CLI via `orange-model` | Only through transactions (§3.3) | UUIDs, stable across save/load; pasted contents receive remapped IDs | Persisted | Any; edits serialized per document |
| `RegistrySnapshot` | `orange-packages` | Immutable after publication | Namespaced IDs + exact versions; interned `TypeKey` inside | One registry generation | Shared `Arc` |
| `ExecutionPlan` | `orange-compiler` | Immutable | Dense `u32` indices + source map back to UUIDs | Until the last run using it is disposed | Shared `Arc` |
| Run state | `orange-runtime` run actor | Mutated only by its actor | Run-local `u32`/`u64` indices with generations | One run | Actor task; never locked |

## 2. Identifiers

| ID | Form | Persisted | Notes |
|---|---|---|---|
| `GraphId`, `NodeId`, `EdgeId`, `VariableId`, `EventDeclId`, `FunctionId` | UUID (v4 or v7) | yes | Paste remaps every one of them ([01](01-product-requirements.md) FR-02) |
| `PortId` | `[a-z][a-z0-9_]{0,63}` | yes | Unique per definition |
| `PortKey` | `port_id` or `port_id#k`, `k: u16` | yes | Variadic counter is never reused within a node |
| `DefinitionId` | `namespace/name` | yes | `core/` reserved for language intrinsics |
| `TypeExpr` | Canonical string ([11 §5.1](11-graph-language.md)) | yes | Parsed and interned at load |
| `SlotId` | `node:…`, `connection:…`, `convert:<from>-><to>`, … | yes | Contract version recorded in the slot descriptor ([04](04-expansion-plugins.md)) |
| `TypeKey`, `KernelIx`, `NodeIx`, `SlotIx`, `BlockIx`, `PullIx`, `RegionIx` | `u32` | no | Plan-local, dense |
| `RunId`, `ActivationId` | `u64` monotonic per host | no | Never reused within a host process |
| `Generation` | `u32` | no | Run restarts and handle reuse |
| `ContinuationId`, `HandleId` | `{index: u32, gen: u32}` (handle also has `kind: TypeKey`) | no | A stale generation is rejected (`core/StaleHandle`) |

## 3. Document model

### 3.1 Structures

```rust
// Conceptual — not compiled.
pub struct GraphDocument {
    pub format_version: u32,                 // 1
    pub graph_id: GraphId,
    pub revision: u64,                       // +1 per committed transaction
    pub package_lock: Vec<PackageLockEntry>, // exact id + version + content digest
    pub type_lock: Vec<TypeLockEntry>,       // canonical named type + codec
    pub variables: IndexMap<VariableId, VariableDecl>,
    pub events: IndexMap<EventDeclId, EventDecl>,
    pub signatures: BTreeMap<SignatureKey, SignatureSnapshot>, // shared by nodes
    pub root: GraphBody,
    pub functions: IndexMap<FunctionId, FunctionDecl>,
    pub exports: Vec<GraphExport>,           // evaluate outputs for preview/CLI
    pub editor: EditorState,                 // layout only; never read by the compiler
    pub extensions: OpaqueExtensionMap,      // unknown fields, bounded, preserved
}
pub struct GraphBody {
    pub nodes: IndexMap<NodeId, NodeInstance>,
    pub edges: IndexMap<EdgeId, Edge>,
}
pub struct NodeInstance {
    pub id: NodeId,
    pub definition: DefinitionRef,           // id + definition_version
    pub provider: Option<ProviderBinding>,   // None only for core/ intrinsics
    pub config_version: u32,
    pub config: ConfigValue,                 // validated by definition config_schema
    pub type_args: BTreeMap<TypeParamName, TypeExpr>,  // persisted inference result
    pub inputs: BTreeMap<PortKey, Literal>,  // includes shadowed literals
    pub variadic: BTreeMap<PortId, VariadicState>,     // order + next counter
    pub signature: Option<SignatureKey>,     // required for every non-intrinsic node
    pub activation: Option<ActivationPolicy>,// entries only
}
pub struct Edge { pub id: EdgeId, pub from: PortRef, pub to: PortRef }
pub struct PortRef { pub node: NodeId, pub port: PortKey }
pub struct VariadicState { pub order: Vec<u16>, pub next: u16 }
pub struct SignatureSnapshot { pub kind: NodeKind, pub ports: Vec<ResolvedPort> }
pub struct VariableDecl {
    pub id: VariableId, pub name: String, pub ty: TypeExpr,
    pub scope: VarScope,                     // Local | Run | Workspace
    pub init: Option<Literal>,               // None → UninitializedVariable on read
    pub persistent: bool,                    // Run/Workspace only; requires `serialize`
    pub conflict: ConflictPolicy,            // Reject (default) | LastWriterWins
}
pub struct EventDecl { pub id: EventDeclId, pub name: String, pub payload: TypeExpr, pub scope: EventScope }
pub struct FunctionDecl {
    pub id: FunctionId, pub name: String, pub kind: NodeKind,
    pub type_params: Vec<TypeParam>,
    pub inputs: Vec<PortDescriptor>, pub outputs: Vec<PortDescriptor>,
    pub body: GraphBody,                     // contains function-entry/return nodes
}
pub struct ActivationPolicy {
    pub mode: ReentrancyMode,                // Queue | Drop | Replace | Parallel{max}
    pub capacity: u32, pub overflow: OverflowPolicy,
    pub on_fault: FaultPolicy, pub order: u32,
}
```

`SignatureKey` is a document-local key. The implementation uses `sha256:` over the canonical JSON of the snapshot, so identical signatures share one entry; examples use short keys. Core intrinsic signatures are defined by `format_version` and config, so they are not stored. Serialization order is `IndexMap` order (creation order, compacted on removal), which makes output deterministic for the same command history.

Implementation correction (00b review): replacing an existing entry at its current position updates that slot in place. Insertions, removals and actual reordering retain positional patches so undo restores the exact order. The replacement path avoids shifting unrelated entries; order-preserving removal and reordering can still be linear. NFR-15 timing targets remain unmeasured.

`ResolvedPort` is the post-template, post-type-substitution semantic port projection from [11 §4](11-graph-language.md): stable port key, direction/kind, resolved type where applicable, source/availability, literal/default/optional, variadic/lazy and sensitivity policy where applicable. Its snapshot excludes presentation `group`, TextRef labels/descriptions and `editor` fields, including non-authoritative `display` text on a default literal. The default's type and encoded value remain semantic. Hash only that semantic snapshot. Optional cached presentation belongs in `editor`, keyed by stable definition/port IDs, outside signature and semantic plan hashes. Without a catalogue or cache, show stable IDs. Locale changes cannot trigger `E-SIGNATURE-DRIFT` (G-39).

### 3.2 Derived indexes (not persisted)

| Index | Structure | Used by | Update cost |
|---|---|---|---|
| Dense document index | `NodeId → DocIx(u32)` slot map | All derived indexes | O(1) |
| Incoming / outgoing | `HashMap<PortRef, SmallVec<[EdgeId; 2]>>` | Fan-in/fan-out checks, deletion | O(1) per edge |
| Node incidence | `HashMap<NodeId, SmallVec<[EdgeId; 8]>>` | Remove node, copy/paste | O(degree) |
| Data topological order | Dynamic topological order over D ([11 §8.2](11-graph-language.md)); candidate: Pearce–Kelly | Incremental `E-CYCLE-DATA` on connect | Bounded by the affected order interval |
| Type components | Union-find over generic nodes joined by data edges | Inference scope | Near O(1); a split recomputes one component |
| Region index | Region root → member exec nodes; dominator tree per root (candidate: Cooper–Harvey–Kennedy iterative) | Availability, Break/Return placement | Recomputed per affected root |
| Diagnostics | `Subject (node / edge / port / decl) → Vec<Diagnostic>` | Editor projection | Replaced per validated subject |

The spatial index, edge-path cache and widget state belong to `orange-editor`, not `orange-model` ([02](02-architecture.md)).

Implementation note (WU-00/00b.3): `orange-model` provides `DocIndex` with node location, incoming/outgoing edges and node incidence.

- **Kept beside the document:** the index lives next to the document, not inside it, so document equality, saving and undo never involve derived data.
- **Updated from change sets:** each committed `ChangeSet` updates it, and an updated index always equals a fresh build (property-tested).
- **`BTreeSet` edge sets:** used instead of `SmallVec` so equality does not depend on edit history. Performance is evaluated with NFR-15.
- **Deferred rows:** the dense document index, data topological order, type components, region index and diagnostics arrive with their consumers (00c/00d).

### 3.3 Transactions and change sets

```rust
pub enum DocCommand {
    AddNode { node: NodeInstance }, RemoveNodes { ids: Vec<NodeId> },   // incident edges removed and recorded
    Connect { edge: Edge }, Disconnect { edge: EdgeId },
    SetLiteral { port: PortRef, literal: Option<Literal> },
    SetConfig { node: NodeId, config_version: u32, config: ConfigValue },
    SetTypeArgs { node: NodeId, args: BTreeMap<TypeParamName, TypeExpr> },
    InsertVariadic { node: NodeId, port: PortId, at: usize }, RemoveVariadic { node: NodeId, port: PortKey },
    UpsertVariable(VariableDecl), RemoveVariable(VariableId),
    UpsertEvent(EventDecl), RemoveEvent(EventDeclId),
    UpsertFunction(FunctionDecl), RemoveFunction(FunctionId),
    Rebind { nodes: Vec<NodeId>, provider: ProviderBinding },          // explicit only (ADR-07)
    Layout(LayoutCommand),                                             // move, waypoints, frames, comments, viewport
}
pub struct ChangeSet {
    pub from_revision: u64, pub to_revision: u64,
    pub layout_only: bool,                    // true → no validation, compile or preview work
    pub nodes: Delta<NodeId>, pub edges: Delta<EdgeId>, pub decls: Delta<DeclId>,
    pub affected_roots: SmallVec<[RegionRootRef; 4]>,  // compile scope
}
```

A transaction applies a list of commands atomically, produces one `ChangeSet` and one inverse transaction for undo, and increments `revision` once. Converter auto-insert ([11 §6](11-graph-language.md)) is one transaction: AddNode + Disconnect + two Connects. Runtime effects are never recorded in history ([01](01-product-requirements.md) FR-03).

Implementation notes (WU-00/00b.2; Rust spelling only, JSON format unchanged):

- `AddNode` also names its target body (root or a function), because nodes are stored per body.
- The inverse is a list of primitive slot patches rather than `DocCommand`s. Each patch restores an entry together with its position, so undo is byte-exact despite removal compaction.
- Undo and redo are transactions and advance `revision`. Inverses bind to an opaque process-unique content-state ID, not to the revision. A clone preserves the ID while it represents the same transaction-owned state; each new edit allocates a fresh ID, undo/redo restore the earlier ID, and every load allocates a fresh ID. This prevents divergent clones, unrelated documents and reloads from accepting each other's inverses. The transient ID is excluded from serialization and document equality.
- Function insertion checks that every node/edge map key equals the entry's own ID before applying a patch. A mismatch returns a typed transaction error and rolls back the entire transaction. This enforces the existing keyed-map representation without changing graph JSON or graph diagnostics.
- `affected_roots` is derived by the compiler from the node, edge and declaration deltas once regions exist (00d/00e).
- `PutSignature { key, snapshot }` (00b.3) adds a shared snapshot so that pasted nodes keep the snapshots I-4 requires. An identical snapshot under the same key is a no-op; different content under an existing key is rejected. Signatures are therefore transaction-owned. Package locks and exports still have no command.
- Canonical JSON for `SignatureKey` (00b.3): object keys sorted by byte order, no whitespace, serde_json string escaping. This matches RFC 8785 for the ASCII keys and integers in snapshots. WU-03 publishes the cross-language digest vectors. Semantic projection removes `label`, `description`, `group` and `editor` and substitutes `$T` parameters from `type_args`.
- Paste (00b.3) gives every pasted node, edge and declaration a new ID. A config string exactly equal to a remapped source ID is rewritten. Referenced declarations travel with the copy and are reused when the target already has them. Signatures are re-keyed to their `sha256:` key.

## 4. Definition schema

`NodeDefinition` fields: `definition_id`, `definition_version`, `config_version`, `kind`, `suspends` (`never`/`maybe`/`always`; async and iterator), `cleanup_safe`, `volatility` (volatile only), `type_params`, `ports`, `iterator` (iterator only), `event_source` (entry only), `port_template` (optional declarative dynamic ports), `config_schema`, `errors`, `capabilities`, `determinism` (`deterministic`/`seeded`/`external`/`nondeterministic`), `cost` (optional hint: `scalar`/`linear`/`heavy`), `implementation_slot`, `label`, optional `description`, `editor`. All human-facing label/help/category/alias fields use TextRef from [13 §2](13-internationalization.md), including port labels/descriptions and declarative configuration-option captions. Implicit kind ports ([11 §4.1](11-graph-language.md)) are never listed; their text comes from host catalogues.

Generic pure definition (replaces the earlier `add-i64` example):

```json
{
  "definition_id": "org.orange.math/add",
  "definition_version": 1,
  "config_version": 1,
  "kind": "pure",
  "label": {"key": "org.orange.math/add-label", "fallback": "Add"},
  "type_params": [{"name": "T", "constraints": ["num"]}],
  "ports": [
    {"port_id": "a", "direction": "input", "kind": "data", "type": "$T", "label": {"key": "org.orange.math/add-port-a", "fallback": "A"}},
    {"port_id": "b", "direction": "input", "kind": "data", "type": "$T", "label": {"key": "org.orange.math/add-port-b", "fallback": "B"}},
    {"port_id": "sum", "direction": "output", "kind": "data", "type": "$T", "source": "computed", "label": {"key": "org.orange.math/add-port-sum", "fallback": "Sum"}}
  ],
  "config_schema": {
    "type": "object",
    "properties": {"int_overflow": {"enum": ["checked", "wrapping", "saturating"], "default": "checked"}, "float_policy": {"enum": ["finite", "ieee"], "default": "finite"}},
    "additionalProperties": false
  },
  "errors": ["core/NumericOverflow", "core/NonFinite"],
  "determinism": "deterministic",
  "cost": "scalar",
  "implementation_slot": "node:org.orange.math/add@1",
  "editor": {
    "category_id": "org.orange.math/category-math",
    "category": {"key": "org.orange.math/category-math", "fallback": "Math"},
    "control": "standard-node",
    "search_aliases": [{"key": "org.orange.math/add-alias-0", "fallback": "plus"}, {"key": "org.orange.math/add-alias-1", "fallback": "+"}, {"key": "org.orange.math/add-alias-2", "fallback": "sum"}]
  }
}
```

Async action:

```json
{
  "definition_id": "org.orange.files/read-text",
  "definition_version": 1,
  "config_version": 1,
  "kind": "async",
  "suspends": "always",
  "cleanup_safe": false,
  "label": {"key": "org.orange.files/read-text-label", "fallback": "Read text file"},
  "ports": [
    {"port_id": "path", "direction": "input", "kind": "data", "type": "org.orange.files/path@1", "label": {"key": "org.orange.files/read-text-port-path", "fallback": "Path"}},
    {
      "port_id": "max_bytes",
      "direction": "input",
      "kind": "data",
      "type": "core/u64@1",
      "default": {"type": "core/u64@1", "value": "8388608"},
      "label": {"key": "org.orange.files/read-text-port-max-bytes", "fallback": "Max bytes"}
    },
    {
      "port_id": "text",
      "direction": "output",
      "kind": "data",
      "type": "core/string@1",
      "source": "execution",
      "available_on": ["then"],
      "label": {"key": "org.orange.files/read-text-port-text", "fallback": "Text"}
    }
  ],
  "config_schema": {"type": "object", "properties": {"encoding": {"enum": ["utf-8", "utf-8-replace"], "default": "utf-8"}}, "additionalProperties": false},
  "errors": ["core/LimitExceeded", "org.orange.files/NotFound", "org.orange.files/PermissionDenied", "org.orange.files/InvalidEncoding"],
  "capabilities": ["file.read"],
  "determinism": "external",
  "implementation_slot": "node:org.orange.files/read-text@1",
  "editor": {"category_id": "org.orange.files/category-files", "category": {"key": "org.orange.files/category-files", "fallback": "Files"}, "control": "standard-node"}
}
```

Iterator (lowered to a structured loop; `in`, `body`, `completed`, `failed`, `error` and `cancelled` are implicit):

```json
{
  "definition_id": "org.orange.files/for-each-line",
  "definition_version": 1,
  "config_version": 1,
  "kind": "iterator",
  "suspends": "maybe",
  "label": {"key": "org.orange.files/for-each-line-label", "fallback": "For each line"},
  "ports": [
    {"port_id": "path", "direction": "input", "kind": "data", "type": "org.orange.files/path@1", "label": {"key": "org.orange.files/for-each-line-port-path", "fallback": "Path"}},
    {"port_id": "line", "direction": "output", "kind": "data", "type": "core/string@1", "source": "iteration", "label": {"key": "org.orange.files/for-each-line-port-line", "fallback": "Line"}},
    {
      "port_id": "line_number",
      "direction": "output",
      "kind": "data",
      "type": "core/u64@1",
      "source": "iteration",
      "label": {"key": "org.orange.files/for-each-line-port-line-number", "fallback": "Line number"}
    }
  ],
  "iterator": {"open": "open", "next": "next", "close": "close"},
  "config_schema": {"type": "object", "properties": {}, "additionalProperties": false},
  "errors": ["org.orange.files/NotFound", "org.orange.files/InvalidEncoding"],
  "capabilities": ["file.read"],
  "determinism": "external",
  "implementation_slot": "node:org.orange.files/for-each-line@1",
  "editor": {"category_id": "org.orange.files/category-files", "category": {"key": "org.orange.files/category-files", "fallback": "Files"}, "control": "standard-node"}
}
```

Package entry (event source acquired in the run's Starting phase):

```json
{
  "definition_id": "org.orange.obs/on-program-scene-changed",
  "definition_version": 1,
  "config_version": 1,
  "kind": "entry",
  "label": {"key": "org.orange.obs/on-program-scene-changed-label", "fallback": "On program scene changed"},
  "event_source": {"service": "connection:org.orange.obs/session@1", "event": "CurrentProgramSceneChanged"},
  "ports": [
    {
      "port_id": "scene_name",
      "direction": "output",
      "kind": "data",
      "type": "core/string@1",
      "source": "payload",
      "label": {"key": "org.orange.obs/on-program-scene-changed-port-scene-name", "fallback": "Scene name"}
    }
  ],
  "config_schema": {"type": "object", "properties": {"profile": {"type": "string"}}, "required": ["profile"], "additionalProperties": false},
  "capabilities": ["connection.use"],
  "determinism": "external",
  "implementation_slot": "node:org.orange.obs/on-program-scene-changed@1",
  "editor": {"category_id": "org.orange.obs/category-obs-events", "category": {"key": "org.orange.obs/category-obs-events", "fallback": "OBS/Events"}, "control": "standard-node"}
}
```

## 5. Type and converter definitions

```json
{
  "format": "orange.type",
  "format_version": 1,
  "type_id": "org.example.media/playback-state",
  "major": 1,
  "minor": 1,
  "kind": "enum",
  "variants": [
    {"variant_id": "a1c0", "name": "stopped", "label": {"key": "org.example.media/playback-state-stopped", "fallback": "Stopped"}},
    {"variant_id": "b7e2", "name": "playing", "payload": "core/duration@1", "label": {"key": "org.example.media/playback-state-playing", "fallback": "Playing"}},
    {"variant_id": "c3d9", "name": "paused", "payload": "core/duration@1", "label": {"key": "org.example.media/playback-state-paused", "fallback": "Paused"}}
  ],
  "constraints": {"eq": "derive", "hash": "derive", "display": "derive", "serialize": "derive"},
  "codec": {"id": "orange.value.v1", "version": 1},
  "editor": {"value_editor": "editor:value-editor/org.example.media/playback-state@1"},
  "sensitivity": "normal",
  "label": {"key": "org.example.media/playback-state-label", "fallback": "Playback state"}
}
```

- `kind`: `record` (`fields`: `field_id`, `name`, `type`, `optional`, `default`), `enum` (`variants`: `variant_id`, `name`, optional `payload`), `scalar` (package codec required), or `handle-kind` (no codec, never persisted).
- Constraint values: `derive` (host derives it structurally; valid only if every field or payload implements the constraint) or an implementation reference into the owning package. Field and variant IDs never change. `name` is source display metadata, not identity; `label: TextRef` supplies translatable captions for types, fields and variants. Names/labels may change without changing identity. Derived graph `display` renders the canonical TypeExpr and field/variant IDs plus recursively invariant payload formatting; it never uses mutable names or translations. User-declared variable/event/function names remain content ([13](13-internationalization.md)).
- Minor version bumps may add optional fields or variants only ([11 §5.3](11-graph-language.md)).

Converter registry entry:

```json
{
  "from": "core/i64@1",
  "to": "core/i32@1",
  "class": "checked-partial",
  "errors": ["core/OutOfRange"],
  "policies": [],
  "slot": "convert:core/i64@1->core/i32@1",
  "provider": {"package": "org.orange.math", "version": "0.1.0", "id": "native-checked"}
}
```

## 6. Saved graph example

Manual trigger adds an amount to run variable `total`. If the new total exceeds 100, it emits `threshold-reached`; a handler logs it. Get node `n2` has three consumption sites in different epochs. The Set (`n4`) reads the old total. The Branch condition (`n5` → `n6`) and the Emit payload (`n7`) read the committed new total ([11 §7.3](11-graph-language.md)). Export `total` is legal because it uses only a volatile cone. An export of `n3.sum` would be `E-CROSS-ACTIVATION`.

```json
{
  "format": "orange.graph",
  "format_version": 1,
  "graph_id": "a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6",
  "revision": 7,
  "package_lock": [
    {"id": "org.orange.math", "version": "0.1.0", "content_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111"},
    {"id": "org.orange.diagnostics", "version": "0.1.0", "content_digest": "sha256:3333333333333333333333333333333333333333333333333333333333333333"}
  ],
  "type_lock": [
    {"type": "core/i64@1", "codec": "orange.value.v1"},
    {"type": "core/bool@1", "codec": "orange.value.v1"}
  ],
  "variables": [
    {"id": "v1", "name": "total", "type": "core/i64@1", "scope": "run", "init": {"type": "core/i64@1", "value": "0"}, "persistent": false, "conflict": "reject"}
  ],
  "events": [
    {"id": "ev1", "name": "threshold-reached", "payload": "core/i64@1", "scope": "run"}
  ],
  "signatures": {
    "s-add-i64": {"kind": "pure", "ports": [
      {"port_id": "a", "direction": "input", "kind": "data", "type": "core/i64@1"},
      {"port_id": "b", "direction": "input", "kind": "data", "type": "core/i64@1"},
      {"port_id": "sum", "direction": "output", "kind": "data", "type": "core/i64@1", "source": "computed"}
    ]},
    "s-gt-i64": {"kind": "pure", "ports": [
      {"port_id": "a", "direction": "input", "kind": "data", "type": "core/i64@1"},
      {"port_id": "b", "direction": "input", "kind": "data", "type": "core/i64@1"},
      {"port_id": "result", "direction": "output", "kind": "data", "type": "core/bool@1", "source": "computed"}
    ]},
    "s-log-i64": {"kind": "action", "ports": [
      {"port_id": "value", "direction": "input", "kind": "data", "type": "core/i64@1"}
    ]}
  },
  "nodes": [
    {"id": "n1", "definition_id": "core/on-manual", "definition_version": 1, "config_version": 1,
     "config": {"payload": "core/i64@1"},
     "activation": {"mode": "queue", "capacity": 256, "overflow": "fault", "on_fault": "continue", "order": 0}},
    {"id": "n2", "definition_id": "core/get-variable", "definition_version": 1, "config_version": 1,
     "config": {"variable": "v1"}},
    {"id": "n3", "definition_id": "org.orange.math/add", "definition_version": 1, "config_version": 1,
     "config": {"int_overflow": "checked"}, "type_args": {"T": "core/i64@1"},
     "provider": {"package": "org.orange.math", "version": "0.1.0", "id": "native-checked"}, "signature": "s-add-i64"},
    {"id": "n4", "definition_id": "core/set-variable", "definition_version": 1, "config_version": 1,
     "config": {"variable": "v1"}},
    {"id": "n5", "definition_id": "org.orange.math/greater", "definition_version": 1, "config_version": 1,
     "config": {}, "type_args": {"T": "core/i64@1"},
     "provider": {"package": "org.orange.math", "version": "0.1.0", "id": "native-checked"}, "signature": "s-gt-i64",
     "inputs": {"b": {"type": "core/i64@1", "value": "100"}}},
    {"id": "n6", "definition_id": "core/branch", "definition_version": 1, "config_version": 1, "config": {}},
    {"id": "n7", "definition_id": "core/emit", "definition_version": 1, "config_version": 1,
     "config": {"event": "ev1"}},
    {"id": "n8", "definition_id": "core/on-event", "definition_version": 1, "config_version": 1,
     "config": {"event": "ev1"},
     "activation": {"mode": "queue", "capacity": 64, "overflow": "drop-oldest", "on_fault": "continue", "order": 1}},
    {"id": "n9", "definition_id": "org.orange.diagnostics/log", "definition_version": 1, "config_version": 1,
     "config": {"level": "info"}, "type_args": {"T": "core/i64@1"},
     "provider": {"package": "org.orange.diagnostics", "version": "0.1.0", "id": "host-log"}, "signature": "s-log-i64"}
  ],
  "edges": [
    {"id": "e1", "from": {"node": "n1", "port": "then"}, "to": {"node": "n4", "port": "in"}},
    {"id": "e2", "from": {"node": "n2", "port": "value"}, "to": {"node": "n3", "port": "a"}},
    {"id": "e3", "from": {"node": "n1", "port": "payload"}, "to": {"node": "n3", "port": "b"}},
    {"id": "e4", "from": {"node": "n3", "port": "sum"}, "to": {"node": "n4", "port": "value"}},
    {"id": "e5", "from": {"node": "n4", "port": "then"}, "to": {"node": "n6", "port": "in"}},
    {"id": "e6", "from": {"node": "n2", "port": "value"}, "to": {"node": "n5", "port": "a"}},
    {"id": "e7", "from": {"node": "n5", "port": "result"}, "to": {"node": "n6", "port": "condition"}},
    {"id": "e8", "from": {"node": "n6", "port": "true"}, "to": {"node": "n7", "port": "in"}},
    {"id": "e9", "from": {"node": "n2", "port": "value"}, "to": {"node": "n7", "port": "payload"}},
    {"id": "e10", "from": {"node": "n8", "port": "then"}, "to": {"node": "n9", "port": "in"}},
    {"id": "e11", "from": {"node": "n8", "port": "payload"}, "to": {"node": "n9", "port": "value"}}
  ],
  "functions": [],
  "exports": [{"id": "total", "node": "n2", "port": "value"}],
  "editor": {
    "nodes": {"n1": {"x": 0, "y": 0}, "n2": {"x": 0, "y": 160}, "n3": {"x": 200, "y": 120}, "n4": {"x": 400, "y": 0},
              "n5": {"x": 400, "y": 160}, "n6": {"x": 600, "y": 0}, "n7": {"x": 800, "y": 0}, "n8": {"x": 0, "y": 360}, "n9": {"x": 200, "y": 360}},
    "edges": {"e5": {"waypoints": [[500, 40]]}},
    "viewport": {"x": 0, "y": 0, "zoom": 1}
  }
}
```

Function bodies serialize as entries in `functions`, each with `id`, `name`, `kind`, `type_params`, `inputs`, `outputs`, `nodes` and `edges`. Node and edge IDs are unique across the whole document, and edges never cross bodies (`E-SCOPE-EDGE`).

### 6.1 Literal encoding

Only the top-level literal carries `type`. Nested values are untagged, and their types come from the outer type.

| Type | Encoding |
|---|---|
| bool | `true` / `false` |
| i32, u32 | JSON integer |
| i64, u64, duration, timestamp | Decimal string (ns for duration/timestamp) |
| f32, f64 | `"bits"` instead of `"value"`: 8 or 16 lowercase hex digits of the IEEE bit pattern; optional non-authoritative `"display"` |
| string | JSON string |
| bytes | Standard base64 with padding |
| list | JSON array |
| map | Array of `[key, value]` pairs (keeps order and non-string keys) |
| option | `{"none": true}` or `{"some": …}`; never JSON `null` |
| record | Object keyed by `field_id`; an absent optional field is a missing key |
| enum | `{"variant": variant_id, "payload": …}` |
| json | Canonical JSON text as a string (keeps exact number text) |
| secret-ref | Reference string (`credential://…`); never secret material |
| handle, stream, result, error | No literal form (`E-HANDLE-LITERAL`) |

Implementation correction (00b.4 review): encoding requires the exact f32/f64 runtime variant; it performs no implicit narrowing or widening. Map encoding rejects floating-point key types and duplicate runtime keys, just as decoding does. Constraint registration and general key eligibility remain 00c work under [11 §5](11-graph-language.md).

Implementation notes (WU-00/00b.4, `orange_model::value::literal_codec`):

- **Nested floats** use the same lowercase-hex bit-pattern strings as a top-level `bits` field.
- **Canonical form required:** decimal strings, bit patterns and base64 must already be canonical (no `+`, no leading zeros, lowercase hex, zero padding bits), so load→save stays byte-stable.
- **`json` literals** must parse and are kept verbatim, preserving exact number text and JSON null.
- **Unknown record fields and enum variants** stay in the stored literal; only decoding them to a `Value` fails.
- **Schemas:** record and enum layouts come from a `Schemas` provider (the registry later).
- **Nesting budget:** 64 levels for types and literals (proposed).
- **JSON text budget:** `core/json@1` text may nest arrays and objects at most 64 levels and hold at most 1 MiB (`MAX_JSON_TEXT_BYTES`, proposed). Both are checked before parsing, and failures report `TooDeep` and `TooLarge`.
- **Map keys:** duplicate keys are detected with a hash set. `Value` implements `Eq` and `Hash` consistently with its structural equality, so the check takes expected linear time.

### 6.2 Presentation message wire contract

TextRef and MessageRef policy is owned by [13](13-internationalization.md). `MessageRef` has required `origin`, `key`, `args`, `fallback`. `MessageOrigin` is one of:

- Package: `{"kind":"package","package_id":"org.orange.math","version":"0.1.0","content_digest":"sha256:<64 lowercase hex digits>"}` matching the exact owning package lock.
- Host: `{"kind":"host","version":"<application version>","build_digest":"sha256:<64 lowercase hex digits>"}` identifying immutable host catalogue bytes.

The angle-bracket strings describe field constraints, not literal example values. Origin is trusted envelope context assigned/verified by the host, never guest authority to impersonate another catalogue namespace. Header-interned origins are expanded on standalone export; absent origin resources use safe fallback, never latest-version lookup.

`args` maps declared names to restricted tagged literals from §6.1. Allowed descriptor types and wire forms are `string` → `{"type":"core/string@1","value":"safe text"}`, `i64`/`u64` → canonical TypeExpr with decimal-string `value`, `f64` → `{"type":"core/f64@1","bits":"<16 hex digits>"}` (finite only), and `timestamp`/`duration` → canonical TypeExpr with nanosecond decimal-string `value`. No nested containers, bytes, secret references, handles, nonfinite values or arbitrary graph values are allowed. Integer/time ranges are their core type ranges in [11 §5.2](11-graph-language.md). A formatter cannot silently round an integer or timestamp; an unsupported conversion takes the bounded safe-fallback path. WU-03 publishes equivalent WIT/binary vectors over these same types before G2.

`NodeError` and compiler diagnostics flatten these fields: `origin` → `message_origin`, `key` → `message_key`, `args` → `message_args`, `fallback` → `message`. Locale-specific rendering is separate non-authoritative view data. This adds no new graph-error codes or severity changes; G-39 covers invariant structured fields and L-03/L-08 cover formatting/redaction.

`editor.category_id` is a stable namespaced category identity; `editor.category` is its TextRef caption. Optional configuration captions use `editor.config_labels` (JSON Pointer into `config_schema.properties` → TextRef) and `editor.option_labels` (the same pointer → option-token → TextRef). They never alter schema property names, defaults or enum tokens. Declarative control/action/accessibility captions use the same TextRef/MessageRef contract. These presentation maps are excluded from semantic signature projection.

## 7. In-memory values

```rust
// Target: 16 bytes on 64-bit targets; verified by a size assertion in WU-00b.
pub enum Value {
    Unit, Bool(bool),
    I32(i32), U32(u32), I64(i64), U64(u64), F32(f32), F64(f64),
    Duration(i64), Timestamp(i64),
    Str(ArcStr), Bytes(ArcBytes),
    List(Arc<ListRepr>), Map(Arc<MapRepr>), Record(Arc<RecordRepr>),
    Enum { tag: u32, payload: Option<Arc<Value>> },
    None, Some(Arc<Value>),                  // Some is boxed only for nested option<option<T>>
    Err(Arc<NodeError>),                     // only inside result<T>
    Json(Arc<JsonRepr>),
    Handle(HandleId),
}
pub enum ListRepr { Bool(BitBox), I32(Box<[i32]>), I64(Box<[i64]>), U32(Box<[u32]>), U64(Box<[u64]>),
                    F32(Box<[f32]>), F64(Box<[f64]>), Str(Box<[ArcStr]>), Values(Box<[Value]>) }
pub struct RecordRepr { pub ty: TypeKey, pub fields: Box<[Option<Value>]> } // schema order; None = absent
pub struct MapRepr { /* insertion-ordered; keys restricted per 11 §5.2 */ }
pub enum JsonRepr { Null, Bool(bool), Number(NumberText), String(ArcStr), Array(Box<[JsonRepr]>), Object(/* ordered */) }
```

- `option<T>` is `None`, or the unboxed `T` value (static types disambiguate). Nesting boxes the inner layer. `result<T>` is `Err(…)` or the unboxed `T`; `result<core/error@1>` boxes. **Missing** exists only as an absent record field or JSON key, never as a free-standing `Value`. Together these make None, missing, JSON null and Error distinct by construction (FR-08).
- Clone is an `Arc` increment. Mutation-style nodes consume a uniquely owned value or copy on write ([03](03-data-and-execution.md)).
- Records are addressed by compiled field index, never by name at run time.
- Secrets are never `Value`s. Brokers hold them ([05](05-services-and-platforms.md)).

Implementation notes (WU-00/00b.4; Rust spelling only):

- **Size:** a compile-time assertion keeps `Value` ≤16 bytes. `ArcStr`/`ArcBytes` are thin `Arc<String>`/`Arc<Vec<u8>>`, because a fat `Arc<str>` would make it 24 bytes.
- **List storage:** `ListRepr::Bool` is `Box<[bool]>`.
- **Record type:** `RecordRepr::ty` retains canonical `TypeExpr` even with 00c interning. The standalone codec returns freely owned values without retaining a registry snapshot. Migration to a transient key waits for a runtime consumer that owns the matching snapshot for every key use; a detached key must not replace persisted type identity.
- **Enum tag:** the variant's index in schema order.
- **JSON:** `Json` holds verbatim JSON text; the structured `JsonRepr` arrives with the JSON node family (05d).
- **`NodeError`:** carries only its stable code until the WU-03 message contract.
- **Equality and hashing** are structural, with floats compared by bit pattern and lists by elements whatever their storage; they are not the language `eq`/`hash` constraints.

## 8. Registry snapshot

```rust
pub struct RegistrySnapshot {
    pub generation: RegistryGeneration,
    pub interner: TypeInterner,                                       // TypeExpr ↔ TypeKey, frozen
    pub definitions: HashMap<DefinitionRef, Arc<NodeDefinition>>,
    pub types: HashMap<NamedType, Arc<TypeDefinition>>,
    pub constraint_impls: HashMap<(TypeKey, Constraint), ImplRef>,    // coherence-checked at publication
    pub converters: HashMap<(TypeKey, TypeKey), SmallVec<[ConverterRef; 1]>>,
    pub providers: HashMap<SlotId, Vec<ProviderDescriptor>>,
    pub new_binding_defaults: HashMap<SlotId, ProviderId>,           // never applied to existing locks
}
```

Hash lookups here are compile-time only. Publication validates namespace ownership, constraint coherence and converter ownership before the snapshot is visible ([04](04-expansion-plugins.md)).

Implementation (00c.1): `orange_model::types::TypeRegistryBuilder` registers type metadata and interns concrete `TypeTerm`s; `build` validates schema references/member IDs and publishes an immutable `TypeRegistry`. The registry supplies the existing literal `Schemas` interface directly, preserving one record/enum layout model. Package registrations cannot replace core types or claim another package namespace. Handle constructors require registered handle-kind arguments. Recursive nominal schema references do not expand during registration; literal decoding/encoding retains its depth budget.

The public host-side `TypeKey` wraps a dense `u32` slot with an opaque registry owner token; lookup rejects a key from another registry even when its numeric slot exists there. Registries are not cloned into independently mutable identity spaces. This checked host wrapper is not a wire representation or a change to future dense execution-plan indexes; keys are never persisted. Resolution rejects unresolved parameters, unknown names/majors, wrong arity and nesting beyond the existing parser budget. Read-only validation and owner/kind queries also accept uninterned terms, so inference need not mutate a published snapshot.

## 9. Execution plan IR

Illustrative IR. The exact encoding is fixed in WU-00e, while the separation into pull programs, control blocks and dense slots is a contract.

```rust
pub struct ExecutionPlan {
    pub plan_hash: Digest, pub graph_id: GraphId, pub source_revision: u64,
    pub snapshot: Arc<RegistrySnapshot>,
    pub types: Box<[TypeInfo]>,               // by TypeKey: layout, constraints, codec
    pub consts: Box<[Value]>,                 // folded const-class values
    pub kernels: Box<[KernelEntry]>,          // Native(fn ptr) | Guest(component, export) | Worker(op)
    pub pulls: Box<[PullProgram]>,            // one per consumption site
    pub blocks: Box<[Block]>,                 // control IR
    pub regions: Box<[RegionInfo]>,           // parent, kind (entry/event/step/branch/body/function), join mode
    pub loops: Box<[LoopInfo]>, pub zones: Box<[ZoneInfo]>,
    pub entries: Box<[EntryPlan]>,            // root block, payload slots, ActivationPolicy, event source
    pub functions: Box<[FunctionPlan]>,
    pub watches: Box<[WatchPlan]>,            // dependency state slots + pull program
    pub frame: FrameLayout,                   // slot count, memo bit counts per class
    pub state: StateLayout,                   // run vars, registers, node state, versions
    pub source_map: SourceMap,                // NodeIx/BlockIx/instr → NodeId/PortKey/EdgeId
}
pub struct PullProgram {
    pub prologue: Box<[VolatileRead]>,        // state-read + clock, program order
    pub steps: Box<[PullStep]>,               // post-order, memo bit per step
    pub result_slots: SlotSpan,
}
pub enum PullStep {
    Kernel { node: NodeIx, kernel: KernelIx, inputs: SlotSpan, outputs: SlotSpan, memo: MemoBit },
    Entropy { node: NodeIx, kernel: KernelIx, inputs: SlotSpan, out: SlotIx, memo: MemoBit },
    Lazy { op: LazyOp, cond: SlotIx, arms: [PullIx; 2], out: SlotIx },   // select/and/or/coalesce/try
    Zone { zone: ZoneIx },                    // resumable; budget checked per iteration
    Offload { program: PullIx },              // heavy; strand suspends
}
pub struct Block { pub region: RegionIx, pub instrs: Box<[Instr]> }
pub enum Instr {
    Pull(PullIx),                                                     // opens an epoch
    Act   { node: NodeIx, kernel: KernelIx, inputs: SlotSpan, outputs: SlotSpan, outcomes: OutcomeTable },
    Await { node: NodeIx, kernel: KernelIx, inputs: SlotSpan, outputs: SlotSpan, outcomes: OutcomeTable },
    Branch { cond: SlotIx, on_true: BlockIx, on_false: BlockIx },
    Switch { key: SlotIx, table: SwitchIx },
    Jump(BlockIx),
    Sequence { steps: Box<[BlockIx]> },
    Fork { branches: Box<[BlockIx]>, join: JoinMode, completed: BlockIx },
    LoopEnter(LoopIx), LoopNext(LoopIx), Break(LoopIx), Continue(LoopIx),
    IterOpen(LoopIx), IterNext(LoopIx),
    StoreVar { var: VarIx, src: SlotIx }, StoreRegister { reg: RegIx, src: SlotIx },
    Emit { event: EventIx, payload: SlotIx },
    Yield, Call { function: FunctionIx, args: SlotSpan, results: SlotSpan }, Return { results: SlotSpan },
    Fail { error: SlotIx }, EndStrand,
}
```

- Control intrinsics compile to instructions, not kernels. Their semantics live in one place and cost no dispatch.
- `OutcomeTable` maps each outcome index (including implicit `failed` and `cancelled`) to a block or `Fault`.
- Memo bits are grouped by stability class. The `const` class is folded into `consts`, `activation` bits reset per activation, and `epoch` bits reset per `Pull` (O(words)).
- Each slot has one writer site. Its static type is in `FrameLayout`, so kernels need no tag checks in release builds (debug builds assert).

## 10. Runtime structures

```rust
pub struct Run {
    pub id: RunId, pub generation: Generation, pub plan: Arc<ExecutionPlan>,
    pub vars: Box<[Value]>, pub var_versions: Box<[u64]>,
    pub registers: Box<[Value]>, pub node_state: Box<[Value]>,
    pub activations: Slab<Activation>,
    pub ready: VecDeque<StrandRef>,                       // FIFO by ready time
    pub entry_queues: Box<[BoundedQueue<EventEnvelope>]>,
    pub inbox: BoundedReceiver<RunMessage>,
    pub continuations: GenSlab<Continuation>,
    pub handles: HandleTable,                             // generational slab, owner-checked
    pub frames: FramePool,                                // preallocated per FrameLayout
    pub segment_changes: ChangeBitSet,                    // vars/registers written this segment
    pub trace: TraceRing, pub ingestion_log: Option<IngestionLog>,
    pub clock: ClockRef, pub seed: u64, pub max_cause_depth: u16,
}
pub struct Activation {
    pub id: ActivationId, pub entry: EntryIx, pub cause_depth: u16,
    pub frame: FrameRef, pub strands: SmallVec<[StrandIx; 4]>,
    pub regions: SmallVec<[RegionInstance; 4]>,           // live strand counts per instance
    pub status: ActivationStatus, pub cancel: CancelNode, pub entropy_counter: u64,
}
pub struct Frame {
    pub slots: Box<[Value]>, pub locals: Box<[Value]>,
    pub memo_activation: BitBox, pub memo_epoch: BitBox,
    pub calls: SmallVec<[CallFrame; 2]>,
}
pub struct Strand {
    pub pc: (BlockIx, u32), pub region_stack: SmallVec<[RegionCursor; 4]>,
    pub loop_stack: SmallVec<[LoopCursor; 2]>, pub status: StrandStatus,
    pub cleanup: Option<CleanupBudget>,
}
pub enum RunMessage {
    Event(EventEnvelope),
    Completion { cont: ContinuationId, outcome: CompletionOutcome },  // generation checked
    Timer { cont: ContinuationId },
    Command(RunCommand),                                             // stop, trigger, inspect, breakpoint
}
pub struct EventEnvelope {
    pub ingestion_seq: u64, pub source: SourceId, pub source_seq: u64,
    pub cause: Option<ActivationId>, pub cause_depth: u16,
    pub monotonic_ns: u64, pub wall_ns: i64,
    pub payload: Value, pub payload_bytes: u32,
}
```

Kernel contract (internal Rust; guests use the versioned component interface from WU-03, which carries the same outcomes):

```rust
pub type PureKernel = fn(inputs: &[Value], outputs: &mut [Value], cfg: &CompiledConfig) -> Result<(), NodeError>;
pub trait ActionKernel { fn run(&self, cx: &mut ActionCx<'_>) -> ActionOutcome; }
pub enum ActionOutcome { Outcome(OutcomeIx), Suspend(PendingOp), Fail(NodeError) }
pub trait IteratorKernel {
    fn open(&self, cx: &mut ActionCx<'_>) -> IterStep;
    fn next(&self, cx: &mut ActionCx<'_>) -> IterStep;   // Item | End | Suspend(PendingOp) | Fail
    fn close(&self, cx: &mut ActionCx<'_>);              // always called; cleanup-safe
}
```

`ActionCx` exposes only the node's input and output slots, the node's own state cells, checked host-service handles for granted capabilities, the run clock and the node's entropy stream. A kernel cannot reach other slots, the scheduler or the document. This replaces the earlier `Complete | Suspend | Failed` outcome in [02](02-architecture.md), which could not select a Branch arm or an outcome group.

## 11. Invariants and complexity budgets

| ID | Invariant |
|---|---|
| I-1 | Each data input has ≤ 1 edge; each exec and event output ≤ 1; each stream output ≤ 1. |
| I-2 | D is acyclic; every control cycle contains `suspends: always`; the call graph is acyclic. |
| I-3 | Every exec node is in exactly one region; every consumed `execution` value satisfies availability ([11 §7.2](11-graph-language.md)). |
| I-4 | Every non-intrinsic node has a signature snapshot, a provider binding and a lock entry. Edges to unknown ports are retained as orphan edges. |
| I-5 | `type_args` fully resolve every type parameter of a compiled node. |
| I-6 | Plans and snapshots are immutable. Run state is mutated only by its actor. Workspace state only by the host actor. |
| I-7 | No Bevy `Entity`, Rust `TypeId`, pointer, live handle or secret is ever persisted. |
| I-8 | The warm pure path for scalar types performs no hashing and no heap allocation. |

| Operation | Complexity | Proposed budget |
|---|---|---|
| Connect/disconnect validation (kind, fan-in, types, cycle, region, availability) | Affected type component + topological interval + affected region roots | NFR-15: ≤ 2 ms p95 on W4 |
| Layout edit | O(1) document + editor spatial index | Zero compile or preview work (NFR-04) |
| Semantic edit compile | Nodes in the affected roots and their dependent functions | NFR-04: ≤ 50 ms p95 |
| Epoch reset | O(memo words of one program) | Included in NFR-01 |
| Scalar pure step | One indexed kernel call | NFR-01, NFR-05 |
| Event enqueue / dispatch | O(1) amortized, bounded queue | NFR-02 |
| Handle or continuation lookup | O(1) index + generation compare | NFR-02 |

Requirements: FR-05, FR-08, FR-09, FR-44–50. Tests: G-suite in [11 §18](11-graph-language.md), V01/V02/V05 in [06](06-quality-and-validation.md). ADR-06, ADR-13–16 in [08](08-decisions.md).
