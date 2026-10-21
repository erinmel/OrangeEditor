# Delivery plan and gates

No calendar estimates are asserted. WU IDs below are **work packages**, not single implementation changes. Their ordered subunits each produce a small reviewable output, acceptance evidence and a decision update. This planning task stops at documentation. Under a later implementation request, reversible feasibility work proceeds autonomously within its authorized directory/tool permissions; these gates are engineering checks, not extra user approval requests.

Each executable subunit normally becomes one topic branch/PR, or smaller independently reviewable PRs, under [14](14-git-workflow.md). Dependent PRs retain the dependencies below; merging a PR does not pass its engineering gate automatically. Repository/bootstrap CI evidence is tracked separately in 14/15 and 10; it is not WU-01 compilation evidence or a new dependency on Bevy for WU-00. Application/plugin placement follows the monorepo layout in [15](15-monorepo-layout.md).

## Readiness gates

| Gate | Required evidence / stop-go rule |
|---|---|
| G0 Planning consistency/readiness | FR-01–51 and NFR-01–16 trace to owners/work units/tests; examples agree on IDs/provider locks/auth schema and on the graph format in [12](12-graph-data-structures.md); every rule in [11](11-graph-language.md) has a G-suite fixture ID; source limitations explicit; Bevy pin/MSRV/environment gaps recorded; no production code or fabricated test results. Unresolved consequential product choices have recommended defaults and a decision deadline. Passing G0 authorizes no installations by itself; a subsequent implementation request selects the applicable work unit, prioritizing WU-00/00a alongside WU-01/01a as described below. |
| G1 Feasibility before production architecture freeze | WU-01/02/03 evidence for exact Bevy/toolchain, UI text/focus/accessibility and localization (L-05/06 prototype), bounded async bridge, component SDK isolation/batch costs and real hot addition/override. **B-P04 early audio feasibility** is required before committing the audio adapter. Hardware unavailable leaves audio selection conditional while independent graph/component work continues. Fail UI → evaluate compatible alternative shell/adapter before large UI work; fail Wasm budget → revise batching/service boundary, not claim performance. No broad editor shell first. |
| G2 Core + expansion contract freeze | WU-00 G-suite passes on the reference interpreter and NFR-15 is measured before WU-03 freezes node/type/port contracts; WU-03/04 schema/component/registry conformance, semantics/property tests, immutable lock/missing-node recovery, remove/revoke races and measured native/component budgets. API v1 freeze only after synthetic third-party install/replacement works. |
| G3 Authoring preview | WU-05 UI/persistence/base language all baseline authoring tasks; B-P02 accessibility/text/localization and NFR-03/04/10/12/16; L-01–06 qualification. A failed accessibility capability is a product gap with a remediation owner, not waived by attractive rendering. |
| G4 Services preview | WU-06–09 safe files/vault/profiles, all protocol fixtures, gRPC all streaming shapes and webhook listeners; security/redaction gates. Partial previews enumerate missing capabilities and cannot call themselves full baseline. |
| G5 1.0 release | WU-10–12 audio and initial OBS/Twitch verified on declared OS/hardware, required M FRs pass, applicable NFRs meet budgets or approved revised ADRs with evidence, no unresolved data-loss/security/lifecycle defect. Release capability matrix names supported OS/driver/format/protocol versions and fully covered shipped locales (L-07/08). |
| G6 Later expansion | WU-13 graphics command/editor scope, registry and AMQP1.0 have their own contracts/test evidence; do not retroactively imply 1.0 included a scene editor. |

## Dependency-ordered work units

| Unit / phase | Dependencies | Concrete output and acceptance / stop rule |
|---|---|---|
| **WU-00 Graph language core / P1→P2** | G0; later implementation authorization. Needs no Bevy, GPU, network, vault or package host, so it runs before or alongside WU-01 | Implements [11](11-graph-language.md) and [12](12-graph-data-structures.md) as pure Rust with no Bevy dependency: document model, transactions and indexes; type system and converter registry; validator with the full diagnostics catalog; compiler to plan IR; reference interpreter; run actor on a virtual clock. Publishes JSON schemas and golden vectors for WU-03. Acceptance: G-01–G-39 pass, NFR-15 and W9 measured, `Value` size assertion holds. Stop rule: if a G-case cannot be met by the rule as written, revise 11/12 plus the ADR before WU-03 freezes component contracts. |
| **WU-01 Evidence/toolchain harness / P1** | G0; later implementation authorization | Verify Bevy `=0.20.0`, release source and Rust ≥1.97.1; record exact compatible pins/features/licenses for chosen dependencies. Create disposable minimal harness only after environment prerequisites resolved. Run **B-P01** pin/features and **B-P06** OS dependencies/build probes from 09; no old-version API copied as proof. If pin/source/toolchain disagrees, resolve evidence first. |
| **WU-02 Editor/bridge feasibility / P1** | WU-01 | **B-P02** text/IME/focus/accessibility/picking, **B-P03** schedules/messages + Tokio bounded bridge, **B-P05** headless/render handles. Demonstrate a text edit surviving unrelated graph update and locale switching with expanded/RTL/CJK text, keyboard/screen-reader graph traversal and a request finishing with render loop paused. Compare alternative shell only if required capability fails. Update ADR-02 before production UI. |
| **WU-03 Installable expansion vertical slice / P1→P2** | WU-00 (00b–00d model, type and validator contracts and schemas); WU-01; bounded bridge from WU-02 | Minimal headless host plus tiny visible palette surface: install third-party Wasm package while app remains open, register custom type/node/value editor, select a replacement standard node provider, run, update while old run pinned, revoke/remove/fail worker and recover. Publish component/WIT/wire and text/catalogue schemas, L-01–04/08 vectors, and canonical digest/signature vectors; batch/native/component/worker W3 measurements. Standard math fallback never replaces selected guest silently. Failure blocks SDK freeze, not unrelated UI probing. |
| **WU-04 Production executor and host integration / P2** | WU-00; WU-03 contracts; G1 | Optimized executor over the WU-00 plan IR: binary codecs and typed buffers, kernel batching/fusion, guest kernels, the Tokio host bridge for async continuations, real timers and subscriptions, workspace state actor, checkpoints and the run lifecycle. Equivalence against the WU-00 reference interpreter on the whole G-suite; headless calculation→variable→event→effect fixture; V01/V02/V03; W1/W2/W5 measured. G2 requires real component provider compatibility, not native-only tests. |
| **WU-05 Editor + persistence + base nodes / P3** | WU-02, WU-04; G2 | Incremental canvas/palette/inspectors/history/groups/subgraphs, numeric/string/JSON/list/random nodes, diagnostics and accessible controls. Copy IDs remap; save/load/recovery uses immutable bundle generations + atomic current-pointer replacement; unknown nodes preserved. W4/G3, L-01–06 qualification and NFR-16 text coverage, plus fault injection on save checkpoints. |
| **WU-06 Files/vault/profiles / P4** | WU-04; editor profile dialogs from WU-05 | Streaming file nodes, OS path enforcement, atomic/durable modes, app/workspace profiles, protected vault adapters, secure input, broker/auth transforms/rotation and typed errors. W6, V07/V09, vault locked/headless unavailable, no plaintext export; deny restricted-root races. First credential-backed local request uses mock service. |
| **WU-07 HTTP/WebSocket/API facilities / P4** | WU-06 | HTTP/HTTPS stream + TLS/redirect/idempotency tests, owned WebSocket sessions, REST/GraphQL query and `graphql-transport-ws`, outbound and authenticated inbound webhooks. Install a third-party adapter using shared leases; stop closes listener and cancels pending responses. No live internet account required. |
| **WU-08 MQTT/AMQP / P4** | WU-06, WU-07 lifecycle patterns | MQTT5/3.1.1 topic/QoS/session and RabbitMQ AMQP0-9-1 channel/confirm/ack/prefetch fixtures; broker restart/reconnect/backpressure and redelivery trace. Require actual broker/version matrix before compatibility claims; AMQP1.0 remains separate WU-13. |
| **WU-09 gRPC / P4** | WU-06, HTTP2/service stream contracts in WU-07 | Descriptor import/dynamic types, unary/client/server/bidi fixtures, half-close, deadlines/trailers/status, cancellation and bounded backpressure. Reflection optional. G4 cannot pass with only JSON-over-HTTP or unary demo. |
| **WU-10 Audio platform vertical slice / P1 probe, P5 implementation** | **B-P04** early after WU-01; production WU-04/06 | Early native probe routes decoded file to selected output and Linux app-owned virtual source; verify external client captures it and source disappears on exit. Production asset/player/mixer/playlist/random-folder/opened-asset nodes, format/rate/route/hotplug tests, no callback allocations/guest calls. Windows existing cable + macOS existing device/hardware test. Failed Linux source probe requires adapter redesign before claiming FR-33. |
| **WU-11 OBS/Twitch expansions / P5** | WU-03, WU-06/07, WU-10 for media workflow | Implement exactly the 05 node/event sets; OBS V5 broker transform/correlation; public Twitch device auth/single-use refresh/validation/EventSub/rate policy. Third-party OBS package uses shared OBS service without raw transport authority. Mock CI plus owner-authorized live smoke test with secrets excluded from artifacts. |
| **WU-12 Release qualification / P5** | WU-05–11 | Cross-OS signed/distributable candidate, dependency/license/security review, 8 h soak, package update/remove fault injection, recoverability/accessibility/audio hardware evidence, documentation and SDK examples. Apply G5 coverage and capability matrix. No unsupported capability presented as working. |
| **WU-13 Graphics + advanced expansion / P6** | G5; B-P05; new requirements review | Typed scene/resource handles, 2D/3D command/event packages and bounded preview with Bevy; graphics services remain optional. Separately deliver registry/signature revocation/publisher UX and AMQP1.0 adapter if prioritized. Full scene authoring remains separately scoped. |

WU-10's B-P04 probe intentionally happens early alongside UI/plugin feasibility; product media implementation waits for the stable runtime/services. This prevents a late discovery that the desired virtual-source requirement is not feasible with the selected audio adapter.

## Executable subunits

Subunits within a package run in listed order unless their inputs explicitly permit parallel work. An output is accepted only with its listed evidence; do not combine an entire broad package into one opaque editor rewrite. Status and limitations are recorded per row; unimplemented semantic/runtime checks remain **NOT RUN**.

| Subunit | Output | Acceptance |
|---|---|---|
| 00a | Semantic review, golden fixtures and structural checks against the existing model; no validator/interpreter implementation | **Reviewed fixture baseline, 2026-10-21; acceptance still open.** Every inventoried rule in 11 §4–15 maps to G fixture files; the 03/11/12 JSON examples parse and repeated definitions agree; G-39 is defined; OPEN-10 confirmed, OPEN-11 revised to both Float precisions plus integer math, OPEN-12's existing boundary explained/retained. [Review](../conformance/REVIEW.md) lists prose-only assertions, missing specialization/template contracts and owners. Structural tests pass; expected graph diagnostics/traces remain NOT RUN. No G0/00a pass is claimed |
| 00b | `orange-model`: IDs, document structs, literal codec, signature snapshots, transactions/change sets/undo, derived indexes. Delivered as 00b.1–00b.4 below; 00b is accepted only when all four are | Round-trip property tests including unknown fields; paste remap; deterministic serialization; `Value` ≤16 bytes; G-29–31, G-36, G-39 semantic snapshot/message wire vectors |
| 00b.1 | Root workspace/toolchain pin/`rust` CI; typed IDs, document structs and format v1 JSON codec (structural checks only) | Golden 12 §6 round trip with byte-stable save; unknown keys preserved; unresolved package node and orphan edge kept; structural error cases; save→load property test. **Implemented and merged into `dev`; evidence in [10](10-tooling-and-sources.md)**; G0/00a not passed |
| 00b.2 | `DocCommand` transactions, `ChangeSet`, inverse/undo, single revision increment | G-30, G-31, G-36; undo-all/redo-all property test. **Implemented and merged into `dev`; evidence in [10](10-tooling-and-sources.md)**; G0/00a not passed |
| 00b.3 | Derived indexes, paste remap, semantic signature projection and `sha256:` key | Paste remap property test; G-29, G-39 semantic snapshot vectors. **Implemented and merged into `dev`; evidence in [10](10-tooling-and-sources.md)**; G0/00a not passed |
| 00b.4 | `Value` enum and typed literal codec per 12 §6.1 | `Value` ≤16 bytes assertion; scalar/aggregate literal round trips. **Implemented and merged into `dev`; evidence in [10](10-tooling-and-sources.md)**. All four 00b slices are merged and pass remote CI on Linux, Windows, macOS and MSRV 1.97.1; the 00b acceptance row stays open until G0/00a pass |
| 00c | Types and conversions: type-expression parser/interner, constraints and coherence, unification, converter registry and auto-insert transaction | G-19–21; shuffled-insertion property test yields identical `type_args` |
| 00d | Validator: connection matrix, regions, dominators/availability, cycle classes, write conflicts, function rules; incremental and full modes | G-04–07, G-10, G-11, G-24, G-25, G-27, G-35, G-37; incremental = full on random edit sequences; NFR-15 on the W9 edit set |
| 00e | Compiler to plan IR (pull programs, stability classes, control blocks) and a simple tree-walking reference interpreter | G-01–03, G-08, G-09, G-17, G-18, G-26, G-33, G-39 derived-display/diagnostic/output invariance; plan execution equals the reference on random pure graphs |
| 00f | Run actor on a virtual clock: activations, strands, segments, entry queues/reentrancy, events, causal depth, watches, cleanup mode, streams, ingestion-log replay | G-12–16, G-22, G-23, G-28, G-32, G-34, G-38, G-39 canonical trace invariance; W9 storm bounded with exact counters |
| 01a | Exact Bevy/tag/MSRV/feature/dependency evidence sheet, including candidate Fluent parser/formatter and Unicode/CLDR data pins | Tag agrees with crate metadata; toolchain gap and license/OS requirements recorded |
| 01b | Disposable pin/feature compile harness (after prerequisite environment changes are authorized) | B-P01 exact-version compilation with lock; no old-version API fallback |
| 01c | Cross-OS minimal build report | B-P06 named runner/build artifact per OS; unsupported targets explicit |
| 02a | Minimal editable node + text field + unrelated background update | IME/caret/focus/selection persist through locale change; clipboard/native dialog/DPI and L-05/06 prototype records |
| 02b | Accessible graph interaction prototype | Keyboard connect/edit/run and screen-reader node/port/error/link navigation in B-P02 matrix |
| 02c | Bounded Tokio↔Bevy and headless bridge | B-P03/05 completes request with rendering paused; queue saturation doesn't block main thread |
| 03a | Versioned manifest/component interfaces, wire/digest vectors and validator | Cross-language fixtures accept known good/reject incompatible, malformed and tampered packages; catalogue/TextRef/MessageRef validation and L-01–04/08 contract vectors |
| 03b | Capability-limited component host and synthetic guest | File/network/secret denial, memory/fuel limit and guest trap tests pass |
| 03c | Hot registration plus exact provider selection in a tiny visible surface | Install while open; new node/type/editor available; alternative math provider runs as selected |
| 03d | Update/remove/revoke/rollback and trusted worker adapter | Old run pinned; stale handles rejected; worker kill isolated; W3 measured before G2 |
| 04a | Production binary codecs, typed buffers and numeric/Unicode kernels on the WU-00 model | Scalar/aggregate/unknown-value binary round trips; numeric/Unicode property tests |
| 04b | Optimized pure execution (native batching/fusion, guest kernels) | W1/W2 budgets; G-suite equivalence with the reference interpreter; layout edits trigger no compile |
| 04c | Host-backed state: workspace actor, persistent checkpoints, host external writes | G-34; checkpoint atomicity and version validation; no repeated write from a data read |
| 04d | Tokio bridge for async continuations, real timers/subscriptions, run ownership and restart generations | Cancel/completion races, bounded queues, stale generations, cleanup tests; W5 / NFR-02 |
| 05a | Canvas/palette/typed port edit slice | Add/connect/configure a calculation; invalid edge rejected with diagnostic; incremental changed-ID update |
| 05b | Focus-preserving history/selection/copy/groups/subgraphs | Undo/redo and paste remap references; unrelated edit or locale switch preserves caret; translated palette/search/accessibility and keyboard parity |
| 05c | Versioned save/autosave/recovery | Crash at each current-manifest swap point recovers a valid generation; missing plugin payload retained |
| 05d | Numeric/string/list/JSON/random node families | Boundary/Unicode/parse/seed tests plus visible error/control outcomes; no implicit casts |
| 05e | Large graph diagnostics/inspection/accessibility qualification | W4 and G3, L-01–06/NFR-16, bounded trace memory and translated accessible logical connections |
| 06a | File/path/stream service + scoped grants | 1 GiB transform uses bounded buffers; traversal/reparse denied; cancellation and encoding errors |
| 06b | Atomic/durable file nodes, append/list/watch | Disk-full/partial replace leaves previous file recoverable; watch overflow triggers rescan diagnostic |
| 06c | Native vault adapters + secure host dialog | Each OS create/rotate/delete/lock/unavailable test; canary absent from logs/exports |
| 06d | Typed profiles/leases/broker/auth transform contracts | Import remaps refs, profile version pinning, last-owner cleanup, destination and raw-secret grant distinctions |
| 07a | HTTP/HTTPS request/response/stream nodes | TLS/redirect/deadline/idempotency and large-body fixtures; status/transport errors distinct |
| 07b | WebSocket sessions/send/receive/subscriptions | Fragment/close/reconnect and bounded fan-out; stop releases only owner's leases |
| 07c | REST/GraphQL conveniences and WS subscription layer | Mixed GraphQL data+errors retained; graphql-transport-ws negotiation/operation IDs tested |
| 07d | Outbound templates and authenticated inbound listeners | Explicit loopback/public bind, HMAC/auth/replay/limit checks; single response completion; stop closes listener |
| 08a | MQTT publish/subscribe with selected version/session/QoS | MQTT5/3.1.1 broker fixtures, retained/duplicate metadata and backpressure |
| 08b | AMQP0-9-1 publish/consume/resource nodes | RabbitMQ confirms, manual ack/nack, prefetch and per-channel ownership |
| 08c | Broker loss/restart/delivery qualification | Lost/redelivered outcomes recorded; downstream effects not falsely claimed exactly once |
| 09a | Descriptor import/service/type mapping + unary | FileDescriptorSet includes imports; unknown/invalid descriptor boundedly rejected; status/trailers intact |
| 09b | Server/client streaming | Half-close/deadline/cancel and bounded message queues under slow producer/consumer |
| 09c | Bidirectional stream and concurrency qualification | Independent directions stop/drain correctly; G4 requires all method shapes |
| 10a | Early native B-P04 routing/source probe | External app captures Linux source; existing Windows/macOS output verified; permission gaps recorded |
| 10b | Audio asset/decoder/player control slice | Supported-format fixture matrix, play/pause/stop/seek/completion on selected endpoint |
| 10c | Mixer/resampling/channel-map/hotplug service | W7 callback counters, multiple voices, device disappearance and no silent fallback |
| 10d | Playlist/folder-random/opened-assets nodes | Sorted snapshot+seed selection replay; empty/missing/changed asset errors; shared assets not duplicated |
| 11a | OBS shared session/provider + initial nodes/events | Mock v5 challenge/correlation/subscription/status errors; downstream expansion uses checked service |
| 11b | Twitch public OAuth/Helix/EventSub package | Mock single-use refresh atomicity, hourly validation, rate/reconnect/loss policy; exact bounded node/event set |
| 11c | Owner-configured live integration qualification | Redacted OBS/Twitch smoke report; no confidential desktop secret or personal credentials in CI |
| 12a | OS packaging/dependency/license/contract audit | Exact platform/feature/capability artifacts, formatter/locale-data/font licenses and SDK/catalogue compatibility vectors; OPEN-13 locale list |
| 12b | Soak/fault/recovery/security qualification | W8 and V08/V09 pass; no unresolved data-loss/security/lifecycle defect |
| 12c | Hardware/accessibility/performance release report | All M FR/NFR evidence rows including L-07/08 and advertised locale coverage complete or explicitly revised ADR; G5 decision based on actual results |
| 13a | Scene resource/command/event SDK extension | 2D then 3D sample through handles; headless baseline remains GPU-free |
| 13b | Registry/publisher trust/revocation workflow | Package signature/discovery/rollback fixtures and offline local-install regression |
| 13c | Independent AMQP1.0 adapter | Named broker/version interoperability and delivery/settlement conformance; no 0-9-1 alias |

The 00a fixture baseline and review are now present; its full acceptance remains open for the gaps in [conformance/REVIEW](../conformance/REVIEW.md). Next core work is **00c contract resolution/design**, including types, constraints, template/specialization wire shapes and converter metadata, before implementing that slice under G0. **01a** (Bevy evidence review) remains independent evidence work. 00b is implemented; 01b follows only once its toolchain prerequisites are concretely satisfied. The core crates do not need Bevy, but selecting one stable toolchain ≥1.97.1 for the whole workspace avoids maintaining two pins.

00b post-implementation review (2026-10-20): corrected undo state identity, function-body key integrity, signature default presentation, ordered replacement cost, float-width encoding and map encoding validation. Local integration passes 100 tests, formatting and Clippy; exact commands/environment are in [10](10-tooling-and-sources.md#model-review-fixes-and-slice-4-analysis-2026-10-20). FR-03/FR-08 and G-39 behavior have regression coverage; the ordered replacement test counts operations without measuring NFR-15 timing. A follow-up (2026-10-21) closed the JSON text nesting/size and map duplicate-key items (12 §6.1); remaining work is type/validator work (00c/00d). No G0/00a, G1/G2, Bevy or remote-CI gate is passed by these fixes.

## Traceability matrix

Numbers below refer to the FR/NFR catalogs, not completion status. Coverage includes dependencies: work unit acceptance must produce evidence for every listed item.

| Unit | FR coverage | NFR coverage | Verification |
|---|---|---|---|
| WU-00 | 02 (subgraph semantics),05 (unresolved-node preservation),08–16,44–51 (semantic text/snapshot invariance) | 01,02,04,05,14,15,16 | G-suite, V01/V02/V05/V11 |
| WU-01 | 07,33,38,51 dependency evidence | 11,14,16 | B-P01/06, V07/V11 |
| WU-02 | 01,03,06,07,15,41,51 feasibility | 02,03,10,11,12,16 | B-P02/03/05, V06/V07/V11 |
| WU-03 | 09,22,34–38,51 catalogue/SDK contracts | 01,05,06,08,09,13,14,16 | V02/V03/V05/V09/V11 |
| WU-04 | 04,05,07–16,44–51 (production executor/CLI diagnostics) | 01,02,04,05,06,09,13,14,16 | V01/V02/V03/V05/V08/V11, G-suite equivalence |
| WU-05 | 01–14,44–49,51 (editor ports, conversions, diagnostics, preview, localization) | 03,04,05,09,10,12,13,15,16 | V01/V06/V08/V11 |
| WU-06 | 13,17–22 | 05,06,08,11,13 | V02/V04/V07/V08/V09 |
| WU-07 | 15,16,19,20,23,24,27,29,35 | 02,05,06,08,13 | V02/V03/V04/V09 |
| WU-08 | 25,26 | 02,06,08,13 | V04/V08 |
| WU-09 | 28 | 02,05,08,13 | V04/V05 |
| WU-10 | 30–33 | 05,07,11,13 | B-P04, V07/V10 |
| WU-11 | 19–22,24,39,40 | 06,08,09,13 | V03/V04/V09 |
| WU-12 | All M FR-01–40 and FR-44–51 | All NFR-01–16 applicable to 1.0 | V01–V11, G-suite, G5 |
| WU-13 | 41–43 | 01,03,08,09,11 plus new graphics/registry budgets | B-P05, V03/V04/V06/V09 |

## Risk register

| Risk / owner role | Early evidence / mitigation | Decision deadline |
|---|---|---|
| Bevy UI cannot satisfy IME/accessibility / editor lead | B-P02 manual matrix; keep domain/view model independent; compare verified integration or native shell fallback | G1 / before WU-05 |
| Translation integration arrives after SDK/UI freeze / editor + SDK leads | TextRef/MessageRef and catalogues before G2; L-05/06 in B-P02; source coverage, language-switch and bidi/font evidence before G3/G5 | G1/G2/G3 |
| Scalar component call cost overwhelms tiny nodes / runtime+SDK lead | W3 batch/transfer profiles; native bundled fast paths; component macro/subgraph calls; publish cost, preserve selected provider semantics | G2 |
| Plugin API either too weak or unsafe / SDK lead | WU-03 third-party provider/editor/override sample, denied grants, worker trust disclosure and remove races | G2 |
| UI/graph schedules accidentally depend on frames / runtime lead | B-P03/05: pause rendering during timer/network completion, core dependency check | G1/G2 |
| Vault unavailable or token rotation loses refresh pair / service lead | All OS vault fixtures, serialized refresh, atomic pair write; reauthorize on remote-success/local-commit failure | G4/WU-11 |
| Linux virtual source fails under desktop policy / audio lead | B-P04 PipeWire independent capture test; investigate client/session API, expose capability reason; don't fake as sink playback | G1 probe / G5 release |
| Audio backend lacks stable route/hotplug/seek / audio lead | Native device-ID matrix and loopback recordings before high-level media nodes | WU-10 |
| Silent implementation substitution after package update / SDK lead | Exact locks and registry snapshots, user-visible explicit rebind tests | G2 |
| Filesystem atomicity/durability claim too broad / persistence lead | Per-OS replace/flush/crash injections and immutable bundle generations | G3/G4 |
| Protocol library dependencies conflict/MSRV drift / platform lead | Exact lock/features/licenses and OS CI; no SDK freeze based only on latest docs | WU-01 then each dependency change |
| Structured-region and availability rules feel restrictive to users who expect free exec wiring / language lead | Diagnostics with suggested fixes (local, register, `select`); 00a fixtures replayed in 02b/05a usability tests; relax a rule only through an ADR with a new G fixture | G2 / G3 |
| One run actor limits compute-heavy graphs / runtime lead | Offload heavy pure programs; parallelism across runs; W2/W9 profiles before considering intra-run parallel execution | G2 |
| Generic inference surprises users or drifts between versions / language lead | Persisted `type_args`, deterministic solver order, shuffle property test, explicit pinning UI | G2 |
| Plugin API freezes before node/port/type semantics are proven / SDK + language leads | WU-00 precedes WU-03 contract freeze; G2 requires a passing G-suite | G2 |
| Scope creates a large shell without working core / architecture lead | WU-03 end-to-end hot install/override first; small working slices; no 1.0 label until M catalog passes | Every gate |

Open product choices and recommended defaults are in [08](08-decisions.md). No unresolved marketplace/graphics priority blocks baseline core/expansion feasibility. Planning cannot prove reliability; release gates require measured artifacts.
