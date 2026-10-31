# Incremental validation (V12)

Status: **V12a implemented; bounded V12b caching**, 2026-10-25. `AnalysisSession` retains fresh combined analysis, an optional document index, configuration/declaration-type buckets and, since 2026-10-30, per-node input checks, per-edge connection checks and per-component type inference. Literal, type argument, type pin and package-node configuration edits in root bodies that leave every resolved signature unchanged, and additions and removals of data edges and pure package nodes in a clean root body, are applied as scoped updates without a full analysis; other semantic edits still run full admission and all other validation passes, and their publication still scans current/previous results. The remaining incremental design targets NFR-15 ([06](06-quality-and-validation.md)) and [12 §11](12-graph-data-structures.md), under ADR-23 in [08](08-decisions.md). Pass names refer to `orange_model::validation`. Neither full V12b nor NFR-15 is complete.

## 1. Goal

A single semantic edit on the W4 graph (10,000 nodes, 20,000 edges) returns diagnostics and inferred types within 2 ms p95 and 10 ms p99 (NFR-15). Work is bounded by the affected type component, topological interval and region roots. Layout-only edits do no validation work. Unrelated components are never scanned. Connected worst cases may span the whole graph; they are measured and reported separately, not hidden.

The one correctness rule: **incremental equals full**. After any sequence of committed edits, the session's result equals `analyze(document, registry)` with the same leaf effects and options. That includes diagnostics and their order, resolved signatures, incomplete sets, function summaries, region index, cycle witnesses and coverage. The full pass stays the oracle and the fallback.

Historical starting point (local release runs after PR15, recorded in [10](10-tooling-and-sources.md)), all whole-document and roughly linear:

| Operation on a 10k-node generic chain | Time |
|---|---|
| `analyze` | about 230 ms |
| Inferred connect at the chain tail | about 110 ms |
| Inferred connect of an isolated pair (inference only) | about 0.1 ms |

Constant factors are no longer the main gap; scope is.

## 2. Core status

The core is **not complete**.

**Done:**

- Document model, transactions and undo.
- Types, inference and converter planning.
- The validator for the supported control vocabulary.
- The pure/data compiler and the root-data runtime with seeded entropy epochs.
- E3 prerequisites: owned cancellation, activation fuel, frame-scoped entropy and host input slots.
- Checked Start/Stop, Branch, Sequence, Count/While, Break/Continue, Wait/Yield and Fork all/any/race/detach, with local/run Get/Set and a bounded headless actor on virtual time.
- V12a analysis sessions and bounded V12b configuration/declaration-type reuse; retained-index planning and borrowed diagnostics/signatures.

**Open:**

- Function lowering/execution and remaining controls, value types and kernel families.
- General event ingress/reactivity, resources/services, workspace state and checkpoints (remaining R1–R7).
- The remaining incremental validator (V12b–f) and NFR-15 measurements.
- Installed-package lifecycle and migrations.

V12 does not depend on E3 or R1, and they do not depend on it. It can proceed in parallel, provided equivalence with the full pass is kept.

## 3. Session API

An `AnalysisSession` lives beside the document, the way a `DocIndex` does today. It is never persisted and never stored inside the document.

| Operation | Contract |
|---|---|
| `AnalysisSession::new(document, registry, effects, options)` | Runs full admission/domain prerequisites and the combined pipeline. Semantic evidence binds `semantic_state_id`; session lineage also records whole-content identity/revision. The retained index is absent if construction is rejected before indexing. |
| `update(&mut self, document, registry, change, options) -> &EditStats` | Checks a journal-owned committed receipt's exact before/after identities/revisions. Undo/redo are ordinary commits; missing, foreign, skipped or replayed lineage rebuilds. Current cancellation is checked before every reuse path. Valid layout changes preserve semantic evidence with zero validation-unit visits. |
| `set_leaf_effects(document, registry, effects, options) -> &EditStats` | Replaces host-trusted effects. Even unchanged effects require current document, registry, limits and cancellation checks before reuse. |
| `diagnostics()`, `diagnostics_for(subject)`, `resolved(node)`, `analysis()` | Borrow current results without copying the report. Diagnostic subject lookup uses a retained subject index. |
| `changed_subjects()`, `changed_nodes()` | Borrow diagnostic-list changes (including removals) and added/changed/removed resolved-signature IDs. These sets are computed by scanning current/previous results at publication, not by an O(delta) publication algorithm. |
| `index()`, `plan_edit(document, commands, registry)` | `index()` borrows the optional retained index. `plan_edit()` owns mutable access for existing journaled inference planning; trial success/failure does not publish analysis. A stale or unwinding planner index rebuilds on a later operation. Planning remains caller-budgeted. |
| `materialize() -> ValidationAnalysis` | Clones the retained full report, bound to semantic state and registry generation. This is a report-size operation for compiler/CLI/tests; UI readers use borrowed views. |

A conservative rebuild runs full analysis without reusing local buckets when:

- the registry generation changes;
- the leaf effects or limits change;
- shared-signature or package-lock metadata changes (`ChangeSet::global_dirty()`);
- committed content/semantic lineage or revision does not match;
- the previous request was cancelled.

Current cancellation returns the full oracle's cancellation result rather than a reused success. Every semantic analysis performs admission and domain checks. A fresh deterministic quota/domain error remains current through layout/undo/redo under unchanged semantic/registry/effect/limit inputs, with zero validation runs; it never establishes compiler readiness. Only cancellation requires recovery solely because of the previous result. Both routes borrow effect metadata and share prerequisites, so an already rejected request does not first clone retained write vectors. A dirty-set threshold is a future optimization, not implemented policy.

Each rebuild records its reason in `EditStats`.

`EditStats::input_nodes/input_edges` are input cardinalities, not total pass visits. `full_analysis_runs` counts full orchestration attempts (including rejection); `index_rebuilt` reports index rebuilding. `local: LocalWork` counts actual current node/declaration visits, configuration/declaration computations and cache hits. Cache assembly still visits current IDs. Other passes' visits, allocations and retained memory are not represented by these counts; broader instrumentation remains required for NFR-15.
Background analysis (2026-10-28): `AnalysisWorker` runs combined analysis on its own thread for a host that must not block its UI thread. `submit(snapshot, registry, effects)` takes an immutable `Arc<GraphDocument>` snapshot the host builds where it chooses, returns a new `AnalysisRequestId` for every submission, including one that changes only effect evidence, cancels the previous submission at its next pass boundary and replaces any pending one. `current(document, registry)` returns a finished analysis only for the newest request and only while it belongs to that document and registry, so rapid edits, undo, divergent copies, effect changes and registry replacements never show a result for content or evidence that is gone; `wait()` blocks for the newest request. The mailbox holds at most one pending request and one finished analysis, so a burst of submissions retains at most the snapshot in analysis and the newest one, and submission never waits for analysis. The host owns the thread's lifetime: `request_shutdown()` returns promptly and leaves releasing the pending snapshot to the thread, `join()` waits for the thread and reports one that panicked, and dropping the worker requests shutdown without joining. A submission after shutdown or after the thread stopped fails with `AnalysisWorkerError`. Hosts skip layout-only changes, which keep the semantic state and the current result. Per-pass interruption and per-body incremental analysis remain the V12 work below.
Input check reuse (2026-10-30, V12b): the session keeps each node's input diagnostics with the facts the check read: the resolved signature, the enclosing function's rigid type parameters and, in order, every port it asked about together with whether that port had an incoming wire. A later analysis in the same session reuses the diagnostics only when the change journal left the node untouched and all of those facts are equal; any registry change, missing or mismatched journal clears the cache. Reuse also applies within one analysis, between function acceptance and the final round, when a node's signature did not change between them. The check still runs over every node, so this saves literal decoding and type validation, not traversal. `LocalWork` reports `inputs_computed` and `input_hits`.
Connection check reuse (2026-10-30, V12b): each edge's endpoint, direction, kind and type check is kept with the fresh resolution of both endpoint nodes it saw. A later analysis reuses its diagnostics and incomplete nodes while the journal left the edge and both endpoint nodes untouched and both resolutions are equal; any function declaration change, registry change, missing or mismatched journal clears all edge entries, since the check reads a body's rigid type parameters. Body-boundary, fan-in and fan-out checks still run over every edge. `LocalWork` reports `edges_computed` and `edge_hits`.
Inference reuse (2026-10-30, V12b): node resolution keeps each generic component's inference together with the data edges incident to its nodes and the nodes at their far ends. A later analysis reuses a component while the journal touches none of its nodes, boundary nodes or incident edges and no endpoint of an added or rewired edge lies in it; any declaration change clears every component, since contracts and call interfaces read declarations, and so do registry changes and missing or mismatched journals. Components without a valid entry are solved by the same inference, which skips the nodes of reused ones. Preflight, projection and drift checks still run for every node. `LocalWork` reports `components_computed` and `component_hits`.
Scoped literal edits (2026-10-30, V12b): the journal marks a committed transaction whose only semantic change replaces literals of existing nodes, keeping their bodies and every other field, and records their prior literals (at most 64 nodes). For such an edit in root bodies, the session updates the admission totals by the edited literals and re-resolves only the edited nodes, their neighbours and those nodes' generic components, with every neighbour of that set available, under the callee acceptance the last full analysis used. If every re-resolved signature, incompleteness and node diagnostic is unchanged, it re-checks the edited nodes' inputs and, unless one gains or loses an input error, patches those input diagnostics into the published list in place; `SessionWork::Scoped` reports it, with no full analysis run. Literals are read only by admission, inference and the input check, so no other node or pass can observe the edit. Anything else, including an edit inside a function body, an inference change, an input error appearing or disappearing, or admission failing, runs the full analysis. `diagnostics_for` now searches the sorted list instead of a per-subject index.
Scoped type edits (2026-10-30, V12b): the journal's node edit now also covers a node's persisted type arguments and type pins, alone or with its literals. Validation reads persisted arguments only for admission and pins only for admission and inference, so such an edit takes the literal path: admission is updated from the changed fields and the edited nodes are re-resolved with their components; a pin that changes inference runs the full analysis.
Scoped configuration edits (2026-10-30, V12b): the node edit also covers configuration and its version. Structural passes read configuration only of core intrinsics (Fork, Switch, While, zones, variables and registers), so a configuration change is scoped only on a provider-bound node; there it reaches admission, the node's configuration check and its contract. Configuration diagnostics are kept per node beside the input diagnostics and patched the same way; a node gaining or losing a configuration error, or any intrinsic's configuration change, runs the full analysis.
Scoped node additions and removals (2026-10-30, V12b): the journal's edge edit becomes a structure edit that may also add or remove whole nodes, recording each removed node. In a clean report, a structure edit is scoped when every added or removed node is a provider-bound pure node in the root body whose ports are all typed data ports with sourced outputs, and no export names it. Region, control, availability, zone, state and function passes never see such a node, and the data-cycle pass needs nothing of it but its typed ports. The session charges or credits each node's admission, resolves added nodes, which must resolve without errors, stages their signatures for the edge checks and restores the published ones if a check fails, then publishes the new signatures and patches the node, input and configuration diagnostics of added and removed nodes. Node diagnostics are therefore kept per node beside the input and configuration diagnostics.
Scoped exec edges (2026-10-30, V12b): a structure edit made only of exec or event edges between existing root nodes, in a report without errors, incomplete nodes or zones, keeps resolution, inputs, connections, data cycles and exports local as for data edges and reruns only the control-flow passes: regions, control cycles, availability and state, over the whole document with the clean connection result. Their diagnostics are kept together in the session and replaced as one list, and the region index is replaced; an error, an incomplete node or a control cycle from them, a second connection on an exec or event output, or exec and data edges in one edit run the full analysis. Possibly suspending nodes no longer block this case, since the state pass reruns; they still block scoped data-edge and node edits.
Scoped edge edits (2026-10-30, V12b): the journal also marks a transaction that only adds or removes whole edges, recording each removed edge. When the last full report had no error, incomplete node or function, zone or possibly suspending node, and every edge joins a data output to a data input in the root body, the session checks each edge alone. It confirms re-resolution as for literals; that the edge's own connection checks pass with its input holding one wire and a shared output not being a stream; that an added edge's target cannot reach its source through data edges; that no export lies downstream; that neither the edge's producer nor the lazy cone behind it carries an availability requirement; and that a removed edge had no diagnostic. Then only the targets' input diagnostics can change, and they are patched as for literals. Regions and control cycles read only execution edges, and zone membership, unreachable-node and stale-capture checks cannot see a data edge under those conditions. Everything else, including any exec or event edge, runs the full analysis.
Session edit cost (2026-10-30, V12b): `orange-measure session-edit` times one literal edit on 10,000 generic nodes. The data-cycle pass now runs over dense, borrowed vertices, which cut that edit from 166 ms to 106 ms p50 on 1,000 independent chains; every pass still visits every node, so the edit remains far above NFR-15 (10).

## 4. Dependency model

The target stores each pass result per **unit** and replaces it only when dirty. Currently only configuration and variable/event declaration-type results are retained across semantic edits, using the same unit functions as full validation. Changed nodes invalidate config units; function replacement/removal invalidates its owned nodes; changed declarations invalidate their type units. Registry/global metadata or unproven lineage clears local reuse. Inputs have shared unit helpers but are not cached. The table describes the remaining dependency model, not delivered caches for every row.

| Pass | Unit | Reads | Dirty when |
|---|---|---|---|
| Body domains | body | domain extension | extension or function declaration changed |
| Node resolution | node | definition, provider binding, package lock, signature entry, referenced declarations, callee acceptance | node changed; its lock or signature entry changed; a declaration it references changed; its callee's acceptance changed |
| Inference | generic component | pins, active literals, data edges, neighbouring concrete ports | the same touched set `plan_edit` already computes (both endpoints of changed edges, changed nodes and their neighbours, declaration consumers) |
| Projection | node | contract, solved arguments, body | resolution dirty or arguments changed |
| Connections | edge, plus its endpoint ports for fan-in and fan-out | endpoint signatures, edge | edge changed; an endpoint's signature changed; another edge at the same port changed |
| Inputs | node | signature, literals, incoming edges | node dirty; an incoming edge changed |
| Config, declaration types | node / declaration | config and schema / declaration | node / declaration changed |
| Data cycles | body port graph | data edges, port sets | a data edge or a node's data ports changed (§5.3) |
| Regions, control cycles | top-level root | exec edges, kinds, suspension | an exec edge or exec node changed in the root, before or after the edit |
| Availability | top-level root | the root's control-flow graph, data cones into its owned exec consumers | root dirty; a data edge changed whose downstream exec consumer (through pure and volatile cones) the root owns |
| Exports | export | export cone | export changed; a node in its cone changed |
| Streams | declaration / export | resource types | declaration or export changed |
| State | Fork / root | region index, summaries, leaf effects | root dirty; a callee summary changed |
| Function acceptance, summaries, conformance | function | its body, callee summaries | body or interface changed; transitively, every caller of a function whose summary or acceptance changed |

## 5. Algorithms

### 5.1 Seeds and propagation

The remaining design uses immutable `ChangeSet` node, edge and declaration deltas as seeds. Shared metadata has an explicit conservative global-dirty signal. Future propagation follows the order of `run_analysis`:

1. domains
2. functions
3. node resolution
4. inference and projection
5. connections, inputs and config
6. data cycles
7. regions and control cycles
8. availability
9. exports, state and streams
10. assembly

Each step recomputes only its dirty units, replaces their stored outputs, and marks units of later steps dirty when an output actually changed. An output that comes back identical stops propagation.

### 5.2 Reverse indexes

The session maintains, from change sets:

- declaration → referencing nodes, through typed `RegistrySnapshot::references`, as `plan_edit` already scans;
- package lock and signature key → nodes;
- function → its call nodes;
- node → owning top-level root;
- data predecessors, from the `DocIndex` incoming map.

These are what keep propagation bounded. A missing reverse edge shows up as a broken equivalence (§6), not as a silently stale result.

### 5.3 Data cycles

Per body, keep a dynamic topological order of the data port graph (Pearce–Kelly, as [12 §3.2](12-graph-data-structures.md) proposes). Inserting an edge updates only the affected order interval.

An edge that would close a cycle is kept in a per-body cyclic set instead of the order. While that set is non-empty, strongly connected components are recomputed only over the vertices reachable both ways from the cyclic edges' endpoints. The witness is chosen exactly as the full pass chooses it, so paths stay equal. Removing an edge retries the body's cyclic edges; cycles are errors, so that set stays small.

### 5.4 Roots: regions, control cycles, availability

The region, control-cycle and availability passes become root-scoped. A root is a top-level entry or event region; the existing `Scope` grows a root variant.

- An exec change dirties the roots that own its endpoints before and after the edit.
- A node that moves between roots dirties both.
- Availability already builds one dominator tree per root (11 §7.2), so recomputing a root is proportional to that root.
- A data edge change dirties only the roots owning the exec consumers downstream of it.

### 5.5 Functions

Functions keep the callee-first acceptance of `accept_functions`, per function. A dirty function is re-accepted and re-summarized. If its acceptance or summary changes, its callers (through the reverse call graph) and its call nodes become dirty. Recursion is already rejected, so this terminates. A chain of changes through the call graph is a reported full-scope case.

### 5.6 Diagnostics assembly

Configuration and declaration-type diagnostics currently have retained unit buckets, while the shared full pipeline assembles, sorts, deduplicates and suppresses explained dependency errors in full. The remaining target stores all diagnostics per (pass, unit), with a merged `sort_diagnostics` view and reference counts per identical record for O(Δ log D) updates. The cross-pass `E-DEPENDENCY-UNAVAILABLE` suppression would use per-node error counts and re-evaluate affected nodes; incomplete sets would aggregate likewise. Neither delta assembly nor these future per-pass buckets are implemented by the current subject/change views.

## 6. Equivalence, tests and measurement

- **Oracle:** in tests, after every update, `materialize()` must equal `analyze()` on the same document: full report equality, coverage and state binding.
- **Property tests:** small, bounded random edit sequences on generated documents. Edits include connect, disconnect, literal, type argument, pin, node add and remove, declaration edit, function body replacement, undo and redo. Failing seeds are committed.
- **Focused tests:** one known-answer test per invalidation rule. For example, a variable type change re-validates a referencing Get inside a function body.
- **Mutation checks:** removing any reverse-index propagation, or skipping a dirty pass, must fail tests, as was done for the availability and caching changes.
- **Measurement:** W4 graph with W9's 1,000 random edits. Report p95 and p99 latency, the visit counters from §3, rebuild counts with reasons, and connected worst cases separately. A layout-only edit must show zero visits.

## 7. Memory, limits and cancellation

The target session adds O(nodes + edges) indexes and unit results to the existing report, including future reverse dependencies and per-body topological orders. The current session retains the report, optional DocIndex, configuration/declaration buckets and subject/change views; it has no persistent topological order or general reverse-dependency index. V12f must measure retained memory against the NFR-05 W4 budget; this design states no measured memory gate.

`AnalysisOptions` admission quotas and cooperative cancellation apply to `update`:

- Quotas are checked against post-edit counts.
- A cancelled update marks the session stale, and the next update rebuilds.
- Deterministic output never depends on `HashMap` iteration order.

## 8. Integration

- The editor host keeps the document and session. Every committed `apply`, `revert` or `apply_inferred` passes its `ChangeSet` to `update`; `plan_edit()` reuses the session's optional index for trials.
- Trial edits made with `GraphDocument::scratch` never reach the session; they are rolled back before commit.
- The compiler requires a fresh `ValidationAnalysis`; `analysis()` already supplies a borrow for immediate compilation. `materialize()` clones an owned snapshot when a worker or retained request needs ownership. Neither route constructs compiler evidence directly from individual cache buckets.
- The session and published facts are `Send + Sync`; Debug omits payloads. Background hosts must compare semantic state, registry and request/effects/options identity before publishing a result. Older work must not overwrite a newer request. This is a headless core contract, not UI scheduling or bounded interruption within individual passes/trusted kernels.

## 9. Delivery slices

Closely related slices may share a reviewable PR to `dev`; each implemented behavior keeps the full-pass equivalence gate. Current delivery is V12a plus bounded V12b, with conservative full passes elsewhere.

| Slice | Content | Done when |
|---|---|---|
| V12a | Implemented session: semantic/content binding, immutable committed provenance, rebuild triggers, layout no-op, retained index/trial adapter, borrowed result/change views, attributed stats and owned materialization. | Full-pass equivalence and freshness/rollback/embedding checks; executed results in 10. |
| V12b | Partially implemented (exec edges rerun the control-flow passes in full; V12c makes them root-scoped): configuration and declaration-type buckets, per-node input checks, per-edge connection checks and per-component inference, all with shared validators; scoped literal, type argument, pin, package-node configuration and root data-edge edits with in-place diagnostics. Intrinsic configuration, non-pure or intrinsic node additions and removals, function bodies and structure edits in reports with errors still run every pass in full. | Complete only when isolated edits visit only affected components/edges, including orchestration overhead. |
| V12c | Root-scoped regions, control cycles, availability and state. | Exec edits visit only affected roots. |
| V12d | Data cycles with a dynamic topological order; export cones. | Data edits visit only their order interval and cones. |
| V12e | Functions: acceptance, summaries and conformance through the reverse call graph. | Body edits visit the function and its changed callers. |
| V12f | W4/W9 measurement and NFR-15 evidence. Tuning, such as dense or interned identifiers, only where measured. | p95/p99 and counters recorded in 10; NFR-15 judged on evidence. |

## 10. Risks and open questions

- Regions and control cycles are body-scoped today. Making them root-scoped is a refactor that must keep results identical; V12c starts with that.
- Equal cycle witnesses depend on witness selection being independent of discovery order. V12d checks this before relying on it.
- Function cascades and densely connected generic components can make an edit full-scope. NFR-15 allows this but requires it to be reported.
- `materialize` is O(document). The editor must not call it per edit; it uses the session reads.
- The session retains configuration/declaration diagnostics alongside the full report, subject indexes and changed-ID sets. These add memory without cloning config/value payloads; V12f must measure retained memory and allocations before any NFR-05 claim.

Availability cone sharing (2026-10-27): availability analysis still recomputes per body, with no cache across calls. Within one `check_body`, each directly read, unowned pure or volatile producer's requirement-relevant cone is collected once and shared by all its consumers. The requirement and the subject are still evaluated per consumer, because they depend on the consumer's region and direct edge. Nested directly read pure producers each get their own walk.
