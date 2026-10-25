# Incremental validation (V12)

Status: **proposed design**, 2026-10-24. Nothing here is implemented. It targets NFR-15 ([06](06-quality-and-validation.md)) and the incremental rows of [12 §11](12-graph-data-structures.md), and is recorded as ADR-23 in [08](08-decisions.md). It describes the validator as it exists after PR #15; pass names refer to `orange_model::validation`.

## 1. Goal

A single semantic edit on the W4 graph (10,000 nodes, 20,000 edges) returns diagnostics and inferred types within 2 ms p95 and 10 ms p99 (NFR-15). Work is bounded by the affected type component, topological interval and region roots. Layout-only edits do no validation work. Unrelated components are never scanned. Connected worst cases may span the whole graph; they are measured and reported separately, not hidden.

The one correctness rule: **incremental equals full**. After any sequence of committed edits, the session's result equals `analyze(document, registry)` with the same leaf effects and options. That includes diagnostics and their order, resolved signatures, incomplete sets, function summaries, region index, cycle witnesses and coverage. The full pass stays the oracle and the fallback.

Starting point (local release runs, recorded in [10](10-tooling-and-sources.md)), all whole-document and roughly linear:

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

**Open:**

- Control and function lowering (E3: Branch, Sequence, Fork, loops, calls).
- The run actor, scheduler and services (R1–R7).
- This incremental validator (V12).
- Installed-package lifecycle and migrations.

V12 does not depend on E3 or R1, and they do not depend on it. It can proceed in parallel, provided equivalence with the full pass is kept.

## 3. Session API

An `AnalysisSession` lives beside the document, the way a `DocIndex` does today. It is never persisted and never stored inside the document.

| Operation | Contract |
|---|---|
| `AnalysisSession::new(document, registry, effects, options)` | Runs the full analysis. Records the document `state_id`, the registry generation and an owned `DocIndex`. |
| `update(&mut self, document, change: &ChangeSet) -> Result<&EditStats, …>` | Applies one committed transaction. The change must start at the session's recorded state; undo and redo change sets are ordinary changes. A mismatch rebuilds. Layout-only changes return immediately with zero visits. |
| `diagnostics()`, `diagnostics_for(subject)`, `resolved(node)`, `changed()` | Reads from session state without copying the report. `changed()` lists the subjects whose diagnostics or signatures changed in the last update, for the editor projection. |
| `index()` | The session's `DocIndex`. Inference planning (`apply_inferred_indexed`) reads it, so the host keeps one index, not two. |
| `materialize() -> ValidationAnalysis` | Builds the full report, bound to the document state and registry generation like `analyze`. It costs O(document) and serves the compiler, CLI and tests (NFR-04 budget). |

A rebuild runs the full analysis. It happens when:

- the registry generation changes;
- the leaf effects or options change;
- the document crosses an admission quota;
- a domain extension changes;
- the change does not start at the recorded state;
- an update was cancelled, so no half-updated state survives;
- the dirty set exceeds a fraction of the document at which a rebuild is cheaper.

Each rebuild records its reason in `EditStats`.

`EditStats` reports visit counts: nodes, edges, components, roots, functions, dirty units per pass, and whether and why a rebuild happened. NFR-15 requires these counters next to timing.

## 4. Dependency model

Every pass result is stored per **unit** and replaced only when that unit is dirty. The table gives each pass's unit, what it reads, and what marks a unit dirty.

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

The `ChangeSet` supplies seed nodes, edges and declarations. Propagation follows the order of `run_analysis`:

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

Diagnostics are stored in per-unit buckets keyed by (pass, unit). A merged view is kept in `sort_diagnostics` order with a reference count per identical record, which reproduces today's deduplication; an update costs O(Δ log D). The one cross-pass rule is that `E-DEPENDENCY-UNAVAILABLE` is dropped for a node that already has another error. It is kept with a per-node error count and re-evaluated only for nodes whose count changed. Incomplete-node sets aggregate the same way.

## 6. Equivalence, tests and measurement

- **Oracle:** in tests, after every update, `materialize()` must equal `analyze()` on the same document: full report equality, coverage and state binding.
- **Property tests:** small, bounded random edit sequences on generated documents. Edits include connect, disconnect, literal, type argument, pin, node add and remove, declaration edit, function body replacement, undo and redo. Failing seeds are committed.
- **Focused tests:** one known-answer test per invalidation rule. For example, a variable type change re-validates a referencing Get inside a function body.
- **Mutation checks:** removing any reverse-index propagation, or skipping a dirty pass, must fail tests, as was done for the availability and caching changes.
- **Measurement:** W4 graph with W9's 1,000 random edits. Report p95 and p99 latency, the visit counters from §3, rebuild counts with reasons, and connected worst cases separately. A layout-only edit must show zero visits.

## 7. Memory, limits and cancellation

Session memory is O(nodes + edges): resolved signatures (already part of the report), per-unit diagnostics, reverse indexes and per-body topological orders. It is measured against the NFR-05 W4 budget.

`AnalysisOptions` admission quotas and cooperative cancellation apply to `update`:

- Quotas are checked against post-edit counts.
- A cancelled update marks the session stale, and the next update rebuilds.
- Deterministic output never depends on `HashMap` iteration order.

## 8. Integration

- The editor host keeps the document, the session and the session's index. Every committed `apply`, `revert` or `apply_inferred` passes its `ChangeSet` to `update`.
- Trial edits made with `GraphDocument::scratch` never reach the session; they are rolled back before commit.
- The compiler keeps requiring a fresh `ValidationAnalysis` from `materialize()`. Compiling from a session view without materializing is a later option.
- The session is `Send`, so validation can later move off the UI thread without changing results.

## 9. Delivery slices

Each slice is its own PR to `dev` and keeps the full-pass equivalence gate.

| Slice | Content | Done when |
|---|---|---|
| V12a | Session skeleton. `update` always does a full recompute. State binding, rebuild triggers, layout-only no-op, `EditStats`, `materialize` and the oracle harness. | Equivalence property test green; layout edits visit nothing. |
| V12b | Node-local units: resolution, projection, inputs, config, connections, declaration types. Inference through touched components; diagnostics store and assembly. | Isolated edits visit only their component and edges. |
| V12c | Root-scoped regions, control cycles, availability and state. | Exec edits visit only affected roots. |
| V12d | Data cycles with a dynamic topological order; export cones. | Data edits visit only their order interval and cones. |
| V12e | Functions: acceptance, summaries and conformance through the reverse call graph. | Body edits visit the function and its changed callers. |
| V12f | W4/W9 measurement and NFR-15 evidence. Tuning, such as dense or interned identifiers, only where measured. | p95/p99 and counters recorded in 10; NFR-15 judged on evidence. |

## 10. Risks and open questions

- Regions and control cycles are body-scoped today. Making them root-scoped is a refactor that must keep results identical; V12c starts with that.
- Equal cycle witnesses depend on witness selection being independent of discovery order. V12d checks this before relying on it.
- Function cascades and densely connected generic components can make an edit full-scope. NFR-15 allows this but requires it to be reported.
- `materialize` is O(document). The editor must not call it per edit; it uses the session reads.
- Session memory doubles part of the report's footprint until the report is a view over the session; V12f measures it.
