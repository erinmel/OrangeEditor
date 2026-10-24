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

Persistence and execution are separate decisions: format v1 stores GraphDocument as compact versioned JSON; compilation lowers it into typed in-memory values, dense indices and pinned bindings. A binary document encoding would change the saved representation, while a binary plan cache would store a separate derived representation. Neither replaces typed execution data. The JSON-versus-MessagePack experiment in 10 does not adopt another interchange format or a cache ABI. Future caches require versioning, source/provider identity checks and executable rebinding; Rust memory images, pointers and live bindings are never portable persisted data (I-7).

| ID | Form | Persisted | Notes |
|---|---|---|---|
| `GraphId`, `NodeId`, `EdgeId`, `VariableId`, `EventDeclId`, `FunctionId` | UUID (v4 or v7) | yes | Paste remaps every one of them ([01](01-product-requirements.md) FR-02) |
| `PortId` | `[a-z][a-z0-9_]{0,63}` | yes | Unique per definition |
| `PortKey` | `port_id` or `port_id#k`, `k: u16` | yes | Variadic counter is never reused within a node |
| `DefinitionId` | `namespace/name` | yes | `core/` reserved for language intrinsics |
| `TypeExpr` | Canonical string ([11 §5.1](11-graph-language.md)) | yes | Stored as validated text; analysis parses terms and type resolution uses registry-local interning |
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
    pub domain: Option<DomainId>,            // absent = `control` (ADR-21)
    pub nodes: IndexMap<NodeId, NodeInstance>,
    pub edges: IndexMap<EdgeId, Edge>,
}
pub struct NodeInstance {
    pub id: NodeId,
    pub definition: DefinitionRef,           // id + definition_version
    pub provider: Option<ProviderBinding>,   // None only for host-owned language intrinsics
    pub config_version: u32,
    pub config: ConfigValue,                 // validated by definition config_schema
    pub type_args: BTreeMap<TypeParamName, TypeExpr>,  // resolved parameters (cache)
    pub type_pins: BTreeMap<TypeParamName, TypeExpr>,  // explicit user choices (11 §5.5)
    pub inputs: BTreeMap<PortKey, Literal>,  // includes shadowed literals
    pub variadic: BTreeMap<PortId, VariadicState>,     // order + next counter
    pub signature: Option<SignatureKey>,     // required for every non-intrinsic node
    pub activation: Option<ActivationPolicy>,// entries only
}
pub struct Edge { pub id: EdgeId, pub from: PortRef, pub to: PortRef }
pub struct PortRef { pub node: NodeId, pub port: PortKey }
pub struct VariadicState { pub order: Vec<u16>, pub next: u16 }
pub struct SignatureSnapshot {
    pub kind: NodeKind,
    pub suspends: Option<Suspends>,          // semantic async/iterator fact; see below
    pub ports: Vec<PortDescriptor>,
}
pub enum Suspends { Never, Maybe, Always }   // synchronous absence is equivalent to Never
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
    pub body: GraphBody,                     // function-entry/return/outputs markers take their ports from this interface (11 §14)
}
pub struct ActivationPolicy {
    pub mode: ReentrancyMode,                // Queue | Drop | Replace | Parallel{max}
    pub capacity: u32, pub overflow: OverflowPolicy,
    pub on_fault: FaultPolicy, pub order: u32,
}
```

`SignatureKey` is a document-local key. The implementation uses `sha256:` over the canonical JSON of the snapshot, so identical signatures share one entry; examples use short keys. Host-owned language intrinsics are identified by the host's intrinsic definitions and use their `format_version` and config to derive signatures, so they do not store snapshots, provider bindings or package locks. A `core/` namespace alone does not make an ordinary definition intrinsic or compile-ready without a provider binding and exact package lock.

Validation resolves definitions and providers from immutable host-supplied registry metadata; it never executes package code. A non-intrinsic node requires its stored signature snapshot, exact provider binding and matching package lock. If the package or exact provider metadata is unavailable, the node remains unresolved and cannot be reported as successfully resolved. Its stored snapshot still supplies ports for edge checks, including orphan-edge reporting, while checks that require unavailable current definition metadata are incomplete.

Serialization order is `IndexMap` order (creation order, compacted on removal), which makes output deterministic for the same command history.

`domain` names the execution domain of a body (ADR-21): the root graph's key sits beside its `nodes`/`edges`, and each function body carries its own. It is a namespaced ID such as `org.example.music/signal`; absent means the built-in `control` domain (11 §2–§15), so existing format v1 documents are unchanged. A domain the host does not provide makes the body load and round-trip but not compile, like an unresolved provider. Domains come from crates linked by the embedding product, never from installed packages. The typed field is added to `orange-model` when a host first registers a second domain; until then a `domain` key is preserved in the graph or flattened function declaration extension map. The current domain prerequisite accepts absence or the exact string `control`. Any other JSON value, including null, is preserved and reports E-DOMAIN-UNSUPPORTED at the owning body, even if empty.

Implementation correction (00b review): replacing an existing entry at its current position updates that slot in place. Insertions, removals and actual reordering retain positional patches so undo restores the exact order. The replacement path avoids shifting unrelated entries; order-preserving removal and reordering can still be linear. NFR-15 timing targets remain unmeasured.

`PortDescriptor` is the post-template, post-type-substitution semantic port projection from [11 §4](11-graph-language.md): stable port key, direction/kind, resolved type where applicable, source/availability, literal/default/optional, variadic/lazy and sensitivity policy where applicable. Its snapshot excludes presentation `group`, TextRef labels/descriptions and `editor` fields, including non-authoritative `display` text on a default literal. The default's type and encoded value remain semantic. Hash only that semantic snapshot. Optional cached presentation belongs in `editor`, keyed by stable definition/port IDs, outside signature and semantic plan hashes. Without a catalogue or cache, show stable IDs. Locale changes cannot trigger `E-SIGNATURE-DRIFT` (G-39).

`suspends` is semantic: it decides control-cycle legality and an enclosing function's kind (11 §8.2, §14), so a node whose package is missing must still carry it, and it is part of the hash. For a current synchronous definition, absence means `never`. For a current `async` or `iterator` definition, absence means unknown metadata and reports `E-SUSPENDS-MISSING`; it is not silently changed to `maybe` or `always`. A value on an incompatible kind, or an invalid value, reports `E-SUSPENDS-INVALID`. Exact historical snapshot bytes remain preserved until an explicit re-projection/migration; validators do not rewrite old snapshots to manufacture a fact. The current host may re-project a supported intrinsic or exact provider definition and compare semantic signatures.

At registry publication, the same kind/value matrix is enforced on definition metadata before either declared ports or contextual variable ports are processed. `async` and `iterator` require `maybe` or `always`; synchronous kinds may omit the field or use `never`. Typed `RegistryError` variants distinguish missing from incompatible metadata and identify the definition. This publication check does not change passive snapshot deserialization, so legacy saved signatures remain available to validation and repair.

Function declarations remain graph-local. `core/call.config.function` contains a canonical `FunctionId`; variable intrinsics use `config.variable` with a canonical `VariableId`, and update nodes use `config.operation` with a stable operation token. These are typed references, not names or arbitrary strings to search. Function summaries (inferred kind, suspension, transitive run-variable writes, and completeness) are derived validation/compile data, not persisted document fields. A host may provide explicit trusted native-leaf semantics; unresolved external bodies remain incomplete, never assumed effect-free. The call graph is analyzed iteratively with bounded explicit traversal state; recursive calls report `E-RECURSION` without relying on the Rust call stack.

### 3.2 Derived indexes (not persisted)

Combined validation is produced by `analyze(document, registry)`, `analyze_with_effects` or `analyze_with_options`; the legacy `validate` and `validate_with_effects` functions extract the same `ValidationReport` from this orchestration. `ValidationAnalysis` privately binds the report and immutable actual coverage to transient document `state_id` and registry `generation`. It exposes `covered_checks()` and `missing_checks()`; required checks are listed by `required_checks()`. For supported control documents, coverage is declaration types, state metadata rules, node resolution, input values, connections, function summaries, definition and config, body domains, contextual contracts, function restrictions, regions, data-dependency cycles, control cycles, value availability, stream rules and entry policies and exports. The region pass (`validate_regions`) exposes its `RegionIndex` as `ValidationReport.regions` for the availability pass; zone structure and switch cases are enforced, so `Regions` is covered (11 §7.1). The control-cycle pass (`validate_control_cycles`) also runs and publishes closed node witnesses as `ValidationReport.control_cycle_paths`; `ControlCycles` is covered because every 11 §8.1/§8.2 control-cycle rule is enforced (`E-CYCLE-SYNC-CONTROL`, `W-BUSY-LOOP`, and `E-RECURSION` via function summaries). `ConstantEvaluation` remains missing because `W-CONST-ERROR` and error preservation require compiler constant folding (11 §7, §11; 08). `StateRules` covers the implemented metadata checks. `is_ready_for_lowering()` permits a fresh, valid analysis with only `ConstantEvaluation` missing to enter compiler analysis; it does not authorize plan publication or execution. Graph-local functions are accepted as call contracts callee first: each function body is checked once, with its callees already accepted or rejected, and the whole-document passes then run once at that fixed point. This avoids one whole-document round per call-chain level; it is not a linear-time guarantee for individual validation passes or transitive effect summaries. Whole-document rounds remain as a fallback that converges to the same result. Body-domain validation is a prerequisite: any unsupported body stops combined control analysis with no resolved signatures or summaries, all nodes/functions marked incomplete, and E-DOMAIN-ANALYSIS-BLOCKED subjects explaining the skipped analysis. Only BodyDomains is then covered; sixteen checks remain unexecuted. The private immutable coverage list belongs to the invocation, rather than a global completion claim. Hosts may still use standalone structural inspection APIs on preserved data. `is_ready_for_compilation()` requires matching identities, a pass-local valid report and no missing checks, so it remains false until all required checks are implemented. The report accessor is read-only, and a default or extracted report cannot construct analysis evidence. Unknown domains remain unsupported and never become ready through control-domain checks. Host-provided leaf effects are bound to the analysis invocation but are not authenticated by it. Data-dependency analysis uses only fresh resolved signatures and same-body valid data edges, expands sparse concrete variadic keys and connects each data input to computed outputs only. Wide computations use an internal hub to represent this complete relation with a linear number of dependency edges; narrow computations retain direct edges. Hubs never appear in the concrete-port witnesses. It returns one deterministic closed port witness per cyclic component in ValidationReport.data_cycle_paths and E-CYCLE-DATA; non-computed output sources break dependencies. Missing metadata stays incomplete and invalid connection edges are not guessed into D. No stored signature fallback, control-cycle, region, availability or runtime guarantee follows from this pass.

Contextual contracts are computed, never stored. A definition declares its ports (`Ports::Declared`) or derives them from a referenced declaration (`Ports::Variable`), and `RegistrySnapshot::contract(document, node)` returns the node's contract as a pure function of the definition and the document. Get resolves `value` as a state output typed by the referenced VariableDecl; synchronous Set requires `value` as an input and rejects workspace targets with `E-VARIABLE-SCOPE`; Update Add/Sub/Min/Max resolves required `value` input and execution-sourced `old`/`new` outputs from the current local/run declaration, rejecting workspace scope and requiring exact numeric-operation evidence for Add/Sub or Ord evidence for Min/Max (11 §11.1). A missing or malformed reference reports `E-VARIABLE-UNKNOWN`. Two nodes of one definition therefore have independent contracts, a declaration edit or undo changes the result on the next lookup, and there is no overlay, fingerprint or staleness state to maintain. Inference, edit persistence, connection planning and node validation all read this one lookup. Computing a contract changes no document fields, revision, state identity or undo history. Compare-and-set additionally projects required `expected: T` before `value: T` and an execution-sourced `exchanged: core/bool@1` output on `then`, with exact Eq evidence; the `old`/`new` types remain declaration-owned. And/Or/Xor use the ordinary Update port shape only for exact core bool/integer types. Append retains that shape for exact string/bytes/list concatenation; list element types need validation but no equality or numeric proof. Shared WriteShared/UpdateShared projections accept workspace declarations only, are Async/Maybe and reuse these data/type rules without establishing a host actor. Their exact language facts also classify function effects; workspace writes never enter run_writes. Call-site authoring now uses a separate type-only `catalog::call_interface` projection: target FunctionId, declared parameters/constraints and data descriptors, with no kind/suspension/effects and no output provenance. It validates identity, target, supported domains and coherent data interface shape; malformed targets never fall back to saved snapshots. Inference scopes one instantiation per call node and persists arguments only. Variable/function declaration changes discover dependents through typed before/after references, including removals; calls losing a generic interface clear cached arguments atomically with the edit. Pins and saved signatures remain untouched. Executable call contracts come only from accepted callees (complete, conformant and free of function errors); a generic callee keeps its type parameters rigid in its own body, and each call instantiates them (11 §14). Connection validation now consumes fresh node resolution, including declaration-owned Get/Set/Update ports; only unresolved package nodes fall back to stored signatures for offline inspection. `analyze(document, registry)` / `analyze_with_effects` produce `ValidationAnalysis`; the legacy `validate` wrappers extract its same report for compatibility. It supplies no function leaf effects by default, so a package action leaf remains incomplete (`E-FUNCTION-METADATA-UNAVAILABLE`). `ValidationReport::is_valid()` describes only passes actually run; see the coverage contract above.

Declaration-driven inference invalidation (C2): `plan_edit` compares declaration IDs and contents across the before/after documents. For each added, changed or removed variable declaration it scans typed `RegistrySnapshot::references` over nodes in every body in both states, then seeds the referenced consumers and their incident edge endpoints into existing component inference. This correctness-first scan handles reference removal/config replacement and cases where a removed Get can no longer expose a contextual port; stale cached `type_args` never seed inference. Updates stay in the declaration edit's single transaction, preserving pins and leaving unrelated components untouched. The scan cost is temporary implementation overhead; no reverse reference index is persisted and NFR-15 is unmeasured.


Function signature/effect evidence is captured for a document content state and checked before function analysis. A changed document must receive fresh host evidence; stale input produces incomplete functions and no usable summaries. This binds evidence lifetime, not authenticity: the host remains responsible for supplying truthful metadata from exact available providers.

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

- **Kept beside the document:** the index lives next to the document, not inside it, so document equality, saving and undo never involve derived data. The one exception is node location: `GraphDocument::locate_node` and `node` use a node-to-body map built on first lookup and kept current by every node and function patch. It is never serialized or compared, so equality, saving and undo are unaffected, and lookups no longer scan every function body.
- **Updated from change sets:** each committed `ChangeSet` updates it, and an updated index always equals a fresh build (property-tested). The index records the content state it describes (`is_current`). An undo or plain `apply` that bypasses it leaves it stale; `apply_inferred_indexed` then rebuilds it before planning instead of reading stale edges.
- **`BTreeSet` edge sets:** used instead of `SmallVec` so equality does not depend on edit history. Performance is evaluated with NFR-15.
- **Deferred rows:** the dense document index, data topological order, type components, region index and diagnostics arrive with their consumers (00c/00d).

Review correction (2026-10-23; FR-44/46, NFR-05/08/15): export activation analysis builds root-body incoming data adjacency once and traverses computed-output dependencies only. State outputs terminate the pull cone, so a register's write inputs cannot make its current value activation-scoped. No exports means no adjacency work. Availability groups distinct consumer subjects by activation root, producer and available outcomes, processes one reachability set at a time and releases it immediately. For V control-flow vertices, E edges, P producer/outcome groups and Q discovered use queries, this removes the retained P×V reachability cache; scratch storage follows V+E+Q, while traversal CPU can still follow P×(V+E). Shared lazy cones can increase Q. This is not a global analysis budget or NFR qualification. Resource containment traverses registered nominal record fields and enum payloads as well as type arguments; each nominal schema expands once per query so cycles terminate. Registers use fresh resolved cell metadata. [10](10-tooling-and-sources.md#security-and-performance-verification-2026-10-23) records measurements and remaining gaps.

Combined analysis applies `AnalysisOptions` before indexing/resolution. Defaults are 250,000 aggregate graph/inspected metadata items, 1,000,000 inspected JSON values, 64 MiB of inspected UTF-8 metadata, and a maximum squared item-count admission estimate of 25,000,000,000. Descriptors/defaults and matched definitions are charged per node, including referenced function interfaces. Configuration, literal and schema payloads are traversed by reference without serializing or cloning them. Counts use checked arithmetic. These are admission quotas, not elapsed-time, allocation or instruction-count guarantees; registry construction and host-owned type-schema traversal have separate lifecycles. Ignored editor/presentation data is outside the analysis quotas. Import limits remain independently necessary.

`analyze_with_options(document, registry, leaf_effects, options)` permits host overrides and an optional borrowed `AtomicBool` cancellation signal. Cancellation is polled during admission, between passes and between function bodies, with no bounded interruption latency within a pass. Legacy combined entry points use the default quotas. Standalone passes remain caller-controlled. Rejection/cancellation discards partial results, returns one global `E-ANALYSIS-LIMIT`/`E-ANALYSIS-CANCELLED`, no projections/summaries and empty coverage; both readiness predicates fail. No partial result may become compiler evidence. Hosts needing hard execution limits must isolate analysis in a worker; IPC alone provides no sandbox.

`validate_declaration_types` checks every variable type and event payload against the current type registry, including unused declarations without initializers. Syntax, registered identity, arity, unresolved parameters, handle kinds and map-key constraints are checked. Variable/event declarations have no generic parameter scope. Invalid types report `E-DECLARATION-TYPE` with a typed `variable` or `event` subject ID; existing nesting/derivation bounds report `E-TYPE-COMPLEXITY`. Messages expose neither declaration names nor raw type text. This pass is included in `DeclarationTypes` coverage and never rewrites saved declarations.

Compatibility: saved graph format is unchanged. The optional diagnostic `event` field is omitted when absent and older records deserialize without it; downstream Rust `Subject` struct literals need `event: None` or a default struct update. Existing combined entry points now enforce default admission quotas; hosts needing different quotas use `analyze_with_options`. The new lowering predicate supplements the existing final predicate, and coverage consumers must distinguish `DeclarationTypes`, metadata `StateRules` and compiler-owned `ConstantEvaluation` (08, readiness clarification).

### 3.3 Transactions and change sets

Inference consumes net edge deltas from both sides of the transaction: added edges use their new endpoints, removed edges use their old endpoints, and changed edges use both. In particular, disconnecting and reconnecting the same `EdgeId` can change its endpoints without reporting an addition/removal. The resulting bindings and projected signatures stay in the originating undoable transaction (11 §5.5); no wire-format change is needed.

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
    PutPackageLock(PackageLockEntry),                                  // one entry per package version
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
- `PutSignature { key, snapshot }` (00b.3) adds a shared snapshot so that pasted nodes keep the snapshots I-4 requires. An identical snapshot under the same key is a no-op; different content under an existing key is rejected. Signatures are therefore transaction-owned. Exports still have no command.
- `PutPackageLock(entry)` (00c) makes `package_lock` transaction-owned too. A provider binding names only `package`, `version` and implementation `id`; the content digest lives once per package version in `package_lock`, so an exported graph lists exactly the package contents it needs and an importer can check them before loading. The command appends a new `(id, version)` entry, is a no-op for the same digest and rejects a different digest (`PackageLockConflict`). Callers pair it with an `AddNode` or `Rebind` naming a package version; the connection planner and paste do. Like superseded snapshots, entries no longer referenced are not pruned here.
- Superseded snapshots are not pruned (00c.7a). A re-inference moves a node to another content-addressed snapshot and leaves the old one in `signatures`. Pruning inside the transaction would need a reference scan over every body (and function) on each commit and a remove-signature command, and it would churn the two snapshots that pin/unpin and undo/redo alternate between. The unreferenced entries are bounded by the distinct type combinations used, are inert (nothing resolves a snapshot that no node names), and are removed by the same undo that added them. Garbage collection belongs to an explicit compaction on save, together with the codec round-trip rules for unknown data; it is not part of this slice.
- `SetTypePins { node, pins }` (00c) replaces a node's explicit type choices; like every node edit it marks the node changed, so `inference::plan_edit` re-infers its component. An `UpsertFunction` reports only its function in the change set, so `plan_edit` treats every node of the added or replaced body as touched.
- `SetSignature { node, key }` (00c.4b) points a node at an existing snapshot (`PutSignature` first); a missing key is rejected. `inference::apply_inferred` commits an edit together with the `SetTypeArgs`, `PutSignature` and `SetSignature` commands its inference implies, so one undo restores the edit, `type_args` and signature references. `apply` itself never infers, and loading never runs inference.
- Inserting a node between connected endpoints composes disconnect, add-node and two connect commands in one transaction. A rejected replacement restores the original wire and content state; one undo restores the pre-insertion structure. The planner/validator separately checks compatible typed ports. Creating a reusable node from multiple nodes uses a typed `FunctionDecl` interface and `core/call`, not presentation grouping or copying the implementation into every caller (11 §14). Search/palette presentation belongs to the host/editor; this transaction contract does not claim those refactorings are implemented.
- Canonical JSON for `SignatureKey` (00b.3): object keys sorted by byte order, no whitespace, serde_json string escaping. This matches RFC 8785 for the ASCII keys and integers in snapshots. WU-03 publishes the cross-language digest vectors. Semantic projection removes `label`, `description`, `group` and `editor` and substitutes `$T` parameters from `type_args`.
- Paste gives every pasted node, edge and declaration a new ID. Only typed configuration reference paths declared by exact definition/config-version metadata are discovered and remapped. Host-owned intrinsic schemas cannot be replaced by package schemas. Unknown configuration and ordinary strings, including strings equal to UUIDs, remain data. Referenced declarations travel transitively with the copy and are reused when the target already has them. Signatures are re-keyed to their `sha256:` key. Lock entries of copied provider bindings are put into the target; a target that locks the same package version to another digest rejects the paste.

Import integrity: each package `(id, version)` and each type-lock identity occurs once, including when duplicate rows are identical; ambiguous locks are a structural load error. Provider resolution must not choose the first matching row. The default importer checks a 64 MiB UTF-8 byte budget before parsing and then checks 100,000 nodes and 500,000 edges across all bodies. `from_json_with_limits` lets an embedding host choose explicit budgets. Structural counts are post-deserialization checks, not allocation limits; a host reading a file or stream must also bound that read before constructing the input string. These defaults are safety limits, not supported performance targets.

### 3.4 Foundation authoring projections (ADR-22)

Variable declaration controls/nodes refer to the existing stable VariableDecl, and callable reusable nodes use FunctionDecl and exact package/provider locks (11 §11/§14). A declaration authoring projection adds no new runtime instruction or initialization order. Selection export must discover references through the definitions' declared reference fields, remap IDs atomically, preserve unresolved payloads and retain provider locks. Event-containing templates insert ordinary nodes and declarations rather than pretending to be callable functions. Exact library/template/editor encodings remain unimplemented and must be specified with schemas and focused round-trip/undo tests in WU-05 before adoption. This scope update changes no current format v1 field or Rust API.

## 4. Definition schema

`NodeDefinition` fields: `definition_id`, `definition_version`, `config_version`, `kind`, `suspends` (`never`/`maybe`/`always`; async and iterator), `cleanup_safe`, `volatility` (volatile only), `type_params`, `ports`, `iterator` (iterator only), `event_source` (entry only), `port_template` (optional declarative dynamic ports), `config_schema`, `errors`, `capabilities`, `determinism` (`deterministic`/`seeded`/`external`/`nondeterministic`), `cost` (optional hint: `scalar`/`linear`/`heavy`), `implementation_slot`, `label`, optional `description`, `editor`. All human-facing label/help/category/alias fields use TextRef from [13 §2](13-internationalization.md), including port labels/descriptions and declarative configuration-option captions. Implicit kind ports ([11 §4.1](11-graph-language.md)) are never listed; their text comes from host catalogues.

Implementation note (`orange_model::catalog`; Rust spelling only): `NodeDefinition` implements a subset: the `DefinitionRef`, `config_version`, `kind`, `suspends`, `type_params`, `ports` (declared, or derived from the referenced variable declaration for Get, Set and supported Update operations), `references` (the config fields that hold declaration IDs), an optional core `specializations` list (`None` means constraints decide) and an optional `config_schema` in the dialect below. Data and event ports carry a type; exec ports carry none. The remaining fields above are not implemented yet. Implicit kind ports are not listed in a package contract.

**Config-schema dialect `orange.config-schema/v1` (V4b).** `config_schema` is not JSON Schema and no JSON Schema validator is implied. Validation enforces only this subset: `type` (one string of `object`, `array`, `string`, `integer`, `number`, `boolean`, `null`; `integer` accepts JSON integers only, so `1.0` is a number but not an integer), `enum` (non-empty), `minimum`/`maximum` (finite, inclusive, numbers only; signed and unsigned 64-bit integer comparisons retain exact precision, including comparisons with binary64 bounds), `required`, `properties` (recursive, present properties only) and `additionalProperties` (boolean only; `false` rejects names outside `properties`), plus the annotations `default`, `title`, `description`, `$comment` and `$schema` (which, when present, must equal the dialect ID). Any other keyword or value shape (`$ref`, `oneOf`, `items`, patterns, a type list, an object-valued `additionalProperties`, a schema nested deeper than 16) reports `E-CONFIG-SCHEMA-UNSUPPORTED` on each node using it, and that config is not judged; it is never accepted silently. Otherwise a config that fails the schema reports `E-CONFIG-INVALID`. A definition with no schema constrains nothing, a node whose `config_version` differs from the definition's is not judged against it (resolution reports that), and diagnostics carry no config values. `validate_config` is the one pass; it does not rewrite configs or apply `default`.

**Exports (V4b).** `validate_exports` checks every `exports` entry against fresh resolved signatures: `E-EXPORT-DUPLICATE` (a later reuse of an `id`), `E-EXPORT-TARGET` (the node is not in the root body, or the port is not a declared data output), `E-EXPORT-TYPE` (the port type is not concrete or lacks `serialize` evidence, so handles, streams, containers of them, and records or enums without a `serialize` derive are rejected), and `E-CROSS-ACTIVATION` (the value, or a producer in its pure/volatile cone, is payload-, iteration- or execution-sourced; G-35). An unresolved target reports `E-DEPENDENCY-UNAVAILABLE`. Diagnostics name the target node and port, never the export ID's content.

`Ports::Variable(VariableAccess::Update)` is a reusable synchronous Action contract; registry publication rejects other kinds with `RegistryError::VariableUpdateKind`. Package definitions may use this contextual port shape with their own exact provider bindings. The shape grants no built-in write-effect authority: function analysis still requires explicit host-supplied leaf effects for package nodes. Only the exact host intrinsic uses the built-in Update effect rule, and that rule rechecks its current contextual operation/type contract even when the caller supplies a projection.

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
  "definition_id": "org.example.device/on-state-changed",
  "definition_version": 1,
  "config_version": 1,
  "kind": "entry",
  "label": {"key": "org.example.device/on-state-changed-label", "fallback": "On device state changed"},
  "event_source": {"service": "connection:org.example.device/session@1", "event": "StateChanged"},
  "ports": [
    {
      "port_id": "state",
      "direction": "output",
      "kind": "data",
      "type": "core/string@1",
      "source": "payload",
      "label": {"key": "org.example.device/on-state-changed-port-state", "fallback": "State"}
    }
  ],
  "config_schema": {"type": "object", "properties": {"profile": {"type": "string"}}, "required": ["profile"], "additionalProperties": false},
  "capabilities": ["connection.use"],
  "determinism": "external",
  "implementation_slot": "node:org.example.device/on-state-changed@1",
  "editor": {"category_id": "org.example.device/category-device-events", "category": {"key": "org.example.device/category-device-events", "fallback": "Device/Events"}, "control": "standard-node"}
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

Converter registry entry (semantic identity and provider implementation are validated separately by 00c.5a):

```json
{
  "converter_id": "core/i64-to-i32",
  "contract_version": 1,
  "from": "core/i64@1",
  "to": "core/i32@1",
  "class": "checked-partial",
  "errors": ["core/OutOfRange"],
  "policies": [],
  "slot": "convert:core/i64@1->core/i32@1",
  "provider": {"package": "org.orange.math", "version": "0.1.0", "id": "native-checked"}
}
```

The example's `org.orange.math` provider implements the host-owned entry; it does not acquire authority to declare new semantics between two foreign types. The host registers core-owned semantic entries. Package-owned entries must belong to their namespace and own at least one endpoint. A frozen converter view contains canonical endpoints, class, finite policy choices, declared errors, exact provider alternatives (including package digest) and explicit new-binding defaults per entry contract. Conflicting content for one entry identity or conflicting digests for the same implementation binding are rejected. Lossy entries additionally declare the fixed JSON loss output from [11 §6](11-graph-language.md); provider alternatives must preserve that contract.

Persisted `core/convert` node (00c.6a). It binds one entry, one provider and validated policy choices; the instance stores no behavior:

```json
{
  "id": "…", "definition_id": "core/convert", "definition_version": 1, "config_version": 1,
  "config": {"converter": "core/i64-to-i32", "contract_version": 1, "options": {}},
  "provider": {"package": "org.orange.math", "version": "0.1.0", "id": "native-checked"},
  "signature": "sha256:…"
}
```

- `config.converter` and `config.contract_version` are the exact entry key; `config.options` must hold every declared policy with its resolved choice, including choices that came from defaults when the node was created. Validation rejects omitted, unknown or invalid choices instead of filling a current default.
- `core/convert` is a host-owned registry special form, distinct from ordinary intrinsic and package-definition resolution. Its definition and config versions are both 1, and a signature is always required. An absent `provider` selects exactly the entry's `Core` implementation. A package provider is the exact binding and its digest is taken from the unique matching `package_lock` entry; missing or ambiguous locks and unavailable exact providers leave the node unresolved. The provider implements the entry's contract version, so no separate provider contract version is stored. It changes only by explicit rebind (ADR-07).
- The node is a `pure` node whose available entry determines its ports: input `value: from`, output `result: to` (`source: computed`) and, for `lossy` entries only, output `loss: core/json@1`. A checked-partial failure is the pure-node error of [11 §13](11-graph-language.md). Available exact metadata is authoritative; the saved signature is compared and drift blocks completeness. When package metadata is unavailable, its saved snapshot remains available for offline edge inspection, while the node stays unresolved.
- A connection plan is an edit (`PutPackageLock` for a package provider, `PutSignature`, `AddNode`, two `Connect`s, plus a `Disconnect` when it replaces a wire) and is applied as one transaction. Its identities are generated once per plan, and its commands are read-only (`ConnectionPlan::commands`). `plan_connection` returns `ConnectionError::TypeConflict` for a wire that introduces a conflict (11 §5.5); `apply_connection(document, registry, plan)` rejects a plan whose document state or `RegistrySnapshot::generation` changed (`PlanApplyError::{StaleDocument, StaleRegistry}`).

The initial core converter executor (N1b) lives in orange-bundled without adding registry entries. A private immutable `ResolvedConversion` retains its Arc registry snapshot, exact ConverterKey, pre-resolved operation and, for f64→f32, two prebuilt JSON loss flags. Binding accepts only an explicitly selected registered Core provider, exact supported entry/version/endpoints/class and complete valid saved options; package alternatives are never native fallbacks. The passive output `ConversionOutput` has `result: Value` and `loss: Option<Value>`; only lossy conversion has Some(JSON `{"lost": bool}`), on every successful evaluation. Failure returns no partial result/loss pair. Float bit-preservation and loss rules are in 11 §6. This scalar API does not manufacture graph freshness, source maps, executable plans or runtime provenance.

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

Implementation correction (00b.4 review): encoding requires the exact f32/f64 runtime variant; it performs no implicit narrowing or widening. Map encoding rejects floating-point key types and duplicate runtime keys, just as decoding does. The registry-aware codec (00c.2a) additionally checks language `hash` + `eq` eligibility on encode/decode, and registry validation rejects invalid map types even without a literal. The standalone codec-only `SchemaSet` retains its historical float exclusion; it is not a substitute for registry/type validation.

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
    Ok(Arc<Value>),                          // only ambiguous result success payloads
    Json(Arc<JsonRepr>),
    Handle(HandleId),
}
pub enum ListRepr { Bool(BitBox), I32(Box<[i32]>), I64(Box<[i64]>), U32(Box<[u32]>), U64(Box<[u64]>),
                    F32(Box<[f32]>), F64(Box<[f64]>), Str(Box<[ArcStr]>), Values(Box<[Value]>) }
pub struct RecordRepr { pub ty: TypeKey, pub fields: Box<[Option<Value>]> } // schema order; None = absent
pub struct MapRepr { /* insertion-ordered; keys restricted per 11 §5.2 */ }
pub enum JsonRepr { Null, Bool(bool), Number(NumberText), String(ArcStr), Array(Box<[JsonRepr]>), Object(/* ordered */) }
```

- `option<T>` is `None`, or the unboxed `T` value (static types disambiguate). Nesting boxes the inner layer. `result<T>` is `Err(…)` or the unboxed `T`, except a success whose static payload type can carry an unboxed Err uses `Ok(Arc<Value>)`. This includes nested Result, Option<Result>, and core/error; adjacent nested options already hide the inner tag in Some. Box selection is static, including a successful None payload. Thus Ok(Err) cannot collapse into an outer Err. **Missing** exists only as an absent record field or JSON key, never as a free-standing `Value`. Together these make None, missing, JSON null and Error distinct by construction (FR-08).
- Clone is an `Arc` increment. Mutation-style nodes consume a uniquely owned value or copy on write ([03](03-data-and-execution.md)).
- Records are addressed by compiled field index, never by name at run time.
- Secrets are never `Value`s. Brokers hold them ([05](05-services-and-platforms.md)).

Implementation notes (WU-00/00b.4; Rust spelling only):

- **Size:** a compile-time assertion keeps `Value` ≤16 bytes. `ArcStr`/`ArcBytes` are thin `Arc<String>`/`Arc<Vec<u8>>`, because a fat `Arc<str>` would make it 24 bytes.
- **List storage:** `ListRepr::Bool` is `Box<[bool]>`.
- **Record type:** `RecordRepr::ty` retains canonical `TypeExpr` even with 00c interning. The standalone codec returns freely owned values without retaining a registry snapshot. Migration to a transient key waits for a runtime consumer that owns the matching snapshot for every key use; a detached key must not replace persisted type identity.
- **Enum tag:** the variant's index in schema order.
- **JSON:** `Json` holds verbatim JSON text; the structured `JsonRepr` arrives with the JSON node family (05d).
- **`NodeError`:** carries its stable code and optional original producer node; the full message/activation contract remains WU-03/runtime work.
- **Equality and hashing** are structural, with floats compared by bit pattern and lists by elements whatever their storage; they are not the language `eq`/`hash` constraints.

## 8. Registry snapshot

```rust
pub struct RegistrySnapshot {
    pub generation: RegistryGeneration,
    pub interner: TypeInterner,                                       // TypeExpr ↔ TypeKey, frozen
    pub definitions: HashMap<DefinitionRef, Arc<NodeDefinition>>,
    pub types: HashMap<NamedType, Arc<TypeDefinition>>,
    pub constraint_impls: HashMap<(TypeKey, ConstraintId), ImplRef>,  // core or versioned package constraint
    pub converters: HashMap<(TypeKey, TypeKey), SmallVec<[ConverterRef; 1]>>,
    pub providers: HashMap<SlotId, Vec<ProviderDescriptor>>,
    pub new_binding_defaults: HashMap<SlotId, ProviderId>,           // never applied to existing locks
}
```

Implementation note (`orange_model::catalog`; subset, Rust spelling only): `RegistryBuilder` collects the type registry, converters, host origin, node definitions and providers; `build` freezes them into an immutable `RegistrySnapshot` with a process-unique `generation` that changes only when a snapshot is built. Lookups are by definition and config version, provider key (`ProviderDescriptor` with its implemented definitions and specializations) and exact provider support. `build` rejects duplicate definitions and providers, package definitions in the `core` namespace or replacing an intrinsic, duplicate port IDs, implicit exec port IDs outside intrinsics, a data or event port without a type or an exec port with one, declared data outputs without a source (`RegistryError::MissingDataOutputSource` identifies the definition and port), event inputs (`RegistryError::EventInputPort` identifies the definition and port), package lazy ports (`RegistryError::LazyPackagePort`; explicit false is non-lazy), incompatible suspension metadata (`RegistryError::SuspendsMissing` or `SuspendsInvalid` identifies the definition and kind/value), exec ports on `pure` definitions (`RegistryError::PureExecutionPort` identifies the definition and port), undeclared `$T` type parameters in a port type, and providers naming an unknown definition. New definitions cannot publish missing async/iterator suspension metadata or incompatible values; when such metadata appears in a passive saved signature, validation still reports the existing `E-SUSPENDS-MISSING`/`E-SUSPENDS-INVALID` diagnostic and leaves the bytes repairable. Default literals are confined to data inputs (`RegistryError::DefaultPortPosition`); true optional flags require a data input (`OptionalPortPosition`) and a well-formed exact Option wrapper with one argument (`OptionalType`). False optional flags are inert. These rules run after established descriptor checks; malformed type syntax and type-name resolution stay with inference diagnostics. For fully registered concrete declared types, publication also checks exact default type equality (`DefaultType`) and canonical decoding against the current type registry (`DefaultLiteral`). The errors identify definition/port without retaining payload text. Generic or unresolved declared types defer this check; no package code runs. Availability publication additionally confines available_on to execution-sourced data outputs (AvailableOnPosition); lists must be nonempty (AvailableOnEmpty), distinct (AvailableOnDuplicate) and reference actual declared or implicit exec outcomes (AvailableOnOutcome). Omission resolves to then and fails if custom outcomes or kind rules remove it. This is metadata consistency, not dominance or runtime availability analysis. Passive saved signatures remain deserializable for repair. Type validity stays with inference diagnostics. The built-in control nodes (branch, sequence, fork, count, while, yield, wait, break, continue) are registered as data through a host-only path that packages cannot use; Get, Set and Update are registered with ports derived from the referenced variable declaration (Update supports all nine operation tokens with operation-specific type eligibility), and a definition lists its identity-reference fields (`references`: an RFC 6901 pointer into the config and a variable, event or function kind). `build` rejects an invalid or duplicate pointer. `references` is the one parser for those fields, and `contract` computes contextual contracts on lookup; nothing is cached beside the snapshot. `catalog::language` is the one table of language rules: every intrinsic ID with the kind and suspension function analysis counts it as, the declaration it references and whether it ends its region (Break, Continue), plus the implicit ports each kind gets. Intrinsics it lists but this core version does not register report `E-INTRINSIC-UNSUPPORTED`; their reference fields still come from the table. Plans record the document state and the snapshot generation. Not implemented: new-binding defaults, executable package constraint-provider integration, slot-keyed providers and the frozen interner.
Registered language intrinsics (V5) are `Ports::Declared` definitions, except `core/for-each` (`Ports::ForEach`, ports chosen by `config.mode`); 11 §8.4 lists the contracts, including the zone pairs. Generic ones (`switch` with `$K: eq`, `register`, `try`, `match-result`, `select`, `coalesce`) resolve `type_args` like any generic definition.

Hash lookups here are compile-time only. Publication validates namespace ownership, constraint coherence and converter ownership before the snapshot is visible ([04](04-expansion-plugins.md)).

The current port-shape publication check also rejects a declared finite variadic range when `min > max` with `RegistryError::InvalidVariadicBounds`, identifying its definition, port and limits. Equal bounds, `0..0`, and an unbounded `max` remain valid. Publication keeps passive persisted port descriptors unchanged. Input-value validation also checks resolved variadic port instance counts from persisted order (absent state is zero) against inclusive bounds in both directions, reporting E-VARIADIC-COUNT on node/base port. It never derives counts from edges, literals or the next counter, and never rewrites authoring state.

Implementation (00c.1): `orange_model::types::TypeRegistryBuilder` registers type metadata and interns concrete `TypeTerm`s; `build` validates schema references/member IDs and publishes an immutable `TypeRegistry`. The registry supplies the existing literal `Schemas` interface directly, preserving one record/enum layout model. Package registrations cannot replace core types or claim another package namespace. Handle constructors require registered handle-kind arguments. Recursive nominal schema references do not expand during registration; literal decoding/encoding retains its depth budget.

The public host-side `TypeKey` wraps a dense `u32` slot with an opaque registry owner token; lookup rejects a key from another registry even when its numeric slot exists there. Registries are not cloned into independently mutable identity spaces. This checked host wrapper is not a wire representation or a change to future dense execution-plan indexes; keys are never persisted. Resolution rejects unresolved parameters, unknown names/majors, wrong arity and nesting beyond the existing parser budget. Read-only validation and owner/kind queries also accept uninterned terms, so inference need not mutate a published snapshot.

Constraint proofs (00c.2a) use the same depth budget for nested terms and nominal-schema traversal. Publication rejects unavailable/cyclic/over-budget declared structural derives and invalid map key constraints before exposing the snapshot. `supports_term` returns a capability witness rather than invoking a kernel; the current numeric-operation proof set is add/subtract/multiply/divide, with other math operations still requiring their own implemented support. This is an implementation surface, not an exclusion of later math node families or performance evidence.

Explicit proof registrations (00c.2b) carry `ConstraintImplementationRef`: the existing `ProviderBinding`, exact content digest and stable implementation ID. `ConstraintId` distinguishes core constraints from canonical versioned package constraints. A package may implement a core constraint only for a type it owns, and its own constraint for foreign types. Duplicate implementation slots and derive/implementation conflicts fail before publication. Numeric operations have separate slots, so a `num` declaration does not manufacture an add/multiply implementation. The future host authenticates the supplied authority and resolves code; this model validates coherence and preserves exact references only.

## 9. Execution plan IR

Illustrative IR. The exact encoding is fixed in WU-00e, while the separation into pull programs, control blocks and dense slots is a contract. The compiler must require current analysis with full successful required-check coverage. Current `ValidationAnalysis` is intentionally never compile-ready; its missing checks are not compiler approval (see §3.2).

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

Native prerequisite (N1a): `orange-bundled::math_definitions()` supplies the ordinary replaceable definition contracts, while `MathPackage::register` adds one exact provider with its scalar specialization table. `MathPackage::resolve` checks that provider/version/digest exists in the supplied snapshot, that the snapshot admits the type arguments, and that the config is supported. Its private `ResolvedKernel` stores the Arc snapshot, ProviderKey, operation, scalar kind and normalized policy enums. `evaluate` accepts declared-order scalar Values and returns one Value or a typed KernelError; it neither validates a graph nor constructs a plan. Successful scalar evaluation contains no dynamic metadata lookups or buffer allocations by construction; no latency/allocation benchmark is implied. Retaining an old binding keeps old metadata alive and never consults a newly published provider default. The host must authenticate build identity and supply fresh resolved graph inputs. The final dense compiler/kernel ABI, source maps and full runtime error envelope remain unimplemented.

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
| I-4 | Every non-intrinsic node has a signature snapshot, a provider binding and a lock entry, except the exact host-owned `core/convert` special form: it always has a signature; absent provider means exactly its registered Core converter implementation, and a package provider requires its exact lock. The `None` specialization bucket of `RegistrySnapshot::specializations` is inference metadata for core providers, not evidence that an ordinary node resolves. Edges to unknown ports are retained as orphan edges, and unavailable package/provider metadata leaves the node unresolved while its snapshot remains available for edge checks. |
| I-5 | `type_args` fully resolve every type parameter of a compiled node. |
| I-6 | Plans and snapshots are immutable. Run state is mutated only by its actor. Workspace state only by the host actor. |
| I-7 | No engine or UI entity ID, Rust `TypeId`, pointer, live handle or secret is ever persisted. |
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

Requirements: FR-05, FR-08, FR-09, FR-44–51. Tests: G-suite in [11 §18](11-graph-language.md), V01/V02/V05 in [06](06-quality-and-validation.md). ADR-06, ADR-13–17 in [08](08-decisions.md).

### E1a concrete subset

orange-compiler::PurePlan is private-construction, immutable and not serializable. It retains source graph/content state/revision, Arc registry snapshot, dense u32 NodeIx/SlotIx, slot types, node/port source maps, incident input EdgeIds, ordered bound steps, export slots and folded values or ConstantFault records. It has no Default or public constructor. All arrays are read-only through public accessors. The source revision is the compilation invocation's revision; freshness uses opaque content-state and registry-generation identities. Undo can restore a prior content identity and make its matching analysis reusable; evidence for the edited state is then stale. Saved type_args are never executable binding evidence.

The provisional PureKernel trait takes Value input/output slices and returns a bounded static KernelFault; PureKernelResolver returns ordered ports and an Arc executable. Trusted implementations promise deterministic, bounded, effect-free behavior. This is the implemented folding subset of the target kernel/ExecutionPlan sketches above, not their final ABI. Slot/operation/admission quotas and cancellation are enforced; constant-byte accounting is conservative retained-value accounting, not a hard allocator or CPU limit. No plan hash, function/control regions, memo epochs or scheduled runtime are supplied yet. Lowering uses O((nodes + edges + ports) log(nodes + edges + ports)) ordered-map/set work and linear retained storage, excluding validation, callback work and value payload size; no throughput/NFR claim is inferred.

E2b public result subset: PureEvaluation has private fields and no Default/public constructor; PurePlan::evaluate is its checked entry. It borrows plan storage and an optional AtomicBool cancellation signal. EvaluatedExport contains borrowed id/source/value-or-ConstantFault references plus PlanType. Dense SlotIx access stays internal to the checked plan, and export iteration preserves persisted order. EvaluationOptions limits the number of exposed export records, not repeated reads, downstream copies or process memory. EvaluationError has StaleDocument/StaleRegistry/ResourceLimit/Cancelled variants with safe static fallbacks. A running immutable result view keeps the old registry snapshot; new starts use the host's current snapshot. There is no persistence/cache encoding change and no heap frame or callback execution on this result-read path (source inspection, not allocator qualification).

E1/E2d concrete lazy subset: PureStep.inputs and its KernelBinding.inputs now contain the actual demanded port projection for compiler-owned core/select/and/or steps; ordinary bindings retain their full fresh signature ABI. Lazy projections are specialized to a single immutable deciding value (or fault), not reusable dynamic branch instructions. Slot sources still name the actual exported node/port and preserve the original failing producer separately. Dead alternatives have no executable binding or materialized slots; another export can demand them later in the same compile, sharing memoized producers. Public Rust fields/method signatures and graph JSON remain unchanged; generic scalar/JSON type support is unchanged. Admission still bounds the entire potential cone and can reject a limit even when actual selected work would be smaller. There is no executable guest/lazy extension API or serialized-plan ABI change.

E1/E2e concrete option subset: PlanType includes Option(Box<PlanType>) and is Clone rather than Copy. Parsing accepts only nested core/option@1 over supported scalar/JSON leaves, up to MAX_NESTING (64); there is no generic collection/Result acceptance. Slot value checks enforce the section 7 None/unboxed/boxed representation. Literal admission walks only the supported option envelope and scalar payload by reference before decoding, without serializing the JSON payload. Retained-value charging includes each nested Some child and JSON text, even when Arc storage is shared. These remain conservative charges, not process-allocation guarantees; type/source metadata and trusted callback costs retain their separate limits.

EvaluatedExport.ty is now &PlanType, borrowed from the private checked type array; reading an export does not clone a recursive type tree. Coalesce bindings record only demanded fresh ports, unwrap exactly one present option layer, and retain the existing immutable constant-specialization limitation. The JSON document schema and Value/literal representation are unchanged.

E2g1 concrete runtime subset: orange-runtime currently contains only entropy primitives. EntropyLayout is immutable Arc-owned UUID/order metadata plus a binding-time lookup map. Private EntropySlot fields retain that layout and a dense index; ActivationEntropy accepts only matching-layout slots, owns per-node next-invocation/memo state, and changes epochs by checked generation stamps. Bound sampling uses indexed state and stack hashing, with no UUID/name map lookup or source-document access. This is source inspection, not allocator/NFR qualification. No scheduler, Run/Activation/Strand, variables, host services or graph execution is implemented by the presence of the crate. Public errors have safe static fallbacks and core/entropy-* message keys.

N1/E2g2 concrete metadata subset: NodeDefinition adds volatility: Option<Volatility> and determinism: Determinism. Both are explicit public Rust construction fields; no Default infers execution authority. RegistryBuilder validates classification after the established descriptor/default/availability checks, and immutable snapshots retain the full definition. Stored port-only signatures and NodeContract projections remain unchanged. Core variable-read registration explicitly chooses StateRead through VariableAccess::Read; bundled scalar definitions declare Deterministic/None. Package entropy is Seeded and may have inputs; host state/clock templates must prove no data input. These declarations are not provider authentication/conformance evidence.

N1/E2g3 concrete binding subset: orange-compiler exports EntropySource, EntropyKernel, EntropyBinding and EntropyKernelResolver separately from pure kernels. Executable binding ports must match fresh ordered semantic inputs/outputs. ResolvedEntropyKernel in orange-bundled retains a registry Arc and exact ProviderKey; binding checks versions, empty config/specialization, class and presentation-stripped port semantics. The raw operation forwards one u64 from the supplied host source only after validating ABI shape. No source-document/type/provider lookup occurs in the native callback. This does not construct a checked dynamic graph plan or provide runtime memoization by itself.
