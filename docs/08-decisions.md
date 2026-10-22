# Architecture decision records

Status key: **Selected** is the planning default; **Conditional** must pass a named feasibility probe before production adoption; **Proposed** awaits the product owner's acceptance and changes no contract until accepted. Neither means implemented. Owners are responsibility roles for future work, not newly assigned people. Primary evidence was retrieved 2026-10-20; moving/development API sources are not exact dependency pins.

## ADR-01 — Standalone typed graph core

**Selected.** `orange-model/compiler/runtime` have no UI, window or engine dependency. Desktop and CLI share the same execution plan/contracts. Alternatives: map every node to ECS systems/schedules; execute directly from widgets; adopt an unspecified external graph engine. Choose a compact plan because data dependencies, explicit effect sequencing, state visibility and async continuations are language semantics independent of frames/ECS. Consequence: Orange owns compiler/debug/source-map work; engine scheduling is not free infrastructure.

Evidence: project requirements FR-07–16 and the headless probe P-05 ([09](09-feasibility-probes.md)); this is an architectural inference, not a benchmark conclusion. **Falsification:** the WU-00 reference interpreter and the WU-04 executor agree on the G-suite and on compiled pure/effect/async fixtures; P-03/05 completes network/timer work while the UI is paused and the core build has no UI or engine dependency. Failure of these invariants requires redesign, not another frame-loop workaround. Owner runtime lead; G2.

## ADR-02 — Editor UI on egui; no Bevy

**Selected by the user, 2026-10-21; Conditional** until the probe below passes. Replaces the earlier Bevy 0.20 native UI/Feathers direction. The editor UI is a Rust library on **egui**, separate from the core (ADR-01, ADR-21). The reference desktop app hosts it through `eframe`. The UI crate depends on `egui` only, not on `eframe`, so a downstream product can host the same editor inside its own window or engine (`egui-wgpu`/`egui-winit` or another egui integration). **Bevy is dropped from the project** (user decision, 2026-10-21): no Bevy application, UI or adapter. A downstream product built on an engine embeds the core and the egui UI on its own side.

Why egui:

- **Accessibility and input:** first pure-Rust toolkit with platform accessibility through AccessKit; IME works; 2026 community tests rate its IME and screen-reader support among the best Rust toolkits (FR-06, NFR-10).
- **Custom canvas:** immediate mode suits a node canvas that culls and draws only visible nodes and wires each frame (FR-07, NFR-03) and repaints only on input or animation (NFR-12).
- **Embeddable UI layer:** the same editor runs standalone or inside any wgpu/winit host, which ADR-21's exportable UI needs.
- **Testable:** `egui_kittest` drives the UI through the AccessKit tree, so focused UI tests also check accessible names and actions.
- **Ecosystem and license:** MIT/Apache-2.0; existing node-graph crates (for example `egui-snarl`) show the canvas pattern, though OrangeEditor keeps its own canvas over its document model.

**Known gap:** epaint's text layout does not implement bidirectional (RTL) reordering and strips bidi control characters (observed in the current source, epaint 0.36). 13 and OPEN-13 require RTL validation. Options, decided by probe: render editor text through a shaping/bidi engine (Parley or cosmic-text) behind egui's text APIs; ship LTR locales first and record the limitation; or switch toolkit.

**Alternatives:**

| Toolkit | Why not selected now |
|---|---|
| Bevy 0.20 UI/Feathers (previous direction) | Couples the editor to a game engine's schedule and version; text, IME and accessibility were the open P1 risk; harder to embed in non-Bevy products. |
| Slint | Good IME/accessibility, but declarative markup fits a dynamic node canvas less naturally; royalty-free license requires attribution in every downstream product, or GPLv3 or a paid license. Fallback candidate. |
| iced | Elm architecture and good text shaping, IME added in 0.14, but no screen-reader support yet (issue open since 2020); fails NFR-10. |
| Xilem/Masonry | Parley text and AccessKit, but still alpha. Revisit later. |
| GPUI (Zed) | Fast, but partial accessibility, no standard text input widget, and tied to Zed's release cadence. |
| Web UI (Tauri plus a JS node library) | Mature node editors and accessibility, but the UI would not be Rust and is harder to embed in Rust products. |

**Consequences (applied 2026-10-21):**

- AGENTS.md has no Bevy rules; the selected egui version is pinned when adopted.
- [09](09-feasibility-probes.md) now lists engine-free probes P-01–06, and the Bevy source register left [10](10-tooling-and-sources.md); [02](02-architecture.md) `orange-desktop` is an eframe app over an `orange-editor-ui` crate; [07](07-delivery-plan.md) WU-01/02 target egui; FR-41 scene handles are downstream work (ADR-21); NFR-14 requires no UI or engine dependency in the core.
- Pin the exact egui/eframe versions and record MSRV and license review in [10](10-tooling-and-sources.md) when the probe starts, as for any dependency.

**Falsification (Conditional probe, replaces P-01/02/06 for the UI):** on Windows, Linux and macOS, demonstrate IME composition with CJK, clipboard, DPI and 200% scale, keyboard-only authoring of a small graph, screen-reader names, roles and actions for nodes, ports and links, an RTL plan that meets 13, and W4 graph frame time within NFR-03. A required capability that cannot be met → compare Slint, then the others, before WU-05. Owner: editor/platform lead; G1.

## ADR-03 — Component-first expansion, trusted worker escape hatch

**Selected, performance/SDK conditional.** Wasm components are normal installable code, importing only versioned host capabilities. Trusted external workers permit unsupported native/library integrations and register service adapters through the same broker-facing contracts. Statically linked built-in kernels/backends are explicit privileged exceptions. Alternatives: Rust dylib ABI, C ABI plugins, all workers, rebuilding Bevy crates. Components give portable hot addition and a guest memory boundary; workers supply practical native reach at IPC/trust cost. C ABI can stabilize a call layout but cannot isolate host crashes or revoke native OS authority.

Consequences: no arbitrary raw ECS/render/widget injection; custom panels/value editors use declarative schema; scene changes use handles; tiny guest calls need batching; native workers are not sandboxed by a manifest. Evidence: [Rust external ABI](https://doc.rust-lang.org/reference/items/functions.html#extern-function-qualifier), [Wasmtime security](https://docs.wasmtime.dev/security.html), [interruption caveats](https://docs.wasmtime.dev/api/wasmtime/struct.Config.html); specific dependency version still pending. **Falsification:** WU-03 installs without restart, traps an isolated guest, kills worker, denies ungranted imports, adds a panel/type/service and benchmarks W3. If essential expansion cannot fit these boundaries, revise the public interface before SDK freeze. Owner SDK lead; G1/G2.

## ADR-04 — Compiled plan, explicit purity/effects and batching

**Selected.** Resolve types/ports/provider dispatch before execution; pure values memoize by compile-time stability class (const/activation/epoch, 11 §7.4); effects execute through sequenced control tokens and async continuations. Standard native kernels run compact batches. Guest batches/macro-nodes are optional optimizations that retain locked provider behavior and source/error mapping. Alternatives: per-node dynamic map lookup; per-sample messaging; arbitrary parallel dispatch; automatic lowering of every guest to a standard math implementation. Chosen design targets low overhead without substituting semantics.

Consequences: optimizer/debug contracts require equivalence tests; side-effecting work cannot be optimized as pure; selected custom provider may be slower than native. Evidence: language design in [03](03-data-and-execution.md), architecture proposal rather than measured Rust advantage. **Falsification:** V01 reference equivalence plus W1/W2/W3 direct/native/component scalar/batched profiles. Fail NFR-01 → optimize or revise batching targets explicitly. Ordinary floating rounding is not a failure; explicit numeric-domain/overflow/cast rules are. Owner runtime lead; G2.

## ADR-05 — Dedicated async host and owned cancellation

**Selected.** Tokio owns I/O/timers; bounded compute executor owns CPU work; the UI thread only bridges commands and view changes. Run/session ownership, generations, bounded queues and continuation state machines determine cleanup. No whole-run pause in 1.0; debugger suspends eligible continuations while external lifetimes continue. Alternatives: frame polling for all I/O; unconstrained task spawning; running network work on UI-owned task pools. Consequence: separate executor integration and lifecycle tests are required; a blocked OS syscall may outlive logical cancellation.

Evidence: [Tokio runtime](https://docs.rs/tokio/latest/tokio/runtime/) resolved 1.54.0, not a project pin; bridge probe P-03 ([09](09-feasibility-probes.md)). **Falsification:** P-03 and WU-04 saturate event sources, pause the UI, race stop/completion, restart generations and verify NFR-02/13. Long UI freeze or stale continuation delivery rejects the model. Owner runtime/services lead; G1/G2.

## ADR-06 — Stable schema IDs, exact locks and immutable generations

**Selected.** Declarative definitions/types/config/codec versions plus exact package/provider digest lock; preserve unknown payloads; migrations on copies; registry snapshots immutable. Small tagged values/large immutable buffers are independent of Rust TypeId/Bevy Entity. Directory bundles commit immutable generation through atomic current-manifest replacement; archives use sibling-temp replacement. Alternatives: serialize native objects/pointers; latest-package resolution on open; rewrite unknown nodes away; assume directory replacement is universally atomic.

Consequences: multiple package versions can coexist while runs drain; disk reclamation and migration tooling required; missing provider blocks execution but preserves authoring data. Evidence: document model [03](03-data-and-execution.md) and filesystem-specific verification requirement [05](05-services-and-platforms.md); no universal crash-durability guarantee. **Falsification:** save/load/migrate cross-language vectors and crash injection at every registry/document commit step; selected provider must remain unchanged after package update. Owner model/persistence lead; G2/G3.

00b implementation review correction: undo tokens identify transaction-owned content states independently of persisted revision. A clone can share an inverse before divergence; new edits and loads allocate unique transient identities. Function-body maps reject key/entry ID mismatches atomically. Ordered replacements preserve their slot without remove/reinsert. These enforce existing identity/edit invariants; graph format and language rules are unchanged. Regression evidence is recorded in [10](10-tooling-and-sources.md#model-review-fixes-and-slice-4-analysis-2026-10-20).

## ADR-07 — Explicit overrides, hot addition and safe draining

**Selected.** Public typed slots cover node implementations, connection/service providers, audio control/backend under boundary limits and editor value/file-picker contributions. User selection defines defaults for new bindings; existing graphs require explicit rebind. Hot add publishes atomically; old runs pin old code; remove drains/cancels/revokes with barriers. Alternatives: last-installed-wins, monkey-patching internals, forced app restart, unloading a library with live pointers. Consequence: uninstall can leave pinned package bytes until references close while app stays open; rollback cannot undo external effects.

Evidence: lifecycle design [04](04-expansion-plugins.md). **Falsification:** WU-03 competing providers, in-flight network callbacks, denied revocation, stale handles and failed migration; unrelated graphs remain running and original graphs never silently switch implementation. Owner SDK lead; G2.

## ADR-08 — Incremental desktop editor and accessible declarative contributions

**Selected, adapter conditional.** Persistent view state keyed by document IDs, transaction change sets, spatial index/culling, retained text edit session and virtualized controls. Guest UI is declarative with host accessibility labels/actions, not native widget pointers. Alternatives: full scene/UI rebuild on every graph change; raw guest widgets; draw-only canvas without accessibility model. Consequence: host provides a bounded widget vocabulary and versioned extension events; advanced widgets may require new host primitives.

Evidence: FR-01–07, W4 performance workload and UI probe P-02 ([09](09-feasibility-probes.md)). **Falsification:** WU-02/05 single-node edit preserves unrelated focus/caret/selection, clipboard/IME works, screen reader traverses graph semantics and W4 meets NFR-03. Owner editor lead; G1/G3.

## ADR-09 — Host credential broker and versioned connection profiles

**Selected.** OS vault refs in profiles/graphs; host owns auth/session/refresh. Guests receive checked authenticated operations or allowlisted auth transforms; arbitrary novel algorithm requires reviewed host primitive or explicit raw-secret grant. Generic WebSocket does not understand OBS: `org.orange.obs/auth-challenge@1` is a namespaced bundled host transform and OBS session adapter validates method-level requests when exposing that authority. Alternatives: plaintext application vault, credentials in node config, raw secrets to every plugin, assume native permissions enforce isolation.

Consequences: locked/missing OS vault disables dependent operations; public desktop OAuth never relies on an embedded client secret; raw transport and protocol-method authority are distinct. Evidence: [WinCred](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew), [Secret Service](https://specifications.freedesktop.org/secret-service/latest/), [Apple Keychain entry point](https://developer.apple.com/documentation/security/keychain-services) (JS-only limitation), [05](05-services-and-platforms.md). **Falsification:** WU-06 and V09 test leaked canaries, redirects, locked vault, path/destination denial, auth-transform inputs and atomic token rotation; worker trust disclosure must accurately describe OS authority. Owner security/services lead; G4.

## ADR-10 — Separate protocol contracts, AMQP0-9-1 baseline

**Selected assumption.** HTTP/REST/GraphQL, WebSocket, MQTT5/3.1.1, AMQP0-9-1, gRPC/HTTP2/protobuf and inbound webhooks have separate typed settings/lifecycle. Initial AMQ means RabbitMQ-compatible AMQP0-9-1; AMQP1.0 is later adapter. All gRPC streaming shapes are M baseline, delivered after a small unary slice. Alternatives: generic request node with opaque untyped settings; claim HTTP alone supplies gRPC/listeners; treat AMQP versions as interchangeable.

Consequences: larger explicit fixture matrix and no universal exactly-once/retry promise; API convenience nodes can share transport services without sharing delivery semantics. Evidence: [HTTP](https://datatracker.ietf.org/doc/html/rfc9110), [MQTT5](https://docs.oasis-open.org/mqtt/mqtt/v5.0/os/mqtt-v5.0-os.html), [RabbitMQ semantics](https://www.rabbitmq.com/docs/semantics), [gRPC concepts](https://grpc.io/docs/what-is-grpc/core-concepts/), sources/limits in [05](05-services-and-platforms.md). **Falsification:** WU-07–09 protocol fixtures, broker restart/duplicates, gRPC half-close/deadlines/trailers, webhook bind/auth/stop; failed broker interoperability changes published capability matrix, not protocol labels. Owner protocol lead; G4.

## ADR-11 — Native audio service and explicit platform routing

**Selected, adapter conditional.** Host audio assets/players/mixer use native preallocated buffers and output leases. Linux PipeWire virtual source is an early required probe; Windows/macOS use existing endpoints, with no driver supplied in baseline. Guest algorithms may prepare buffered PCM but do not execute on real-time callbacks. Alternatives: let a default sink determine every route; run guest calls in callback; create a virtual microphone by renaming a playback sink; assume user-space Windows app can install no driver yet expose a new endpoint.

Consequences: backend/decoder/resampling/clock/hotplug testing is a first-class workstream; multi-device clock sync is not promised. Evidence: [PipeWire loopback 1.6.9](https://pipewire.pages.freedesktop.org/pipewire/page_module_loopback.html), [Windows SYSVAD](https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample/), [Apple driver entry](https://developer.apple.com/documentation/coreaudio/creating-an-audio-server-driver-plug-in) (JS-only detailed evidence gap). **Falsification:** P-04 with independent app capture of Orange source, clean removal on exit, existing Windows cable routing and V10 callback/decode/hotplug checks. Owner audio lead; G1 probe/G5.

## ADR-12 — Bundled integrations remain bounded expansion packages

**Selected.** OBS and Twitch ship as package definitions/components/subgraphs reusing shared transport/profile/vault service contracts; only approved secret transforms/native platform primitives are host privilege exceptions. Initial node/event lists are exactly [05](05-services-and-platforms.md). Third-party OBS package obtains protocol-aware service authority rather than duplicating unmanaged socket lifetime. Twitch public device flow is preferred, with single-use refresh rotation and validation/rate/session policies.

Alternatives: hard-code all integrations into editor/core; promise entire upstream APIs; ship a confidential Twitch client secret. Consequences: service/SDK interfaces must be sufficient for real expansions; upstream changes require package updates/conformance fixtures. Evidence: [OBS5 protocol](https://raw.githubusercontent.com/obsproject/obs-websocket/master/docs/generated/protocol.md), [Twitch public device flow](https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/#device-code-grant-flow), [validation](https://dev.twitch.tv/docs/authentication/validate-tokens/), [EventSub](https://dev.twitch.tv/docs/eventsub/handling-websocket-events/); specific refresh-page conflict recorded in 05. **Falsification:** WU-11 can add a third-party OBS workflow without host rebuild or duplicate socket; public Twitch refresh succeeds without confidential secret; revoked/expired tokens terminate or reauthorize cleanly. Owner integration lead; G5.

## ADR-13 — Hybrid execution model: pull data, token control, async suspension, event activations

**Selected.** Data is pulled lazily and memoized by stability class. Control is explicit exec tokens on strands. Waiting is cooperative suspension with continuations. The outside world, time and custom emits enter as queued events, each starting an activation. Reactivity (variable-change and Watch triggers, live preview) is a layer evaluated at segment boundaries, and streams are pull-based async iterators ([11 §2](11-graph-language.md)). Alternatives:

- a synchronous call-stack interpreter: blocks on I/O, long loops freeze, recursion depth;
- push dataflow/FRP everywhere: undefined effect order, diamond glitches, effects repeated on propagation;
- a task per node: per-node scheduling cost, nondeterministic order, locks;
- an ECS system per node: frame coupling, conflicts with ADR-01.

Consequences: effects happen only where exec wires put them. A Get re-reads committed state at each consuming action. Reactive features are glitch-free but coalesced per segment, so code that needs every intermediate value emits events explicitly. Evidence: architectural reasoning plus known node-editor pitfalls listed in [11](11-graph-language.md); no benchmark yet. **Falsification:** G-01–03, G-08, G-15, G-32 and W1/W5 on the WU-00 reference interpreter and the WU-04 executor; a glitch, a repeated effect or an NFR-02 violation rejects the model. Owner language/runtime lead; G2.

## ADR-14 — One cooperative actor per run

**Selected.** Each run executes one strand segment at a time on one logical actor that owns its state, activations and queues. Strands interleave only at suspension, yield and slice boundaries. Runs execute in parallel, heavy pure programs and async effects run off-actor, and workspace state has its own host actor with published snapshots ([11 §9](11-graph-language.md)). Alternatives: a multi-threaded run with locked state (data races become semantic, nondeterministic order, lock cost on every Get); a thread per activation (oversubscription, no replay). Consequences: no locks on run state and deterministic order given the ingestion log. Read-modify-write is atomic unless a value was captured across a suspension (`W-STALE-CAPTURE`). A single CPU-bound run uses one core plus offload. **Falsification:** W2/W5/W9 throughput and NFR-02 latency with offload enabled; G-14, G-32, G-33. If representative workloads cannot meet NFR-02 without intra-run parallelism, revise with explicit partitioning semantics, not shared locks. Owner runtime lead; G2.

## ADR-15 — Structured regions, value availability and cycle classes

**Selected.** Exec nodes belong to exactly one region. Exec edges cannot leave regions except through terminators. An `execution` output may be consumed only where its outcome dominates the consumer. A data cycle is legal only through a state boundary (register, variable), a zone or a loop. A control cycle must contain a `suspends: always` node. Event and reactive cycles are bounded by causal depth. Synchronous recursion is rejected in v1 ([11 §7–8](11-graph-language.md)). Alternatives: unstructured exec wiring with stale or default outputs (a common source of node-editor bugs); a pure DAG with no cycles (cannot express counters, polling or feedback); bounded synchronous recursion (needs dynamic frame sizes, breaks NFR-05 allocation-free frames and fixed memory). Consequences: some wirings need a local, register or `select`, and diagnostics must suggest them. Loops and feedback are explicit and analyzable. Frames have static size. **Falsification:** G-04–07, G-10, G-12, G-16, G-24, G-26, G-27; representative workflow trials in 02b/05a. If users cannot express listed workflows (01 workflows 1–4) without excessive workarounds, revise the rule by ADR with a focused regression test. Owner language lead; G2/G3.

## ADR-16 — Nominal types, constrained generics and an explicit converter registry

**Selected.** Canonical versioned type expressions, nominal package types, constraint implementations owned by the type's package, deterministic inference persisted as `type_args`, no `any` type, and conversions only as visible converter nodes classified lossless-total / checked-partial / lossy. The editor auto-inserts only a unique lossless-total converter, never a chain ([11 §5–6](11-graph-language.md)). Alternatives: implicit coercion (silent loss, ambiguous overloads); `any`/dynamic ports (tag checks on the hot path, run-time type errors); structural typing (accidental compatibility between unrelated packages); per-type definitions such as `add-i64` (palette explosion, no extension point for package types). Consequences: package types participate in core generic nodes through constraint implementations, and conflicts are impossible by the ownership rules. The editor needs a type-pinning UI and converter picker. **Falsification:** G-19–21; the shuffle property test in 00c; W9 inference cost within NFR-15. Owner language/SDK lead; G2.

Implementation clarification (00c.2a): language constraints are registry proofs, independent of Rust value equality and literal encoding. Explicit host-supported structural derives are validated before publication, with bounded schema traversal; unsupported automatic derivations can use owning-package implementations. Family membership does not imply every arithmetic operation. Serialize capability does not bypass value-level sensitive-detail validation. [11 §5.4](11-graph-language.md) and [12 §8](12-graph-data-structures.md) specify these boundaries; no kernel execution or performance gate is claimed.

Implementation resource policy (00c.3b): structural unification uses iterative alias traversal and bounded expanded terms. The provisional node/depth limits in [10](10-tooling-and-sources.md) report `E-TYPE-COMPLEXITY` instead of unbounded tree allocation or implicit type fallback. They remain subject to NFR-15/memory qualification; passing foundational tests does not freeze the budget.

Converter identity clarification (00c.5a): semantic entries and exact implementation alternatives are separate. Uniqueness counts entry contracts; provider defaults are explicit and affect new bindings only. The initial policy surface uses declared finite choices, and lossy conversions expose JSON loss details. This supports package extension without a generic schema interpreter or implicit selection by registration order ([11 §6](11-graph-language.md)).
Pin clarification (00c.4a): an explicit user type choice is a **pin** stored in `type_pins`; `type_args` holds the resolved value of every parameter (pinned or inferred) and is only a cache. Inference seeds from pins, typed literals, edges and concrete port types, never from persisted `type_args`, so a disconnect cannot leave a stale binding that behaves like a pin. Documents without `type_pins` load with no pins ([11 §5.5](11-graph-language.md), [12 §3.1](12-graph-data-structures.md)). Connection selection follows the same rule: re-infer endpoint types after removing replaced wires, excluding the newly shadowed target literal. Reusing an edge ID invalidates inference at both old and new endpoints (00c review correction); pins and other surviving evidence remain authoritative.

## ADR-17 — Host-owned localization, stable graph semantics

**Selected; formatter/UI feasibility conditional.** Every product-owned visible/accessible string is a namespaced TextRef or structured MessageRef. Host modules resolve immutable package catalogues and project localized UI/CLI text; compiler/runtime retain stable codes and redacted typed arguments. [13](13-internationalization.md) owns the contract. Fluent FTL is the planned resource format; exact parser/formatter/Unicode/CLDR pins and licenses require WU-01/03 evidence. English is the source fallback; shipped language coverage is OPEN-13. Translation bytes remain part of package integrity and exact locks. Semantic signature hashes exclude presentation. Derived graph display uses canonical type/member IDs; UI locale never changes values, error choice, replay records or plan identity.

Alternatives: hard-coded English followed by later extraction (misses SDK fields); pre-render all guest errors (loses locale/arguments); concatenate fragments (breaks reordering/plurals); inherit OS locale in graph nodes (breaks reproducibility); allow global translation overrides (breaks namespace/trust boundaries). Consequences: catalogue/schema validation before SDK freeze, retained IME sessions and search/accessibility updates on language switch, font/RTL/layout work and translation coverage evidence. New shipped languages need human review; language support is not inferred from Unicode input. Evidence: source register in 13; these are planning decisions, not measured capabilities. **Falsification:** G-39 plus L-01–08, NFR-16 and P-02 on declared OSs. Changed graph bytes/outputs after a locale toggle, leaked arguments, clipped required controls or inaccessible translated UI blocks the relevant G2/G3/G5 gate. Owner editor/SDK/host leads.

## ADR-18 — One integration branch and small reviewed changes

**Selected policy; enforcement state recorded in 14.** Use protected `main`, short-lived topic branches tied to delivery subunits, PRs, scoped required checks and squash merges. Release branches exist only for actual stabilization/supported older versions. [14](14-git-workflow.md) owns names, commits, review modes, integration checks, history/revert and release rules. Solo mode requires self-review/checks with zero independent approvals; team mode requires a non-author reviewer with stale-approval handling. AI review does not create independent human approval. Contracts, diagnostic/fixture changes and traceability stay in the same PR.

Alternatives: permanent `develop`/release trains add coordination before there is a product; direct pushes lose a durable review/evidence boundary; a branch per broad WU creates large delayed integrations. Consequences: PR-sized slices and explicit stacked dependencies, immutable protected history, phase-aware CI and separate proof that remote rules are active. Evidence: Git/hosting primary references in 14; initial inspection found no Git repository, followed by the user-authorized bootstrap recorded in 10/14. **Validation:** document consistency and active remote-rule inspection; use a disposable test target to verify direct/force-push, failed-check and unresolved-review rejection, plus one successful PR path. Before WU-12, verify tag/artifact provenance and hotfix forward-port records. Owner repository maintainer; relates to NFR-14 and WU-01/12 evidence, without adding a product FR.

## ADR-19 — One source repository, independent application and expansion packages

**Selected by the user.** Public `erinmel/OrangeEditor` contains `app` for editor/engine source, `plugins/<name>` for authored installable expansions and `sdk` for their versioned public boundary. [15](15-monorepo-layout.md) owns placement, future workspace membership and release tag namespaces. One reviewed commit can coordinate host/SDK/plugin compatibility changes; source location grants no access to private host internals. The future root Cargo workspace builds host members; independent guest/worker plugin workspaces keep their own versions/locks and are excluded from host defaults. The native bundled-kernel exception stays under application crates.

Alternatives: separate repositories now add cross-repository synchronization before contracts are stable; one host build linking every expansion would violate hot installation and boundary/isolation requirements. Consequences: shared access/history/CI orchestration, explicit independent SDK/application/plugin release identities, and affected-component compatibility checks. No production crate or implemented plugin is created by bootstrap. **Validation:** inspect the initial tree and source-layout links now; WU-03 must package/install a plugin without rebuilding/restarting the host, and later SDK changes run host/plugin conformance fixtures together. Owner repository/SDK maintainers; NFR-14 and existing FR-34–38; existing delivery gates remain required.

## ADR-20 — Numeric authoring, v1 control and focused tests

**Selected by the user, 2026-10-21 (WU-00/00a).** Keep Float Single/f32 and Double/f64 precision plus integer math. New untyped numeric input defaults to Double; resolved or explicitly selected types win. Integer input parses directly into its integer type, preserving large exact values. Existing graph types, codecs and visible-converter rules stay unchanged; wider/arbitrary-precision numbers remain separate scope. FR-08/45/46 and 11 §5–6 own the contract; product labels follow 13.

Retain v1's rejection of synchronous recursion (`E-RECURSION`) and control cycles without suspension (`E-CYCLE-SYNC-CONTROL`). Structured loops remain available and yield at iteration boundaries; finite While bounds and cancellation remain required (ADR-15, FR-14/46). This policy prevents non-yielding synchronous loops without removing ordinary loop-based workflows.

Test contracts with Rust tests that live beside the code they verify: plain `#[test]` for focused behavior, `rstest` for named scenarios, `proptest` for invariants over generated inputs and edit sequences. The acceptance rule is that **each implemented semantic rule has enough meaningful evidence to distinguish correct behavior from plausible bugs**. One accept/reject pair is a starting point, never a ceiling; there is no fixed test count, diagnostic-code quota or coverage percentage. Equivalence checks (incremental = full validation, interpreter = executor) are paired with known-answer tests, because two implementations can share a bug. JSON goldens are kept for serialization and wire contracts only; G IDs remain explanatory references. The working rules are in [06](06-quality-and-validation.md#v01-test-layers). Alternatives: an exhaustive JSON G-suite written before code (rejected in 00a: it drifted from code that did not exist), and a test per catalog code (counts codes, not evidence). No runtime behavior or feasibility gate is claimed tested by these decisions.

## ADR-21 — OrangeEditor is a foundation: an embeddable core with fixed extension seams

**Selected by the user, 2026-10-21.** OrangeEditor is a **foundation**, not one end product. This repository builds the **core**: graph model, types, compiler and runtime, as Rust libraries with no UI, window or engine dependency (extends ADR-01). The editor UI is a separate layer on top of the core that a product may reuse, restyle or replace. Downstream products embed the core and specialize it, for example a music tool (MIDI, loops, audio analysis driving visuals), a VTuber or virtual-space program (Warudo-style blueprints driving scene objects), a streamer automation tool, or a game or 3D-art tool. Those products are not built here.

The core must therefore be strong and general on its own, and expandable without forking it or changing the document format. Multi-domain node tools (Unreal, Blender, TouchDesigner, Max/Pure Data, Houdini) share one graph infrastructure and give each kind of graph its own evaluation model; none runs audio DSP, shaders and event automation under one scheduler. The core follows that pattern by fixing a small set of seams instead of designing every domain.

**The core owns:**

1. **Substrate.** Document format, stable IDs, transactions and undo, ports, nominal types with generics, constraints and converters, packages and provider locks ([12](12-graph-data-structures.md), 11 §4–§6, the 00b/00c code). It makes no scheduling assumption.
2. **The `control` domain.** Today's 11 §2–§15: event activations, exec/data wires, async, structured loops. It is the only domain the core ships, and it is complete on its own for automation and blueprint-style logic.
3. **Reusable node groups.** A graph built from nodes and published as one node (Blender node groups, Unity subgraphs): the existing function mechanism (11 §14), graph-local, shared in a workspace library, or published in a package and pinned by the lock. Its interface is versioned, and editing it previews affected callers.
4. **Host adapter interfaces.** `Clock`, services and a scene/object bridge ([02](02-architecture.md)), so any Rust host can embed the runtime: the reference editor, a headless runner, a music or VTuber product, a game engine, later a C ABI or Wasm. The project has no Bevy dependency (ADR-02); an engine-based product embeds the core on its own side.

**Extension seams the core guarantees:**

| Seam | Who uses it | Contract |
|---|---|---|
| Packages | Untrusted installed artifacts | Nodes, types, converters and node groups for existing domains through 11 §16 (unchanged; P7 holds) |
| `domain` field | Documents | Optional on graphs and function bodies, default `control`. An unknown domain loads and is preserved but does not compile, like an unresolved provider. Format v1 stays compatible. |
| Host extensions | Any downstream crate that depends on the core and is linked by the embedding product at build time (trusted code) | May add a domain (its node kinds, evaluation model, clock and backend), visible typed bridge nodes between domains, and an object model of handle kinds with schema-generated property nodes (11 §4.5 port templates). Each brings its own semantics document and tests. |

**Not in the core:** extra domains (`signal`, `frame`, `gpu`, `field`), bridges, clocks and object models. The core provides none of them. Anyone adds them in their own crate by depending on the core crates and implementing the core's public extension interfaces, the same way packages add nodes. Forking stays possible but is never required: the core's job is to keep these seams public, documented and stable, and to guarantee an extension needs no substrate change.

**Alternatives rejected:**

| Alternative | Why rejected |
|---|---|
| One universal scheduler for every domain | Real-time audio cannot share a cooperative event actor, GPU code cannot run node by node, and per-frame push evaluation conflicts with ADR-13's effect ordering. |
| Installed packages define scheduling | Untrusted code with unbounded semantics and no shared debugging or replay; violates P7. Trusted host extensions provide domains instead. |
| One end product with the core as an internal detail | Couples the core to one product's UI and scope, so other products cannot reuse it. |
| Designing music, game or scene domains now | Speculative contracts for products not built here; the seams are enough to keep them possible. |

**Consequences:**

- Public core APIs are a product surface: documented, semantically versioned, and free of editor or engine types.
- 11 marks its substrate and `control` domain sections; [12 §3.1](12-graph-data-structures.md) defines the optional `domain` field.
- The 00c type, constraint, unification and converter work is substrate and continues unchanged.
- [01](01-product-requirements.md) is reframed: the core library is the deliverable and automation is its first reference host. FR-41 and the P5/P6 audio and graphics phases become downstream or host-extension work.

**Falsification:** a downstream prototype must fork the core, patch core code to add a domain, or change the substrate format beyond the `domain` field; a node group cannot be published, versioned and reused through 11 §14 plus package locks; or embedding the core in an engine-based host needs an engine dependency in model, compiler or runtime crates. Any of these requires revising this ADR. Owner: language/runtime lead with the product owner; review before G2.

## Decisions still requiring evidence or product preference

| ID | Owner / deadline | Recommended default and practical consequence |
|---|---|---|
| OPEN-01 Exact UI adapter | Editor lead / G1 | Proposed: egui editor UI hosted by eframe (ADR-02 revision). Run the egui probe first (IME/CJK, accessibility, keyboard, DPI, RTL plan, W4 frame time); fall back to Slint if a required capability fails. |
| OPEN-02 Dependency pins/toolchain/platform minima | Platform lead / WU-01 | Upgrade selected project toolchain to ≥1.97.1 when later authorized; exact feature/license/OS verification before dependency acceptance. Do not install anything in this planning task. |
| OPEN-03 AMQ intended broker | Product owner / before WU-08 fixture freeze | AMQP0-9-1 + RabbitMQ first. If user actually needs ActiveMQ/Artemis/Azure AMQP1.0, reprioritize the separately typed 1.0 adapter rather than renaming 0-9-1. Can continue generic runtime/files/HTTP without answer. |
| OPEN-04 Trusted native worker distribution | Product/security owner / before downloaded-worker UX | Local explicit-trust workers enabled by deliberate choice; downloaded workers disabled by default. A mandatory native-only integration may justify real per-OS sandbox work and new release scope. |
| OPEN-05 macOS/desktop minimums | Platform/product owner / P-06 then G5 | Windows x64, Linux x64 Wayland+X11, macOS arm64 first; publish measured supported versions. Add macOS x64/other architectures when demand/hardware evidence warrants it. |
| OPEN-06 Linux virtual-source adapter | Audio lead / P-04 | Application-owned PipeWire client source with lifetime cleanup; no automatic persistent system config. If feasibility fails, redesign adapter and report gap; do not silently defer M FR-33 to claim complete 1.0. |
| OPEN-07 Twitch application ownership | Product/integration owner / WU-11 live test | Developer-owned public test client for validation; decide product registered public client before distribution. No secret/client registration needed for mock CI. |
| OPEN-08 SDK canonical packaging/signature trust | SDK/security lead / WU-03 before G2 | Canonical manifest/payload digest and explicit trust store with conformance vectors; marketplace later. This must be finalized before actual package distribution, not guessed from synthetic hashes. |
| OPEN-09 Quantitative budgets | Runtime/editor/audio leads / first relevant experiment then each gate | Use 06 targets as falsifiable proposals, attach results and revise only with rationale. No deadline/throughput promise until hardware evidence. |
| OPEN-10 Synchronous recursion | Product owner + language lead / G2 | Confirmed rejected in v1 (ADR-15/20), alongside non-yielding synchronous control cycles. Structured yielding loops and queued event chains remain available. |
| OPEN-11 Numeric literal typing in the editor | Editor lead / WU-05 | Selected Float Single/f32 and Double/f64 plus integer math (ADR-20). New untyped numeric input defaults to Double; resolved/explicit types win. Integer input parses directly without a float intermediate. |
| OPEN-12 Package-defined control intrinsics | Language/SDK lead / after G2 | Not in 1.0. Iterators, async outcome groups and functions cover the known needs (retry, throttle, for-each-row). New intrinsics require a host language release with G fixtures. |
| OPEN-13 Shipped translation languages | Product/editor owner / before WU-12 release packaging | Require complete English source coverage and full i18n contracts from G2/G3. Recommend Spanish as the first additional translation; advertise it only after L-07 human review. Other languages follow demand and complete coverage. Pseudo locales/CJK/RTL validation are required regardless of the shipped language list. |
| OPEN-14 Foundation scope (ADR-21) | Resolved 2026-10-21 | ADR-21 selected: the core library is the deliverable, `control` is the only built-in domain, the optional `domain` field defaults to `control`, and anyone adds domains, bridges and object models in their own crates. 01 is reframed; 11 marks substrate versus `control` sections. |
| OPEN-15 Pure closures and graph instancing | Language/runtime lead / after 00e | Two general core features that strengthen every product. Pure closures: typed function values from a pure body (Blender 5.0 closures), letting node groups and package nodes take behavior as input without new control intrinsics (eases OPEN-12); open choices are the type form and value-only captures. Graph instancing: one graph as a class with many runs bound to host objects, sharing one plan. Each needs its own 11 section, diagnostics and focused tests before implementation. |

These unresolved items have recommended paths. Only consequential user preferences lacking an implied default require clarification; evidence collection and independent reversible work should continue.
