# WU-00c implementation tasks

Status: implementation started on `feat/wu-00c-types-conversions`; 00c.1a/1b and 3a are accepted locally. This is the task packet for the seven slices in [07](07-delivery-plan.md). Keep it as the working handoff; do not create a README, fixture inventory or separate report for every task. The existing development branch retains its cleanup commits and receives small implementation commits.

## Ownership and context

Assign a continuing developer/agent to each role. A role can complete several tasks in sequence; send follow-ups to the same agent to retain its working context. Bind roles to actual agent IDs when implementation starts, and record that mapping here. Roles are responsibilities, not a requirement to launch four agents at once.

| Role | Owns | Consult this owner about |
|---|---|---|
| F — type foundations | 1a, 1b, 2a, 2b | Registered types, snapshot/key lifetime, schemas, constraint evaluation and coherence |
| S — inference | 3a, 3b, 4a, 4b | Structural substitution, unification, explicit pins, component traversal and inferred bindings |
| C — connections | 5a, 5b, 6a, 6b | Converter entries/provider selection, connection plans and insertion |
| Coordinator | 7a, 7b and integration | Contract decisions, shared files, cross-owner APIs, review and task acceptance |

Current agents: F = `/root/type_foundations`, S = `/root/inference`, C = `/root/graph_authoring`, coordinator = `/root`. Consult these owners for follow-up work in this conversation; reconstruct from their handoffs if a later session no longer has those live agents.

Current handoffs: F delivered 1a/1b on base `0059ae0`: `types::TypeRegistryBuilder` registers existing record/enum schemas and handle kinds, resolves keys, then builds frozen `TypeRegistry`; consumers use read-only `validate`, `expression`, `owner`/`kind` or term-based `owner_of`/`kind_of`. Production responsibilities are separated into registry, identity and errors modules. Workspace tests, nine focused type tests, formatting, Clippy, warning-denied rustdoc and architecture metadata checks passed with Rust/Cargo 1.99.1, default features and unchanged lockfile. `RecordRepr::ty` stays canonical text; general map-key constraints remain 2a. S delivered 3a in `0059ae0`: signature substitution uses `TypeTerm` with simultaneous replacements and a depth bound; `try_substitute_params` reports errors, the compatibility wrapper returns invalid opaque input unchanged. Six signature and three parser tests passed; workspace checks include existing signature vectors. C has settled entry/provider identity and prelocked-package planning with the coordinator; no C implementation yet.

Use a cheaper available coding agent for a bounded task with settled inputs. Keep the same owner for its next task. The coordinator handles cross-owner design decisions; a cheaper model is not a reason to omit necessary context or acceptance checks. Do not give an agent all seven slices as one implementation request.

For each dispatch, supply: the task below, common requirements, its context set, accepted dependency handoffs, current commit/working-tree changes, owned paths, and the IDs of owners it may consult. The receiving agent reads the actual files it will change and the relevant normative sections. Owner consultation reduces repeated exploration; it does not replace checking the source.

Before changing another owner's API, message that owner with the caller's requirement, proposed change and affected invariant. The owner answers or makes the change, then both handoffs are updated. If the owner is unavailable, reconstruct context from the handoff and current code; label any unresolved assumption and involve the coordinator before dependent coding. Live agent context is temporary, so never make it the only record of a decision.

Only one agent writes a given file at a time. In the shared checkout, serialize implementation writes unless the coordinator has assigned disjoint paths and stable interfaces. `lib.rs`, `error.rs`, existing document/edit types and normative documents are coordinated shared files. Agents do not switch branches, commit, rebase, push or open PRs; the coordinator reviews explicit paths and makes coherent local commits. Planning/review can run concurrently without editing.

## Common requirements and completion

- Read [README](../README.md), [AGENTS](../AGENTS.md) and [07](07-delivery-plan.md), then only the task's relevant context. Paths below starting `src/` or `tests/` are relative to `app/crates/orange-model/`.
- Preserve Float Single/f32 and Double/f64 plus i32/i64/u32/u64. No implicit casts. The Double editor default does not default unresolved generic parameters to f64.
- Reuse `TypeTerm`, literal schemas, signature projection and document transaction/undo APIs. Keep the core independent of Bevy, UI, network and package hosting. No arithmetic kernels or cycle/recursion validator in this phase.
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

Tasks 1a/1b and 3a are accepted locally; remaining tasks are pending. Dependencies mean reviewed, working outputs, not only a proposed interface. The principal path is 1→2→3→4→6→7. Converter work 5 may proceed after its listed prerequisites, alongside inference work if file ownership allows it.

| Task | Owner | Depends on | Result |
|---|---|---|---|
| 00c.1a | F | Existing 00b/parser | Core type resolution and snapshot-local identity |
| 00c.1b | F | 1a | Nominal package schemas through the same resolver |
| 00c.2a | F | 1b | Constraint evaluation and map key eligibility |
| 00c.2b | F | 2a | Coherence-checked constraint registration |
| 00c.3a | S | 1a | Structural substitution in signatures; parser-only work is independent of package registration |
| 00c.3b | S | 3a, 2a | Node-scoped structural unification |
| 00c.4a | S | 3b | Read-only affected-component inference |
| 00c.4b | S | 4a | Atomic persistence of inference results |
| 00c.5a | C | 1b, 2b | Validated direct converter entries |
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

Acceptance: record actual toolchain, features, lockfile identity, commands/results and unresolved limitations in the commit/PR evidence. No planned test is marked run; no NFR-15, G2, Bevy or cross-OS result is inferred from local unit tests. The coordinator marks accepted tasks complete and supplies the next phase's compact handoff. Publishing follows the user's authorization; this task packet alone does not authorize pushing or opening a PR.
