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

Combined validation is produced by `analyze(document, registry)`, `analyze_with_effects` or `analyze_with_options`; the legacy `validate` and `validate_with_effects` functions extract the same `ValidationReport` from this orchestration. `ValidationAnalysis` privately binds the report and immutable actual coverage to transient document `semantic_state_id` and registry `generation`. It exposes `covered_checks()` and `missing_checks()`; required checks are listed by `required_checks()`. For supported control documents, coverage is declaration types, state metadata rules, node resolution, input values, connections, function summaries, definition and config, body domains, contextual contracts, function restrictions, regions, data-dependency cycles, control cycles, value availability, stream rules and entry policies and exports. The region pass (`validate_regions`) exposes its `RegionIndex` as `ValidationReport.regions` for the availability pass; zone structure and switch cases are enforced, so `Regions` is covered (11 §7.1). The control-cycle pass (`validate_control_cycles`) also runs and publishes closed node witnesses as `ValidationReport.control_cycle_paths`; `ControlCycles` is covered because every 11 §8.1/§8.2 control-cycle rule is enforced (`E-CYCLE-SYNC-CONTROL`, `W-BUSY-LOOP`, and `E-RECURSION` via function summaries). `ConstantEvaluation` remains missing because `W-CONST-ERROR` and error preservation require compiler constant folding (11 §7, §11; 08). `StateRules` covers the implemented metadata checks. `is_ready_for_lowering()` permits a fresh, valid analysis with only `ConstantEvaluation` missing to enter compiler analysis; it does not authorize plan publication or execution. Graph-local functions are accepted as call contracts callee first: each function body is checked once, with its callees already accepted or rejected, and the whole-document passes then run once at that fixed point. This avoids one whole-document round per call-chain level; it is not a linear-time guarantee for individual validation passes or transitive effect summaries. Whole-document rounds remain as a fallback that converges to the same result. Body-domain validation is a prerequisite: any unsupported body stops combined control analysis with no resolved signatures or summaries, all nodes/functions marked incomplete, and E-DOMAIN-ANALYSIS-BLOCKED subjects explaining the skipped analysis. Only BodyDomains is then covered; sixteen checks remain unexecuted. The private immutable coverage list belongs to the invocation, rather than a global completion claim. Hosts may still use standalone structural inspection APIs on preserved data. `is_ready_for_compilation()` requires matching identities, a pass-local valid report and no missing checks, so it remains false until all required checks are implemented. The report accessor is read-only, and a default or extracted report cannot construct analysis evidence. Unknown domains remain unsupported and never become ready through control-domain checks. Host-provided leaf effects are bound to the analysis invocation but are not authenticated by it. Data-dependency analysis uses only fresh resolved signatures and same-body valid data edges, expands sparse concrete variadic keys and connects each data input to computed outputs only. Wide computations use an internal hub to represent this complete relation with a linear number of dependency edges; narrow computations retain direct edges. Hubs never appear in the concrete-port witnesses. It returns one deterministic closed port witness per cyclic component in ValidationReport.data_cycle_paths and E-CYCLE-DATA; non-computed output sources break dependencies. Missing metadata stays incomplete and invalid connection edges are not guessed into D. No stored signature fallback, control-cycle, region, availability or runtime guarantee follows from this pass.

Contextual contracts are computed, never stored. A definition declares its ports (`Ports::Declared`) or derives them from a referenced declaration (`Ports::Variable`), and `RegistrySnapshot::contract(document, node)` returns the node's contract as a pure function of the definition and the document. Get resolves `value` as a state output typed by the referenced VariableDecl; synchronous Set requires `value` as an input and rejects workspace targets with `E-VARIABLE-SCOPE`; Update Add/Sub/Min/Max resolves required `value` input and execution-sourced `old`/`new` outputs from the current local/run declaration, rejecting workspace scope and requiring exact numeric-operation evidence for Add/Sub or Ord evidence for Min/Max (11 §11.1). A missing or malformed reference reports `E-VARIABLE-UNKNOWN`. Two nodes of one definition therefore have independent contracts, a declaration edit or undo changes the result on the next lookup, and there is no overlay, fingerprint or staleness state to maintain. Inference, edit persistence, connection planning and node validation all read this one lookup. Computing a contract changes no document fields, revision, state identity or undo history. Compare-and-set additionally projects required `expected: T` before `value: T` and an execution-sourced `exchanged: core/bool@1` output on `then`, with exact Eq evidence; the `old`/`new` types remain declaration-owned. And/Or/Xor use the ordinary Update port shape only for exact core bool/integer types. Append retains that shape for exact string/bytes/list concatenation; list element types need validation but no equality or numeric proof. Shared WriteShared/UpdateShared projections accept workspace declarations only, are Async/Maybe and reuse these data/type rules without establishing a host actor. Their exact language facts also classify function effects; workspace writes never enter run_writes. Call-site authoring now uses a separate type-only `catalog::call_interface` projection: target FunctionId, declared parameters/constraints and data descriptors, with no kind/suspension/effects and no output provenance. It validates identity, target, supported domains and coherent data interface shape; malformed targets never fall back to saved snapshots. Inference scopes one instantiation per call node and persists arguments only. Variable/function declaration changes discover dependents through typed before/after references, including removals; calls losing a generic interface clear cached arguments atomically with the edit. Pins and saved signatures remain untouched. Executable call contracts come only from accepted callees (complete, conformant and free of function errors); a generic callee keeps its type parameters rigid in its own body, and each call instantiates them (11 §14). Connection validation now consumes fresh node resolution, including declaration-owned Get/Set/Update ports; only unresolved package nodes fall back to stored signatures for offline inspection. `analyze(document, registry)` / `analyze_with_effects` produce `ValidationAnalysis`; the legacy `validate` wrappers extract its same report for compatibility. It supplies no function leaf effects by default, so a package action leaf remains incomplete (`E-FUNCTION-METADATA-UNAVAILABLE`). `ValidationReport::is_valid()` describes only passes actually run; see the coverage contract above.

Projection caching (2026-10-24; NFR-15): within one node pass, nodes whose registry-declared contract, solved type arguments and body are equal share one projected snapshot and its type-validation outcome. A failed projection is still reported on every node that uses it. Drift is decided per node by comparing semantic keys, exactly what `signature::compare` reports. The projected key is computed once per shared projection, each stored snapshot's key once per document signature key, and neither is computed when no snapshot is stored. Edit persistence likewise shares one projected snapshot and key per (definition, arguments) within an inference component. Contextual contracts (variables, events, converters, for-each, calls and markers) are still projected per node. These caches live for one pass; nothing is kept between edits.

Inference likewise resolves each node template and each data edge (both endpoint terms and whether either mentions a variable) once per `infer_components` call, shared by the component search and equation building. The unifier borrows equation context and clones it only into an error. Argument checks are shared across nodes with the same parameter declaration, solved type and body, and a failed check is reported under each node's own ID. Edges whose ends are both concrete still never become inference evidence, even at a generic node; they are the validator's to check.

Analysis session (V12a and bounded V12b, 2026-10-25; NFR-15). `validation::AnalysisSession` keeps the combined analysis of one document across committed edits.

- **Lifecycle:** `new` uses the full analysis pipeline after admission. `update(document, registry, change, options)` accepts every committed `ChangeSet` in commit order, including undo/redo. Semantic evidence binds `semantic_state_id`; edit lineage and indexes separately bind whole-content identity and revision.
- **Invariant:** after an update the session's `analysis()` equals `analyze_with_options` for that document, registry, leaf effects and options, including diagnostics, ordering, summaries, cycle witnesses and coverage. A stale/rejected session cannot manufacture compiler readiness.
- **Layout-only changes** preserve the existing semantic evidence and visit no validation units. Proven content bindings advance without mutating analysis identity. An invalid retained index is dropped for a later rebuild rather than scanned during this fast path.
- **Semantic changes** reuse unchanged configuration and variable/event declaration-type buckets. Both paths call the same unit validators. Changed nodes and replaced/removed function bodies invalidate their configuration units; changed declarations invalidate their type units. Bucket refresh still visits current IDs. Resolution, inputs, connections, function acceptance, cycles, regions, availability, state, streams and exports remain full passes; no complete V12b or NFR-15 claim follows.
- **Scoped literal edits** (2026-10-30): a journal-marked edit of literals of root-body nodes re-resolves only those nodes, their neighbours and their generic components, and patches their input diagnostics into the published list when nothing else changed; otherwise the full pipeline runs ([16 §3](16-incremental-validation.md)).
- **Scoped edge edits** (2026-10-30): a journal-marked edit that only adds or removes root data edges in a clean report checks each edge alone (connection, fan-in, stream fan-out, cycle, export and availability conditions) and patches its targets' input diagnostics; otherwise the full pipeline runs ([16 §3](16-incremental-validation.md)).
- **Scoped type edits** (2026-10-30): persisted type arguments and type pins join literals in the journal's node edit and take the same scoped path; validation reads arguments only for admission and pins only for admission and inference ([16 §3](16-incremental-validation.md)).
- **Scoped configuration edits** (2026-10-30): a provider-bound node's configuration joins the node edit; its configuration diagnostics are patched beside its inputs. Intrinsic configuration, which structural passes read, still runs the full pipeline ([16 §3](16-incremental-validation.md)).
- **Rebuilds:** full analysis handles registry generation, limits, effects, shared-signature/package-lock changes, missing/foreign/skipped/replayed edit lineage and cancelled requests. Every update path checks current cancellation before reuse, including layout and unchanged-effects calls. Fresh deterministic admission/domain rejections remain current across layout/undo/redo and unchanged-effects updates under the same inputs, while compiler readiness remains false. A cancelled request requires recovery; a semantic/input change reruns prerequisites. Both analysis routes share cancellation/admission/domain handling and borrow retained host effects before these checks. Host leaf effects remain trusted facts, not authenticated by the session.
- **Index/planning:** one optional `DocIndex` is constructed only after successful analysis admission/domain prerequisites, then updated from proven edits. `index()` borrows it or returns None. `plan_edit()` reuses it for journaled inference trials and never publishes trial analysis; errors roll back, and unwinds invalidate the planner index for later rebuilding. Trial planning has its existing caller-controlled budgets rather than silently inheriting combined-analysis admission.
- **Counts:** `EditStats::input_nodes/input_edges` are distinct input cardinalities, not actual pass visits. `full_analysis_runs` counts orchestration attempts (including rejection), `index_rebuilt` reports rebuilding, and `local: LocalWork` records actual cache visits, computations and hits for configuration/declaration units only. Other passes' visits are not instrumented by these counters.
- **Reads:** `materialize()` clones the report into owned compiler evidence. `diagnostics()`, `diagnostics_for(subject)`, `resolved(node)` and `analysis()` borrow current results. `changed_subjects()` reports diagnostic-list changes, including removals; `changed_nodes()` reports resolved signatures added/changed/removed. Publication computes these sets by scanning current/previous result maps; the changed views do not imply O(delta) assembly. `ValidationAnalysis` is `PartialEq`.
- **Embedding:** the session and published facts remain Send + Sync. Session Debug reports only metadata/counts, excluding document values/effects. Background hosts must discard older semantic/registry/request results before publication. A running immutable plan remains pinned through document edits; this is not a UI thread, runtime inbox or incremental-preview implementation.

Review refinements: node configuration checks borrow their existing JSON object, sharing object-kind/enum/member rules with public value validation. Schema-dialect support is cached per exact registered definition in its immutable registry generation; unsupported node config versions are still skipped by this pass. Combined input checking reuses its retained incoming-edge index and appends diagnostics to one pass buffer with one final sort; standalone callers keep an equivalent scoped fallback. Invalid wires still shadow literals within the same scope. Both change views merge sorted old/new maps once, comparing each shared value once; publication remains a full result scan.

Declaration-driven inference invalidation (C2): `plan_edit` compares declaration IDs and contents across the before/after documents. For each added, changed or removed variable declaration it scans typed `RegistrySnapshot::references` over nodes in every body in both states, then seeds the referenced consumers and their incident edge endpoints into existing component inference. This correctness-first scan handles reference removal/config replacement and cases where a removed Get can no longer expose a contextual port; stale cached `type_args` never seed inference. Updates stay in the declaration edit's single transaction, preserving pins and leaving unrelated components untouched. The scan cost is temporary implementation overhead; no reverse reference index is persisted and NFR-15 is unmeasured.


Function signature/effect evidence is captured for a document content state and checked before function analysis. A changed document must receive fresh host evidence; stale input produces incomplete functions and no usable summaries. This binds evidence lifetime, not authenticity: the host remains responsible for supplying truthful metadata from exact available providers.

| Index | Structure | Used by | Update cost |
|---|---|---|---|
| Dense document index | `NodeId → DocIx(u32)` slot map | All derived indexes | O(1) |
| Incoming / outgoing | `HashMap<PortRef, SmallVec<[EdgeId; 2]>>` | Fan-in/fan-out checks, deletion | O(1) per edge |
| Node incidence | `HashMap<NodeId, SmallVec<[EdgeId; 8]>>` | Remove node, copy/paste | O(degree) |
| Data topological order | Dynamic topological order over D ([11 §8.2](11-graph-language.md)); candidate: Pearce–Kelly | Incremental `E-CYCLE-DATA` on connect | Bounded by the affected order interval |
| Type components | Union-find over generic nodes joined by data edges | Inference scope | Near O(1); a split recomputes one component |
| Region index | Region root → member exec nodes; dominator tree per root (Cooper–Harvey–Kennedy iterative; used by availability, [11 §7.2](11-graph-language.md)) | Availability, Break/Return placement | Recomputed per affected root |
| Diagnostics | `Subject (node / edge / port / decl) → Vec<Diagnostic>` | Editor projection | Replaced per validated subject |

The spatial index, edge-path cache and widget state belong to `orange-editor`, not `orange-model` ([02](02-architecture.md)).

Implementation note (WU-00/00b.3): `orange-model` provides `DocIndex` with node location, incoming/outgoing edges and node incidence.

- **Kept beside the document:** the index lives next to the document, not inside it, so document equality, saving and undo never involve derived data. The one exception is node location: `GraphDocument::locate_node` and `node` use a node-to-body map built on first lookup and kept current by every node and function patch. It is never serialized or compared, so equality, saving and undo are unaffected, and lookups no longer scan every function body.
- **Updated from change sets:** each committed `ChangeSet` updates it, and an updated index always equals a fresh build (property-tested). The index records the content state it describes (`is_current`). An undo or plain `apply` that bypasses it leaves it stale; `apply_inferred_indexed` then rebuilds it before planning instead of reading stale edges. Planning applies the edit to the document, reads it and rolls it back. That rollback also runs if planning panics, and the kept index is then reset to one that is never current, so a host that catches the unwind keeps the original content and the next indexed edit rebuilds the index.
- **`BTreeSet` edge sets:** used instead of `SmallVec` so equality does not depend on edit history. Performance is evaluated with NFR-15.
- **Deferred rows:** the dense document index, data topological order, type components, region index and diagnostics arrive with their consumers (00c/00d).

Review correction (2026-10-23; FR-44/46, NFR-05/08/15): export activation analysis builds root-body incoming data adjacency once and traverses computed-output dependencies only. State outputs terminate the pull cone, so a register's write inputs cannot make its current value activation-scoped. No exports means no adjacency work. Availability groups distinct consumer subjects by activation root, producer and available outcomes, processes one reachability set at a time and releases it immediately. For V control-flow vertices, E edges, P producer/outcome groups and Q discovered use queries, this removes the retained P×V reachability cache; scratch storage follows V+E+Q, while traversal CPU can still follow P×(V+E). Shared lazy cones can increase Q. This is not a global analysis budget or NFR qualification. Resource containment traverses registered nominal record fields and enum payloads as well as type arguments; each nominal schema expands once per query so cycles terminate. Registers use fresh resolved cell metadata. [10](10-tooling-and-sources.md#security-and-performance-verification-2026-10-23) records measurements and remaining gaps.

Combined analysis applies `AnalysisOptions` before indexing/resolution. Defaults are 250,000 aggregate graph/inspected metadata items, 1,000,000 inspected JSON values, 64 MiB of inspected UTF-8 metadata, and a maximum squared item-count admission estimate of 25,000,000,000. Descriptors/defaults and matched definitions are charged per node, including referenced function interfaces. Configuration, literal and schema payloads are traversed by reference without serializing or cloning them. Counts use checked arithmetic. These are admission quotas, not elapsed-time, allocation or instruction-count guarantees; registry construction and host-owned type-schema traversal have separate lifecycles. Ignored editor/presentation data is outside the analysis quotas. Import limits remain independently necessary.

`analyze_with_options(document, registry, leaf_effects, options)` permits host overrides and an optional borrowed `AtomicBool` cancellation signal. Cancellation is polled during admission, between passes and between function bodies, with no bounded interruption latency within a pass. Legacy combined entry points use the default quotas. Standalone passes remain caller-controlled. Rejection/cancellation discards partial results, returns one global `E-ANALYSIS-LIMIT`/`E-ANALYSIS-CANCELLED`, no projections/summaries and empty coverage; both readiness predicates fail. No partial result may become compiler evidence. Hosts needing hard execution limits must isolate analysis in a worker; IPC alone provides no sandbox.

`validate_declaration_types` checks every variable type and event payload against the current type registry, including unused declarations without initializers. Syntax, registered identity, arity, unresolved parameters, handle kinds and map-key constraints are checked. Variable/event declarations have no generic parameter scope. Invalid types report `E-DECLARATION-TYPE` with a typed `variable` or `event` subject ID; existing nesting/derivation bounds report `E-TYPE-COMPLEXITY`. Messages expose neither declaration names nor raw type text. This pass is included in `DeclarationTypes` coverage and never rewrites saved declarations.

Compatibility: saved graph format is unchanged. The optional diagnostic `event` field is omitted when absent and older records deserialize without it; downstream Rust `Subject` struct literals need `event: None` or a default struct update. Existing combined entry points now enforce default admission quotas; hosts needing different quotas use `analyze_with_options`. The new lowering predicate supplements the existing final predicate, and coverage consumers must distinguish `DeclarationTypes`, metadata `StateRules` and compiler-owned `ConstantEvaluation` (08, readiness clarification).

### 3.3 Transactions and change sets

Whole-content `state_id()` binds edit indexes, connection proposals and undo inverses. The separate transient `semantic_state_id()` binds validation analysis, pure/data/control plans and execution entry. Layout-only transactions advance revision and whole-content identity but preserve semantic identity, so they require no revalidation or recompilation. Semantic transactions allocate both identities; undo/redo restore both exact prior identities. Clones retain them and loads allocate fresh identities. Identity counters are not saved fields and do not change format v1. `ValidationAnalysis::document_state()` now returns semantic identity.

Committed `ChangeSet` evidence has journal-owned construction and read-only getters. It records exact before/after content and semantic identities as well as revisions; consumers must not infer lineage from revisions alone. Node/edge/declaration deltas are borrowed through `nodes()`, `edges()` and `decls()`. `global_dirty()` reports shared-signature/package-lock patches requiring conservative invalidation. `is_committed()` distinguishes ordinary/undo/redo commits from private rolled-back trials. `DocIndex::update` rebuilds on mismatched identity lineage. This tightens the unfrozen Rust API: external field reads become getters and external mutation/construction is disallowed. No saved JSON field changes.

Both ordinary and scratch transactions roll back successfully applied commands if command iteration unwinds; the original panic resumes after restoration. Scratch observation has the same guarantee. This covers unwind-enabled builds, not process aborts. Returned errors preserve content, revision and both identities. Paste checks generated IDs against all target bodies/declarations and earlier generated IDs within each typed identity namespace before constructing keyed maps; collisions return `IdError::GeneratedCollision` rather than truncating copied bodies or replacing declarations. The model does not retry a failing host generator.


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
    // Private journal-owned provenance and deltas; exposed through read-only getters.
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
- `PutTypeLock(entry)` and `RemoveTypeLock(type)` make `type_lock` transaction-owned. Put appends a new type identity, is a no-op for the same codec and rejects a different codec (`TypeLockConflict`), so a saved interpretation is never replaced silently. Remove deletes an existing entry (`TypeLockNotFound` otherwise); changing a codec is an explicit Remove then Put, and undo restores the original position. Both mark the change set globally dirty.
- `UpsertExport(export)` replaces the export with the same `id` in place or appends it, and `RemoveExport { id }` removes it (`ExportNotFound` otherwise); a loaded document that repeats an export ID edits its first entry. The target is not checked by the command: analysis reports a missing node or unsuitable port. `SetActivation { node, policy }` sets or clears a node's activation policy, which the validator checks. All three are semantic edits.
- Import-only metadata: `format_version`, `graph_id` and unknown top-level fields have no command; they come from the loaded or newly created document and are preserved on save.
- Superseded snapshots are not pruned (00c.7a). A re-inference moves a node to another content-addressed snapshot and leaves the old one in `signatures`. Pruning inside the transaction would need a reference scan over every body (and function) on each commit and a remove-signature command, and it would churn the two snapshots that pin/unpin and undo/redo alternate between. The unreferenced entries are bounded by the distinct type combinations used, are inert (nothing resolves a snapshot that no node names), and are removed by the same undo that added them. Garbage collection belongs to an explicit compaction on save, together with the codec round-trip rules for unknown data; it is not part of this slice.
- `SetTypePins { node, pins }` (00c) replaces a node's explicit type choices; like every node edit it marks the node changed, so `inference::plan_edit` re-infers its component. An `UpsertFunction` reports only its function in the change set, so `plan_edit` treats every node of the added or replaced body as touched.
- `SetSignature { node, key }` (00c.4b) points a node at an existing snapshot (`PutSignature` first); a missing key is rejected. `inference::apply_inferred` commits an edit together with the `SetTypeArgs`, `PutSignature` and `SetSignature` commands its inference implies, so one undo restores the edit, `type_args` and signature references. `apply` itself never infers, and loading never runs inference.
- Inserting a node between connected endpoints composes disconnect, add-node and two connect commands in one transaction. A rejected replacement restores the original wire and content state; one undo restores the pre-insertion structure. The planner/validator separately checks compatible typed ports. Creating a reusable node from multiple nodes uses a typed `FunctionDecl` interface and `core/call`, not presentation grouping or copying the implementation into every caller (11 §14). Search/palette presentation belongs to the host/editor; this transaction contract does not claim those refactorings are implemented.
- Canonical JSON for `SignatureKey` (00b.3): object keys sorted by byte order, no whitespace, serde_json string escaping. This matches RFC 8785 for the ASCII keys and integers in snapshots. WU-03 publishes the cross-language digest vectors. Semantic projection removes `label`, `description`, `group` and `editor` and substitutes `$T` parameters from `type_args`.
- Paste gives every pasted node, edge and declaration a new ID. Only typed configuration reference paths declared by exact definition/config-version metadata are discovered and remapped. Host-owned intrinsic schemas cannot be replaced by package schemas. Unknown configuration and ordinary strings, including strings equal to UUIDs, remain data. Referenced declarations travel transitively with the copy and are reused when the target already has them. Signatures are re-keyed to their `sha256:` key. Lock entries of copied provider bindings are put into the target; a target that locks the same package version to another digest rejects the paste. Every source type lock travels with the fragment and is merged with `PutTypeLock`: the precise set of types a copy depends on is not computed yet, so all locks are kept rather than risk dropping a codec requirement. A matching target lock is reused; a different codec rejects the whole paste. An unavailable codec therefore stays unavailable (`E-TYPE-CODEC`) in the target.

Import integrity: each package `(id, version)` and each type-lock identity occurs once, including when duplicate rows are identical; ambiguous locks are a structural load error. Provider resolution must not choose the first matching row. A JSON object that repeats a key at any depth, compared after unescaping, is CodecError::DuplicateKey naming the first repeat; serde maps would otherwise keep the last value silently, as a merge could produce. The check runs in the envelope pass after the format/version gate, so a newer document still reports its version. JSON-typed values are stored as text and are not inspected. The default importer checks a 64 MiB UTF-8 byte budget before parsing and then checks 100,000 nodes and 500,000 edges across all bodies. `from_json_with_limits` lets an embedding host choose explicit budgets. Structural counts are post-deserialization checks, not allocation limits; a host reading a file or stream must also bound that read before constructing the input string. These defaults are safety limits, not supported performance targets.

Construction boundary: `GraphDocument` implements Serde `Deserialize` by reading its wire layout and then running the same admission as `from_json`: an optional `format` tag must name `orange.graph` and is dropped, the version must be 1, node and edge IDs must be unique across bodies and lock identities must be unique. A rejected document is a deserializer error, and no unchecked deserializer is public. Duplicate-key detection and import quotas need the original text, so they stay with `from_json`/`from_json_with_limits`; hosts reading untrusted bytes use those. Serde `Serialize` output is unchanged and has no envelope tag.

Nesting budget: a saved document nests at most `MAX_NESTING` (64) JSON objects and arrays, counting its root object as 1. Every transaction patch is checked against the depth its entry will have in the saved document, including the containers above it (a function body adds two), before it is applied, so a rejected edit (`TxError::NestingLimit`) changes nothing. The importer measures nesting in its streaming pass and reports `CodecError::NestingLimit` after the quotas; Serde construction applies the same budget. What an edit commits therefore saves and reloads. Nesting deeper than the JSON parser's own recursion bound still fails as a JSON error. Byte and node/edge quotas remain import policy: a document edited past the default quotas reopens with `from_json_with_limits`.

### 3.4 Foundation authoring projections (ADR-22)

Variable declaration controls/nodes refer to the existing stable VariableDecl, and callable reusable nodes use FunctionDecl and exact package/provider locks (11 §11/§14). A declaration authoring projection adds no new runtime instruction or initialization order. Selection export must discover references through the definitions' declared reference fields, remap IDs atomically, preserve unresolved payloads and retain provider locks. Event-containing templates insert ordinary nodes and declarations rather than pretending to be callable functions. Exact library/template/editor encodings remain unimplemented and must be specified with schemas and focused round-trip/undo tests in WU-05 before adoption. This scope update changes no current format v1 field or Rust API.

## 4. Definition schema

`NodeDefinition` fields: `definition_id`, `definition_version`, `config_version`, `kind`, `suspends` (`never`/`maybe`/`always`; async and iterator), `cleanup_safe`, `volatility` (volatile only), `type_params`, `ports`, `iterator` (iterator only), `event_source` (entry only), `port_template` (optional declarative dynamic ports), `config_schema`, `errors`, `capabilities`, `determinism` (`deterministic`/`seeded`/`external`/`nondeterministic`), `cost` (optional hint: `scalar`/`linear`/`heavy`), `implementation_slot`, `label`, optional `description`, `editor`. All human-facing label/help/category/alias fields use TextRef from [13 §2](13-internationalization.md), including port labels/descriptions and declarative configuration-option captions. Implicit kind ports ([11 §4.1](11-graph-language.md)) are never listed; their text comes from host catalogues.

Implementation note (`orange_model::catalog`; Rust spelling only): `NodeDefinition` implements a subset: the `DefinitionRef`, `config_version`, `kind`, typed `volatility`/`determinism`, `suspends`, `type_params`, `ports` (declared, or derived from the referenced variable declaration for Get, Set and supported Update operations), `references` (the config fields that hold declaration IDs), an optional core `specializations` list (`None` means constraints decide) and an optional `config_schema` in the dialect below. Data and event ports carry a type; exec ports carry none. The remaining fields above are not implemented yet. Implicit kind ports are not listed in a package contract.

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
- An inference plan follows the same freshness rule. `plan_edit` and `AnalysisSession::plan_edit` record the content state and registry generation they planned against, `InferencePlan::commands` is read-only, and `inference::apply_plan(document, registry, plan)` commits it as one undo step or rejects it with the same `PlanApplyError`. Any edit since planning makes it stale, a layout-only edit included: the plan was derived from exact content, so the host plans again rather than rebasing. `apply_inferred` still plans and commits in one call.

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

An explicit `type_lock` must resolve to the exact codec implemented for its registered type. `TypeRegistry::literal_codec_identity` queries that capability, currently the structural codec `literal_codec::CODEC_ID` (`orange.value.v1`) for validated registered types, including registered record/enum schemas. Unknown types or codec identities report `E-TYPE-CODEC` and block lowering; saved locks and raw values still round-trip. Format-v1 documents without explicit locks retain the existing structural-codec default. All explicit locks are checked, including unused locks; the document's declared interpretation must be available before publication. Installing metadata alone does not install a decoder. Additional codec implementations require a registry/decoder extension, never an arbitrary claimed codec string. Aggregate decoding and encoding borrow constructor arguments rather than cloning their nested type trees for each value.


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
- **Map keys:** maps are built only by `MapRepr::new` (or empty by `Default`), which rejects a repeated key (`DuplicateKey`; the codec reports `LiteralError::DuplicateMapKey`). Up to 16 entries are compared pairwise; larger maps use one presized hash set. `Value` implements `Eq` and `Hash` consistently with its structural equality, so the check takes expected linear time. Because keys are unique by construction, encoding, shape checks and equality never search for duplicates.

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
    None, Some(Arc<Value>),                  // only payloads that can be a bare None
    Err(Arc<NodeError>),                     // only inside result<T>
    Ok(Arc<Value>),                          // only ambiguous result success payloads
    Json(Arc<JsonRepr>),
    Handle(HandleId),
}
pub enum ListRepr { Bool(BitBox), I32(Box<[i32]>), I64(Box<[i64]>), U32(Box<[u32]>), U64(Box<[u64]>),
                    F32(Box<[f32]>), F64(Box<[f64]>), Str(Box<[ArcStr]>), Values(Box<[Value]>) }
pub struct RecordRepr { pub ty: TypeKey, pub fields: Box<[Option<Value>]> } // schema order; None = absent
pub struct MapRepr { /* insertion-ordered; unique keys, restricted per 11 §5.2 */ }
pub enum JsonRepr { Null, Bool(bool), Number(NumberText), String(ArcStr), Array(Box<[JsonRepr]>), Object(/* ordered */) }
```

- `option<T>` is `None`, or the unboxed `T` value (static types disambiguate). `result<T>` is `Err(…)` or the unboxed `T`. A layer boxes only when its payload type can itself show the layer's own tag bare: `option<T>` uses `Some(Arc<Value>)` when a valid `T` can be a bare `None`, and `result<T>` uses `Ok(Arc<Value>)` when a valid `T` can be a bare `Err`. The two rules compose through each other. A bare `None` comes from an option, or from a result whose success is unboxed and whose payload can be a bare `None`. A bare `Err` comes from a result, `core/error`, or an option that does not box and whose payload can be a bare `Err`. So `option<option<T>>` and `option<result<option<T>>>` box Some, while `option<result<result<T>>>` does not, because the boxed `Ok` already hides the inner None. Box selection is static, including a successful None payload. Every compact value therefore decodes to one semantic value, so Some(Ok(None)) cannot collapse into None, and Ok(Err) cannot collapse into an outer Err. **Missing** exists only as an absent record field or JSON key, never as a free-standing `Value`. Together these make None, missing, JSON null and Error distinct by construction (FR-08).
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

Value shape ownership (2026-10-27; `core/data-shape`):
- `orange_model::value::shape::ValueShape` replaces the compiler's `PlanType`, with the same variants. Earlier notes that say `PlanType` now refer to it.
- A shape resolves from the parsed `TypeTerm`, never from type-expression text.
- The model owns, in one module:
  - the bounded value check and its nominal charges: `VALUE_CHARGE`, `MAP_ENTRY_CHARGE` and `ERROR_CHARGE`;
  - checked equality;
  - the error-code rule;
  - the option/result boxing rule above.
- `ListBuilder` is the one list packing. The literal codec and runtime builders both use it, so every producer stores scalar and string elements natively.
- `Value`, `ListRepr`, `ValueShape`, `ValueCheckError` and `ListPushError` are `#[non_exhaustive]`, because they will grow with records, enums, handles and OPEN-15. Language-closed enums stay exhaustive: node kind, port kind, output source and zone kind.
- Package records, enums and handles still have no executable shape. The compiler refuses them before binding.

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

Implementation note (`orange_model::catalog`; subset, Rust spelling only): `RegistryBuilder` collects the type registry, converters, host origin, node definitions and providers; `build` freezes them into an immutable `RegistrySnapshot` with a process-unique `generation` that changes only when a snapshot is built. Lookups are by definition and config version, provider key (`ProviderDescriptor` with its implemented definitions and specializations) and exact provider support. `build` rejects duplicate definitions and providers, package definitions in the `core` namespace or replacing an intrinsic, duplicate port IDs, implicit exec port IDs outside intrinsics, a data or event port without a type or an exec port with one, declared data outputs without a source (`RegistryError::MissingDataOutputSource` identifies the definition and port), event inputs (`RegistryError::EventInputPort` identifies the definition and port), package lazy ports (`RegistryError::LazyPackagePort`; explicit false is non-lazy), incompatible suspension metadata (`RegistryError::SuspendsMissing` or `SuspendsInvalid` identifies the definition and kind/value), exec ports on `pure` definitions (`RegistryError::PureExecutionPort` identifies the definition and port), undeclared `$T` type parameters in a port type, and providers naming an unknown definition. New definitions cannot publish missing async/iterator suspension metadata or incompatible values; when such metadata appears in a passive saved signature, validation still reports the existing `E-SUSPENDS-MISSING`/`E-SUSPENDS-INVALID` diagnostic and leaves the bytes repairable. Default literals are confined to data inputs (`RegistryError::DefaultPortPosition`); true optional flags require a data input (`OptionalPortPosition`) and a well-formed exact Option wrapper with one argument (`OptionalType`). False optional flags are inert. These rules run after established descriptor checks; malformed type syntax and type-name resolution stay with inference diagnostics. For fully registered concrete declared types, publication also checks exact default type equality (`DefaultType`) and canonical decoding against the current type registry (`DefaultLiteral`). The errors identify definition/port without retaining payload text. Generic or unresolved declared types defer this check; no package code runs. Availability publication additionally confines available_on to execution-sourced data outputs (AvailableOnPosition); lists must be nonempty (AvailableOnEmpty), distinct (AvailableOnDuplicate) and reference actual declared or implicit exec outcomes (AvailableOnOutcome). Omission resolves to then and fails if custom outcomes or kind rules remove it. This is metadata consistency, not dominance or runtime availability analysis. Passive saved signatures remain deserializable for repair. Type validity stays with inference diagnostics. The built-in control nodes (branch, sequence, fork, count, while, yield, wait, break, continue) are registered as data through a host-only path that packages cannot use; Get, Set and Update are registered with ports derived from the referenced variable declaration (Update supports all nine operation tokens with operation-specific type eligibility), and a definition lists its identity-reference fields (`references`: an RFC 6901 pointer into the config and a variable, event or function kind). `build` rejects an invalid or duplicate pointer. Within an array, a token selects an element only as an RFC 6901 §4 index (`0`, or digits without a leading zero); `+0`, `01` and `-` select nothing, while an object key spelled that way stays an ordinary key. `references` is the one parser for those fields, and `contract` computes contextual contracts on lookup; nothing is cached beside the snapshot. `catalog::language` is the one table of language rules: every intrinsic ID with the kind and suspension function analysis counts it as, the declaration it references and whether it ends its region (Break, Continue), plus the implicit ports each kind gets. Intrinsics it lists but this core version does not register report `E-INTRINSIC-UNSUPPORTED`; their reference fields still come from the table. Plans record the document state and the snapshot generation. Not implemented: new-binding defaults, executable package constraint-provider integration, slot-keyed providers and the frozen interner.
Registered language intrinsics (V5) are `Ports::Declared` definitions, except `core/for-each` (`Ports::ForEach`, ports chosen by `config.mode`); 11 §8.4 lists the contracts, including the zone pairs. Generic ones (`switch` with `$K: eq`, `register`, `try`, `match-result`, `select`, `coalesce`) resolve `type_args` like any generic definition.

Hash lookups here are compile-time only. Publication validates namespace ownership, constraint coherence and converter ownership before the snapshot is visible ([04](04-expansion-plugins.md)).

The current port-shape publication check also rejects a declared finite variadic range when `min > max` with `RegistryError::InvalidVariadicBounds`, identifying its definition, port and limits. Equal bounds, `0..0`, and an unbounded `max` remain valid. Publication keeps passive persisted port descriptors unchanged. Input-value validation also checks resolved variadic port instance counts from persisted order (absent state is zero) against inclusive bounds in both directions, reporting E-VARIADIC-COUNT on node/base port. It never derives counts from edges, literals or the next counter, and never rewrites authoring state.

Implementation (00c.1): `orange_model::types::TypeRegistryBuilder` registers type metadata and interns concrete `TypeTerm`s; `build` validates schema references/member IDs and publishes an immutable `TypeRegistry`. The registry supplies the existing literal `Schemas` interface directly, preserving one record/enum layout model. Package registrations cannot replace core types or claim another package namespace. Handle constructors require registered handle-kind arguments. Recursive nominal schema references do not expand during registration; literal decoding/encoding retains its depth budget.

The public host-side `TypeKey` wraps a dense `u32` slot with an opaque registry owner token; lookup rejects a key from another registry even when its numeric slot exists there. Registries are not cloned into independently mutable identity spaces. This checked host wrapper is not a wire representation or a change to future dense execution-plan indexes; keys are never persisted. Resolution rejects unresolved parameters, unknown names/majors, wrong arity and nesting beyond the existing parser budget. Read-only validation and owner/kind queries also accept uninterned terms, so inference need not mutate a published snapshot.

Constraint proofs (00c.2a) use the same depth budget for nested terms and nominal-schema traversal. Publication rejects unavailable/cyclic/over-budget declared structural derives and invalid map key constraints before exposing the snapshot. `supports_term` returns a capability witness rather than invoking a kernel; the current numeric-operation proof set is add/subtract/multiply/divide, with other math operations still requiring their own implemented support. This is an implementation surface, not an exclusion of later math node families or performance evidence.

Explicit proof registrations (00c.2b) carry `ConstraintImplementationRef`: the existing `ProviderBinding`, exact content digest and stable implementation ID. `ConstraintId` distinguishes core constraints from canonical versioned package constraints. A package may implement a core constraint only for a type it owns, and its own constraint for foreign types. Duplicate implementation slots and derive/implementation conflicts fail before publication. Numeric operations have separate slots, so a `num` declaration does not manufacture an add/multiply implementation. The future host authenticates the supplied authority and resolves code; this model validates coherence and preserves exact references only.

## 9. Execution plan IR

Illustrative full IR. The exact encoding is fixed in WU-00e, while the separation into pull programs, control blocks and dense slots is a contract. Lowering requires fresh successful semantic evidence through `is_ready_for_lowering`; the compiler additionally owns constant evaluation, preserved faults and executable-vocabulary proof before publishing its checked plan. The model's general `is_ready_for_compilation` remains false and is not an alternate approval path (see §3.2). The full IR below is not an implementation inventory.

Concrete control subset (2026-10-24): `ControlPlan` privately owns a frozen `DataPlan`, dense `ControlIx` adjacency, source node IDs, ordered Sequence targets, Count/While metadata, declaration-owned variable layouts and a pre-resolved host-input source table. Consumption roots use original document ports without adding saved exports. Runtime input slots project Get, loop indices and failed-output Error values; control admission checks their region/outcome availability first. Unfrozen `PlanType` adds Duration/Error; no saved JSON/schema version changes. Unsupported executable families fail closed before resolver callbacks.

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

Structured-concurrency implementation separates activation-owned data evaluation, local cells and execution outputs from strand-owned control cursors, pending input snapshots and waits. The actor lends one shared activation state exclusively while executing a strand; it introduces no shared-state lock or new provider interface. A Fork must not initialize another activation or replenish its entropy/fuel.

Live region instances now retain suspended structured continuations explicitly. Sequence steps and loop bodies retire before their caller resumes; their frames preserve the logical strand identity. `RunLimits::max_strands` (default 4096, including suspended continuation frames) and `max_regions` (8192) bound retained ownership, including the separately reserved On Stop entries. These additive unfrozen Rust fields require updates to exhaustive host struct literals; saved graph JSON is unchanged.

Checked `ControlOperation::Fork` retains dense branch targets in persisted variadic order, a completed target and the closed `ForkJoin` enum (All/Any/Race/Detach). An explicit supported join configuration is required; missing or unknown values fail compiler admission before executable resolution. The unfrozen public operation enum gains a variant; downstream exhaustive matches must handle it. This extends existing host language execution, not the package extension authority.

`Run::ownership()` returns bounded metadata counts for retained activations, strand/continuation frames, regions and timers, including reserved On Stop entries. Terminal cancellation releases every supported owner. Cleanup has a non-refilling activation-wide work reserve. Each cancelled operation owns its cleanup deadline, clamped to the remaining run drain deadline; a later cancellation does not inherit an expired earlier deadline. Cleanup temporarily borrows data evaluation with separate fuel, then restores ordinary activation fuel/cancellation settings; it neither restarts entropy identities nor grants the surviving normal strand additional fuel. Pending cleanup pulls retain this scope across host pumps.

The supported entries have dense activation sequences, so the actor stores shared activation frames in indexed optional boxes rather than removing/reinserting an ordered-map entry per work item. This retains one mutable evaluator per activation and moves only an owning pointer while executing. Region identities are non-reused scalar IDs; retired regions release their map storage. Race cancels the losing ownership subtree in one pass instead of rescanning it for each sibling. Admission bounds fan-out/depth and retained owners; bookkeeping and trusted callbacks are not wall-clock preemptible and are not one operation per table entry.

Concrete supported actor (2026-10-24): `Run` owns shared activations and independently scheduled strands, iterative Sequence/loop cursors, run cells, activation-local cells and bounded resumable data frames. Virtual timers own suspended strands and a non-reused insertion sequence under a process-unique run ID. Host methods take exclusive `&mut Run`, so stop/time commands are serialized at pump boundaries without a redundant command queue. R2 adds the general bounded event/completion inbox later. Operation slices rotate unfinished activations to the ready tail; state inputs are captured at epoch start and remain immutable across interleaving. State-store, data-frame, activation, timer, depth, trace and cleanup limits are explicit; they are not a measured RSS or callback-preemption guarantee. Segment inspection returns unique written variable IDs, not R2b notifications. The full structures below remain illustrative for general resources/services and remaining entry families not yet implemented.

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

Audit remediation (2026-10-27; `core/audit-remediation`):
- **Data registers.** Dynamic data slots are dense `Value` registers with a one-byte state and a retained charge: 25 bytes per slot, down from 80 (`Option<FoldedValue>` plus charge) on x86_64-pc-windows-msvc. Full `ConstantFault` provenance lives in a register-keyed side store. An entry exists exactly while its register holds a fault, so the store never outgrows the slot count and needs no per-epoch sweep. Reads return `Result<&Value, &ConstantFault>`, the shape of `EvaluatedExport::outcome`. Charges and retained-byte accounting are unchanged.
- **Single check.** A host input is checked once, then published through a private checked path. Zone results take the same path.
- **Storage admission.** `data::activation_storage` sizes one activation from its own layout: registers, memo, reserved work, scratch, zone cells and entropy counters. Run startup adds its local cells, output cells and frame headers, then admits the total across all start and stop frames before building any.
- **Strand transfers.** The mutually exclusive pending transfers (call, return, fault return, fork, unwind, body entry, wait) are one `Transfer` value. `yielded`, `reenter` and the cleanup budget stay independent, because they coexist with transfers. Idle pull scratch is lent to body and call children and returned on retire, return or unwind. Argument buffers return to the caller, an iteration frame no strand still captures is reused in place, and cancellation reuses run-owned traversal buffers. Logical identities (entropy, traces, timer order) never come from reused storage.
- **Map checks.** Maps of at most 16 entries are checked and compared pairwise without allocating. Larger maps use one presized table. Duplicate rejection, order-independent equality and insertion-order iteration are unchanged.
- **Dependency unions.** Host-input dependency unions are a k-way merge of strictly increasing slices with plan-wide scratch, costing O(Σ|sᵢ| log k). Identical sets still share one `Arc`, and checkpoints and item charges match the previous set semantics. Each node still stores its full transitive set, so a growing chain stays quadratic overall.

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
| Connect/disconnect validation (kind, fan-in, types, cycle, region, availability) | Affected type component + topological interval + affected region roots ([16](16-incremental-validation.md), proposed) | NFR-15: ≤ 2 ms p95 on W4 |
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

E1/E2g4b concrete DataPlan subset: all plan/node construction fields are crate-private and no Default/deserializer exists. Public projections expose ordered DataNode/DataInput/DataOperation, Const/Epoch stability, dense slot/source arrays, initialized constant outcomes versus uninitialized dynamic slots, exports and checked DataResources. Source inputs identify original port/edge and dense producer indices; lazy operands are numeric positions, including specialized constant choices. Shared admission bounds the whole potential cone before callbacks. The private constant folder borrows its already-built metadata maps, retains one memo across roots and supplies original constant faults/diagnostics; the data lowerer never substitutes an entropy callback into PureKernel.

DataPlan::value_charge first validates a value against this plan's bounded resolved slot type, then returns the same nominal retained-value charge used by compilation; fault_charge validates the bounded safe fault envelope. These are runtime publication helpers, not arbitrary type/plan constructors or process-memory guarantees. DataResources bounds later scratch/work storage; it is an immutable projection, not caller-controlled plan metadata. No wire/cache format or global compiler-readiness state is introduced.

E2g4c concrete consumption subset: DataPlan additionally stores ordered export->producer indices and computes checked initial nominal charges. Try instructions include the static result-boxing decision. DataActivation owns frame slots, generation-stamped node memos, checked retained charges, preallocated work/input/output vectors and private layout-bound entropy slots. Old slot storage remains charged across epoch changes. All typed outputs/replacement charges validate before node values/memos commit. Invalid callbacks/global limits abort that epoch; partial output buffers are discarded and released.

ActivationEntropy::invocation_key reserves one identity on actual node demand and returns the same identity for further stream words in that epoch. EntropyReplayMetadata is algorithm/seed/activation scope only. Frame export reads borrow typed values/constant faults and prevent concurrent mutation through Rust lifetimes. No Default/deserializer can construct a frame, and live Debug omits value payloads. This is not the proposed general Run/Activation/Strand or service/resource executor.

E3 prerequisites: DataOperation::Input marks a host-supplied node, and DataPlan::host_inputs lists them as dense NodeIx values. DataActivation::set_input publishes one epoch-stamped value through the same typed, charged path as callbacks, so the memo stamp is the "supplied" flag. EntropyKey carries a 32-byte FrameId, and ActivationEntropy keeps a dense root counter array plus a bounded map of callee-frame arrays. DataActivation holds an owned CancelToken and a non-refilling fuel counter beside the per-epoch operation count.

Control review layout correction (2026-10-24): ControlPlan stores shared sorted host-input dependency slices per consumer root and direct Index/Error output destinations per control owner. Newly allocated dependency entries have a max_graph_items admission bound; identical slices share Arc storage. Runtime frames map data-node indices to host-input indices once. Private host demand retains the iterative pull stack and requests one frozen input only when reached; public DataPoll and MissingInput behavior remain unchanged. Pending host publication persists across pump(1), so small budgets make progress without recapturing state. Run owns separately bounded, preconstructed On Stop frames and preallocated ready capacity; they execute only after the graceful transition.

Indexed authoring reuse (2026-10-24; FR-03/08/44, NFR-15): `plan_edit_indexed` and `plan_connection_indexed` accept an exclusive document borrow and a retained DocIndex. They use journaled trial edits, restore content/revision/state identity and index on return, and rebuild a stale index on entry. An observer unwind restores document content and invalidates the index so its next indexed operation rebuilds it. Existing read-only planning APIs remain available and return the same plans; the connection wrapper needs one scratch copy. Converter/provider selection, shadowed literals, imported conflicts, generated identities and plan freshness checks are unchanged. This is authoring reuse, not incremental validation or NFR acceptance.

Validation projection reuse (2026-10-24): private PortResolver scopes retain immutable document/resolved-signature references and memoize successful concrete-port descriptors in scope-local Rc storage, as well as lookup failures. Body acceptance shares a scope across connections, data cycles, regions and control cycles; the settled whole-document resolution hands its cache to final availability/export/resource checks. The private handoff checks document identity, and debug builds verify retained entries against that same resolution. Standalone pass APIs create their own scope. No cache survives its input resolution, a function-acceptance round or a new analysis invocation. Scope-local sharing introduces no locks or atomic reference counts; public document, registry and analysis results remain Send + Sync. Stored package signatures remain orphan-display fallback metadata; an unresolved intrinsic still has no fallback and cached metadata never establishes execution approval. Cache size follows encountered endpoints and remains within the input graph's admission bounds. General V12 incremental validation remains separate.

Runtime storage reuse (2026-10-24): private PreparedData retains one checked immutable DataPlan, initial nominal charge, entropy layout and dense entropy/host lookup arrays for a run's frames. Slots, memos, entropy counters, fuel and cancellation remain per activation; frame admission still checks its own configured limits. Run and local variable cells use separate dense ownership layouts, and only actual Index/Error execution projections allocate activation output cells. Stable plan variable/source indices map to these private layouts without changing public provider identities. Disposal releases the layout. Public DataPoll and missing-input behavior are unchanged; a private HostPoll carries explicit Input demand. PullState owns active consumer identity, frozen input references and pending publication, resetting together while retaining reusable vector capacity. Cursor resumption and action completion have separate helpers, and one retained plan borrow covers a pump. These representations do not add function/event/resource execution or an allocator/RSS guarantee.

### Pure call plans (2026-10-26 UTC)

`PurePlan` supports the graph-local pure-call subset in 11. Private temporary scopes specialize fresh body signatures and keep separate output/completion maps per potential call instance. Arguments and successful results alias immutable typed slots. There are no synthetic UUIDs, document rewrites, graph clone/revalidation or generic resolver calls for `core/call`. The existing pure folder handles body kernels, lazy demand, decoding, types, faults and constant-byte accounting. Exact concrete provider support is checked before binding callbacks, including potential inactive calls. Uncalled declarations are not execution roots.

`PurePlan::call_sites()` exposes borrowed `PureCallSite` records (original call NodeId, FunctionId and optional parent table index). `PureStep::call_instance()` and `PurePlan::slot_call_instance()` identify source scopes; None means root ownership. Aliases retain their source slot/scope. These are compilation-instance indexes, not persisted/runtime frame IDs. Fault producer identity remains `ConstantFault.source`; these APIs do not supply a full runtime error envelope/call stack. Potential instances skipped by lazy demand may have no executed steps.

`CompileLimits` gains `max_call_depth` (default256); downstream complete struct literals must add it or use `..Default::default()`. Existing graph-item/fold-operation/slot/constant-byte limits additionally bound potential call expansion. For call-containing documents, conservative estimates include every root/called-instance node, structural markers and alias slots; they may reject a graph whose demanded plan would fit. Arithmetic is checked. Signature/index metadata, callback work and process RSS are not bounded by constant-payload accounting. No new dependency, saved format or provider trait is introduced. The runtime extension below admits this bounded Pure subset through `compile_data` and the data plane of `compile_control`; other model-accepted function kinds remain unsupported.

Pure-call storage correction (2026-10-26 UTC): temporary PreparedBody records are shared by (FunctionId, fresh concrete TypeArgs). Unchanged signatures, including defaults, borrow settled ValidationAnalysis storage. A changed generic signature is copied once per specialization; its owned JSON/default/extension payload and nominal descriptor/type bytes are admitted before cloning, then reserved in the same max_constant_bytes total as folded values/export names. This conservative accounting can reject a generic compilation whose published constants alone would fit. Borrowed metadata needs no owned-copy charge. Call-interface and provider port matching uses checked declaration-order pairs; no saved inference cache supplies arguments. Potential operation/item/slot quotas still count every invocation, including cached bodies and inactive arms.

Runtime ownership correction (2026-10-26 UTC): each region indexes child regions and live logical strand IDs, with an allocation-free singleton case and an ordered spill set. A strand-to-timer-key directory permits owned timer removal while the main timer BTreeMap preserves deadline/sequence ordering. Global live counts are adjusted from retired subtree counts. The ready queue owns reusable dense linked slots and an identity-to-slot directory, so front/back order and direct removal remain separate concerns. Directory hashing is never a semantic ordering source. Slot positions may be reused privately; run/owner identities keep their existing lifetime rules. Serial single-ready work avoids directory lookups. Queue links, spill transitions and slot reuse have focused deterministic model tests.

PreparedData additionally maps dense plan slots to activation-owned dynamic cells. Initial immutable outcomes stay in DataPlan and are borrowed on reads; each activation keeps its own dynamic outcomes, memo stamps, entropy, fuel and cancellation. Conservative initial-value charge and the public admission thresholds are unchanged even though constants are now physically shared. This removes per-frame constant cloning/destruction without introducing shared mutable values or changing public data polling.

### Expanded Pure data calls (2026-10-26; PR21)

`DataPlan` retains a borrowed-view `call_sites()` table of `PureCallSite` records. `DataNode::call_instance()` and `DataPlan::slot_call_instance()` index that table, with None for root ownership. Body node IDs remain unchanged across instances; dense indices separate their slots/memos. Function-entry argument aliases retain their source slot scope. Returned boundary slots belong to the calling scope, while body slots retain the child scope. These are compilation source indices, not runtime entropy FrameIds or a complete runtime error stack.

Private lowering tasks and output/completion keys include a call scope. Immutable admission metadata is shared with ConstantFolder through one Arc rather than preparing/copying specialized bodies twice. Proven constant argument slots are imported once per source slot into that folder and aliased into child scopes, so body constant islands reuse the same typed folding/warning rules. Folder and published-plan payload accounting remain conservative quotas, not combined allocator measurements.

`DataOperation::Call { arguments }` contains an argument prefix followed by all declared result references. Runtime Gather checks them in order and short-circuits the first fault before demanding the next reference. Successful publication copies only the result suffix through existing type/retained-byte checks. No provider callback implements a call. Existing prepared-data dynamic cells, epoch memo stamps, work-stack capacity, fuel and cancellation apply to the expanded body. Compile-time potential-expansion limits bound flattening; no runtime recursion or general call-frame lifecycle is added.

Compatibility: the public `DataOperation` enum gains Call; downstream exhaustive matches must handle it. The source metadata accessors are additive; resolver traits, provider locks, dependencies, graph schema and entropy version are unchanged. Action/async/volatile calls, mutable call-local variables, package bodies and Return cleanup remain separate work.

### Dense scoped entropy in expanded calls (2026-10-26; PR22)

Private CallProfile distinguishes Pure compilation from the SeededData profile. Admission prepares the immutable scoped bodies once and passes that evidence into ConstantFolder; this avoids widening the PurePlan contract while reusing typed folding, provider support, lazy demand and expansion quotas. Call-site source records retain the existing PureCallSite Rust name for compatibility; DataPlan tables can now include supported seeded-volatile calls. No resolver trait or public enum variant changes are needed in this phase.

PreparedData deduplicates the source UUID layout and binds each actual entropy DataNode to an EntropySlot, derived FrameId and distinct dense counter index. A temporary ancestry memo hashes only paths needed by those actual instructions; it is discarded after preparation. Pure plans and unused potential calls require no call-frame hashing. Per-activation counter storage follows expanded entropy instruction count, admitted through existing node resources; it does not allocate every source node for every frame. Existing public explicit-frame entropy owners retain their own bounded sparse API semantics. Both modes share invocation stamping/exhaustion logic and the version2 stream algorithm.

Checked plans, immutable provider snapshots and private bindings prevent forged dense associations through public APIs. Dynamic state remains activation-owned; borrowed bindings do not introduce shared counters or mutexes. The structural test compares128dense counters with the public explicit-frame key semantics across epochs and asserts no private sparse frame allocations. This is a representation/count invariant, not allocator/RSS or timing qualification.

### Typed dispatch tables and result projections (2026-10-26; PR23)

ControlOperation gains Switch and MatchResult; ControlOutcome gains Case(original u16 instance), Default, MatchedOk and MatchedErr. HostSource gains Value(owner), ControlPlan exposes its value projection targets, and PlanType gains String. Downstream exhaustive Rust matches must handle these unfrozen enum additions. Saved schemas, resolver traits and dependency versions are unchanged.

SwitchCase stores a typed language-owned SwitchKey, original instance and optional ControlIx. Case JSON is borrowed until its nominal charge is reserved, then converted once. Scalar cases charge64bytes each; string cases charge96 plus UTF-8 bytes. Aggregate case count additionally uses the graph-item budget. An allocation-free unstable sort orders unique keys; runtime binary search does not parse JSON or invoke a package comparator. SwitchKey's default Debug omits payloads. These nominal charges are not process allocation measurements.

MatchResult retains the result root, boxing mode and outcome targets. Fresh value/error output descriptors become one host input each; explicit error ports suppress duplicate implicit materialization. Dense Value target directories share the existing host-source snapshot mechanism. ActivationState tracks retained Match value charges; transactional replacement validates target slot types and the combined local/Match state limit before changing cells. Error/index projections keep their existing fixed bounded representations. Checked data-plan and state quotas remain separate conservative admissions.

Switch admission review correction: model case validation checks signedness/width bounds and uses a temporary HashSet of borrowed serde_json values for presence/duplicate checks. Iteration still follows the original variadic order; hash iteration never chooses diagnostic or runtime order. This removes repeated scans of prior cases without introducing serialized keys, payload cloning or a retained analysis cache. A1024-case regression checks acceptance/identity and a duplicated final case; no latency or NFR claim follows.

State-control representation (2026-10-26): routing fields now carry ExecTarget (dense ControlIx plus resolved ExecInput), while region/loop ownership stays keyed by ControlIx. Inputs in/enter and in/store normalize only at checked lowering; unsupported port instances reject. ControlPlan.states holds checked ControlCell<NodeId> metadata; ControlVariable is the corresponding ControlCell<VariableId> alias. Per-scope dense directories select Run or activation stores without node-ID lookups during execution. HostSource::State reads a frozen typed cell per consumption. Initializers/case tables/variables share compiler byte admission; Register JSON copy is conservatively reserved before cloning. Run/local publication subtracts old charge and reserves replacement before mutation, counting retained Match payloads for local scope. Storage is nominal value accounting, not a hard allocator/RSS bound. Public routing types and enum variants change unfrozen Rust matches; graph JSON/provider ABI stays unchanged.

Update representation (2026-10-26): UpdateOperation carries resolved Core arithmetic/policy or ordered/bitwise/append/CAS tags. UpdateDemand stores one or two export indices and a sorted host-input union. Identical operand directories share the existing Arc; only a distinct union allocates and consumes additional graph-item admission. HostSource::Update pairs canonical ControlIx with Old/New/Exchanged tags, reusing dense output storage/directories. PullState adds two optional Values and a count; advancing roots neither restarts the epoch nor resnapshots. Publication checks replacement cell/version and released/new projection charges before commit. Shared scalar operations carry no host authority; model/compiler still own type/capability/provider admission. New Rust types/variants are unfrozen API changes; saved fields/provider ABI remain unchanged.

Collection representation (2026-10-26): PlanType adds Bytes/Timestamp/List/Map and public bounded ValueCheckError admission. DataPlan/ControlCell expose checked charges while existing optional-charge APIs remain. Map keys are restricted to the executable Core Hash subset; duplicate borrowed keys reject. Typed packed and generic lists have equal semantic values and conservative per-element charges. Raw public PlanType construction cannot bypass shape/depth guards through None or empty containers. Existing literal codecs/JSON stay unchanged. ListRepr::concat reuses matching packed representations and falls back to Value storage, with checked reservation and caller-owned type admission. Typed map equality ignores entry order while iteration preserves it. HostSource::Iteration carries a dense owner and projection tag; private lexical frames own values through Arc links and automatic byte guards, with boxed cursors keeping other cursor variants compact. Region ownership retires snapshots on every exit and retains canceled cleanup context; it adds no package dispatch authority. New Rust enum variants/helpers are unfrozen API additions, not a saved-schema or provider ABI change.

Function state-input representation (2026-10-27): shared prepared call metadata now reaches control lowering before variable collection and is reused by data lowering. All potential called Get declarations are admitted before callbacks, rather than indexing root nodes with body IDs. A dense HostSource::Variable index selects run/activation state; HostSource::CallLocal selects the immutable initializer for an accepted read-only volatile body. Each expanded DataNode retains its call_instance and its own host-input slot. No runtime document traversal or body-name lookup is introduced. Call-local mutable storage, clock inputs and action/async frame lowering remain separate work.

Effectful call representation (2026-10-27): compiler keys control identities and DemandRoots by static call scope plus original NodeId. ControlNode::scope and FunctionLayout expose immutable entry/local/result layout; ControlOperation adds Call/Return with shared ordered demand directories. HostSource adds FrameLocal, Argument and CallOutput; expanded body IDs are never looked up as root IDs. Control preparation admits effectful scopes without routing actions through a pure resolver. The same prepared signatures, slot source/call provenance and frozen union metadata feed data lowering.

Private Run frames contain fresh locals/versions, arguments, scope/parent invocation and a shared RAII nominal-byte lease, keyed by never-reused function-region ID. Regions own their normal frame implicitly and transfer canceled descendants to a rare boxed retained-ID set until cleanup completes. This avoids recursive frame destruction and shared mutable locks. Explicit Return captures a boxed charged result until the barrier; publication releases old projections and checks the complete replacement before writing. Call-local Update reserves both cell and output deltas atomically. Rare pending call/result/fault fields are boxed. Run::control_state distinguishes expanded stateful controls; source-only node_state refuses ambiguous IDs. Write-intent inspection remains source-based. These additions are unfrozen Rust surface changes; saved format, provider locks/ABI, dependency versions and UI boundaries remain unchanged.

Each FunctionLayout also owns a compact list of its execution host-input projection indices. Frame retirement releases those projections and their charges after cleanup, including frames transferred from canceled descendants. A startup source-index directory keeps source-only write intent deduplication independent of expanded state-cell identity. Bounded activation data caches retain their existing policy; these changes introduce no generic cache eviction, allocator/RSS or timing claim.

Compile-bound review correction: Sequence/Fork actual variadic step counts are charged per expanded control before building target arrays, including inactive and unconnected steps. This closes the gap between the document/definition counts and owned expanded step storage; it does not turn nominal compile quotas into an allocator/RSS bound.


Zone analysis projection (2026-10-27): `ValidationReport.regions.zones` is a model-owned immutable `ZoneIndex` with read-only pair/owner queries. `ZoneRegion` retains original begin/end UUIDs, `ZoneKind`, optional function identity and immediate enclosing end UUID. Begin markers own their parameter projections; a nested end is owned by its parent. Only one innermost owner per node is retained, rather than retaining a copy of every nested body set. Pairing is shared by region and port-level dependency analysis; a zone result has strict begin-prefix dependencies without turning begin iteration projections into computed outputs. This is validation metadata, not executable zone IR or a saved-format change.


Zone compiler layout slice (2026-10-27): DataPlan now retains plan-local `ZoneIx`/`DataZone` entries per specialized call scope, original begin/end IDs, lexical parent and optional lowered parameter instruction. DataNode records its lexical memoization domain. `DataOperation::Iteration` has no producer inputs; `DataOperation::Zone` consumes strict begin-prefix inputs followed by end/body inputs. Host-state Input instructions retain the outer consumption domain. Metadata is charged before provider binding; constant calls containing zones lower as programs rather than scalar-folder unrolling. Unused admitted zones need no runtime machine. At this intermediate checkpoint, compile_pure explicitly refuses zones, and DataActivation explicitly rejects zone instructions until resumable execution is implemented. This is unfrozen Rust IR, not a saved-format or provider ABI change.

Compare-and-set and harness (2026-10-27):
- **Compare-and-set** relies on values having been checked when stored or published. It decides equality structurally and uses checked, order-insensitive map equality only for shapes that contain a map.
- **Harness.** `orange-measure` is a development-only harness. Its counting allocator is the only `unsafe` code in the repository, contained by crate-local lints.
