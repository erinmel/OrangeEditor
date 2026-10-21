# WU-00c implementation tasks

Status: all slices 00c.1–7b are implemented and checked locally on `feat/wu-00c-types-conversions` (HEAD at the 7a commit plus this 7b commit). Nothing is pushed, no PR is open and no remote CI result is claimed; the coordinator's acceptance of 6a–7b and the user's review remain. This is the task packet for the seven slices in [07](07-delivery-plan.md). Keep it as the working handoff; do not create a README, fixture inventory or separate report for every task.

## Ownership and context

Assign a continuing developer/agent to each role. A role can complete several tasks in sequence; send follow-ups to the same agent to retain its working context. Bind roles to actual agent IDs when implementation starts, and record that mapping here. Roles are responsibilities, not a requirement to launch four agents at once.

| Role | Owns | Consult this owner about |
|---|---|---|
| F — type foundations | 1a, 1b, 2a, 2b | Registered types, snapshot/key lifetime, schemas, constraint evaluation and coherence |
| S — inference | 3a, 3b, 4a, 4b | Structural substitution, unification, explicit pins, component traversal and inferred bindings |
| C — connections | 5a, 5b, 6a, 6b | Converter entries/provider selection, connection plans and insertion |
| Coordinator | 7a, 7b and integration | Contract decisions, shared files, cross-owner APIs, review and task acceptance |

Current agents: F = `/root/type_foundations`, S = `/root/inference`, C = `/root/graph_authoring`, coordinator = `/root`. Consult these owners for follow-up work in this conversation; reconstruct from their handoffs if a later session no longer has those live agents.

Current handoffs:

- F: 1a/1b in `cd6cb73` supply `TypeRegistryBuilder` and frozen `TypeRegistry` using the existing literal schemas. Term-based `validate`/`owner_of`/`kind_of` work without interning; key queries enforce registry ownership. `RecordRepr::ty` stays canonical text. 2a in `cb513e1` adds bounded capability/derive proofs and authoritative map constraints. Accepted 2b adds versioned `ConstraintId`, exact `ConstraintImplementationRef` using `ProviderBinding` plus digest/implementation ID, core/custom constraint registration and operation-specific proofs; derive slots cannot be overwritten. Eleven constraint tests, workspace tests (144 at this stage), formatting, Clippy and warning-denied docs passed; focused checks reran after canonical-ID validation was tightened. No kernels or package authentication are implemented.
- S: 3a (`0059ae0`) substitutes signature parameters structurally; 3b (`43b2029`) unifies node-scoped terms with a provisional 16,384-node budget. 4a adds `inference::infer_components`: read-only analysis of the generic components touched by an edit, seeded from `type_pins`, active typed literals, edges and concrete neighbour snapshots, never from persisted `type_args`; host templates come from `TemplateCatalog` (`GenericDefinition`). Outcomes map to E-TYPE-MISMATCH, E-TYPE-AMBIGUOUS, E-TYPE-COMPLEXITY, E-CONSTRAINT-UNSATISFIED and E-NO-SPECIALIZATION. Pins are specified in 11 §5.5, 12 §3.1 and ADR-16. Core intrinsics without a snapshot contribute no evidence yet. 4b adds `inference::{plan_edit, apply_inferred}` (`src/inference/persist.rs`): `plan_edit` applies the edit to a scratch clone, infers the touched components and returns `InferencePlan { commands, issues }` without mutating the document. The commands are the edit, then per node `SetTypeArgs` for a changed binding and `PutSignature` + the new `SetSignature` for a changed projected snapshot; `apply_inferred` commits them as one transaction. Issues never block a commit: an ambiguous or conflicting component is saved with its resolved arguments only (none on conflict) and `$name` kept in its signature. `type_pins`, provider bindings and nodes outside the touched components are never written. `GenericDefinition` gained `kind` for the projected snapshot. Superseded snapshots are deliberately not pruned (reasons in 12 §3.3). `SetTypePins` (`1655b9f`) lets `plan_edit` re-infer after a pin or unpin; there is no pin UI. Consumed by 6a: `plan_connection` reuses `plan_edit`. Follow-up: edits inside function bodies, including a replaced body, are inferred (11 §5.5).
- C: 5a (`59ea3aa`) separates semantic converter entries from exact provider alternatives with explicit new-binding defaults; 5b (`4b5c865`) adds `select_for_connection` and `resolve_explicit_choice` with Compatible/Automatic/ChoiceRequired/Rejected outcomes. 6a (`42f07c0`) adds `conversion::plan_connection(document, converters, templates, ConnectionRequest, new_id)`: it resolves both endpoint types (declared port with `type_args` substituted; unresolved generic endpoints connect directly and inference decides), applies `select_for_connection` or the explicit choice, and returns `ConnectionOutcome::{Ready(plan), ChoiceRequired, Rejected}` or a typed `ConnectionError`. A plan is a direct wire, a replaced wire (existing edges into the target are disconnected first) or `PutSignature` + `AddNode(core/convert)` + two `Connect`s, followed by `plan_edit`'s inferred updates. The persisted `core/convert` shape is in 11 §6 / 12 §5. Identities come from `new_id` once per plan; preconditions are the document `state_id`, `ConverterRegistry::generation()` and `TemplateCatalog::generation()`. A wire that introduces an inference conflict is refused with `ConnectionError::TypeConflict` unless its components were already in conflict; plan commands are read-only. 6b (`be742f5`) adds `apply_connection`, which rechecks both and applies the commands in one transaction (`PlanApplyError::{StaleDocument, StaleRegistry, StaleTemplates, Edit}`). The selected provider is not a third check: registry snapshots are immutable, so equal generation implies it is still available. Limits: no cycle or region analysis (00d), no checked-partial exec/error wiring beyond the pure-node error rule, no UI dialog, and policy options are only the defaults or an explicit choice's values. Follow-up: provider bindings keep only package/version/id; the digest is the `package_lock` entry, maintained by the new `PutPackageLock` command (planner and paste put it; 12 §3.3).

Checks above use Windows x64, Rust/Cargo 1.99.1, default features and unchanged `Cargo.lock`. Accepted commit bodies record exact commands and lockfile identity; no performance, remote CI or runtime gate is claimed. At the 7a commit (`499dad6`) `cargo test --workspace --locked` passed 183 tests across 22 test binaries, with `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, warning-denied `cargo doc` and `node tools/check-cargo-metadata.mjs` passing.

Use a cheaper available coding agent for a bounded task with settled inputs. Keep the same owner for its next task. The coordinator handles cross-owner design decisions; a cheaper model is not a reason to omit necessary context or acceptance checks. Do not give an agent all seven slices as one implementation request.

For each dispatch, supply: the task below, common requirements, its context set, accepted dependency handoffs, current commit/working-tree changes, owned paths, and the IDs of owners it may consult. The receiving agent reads the actual files it will change and the relevant normative sections. Owner consultation reduces repeated exploration; it does not replace checking the source.

Before changing another owner's API, message that owner with the caller's requirement, proposed change and affected invariant. The owner answers or makes the change, then both handoffs are updated. If the owner is unavailable, reconstruct context from the handoff and current code; label any unresolved assumption and involve the coordinator before dependent coding. Live agent context is temporary, so never make it the only record of a decision.

Only one agent writes a given file at a time. In the shared checkout, serialize implementation writes unless the coordinator has assigned disjoint paths and stable interfaces. `lib.rs`, `error.rs`, existing document/edit types and normative documents are coordinated shared files. Agents do not switch branches, commit, rebase, push or open PRs; the coordinator reviews explicit paths and makes coherent local commits. Planning/review can run concurrently without editing.

## Common requirements and completion

- Read [README](../README.md), [AGENTS](../AGENTS.md) and [07](07-delivery-plan.md), then only the task's relevant context. Paths below starting `src/` or `tests/` are relative to `app/crates/orange-model/`.
- Preserve Float Single/f32 and Double/f64 plus i32/i64/u32/u64. No implicit casts. The Double editor default does not default unresolved generic parameters to f64.
- Reuse `TypeTerm`, literal schemas, signature projection and document transaction/undo APIs. Keep the core independent of UI, engines, network and package hosting. No arithmetic kernels or cycle/recursion validator in this phase.
- Functions/modules have one clear responsibility. Document public entry points, inputs, ownership, outputs and failure behavior; keep graph analysis separate from mutation. Avoid speculative traits, placeholder modules and parallel representations of the same schema.
- Keep changed semantics, diagnostics, FR trace and ADR consistent in the same task. A missing detail below is a decision to resolve against the contracts, not permission to invent a silent rule. Ordinary implementation choices stay with the owner; escalate material contradictions to the coordinator.
- Use a few strong Rust behavior/property tests for the stated invariants and regressions. Existing small golden vectors stay; no exhaustive JSON suite or coverage-percentage target. Reuse test helpers only when they reduce real duplication.
- During a task, run focused checks. At slice completion run workspace tests, formatting and Clippy using the pinned toolchain and lockfile. Add rustdoc/metadata or other repository checks when affected; do not repeatedly run unchanged checks without cause.
- Return a compact handoff: task/status, base and accepted commit (or pending diff), owned paths/public entry points, decisions and invariants, exact checks/results/toolchain/features/lockfile, known limitations, and next consumer. Mark criteria passed only with evidence. Keep one current handoff per owner here when work begins; replace stale notes rather than append transcripts. The coordinator accepts the diff before dependents consume it.

## Context sets

These are reading boundaries, not copies of the contracts. Dependency handoffs add the implemented APIs and relevant tests as they appear.

| Set | Contract and source context |
|---|---|
| F | [11 §§5.1–5.4, 16](11-graph-language.md), [12 §§5, 6.1, 7–8](12-graph-data-structures.md), [08 ADR-16/20](08-decisions.md); `src/type_term.rs`, `src/id.rs`, `src/value/{mod.rs,literal_codec.rs}`, existing literal tests |
| S | [11 §§4.4–4.5, 5.5, 17](11-graph-language.md), [12 §§3.1–3.3, 8](12-graph-data-structures.md), [08 ADR-16/20](08-decisions.md); `src/{type_term.rs,signature.rs,index.rs}`, `src/document/{node.rs,port.rs,declaration.rs}`, `src/edit/{transaction.rs,change_set.rs}`, signature/transaction tests |
| C | [11 §§4.2–4.4, 6, 17](11-graph-language.md), [12 §§3.1–3.3, 5, 8](12-graph-data-structures.md), [04 installation/override](04-expansion-plugins.md), [08 ADR-06/07/16](08-decisions.md); `src/document/{graph.rs,node.rs,port.rs}`, `src/edit/transaction.rs`, `src/{signature.rs,index.rs}`, transaction tests |
| Q | Accepted F/S/C handoffs, [06 validation](06-quality-and-validation.md), [14 checks](14-git-workflow.md), [01 FR-08/09/44–46/51](01-product-requirements.md), current CI and Cargo manifests |

## Ready order

Tasks 1a/1b, 2a/2b, 3a/3b, 4a/4b and 5a/5b are accepted locally; 6a, 6b, 7a and 7b are implemented and checked locally, pending coordinator acceptance. Dependencies mean reviewed, working outputs, not only a proposed interface. The principal path is 1→2→3→4→6→7.

| Task | Owner | Depends on | Result |
|---|---|---|---|
| 00c.1a | F | Existing 00b/parser | Core type resolution and snapshot-local identity |
| 00c.1b | F | 1a | Nominal package schemas through the same resolver |
| 00c.2a | F | 1b | Constraint evaluation and map key eligibility |
| 00c.2b | F | 2a | Coherence-checked constraint registration |
| 00c.3a | S | 1a | Structural substitution in signatures; parser-only work is independent of package registration |
| 00c.3b | S | 3a, 1b | Node-scoped structural unification; graph inference consumes constraint proofs in 4a |
| 00c.4a | S | 3b, 2b | Read-only affected-component inference |
| 00c.4b | S | 4a | Atomic persistence of inference results |
| 00c.5a | C | 1b | Validated direct converter entries; ownership uses the published type registry |
| 00c.5b | C | 5a | Deterministic selection/choice outcomes |
| 00c.6a | C | 4b, 5b | Read-only connection plan |
| 00c.6b | C | 6a | Atomic converter insertion and undo/redo |
| 00c.7a | Coordinator | 6b | End-to-end test and maintainability review |
| 00c.7b | Coordinator | 7a | Final checks and truthful evidence |

## Task briefs

### 00c.1a — Core resolution and identity

Context: F; consult S about the resolution result it will consume.

Requirements: resolve parsed terms against the v1 core constructor table with exact arities. Distinguish nominal name/major and numeric widths. Intern resolved concrete types within their owning snapshot; unresolved parameters remain solver terms, not pretend concrete types. Define key ownership and lookup failure explicitly; no transient key enters documents, locks or signature hashes.

Acceptance: nested core types resolve; unknown names/majors and wrong arity fail with useful type context. Repeated resolution in one snapshot returns the same identity. Lookups carry or validate the owning snapshot, rejecting a key from another snapshot even if its numeric index exists there; numeric key values need not differ across snapshots or match across insertion orders. This task validates `handle<K>` arity; registered handle-kind validation follows in 1b. No second parser or empty registry facade.

### 00c.1b — Package types and codec integration

Context: F plus the accepted 1a API; coordinate changes to the existing literal schema API.

Requirements: resolve nominal records/enums and their field/payload types, adapting `Schemas`/`SchemaSet` rather than introducing a duplicate schema system. Register nominal handle-kind metadata and require it for `handle<K>`; it has no literal codec. Preserve codec and unknown-data behavior. Bound or reject recursive schema expansion with a documented outcome; do not allow unbounded resolution.

Acceptance: a representative record and enum resolve and pass through literal encode/decode using the shared schema source. Same-shaped types in different namespaces/majors stay distinct; unresolved field types and a non-handle-kind argument to `handle<K>` fail. Existing literal tests remain green. Resolve the 12 §7 `RecordRepr::ty` migration note: demonstrate an owning consumer path that keeps the matching snapshot alive for every key use, or explicitly defer that representation change and retain canonical `TypeExpr` until such a consumer exists. Never put a detached snapshot-local key into a freely owned `Value`.

### 00c.2a — Constraint evaluation

Context: F and 1b; consult C about converter metadata needs, without implementing conversion kernels.

Requirements: evaluate core constraints and declared structural derivations for package records/enums. Validate map keys using language `hash` + `eq` with float exclusion. Structural `Value::Eq/Hash` is duplicate-check machinery, not proof of language constraints. Settle aggregate propagation and any core-table ambiguity against 11 §5.4 before encoding it; bound recursive evaluation.

Acceptance: integer/float constraint distinctions work; a valid map key passes and an ineligible key fails through the registry-aware codec path. One derived package type passes and a field/payload without the constraint causes failure. Existing codec-only callers have an explicit compatibility path; no silent bypass for registered types. No arithmetic is executed.

### 00c.2b — Coherence at registration

Context: F, [04 namespace/registration rules](04-expansion-plugins.md), accepted 2a.

Requirements: validate registration authority from the supplied owning-package identity. Only a type's owner implements a core constraint for it; a package may implement its own constraint for a foreign type. Reject conflicting implementation identities before publishing the registry view. Reuse provider/implementation references instead of invoking guest code. Authentication and package installation remain WU-03.

Acceptance: owning implementation succeeds; foreign implementation of a core constraint and duplicate conflict fail without partially publishing registrations. Cover the allowed package-owned-constraint case. The handoff identifies what authority is assumed from the future package host and what this layer actually checks.

### 00c.3a — Structural substitution

Context: S, 1b; F owns parser/resolution invariants.

Requirements: replace signature token scanning with recursive substitution over `TypeTerm`. Preserve unresolved parameters for diagnostics and the current public signature behavior. Define malformed-input behavior explicitly because `TypeExpr` currently checks nonblank text, not grammar; do not introduce a new panic or an unrelated API break.

Acceptance: nested substitutions work, missing bindings remain unresolved, and same-prefix parameter names are distinct. Existing semantic signature hashes and presentation invariance remain unchanged. Substitution is reusable by the solver; no graph mutation or separate parser.

### 00c.3b — Node-scoped unification

Context: S plus F's resolution/constraint handoff; consult F before changing its APIs.

Requirements: unify nested terms with variables identified by node instance and parameter name. Detect incompatible nominal identities/arity, unresolved variables and recursive bindings. Keep constraint evaluation in F's implementation. Define deterministic input/output ordering for the graph caller; never coerce types or insert nodes.

Acceptance: nested generics resolve, unrelated nodes' `$T` variables remain distinct until an actual equation links them, and mismatches/recursive equations terminate with contextual failures. Equivalent equation orders yield equivalent successful bindings. Return analysis data with enough node/port/type context for `E-TYPE-MISMATCH` and `E-TYPE-AMBIGUOUS`, without localized prose baked into the solver.

### 00c.4a — Component inference and specialization

Context: S and F handoffs; C consumes this analysis later.

Requirements: collect the affected generic components before/after an edit, including both sides of a disconnect. Traverse in stable `NodeId` order, use definition templates rather than treating persisted resolved signatures as generic templates, seed typed literals/pins, solve, then check constraints and exact-provider specialization support. Supply minimal host-side definition metadata; do not implement package loading or the complete port-template language.

Resolve here: `type_args` stores inferred output but currently has no distinct pin provenance. Define persistence, backward compatibility and disconnect/reconnect semantics for explicit pins versus inferred bindings in 11/12 and the ADR before implementing that behavior. Also define how ambiguous editable documents retain diagnostic/signature information; do not make every incomplete edit an unrepresentable document.

Acceptance: a typed source resolves generic math; disconnect/reconnect does not reuse stale inferred output as a pin. Ambiguity, conflicting pins and missing specializations produce typed outcomes mapped to the specified codes, without implementing a general diagnostic store/validator. Shuffled node/edge insertion yields identical successful `type_args`; unrelated components remain untouched. Analysis alone never changes the graph. Tests cover component splitting, not a large graph-case matrix.

### 00c.4b — Persist inference atomically

Context: S, accepted 4a and existing transaction/inverse semantics; coordinate shared edit/document changes.

Requirements: turn accepted inference results into existing document commands, including updated semantic signatures when needed. Combine the originating edit and inferred changes into one transaction. Keep low-level `GraphDocument::apply` structural; do not add the future semantic validator to every storage command. Preserve loaded documents without requiring inference on load.

Acceptance: resolved bindings survive save/load; one undo restores prior content/bindings/signature references and redo restores the accepted result. A failed commit preserves content, revision and state identity. Diagnostic-only analysis has no accidental partial mutation. No changes to unrelated nodes or provider locks.

### 00c.5a — Converter registration and binding

Context: C, F's identities/coherence handoff; consult F about type ownership and the coordinator about override semantics.

Requirements: register direct from/to entries with conversion class, declared policies/errors, stable entry identity and exact provider reference. Check ownership of source or target; validate competing providers through the documented converter override slot, not last-installed-wins.

Resolve here: 12 §8 allows multiple entries per pair while 11 §6 requires uniqueness for auto-insertion. Specify entry identity versus provider alternatives, what eligible set is counted, and how explicit selection/defaults affect a new binding. Preserve existing locks and never silently rebind them. Update the affected contract/ADR together; do not resolve ambiguity by choosing registry iteration order.

Acceptance: valid owned registration succeeds; foreign-to-foreign and conflicting content under the same entry/provider identity fail without partial publication. Distinct providers for the same pair/slot remain selectable under the stated eligibility rule; existing bindings remain unchanged after defaults change. No installer, runtime converter invocation or general override host.

### 00c.5b — Selection and choice outcomes

Context: C and accepted 5a; S supplies resolved endpoint types.

Requirements: expose explicit outcomes for already-compatible endpoints, automatic direct lossless conversion, choice-required candidates, and rejection. Follow 5a's eligibility rule; partial/lossy candidates expose their policy/loss information. Offer direct entries only, with stable candidate ordering; no automatic chains.

Acceptance: i32→i64 auto-selects the unique eligible lossless entry; i64→i32 requires choice; ambiguity never picks arbitrarily; no available entry rejects. Explicitly choosing a partial/lossy candidate or provider alternative returns that exact entry/provider, without fallback. An intermediate path without a direct entry is not auto-inserted. Results contain provider/entry identity and do not mutate a document or execute conversion math.

### 00c.6a — Connection planning

Context: C plus accepted S/F interfaces; consult S about inference changes and coordinator about persisted converter shape.

Requirements: plan a compatible wire or visible converter insertion without modifying the graph. Validate body/endpoints, data-port direction, relevant fan-in/replacement and resolved types. Consume an explicit choice when required. Define the `core/convert` persisted entry/config/provider/signature shape against 11 §6 and 12 before creating instances. Generate identities once per accepted plan; record an opaque document-content precondition, registry generation and exact provider binding. Revision alone is insufficient across clones/reloads.

Acceptance: fresh wire and replacement plans contain the necessary commands, chosen identity and inferred updates. Choice-required/rejected outcomes return no applicable mutation. Missing/orphan endpoints and stale inputs have explicit failures; unrelated region/cycle analysis stays in 00d. Preserve shadowed literals. No UI dialog implementation.

### 00c.6b — Apply, undo and redo

Context: C, accepted 6a and transaction tests; S reviews inferred updates and signature handling.

Requirements: recheck a plan's document/snapshot preconditions and apply its commands in one existing transaction: optional disconnect, converter/signature addition, two edges and required inferred changes. Reuse the existing inverse machinery. Reject stale plans rather than applying analysis from old graph/provider state.

Acceptance: successful insertion advances revision once and exposes one undo step. Replacement removes the old edge in that step. Undo restores prior content, including shadowed literals; redo restores inserted IDs and exact provider binding (revisions still advance normally). An injected command failure rolls back all additions/changes. Changed document content, a replaced registry snapshot or an unavailable selected provider rejects the plan without changing content/revision; use focused stale-precondition checks. Existing direct connection behavior remains available.

### 00c.7a — Integrated flow and readability review

Context: Q and all accepted owner handoffs. Ask the owning agent before changing its implementation.

Requirements: add one small end-to-end Rust test covering resolve→infer→select→insert→save/load→undo/redo, using existing test support where useful. Review the actual public calling path, duplication, error context and ownership. Fix actionable findings with the owner; do not refactor unrelated code or invent an abstraction merely to complete this task.

Acceptance: the integration test proves persisted canonical types/provider selection and reversible document content. Public docs make analysis versus mutation and snapshot lifetime clear; a reviewer can follow the entry points across modules. Contract decisions from 4a/5a/6a are implemented consistently, with no orphan placeholder API or duplicate schema store.

### 00c.7b — Final verification and handoff

Context: Q, final accepted diff and current CI. No new behavior unless a check exposes a defect; route fixes to its owner and rerun affected checks.

Requirements: collect final-version evidence for `cargo test --workspace --locked`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, warning-denied rustdoc and `node tools/check-cargo-metadata.mjs`. Reuse already-current successful results; rerun when accepted changes invalidate them. Run other changed-area checks required by current CI. Review the final diff and trace implemented FR-08/09/44–46/51 portions against existing requirements; do not claim their later UI/runtime/package portions complete.

Acceptance: record actual toolchain, features, lockfile identity, commands/results and unresolved limitations in the commit/PR evidence. No planned test is marked run; no NFR-15, G2 or cross-OS result is inferred from local unit tests. The coordinator marks accepted tasks complete and supplies the next phase's compact handoff. Publishing follows the user's authorization; this task packet alone does not authorize pushing or opening a PR.

## Final trace (00c.7b)

Core-model portions only, traced to code and tests that ran in the workspace suite above. Nothing here is a UI, runtime, package-host, performance or cross-OS result, and no requirement is marked complete.

| FR | Core portion implemented | Tests | Not implemented here |
|---|---|---|---|
| FR-08 | Scalar widths (i32/i64/u32/u64, f32/f64), aggregates and nominal record/enum literals encode and decode with stable type IDs through the shared schema source | `literal_scalars`, `literal_aggregates`, `literal_props`, `codec_*`, `types` unit tests | Null/missing/Error distinctions beyond the 00b codec; runtime values |
| FR-09 | Frozen type registry with versioned schemas, handle kinds, constraint registration and coherence | `types` and `constraints_tests` unit tests | Package codec registration, editor descriptions, retaining payload of an unsupported codec |
| FR-44 | Persisted signature snapshots (projection, canonical key, drift comparison); `SetSignature`/`PutSignature`; data-input replacement and endpoint validation when planning a connection | `signature_vectors`, `inference_persist`, `connection_plan` | The connection matrix and fan-in enforced at edit time (validator, 00d), dynamic port templates, kind inference |
| FR-45 | Canonical type terms, constraints, structural unification, deterministic component inference with persisted `type_args`/`type_pins`, typed outcomes mapped to E-TYPE-* codes, one-transaction persistence | `component_inference`, `inference_persist`, `connection_flow` | Compile diagnostics and their store, pin UI, package specialization loading |
| FR-46 | Converter entries/providers with ownership, class-based selection, never-chained direct conversion, visible `core/convert` insertion with undo/redo | `conversion` unit tests, `connection_plan`, `connection_apply`, `connection_flow` | Cycle classes, runtime conversion kernels and loss output production, converter picker UI |
| FR-51 | Inference and selection results are typed values with stable codes and no prose or locale dependence | `component_inference` (codes) | Catalogues, message arguments, redaction, plurals, UI language switching |
