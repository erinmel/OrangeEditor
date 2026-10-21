# Product requirements

## Perspective and success

OrangeEditor lets a user construct, inspect, save and execute strongly typed workflows without rebuilding the application to add integrations. It serves local automation creators, media operators and expansion developers. Interactive execution and unattended headless execution consume the same graph and package locks. The editor is a product client of that runtime, not the runtime clock.

Success means a third party can install an OBS expansion into the open application, use an existing credential-backed WebSocket connection, run a graph, select an alternative supported base provider, and remove the package without losing the graph document. Speed is assessed per workload; Rust and ECS do not constitute performance evidence.

Priority: **M** required for 1.0; **S** planned after 1.0. Phase: **P1** experiments, **P2** graph/expansion foundation, **P3** editor/base language, **P4** files/connections/protocols, **P5** audio/integrations, **P6** graphics/advanced expansion. P0 is this planning delivery. Each catalog acceptance criterion is an observable contract, supplemented by [NFR gates](06-quality-and-validation.md). Validation IDs Vxx map to test families in that document.

## Glossary

| Term | Meaning |
|---|---|
| Definition | Versioned declarative node/type description; contains no executable code |
| Implementation/provider | Executable realization of a definition or replaceable service slot |
| Package / expansion plugin | Installable OrangeEditor artifact containing definitions, executable components, assets and dependencies |
| Bundled package | Shipped package using public contracts; native kernel exception is explicitly identified |
| Snapshot | Immutable registry and provider choices pinned by a compiled execution |
| Activation | One invocation started by an entry node or event, with frame-local values; contains one or more strands |
| Run | Owned execution lifetime, variables, subscriptions, cancellation and resources; executed by one logical actor |
| Strand / segment | One control token's path through an activation / one uninterrupted execution of a strand between suspension or yield points |
| Region | Exec nodes owned by one structured construct (entry, event, Sequence step, Fork branch, loop body, function) |
| Epoch | One pull evaluation of the data inputs at a consumption site; the unit of volatile snapshot and memoization |
| Node kind | `pure`, `volatile`, `action`, `async`, `entry`, `iterator`, control/zone intrinsic, function call ([11](11-graph-language.md)) |
| Data / exec / event port | Typed value dependency / ordered control continuation / output that starts a new activation per event |
| Resource handle | Opaque capability and lifetime reference; never a serialized pointer or Bevy Entity |
| Connection profile / session | Persisted configuration with secret references / live authenticated transport |
| Override slot | Named, typed and documented replacement interface with visible provider selection |
| Get/read variable | One `volatile` operation, named Get; read in each consuming epoch's prologue, never a write |
| Internationalization (i18n) / localization (l10n) | Architecture making all product text translatable / translated and reviewed resources for a particular locale; Unicode input alone provides neither |

## Functional catalog

| ID | Requirement and observable acceptance | Priority / phase | Design / validation |
|---|---|---|---|
| FR-01 | Palette searches name/category/type; add, connect, disconnect and configure nodes. Invalid edge explains incompatible types and is not committed. | M P3 | 02,03 / V06 |
| FR-02 | Selection, pan/zoom, grouping, subgraphs and copy/paste preserve internal edges; pasted IDs and variable references are remapped without aliasing originals. | M P3 | 02,03 / V06 |
| FR-03 | Transactional undo/redo restores document state. Unrelated edits preserve text focus, caret, selection and viewport. Runtime effects are never undone by document history. | M P3 | 02,03 / V06 |
| FR-04 | Run/stop, per-node diagnostics, execution inspection and bounded logs identify node, activation and error origin. Sensitive fields remain redacted. | M P2,P3 | 03,05 / V02,V06,V09 |
| FR-05 | Versioned save/load, autosave and crash recovery preserve unresolved package nodes/configuration; migration failure leaves original recoverable. | M P2,P3 | 03 / V01,V08 |
| FR-06 | Keyboard traversal/editing, Unicode/IME, clipboard, DPI, accessible labels/actions, non-color status and native file dialogs work in the agreed desktop test matrix. | M P1,P3 | 02,09 / V06,V07 |
| FR-07 | Large graph culling/hit-testing and incremental updates meet NFR-03 without rebuilding unaffected nodes; headless mode loads no rendering window. | M P2,P3 | 02 / V05,V06 |
| FR-08 | Bool, i64/u64/f64, UTF-8, bytes, list, record, JSON, Option and schema enums round-trip with stable IDs. Null, missing and Error remain distinguishable. | M P2 | 03 / V01 |
| FR-09 | Custom types register versioned codecs, schema and editor description; unsupported codec retains original payload but prevents execution. | M P2 | 03,04 / V01,V03 |
| FR-10 | Arithmetic/comparison/math/logic report overflow, zero division and domain/nonfinite errors under the selected numeric policy; ordinary f64 rounding is documented. Explicit casts reject or expose loss under a declared policy; no silent numeric/string coercion. | M P3 | 03 / V01,V05 |
| FR-11 | String encoding, Unicode-aware indexing, parsing/formatting, collection and JSON nodes handle invalid encoding and bounds. Seeded random replays with the same algorithm/version and invocation order. | M P3 | 03 / V01 |
| FR-12 | Declare/initialize/Get/Set/Update variables enforce types, scope, initialization and sequencing; shared state mutations are atomic at runtime-defined commit points. | M P3 | 03 / V01,V02 |
| FR-13 | Persistent variable checkpoints are explicit, versioned and atomic; host access uses the runtime command service, not unsynchronized memory access. | M P3 | 03 / V01,V08 |
| FR-14 | Branch, enum switch including unknown-variant policy, Sequence, Fork, count/foreach/while/iterator loops and break/continue enforce structured control. Synchronous unbounded cycles are rejected; the legal cycle classes in FR-46 are accepted; long loops yield and cancel. | M P2,P3 | 03,11 / V01,V02 |
| FR-15 | Manual triggers, timers, subscriptions and external events expose ordering/reentrancy/queue policy; overflow is observable and no unbounded accumulation occurs. | M P2,P4 | 03,05 / V02,V04 |
| FR-16 | Errors have typed failure paths, cancellation and deadlines; effects execute exactly once per scheduled activation locally, without claiming exactly-once remote delivery. | M P2,P4 | 03,05 / V02,V04 |
| FR-17 | Text/binary read, write, atomic replace, append, directory list/filter and watch handle permissions, encoding and recoverable I/O failures. | M P4 | 05 / V04,V07 |
| FR-18 | Large file streams are bounded, cancellable and chunked; rename atomicity and optional crash durability are distinct documented modes. | M P4 | 05 / V04,V08 |
| FR-19 | Reusable typed connection profiles support app/workspace scope, versioning, TLS/auth/options, validation, redacted export/import and side-effect-free connection tests where possible. | M P4 | 03,05 / V04,V09 |
| FR-20 | Session pooling respects profile version, identity, isolation and protocol state; reconnect/backoff, shutdown and per-run ownership have deterministic tests. | M P4 | 05 / V02,V04 |
| FR-21 | OS vault stores credentials on all supported OSs; graphs/packages contain references only. Locked/unavailable vault blocks dependent operations with remediation; no plaintext fallback. | M P4 | 05 / V07,V09 |
| FR-22 | Secure credential input, rotation, revocation and deletion prevent secret leakage in documents, logs, errors and exports; raw-secret access is exceptional explicit authority. | M P4 | 04,05 / V09 |
| FR-23 | HTTP/HTTPS methods, headers, streaming bodies/responses, TLS verification, bounded payloads, timeout/cancel and typed status/errors work against local fixtures. Retry requires a declared idempotency rule. | M P4 | 05 / V04 |
| FR-24 | WebSocket text/binary send/receive, handshake, ping/pong, bounded messages, close/reconnect and subscription ownership work under network loss. | M P4 | 05 / V04 |
| FR-25 | MQTT 5 and 3.1.1 publish/subscribe expose topic filters, QoS, retain, will, session state and delivery metadata; duplicates and reconnect behavior are tested. | M P4 | 05 / V04 |
| FR-26 | AMQP 0-9-1 exchange/queue/binding, publish/confirms, consume/ack/nack and prefetch are explicit. Unsupported broker/protocol is diagnosed. | M P4 | 05 / V04 |
| FR-27 | REST conveniences and GraphQL query/mutation/variables preserve HTTP and GraphQL errors separately. GraphQL subscription requires an explicit supported subprotocol. | M P4 | 05 / V04 |
| FR-28 | gRPC descriptors/imports, service/method selection, protobuf values, HTTP/2, unary and all three streaming shapes preserve deadlines/status/metadata and cancellation. | M P4 | 05 / V04 |
| FR-29 | Outbound webhooks use HTTP policy; inbound listeners have explicit bind/auth/body-limit/lifetime settings, rejection paths and shutdown tests. | M P4 | 05 / V04,V09 |
| FR-30 | Audio assets open/decode supported formats and players select output per session, play/pause/stop/seek/volume/mix, emit completion/errors and route concurrent playback. | M P5 | 05 / V07,V10 |
| FR-31 | Playlist, seeded random audio from folder and selection among opened assets share the asset/player services; unavailable files and empty sets are visible errors. | M P5 | 05 / V01,V10 |
| FR-32 | Device enumeration/identity/hotplug, resampling, channel mapping and explicit fallback work across the declared audio matrix without UI/network work in callbacks. | M P5 | 05 / V07,V10 |
| FR-33 | PipeWire Linux can expose an application-owned virtual microphone source when supported, with clean destruction and capability reporting. Windows/macOS route to existing virtual endpoints; creation requiring drivers is reported unavailable. | M P1,P5 | 05 / V07,V10 |
| FR-34 | Install a local expansion package while open; validate/activate atomically and expose nodes without application restart or recompilation. Failed installs leave the registry unchanged. | M P2 | 04 / V03 |
| FR-35 | Plugin contributions cover nodes/types/codecs/services/adapters/panels/inspectors/settings/subgraphs; graphics commands later use the same handles. Supported boundary and limits are explicit. | M P2,P3; graphics P6 | 04 / V03,V06 |
| FR-36 | Supported base override slots offer deterministic visible selection/conflict handling. Existing graphs retain exact providers until an explicit rebind/migration transaction. | M P2 | 04 / V03 |
| FR-37 | Upgrade/disable/remove supports active pins, safe points, revocation, cleanup, rollback and missing-node preservation. Open application survives failed guest components. | M P2 | 04 / V02,V03 |
| FR-38 | Versioned SDK/package manifest/dependency locks/permissions/integrity checks reject incompatible or malformed packages; trusted workers disclose weaker isolation. | M P2 | 04 / V03,V09 |
| FR-39 | Bundled OBS package supports the bounded node/event set in 05 using shared profile/vault/WebSocket sessions and correlated requests; third-party extension reuses that service. | M P5 | 04,05 / V04 |
| FR-40 | Bundled Twitch package supports the bounded Helix/EventSub set in 05, public-client auth, refresh/validation/rate limits, reconnect and token redaction. | M P5 | 05 / V04,V09 |
| FR-41 | Optional Bevy scene/resource handles and bounded command/events permit 2D/3D expansion without coupling math/files/networking to frames. | S P6 | 02,04,09 / V03,V07 |
| FR-42 | Package registry/marketplace, remote discovery and publisher workflows support signed packages and revocation. Local installation remains usable offline. | S P6 | 04 / V03,V09 |
| FR-43 | AMQP 1.0 is a separately versioned adapter with negotiated broker support, never an alias for 0-9-1. | S P6 | 05 / V04 |
| FR-44 | Node kinds and port contracts: every definition declares one kind; ports declare kind, canonical type, source/availability, default/optional, variadic and laziness; the connection matrix and fan-in/fan-out limits are enforced at edit time; variadic keys are stable; dynamic ports come from declarative templates; every non-intrinsic node persists a signature snapshot so missing packages keep ports and edges. | M P2 | 11,12 / V01, G-29–31 |
| FR-45 | Typed generics and constraints: canonical type expressions; i32/u32/f32/duration/timestamp/map/result/stream in addition to FR-08; nominal package types; constraint implementations limited to the owning package; deterministic inference with persisted `type_args`; ambiguity and missing specializations are compile diagnostics. The editor presents Float Single/f32 and Double/f64 precision plus integer math. New untyped numeric input defaults to Double; resolved or explicitly selected integer inputs parse exactly without a float intermediate (ADR-20). | M P2 | 11,12 / V01, G-19, G-49 |
| FR-46 | Conversions and cycles: no implicit conversion; registry converters are classified lossless-total/checked-partial/lossy, auto-inserted visibly only when exactly one lossless-total converter exists, never chained, and owned by a type's package. Legal cycle classes (register/variable feedback, pure zones, structured loops, control cycles through suspension, event and reactive cycles under causal depth) compile; data cycles, synchronous control cycles and synchronous recursion are rejected with the cycle path. | M P2 | 11 / V01, G-06,07,12,16,20,21,24 |
| FR-47 | Structured execution order: regions, Sequence completion before the next step, Fork join modes, the value-availability (dominance) rule, deterministic pure evaluation order and first-error choice, epoch-consistent volatile reads, and deterministic replay from a recorded ingestion log. | M P2 | 11 / V01,V02, G-01–05,08–11,17,26,32 |
| FR-48 | Events and triggers: on-start/on-stop/manual/timer/custom event/variable-changed/watch and package entries; typed custom event declarations; emit enqueues and never runs a handler inside the emitter; handler order is persisted; per-entry reentrancy is queue/drop/replace/parallel with bounded queues and overflow counters; causal-depth guard against runaway chains. | M P2 | 11 / V02, G-12–16,38 |
| FR-49 | Reactive layer: variable-change and watch triggers fire at segment boundaries against one consistent state (glitch-free, coalesced), equal writes do not notify, and live preview re-evaluates only the changed pure cone without executing effects or mutating a running plan. | M P2,P3 | 11 / V02,V06, G-15,16,36 |
| FR-50 | Streams: affine `stream<T>` handles with a single consumer, pull consumption with backpressure (for-each-item, next, collect with a limit), explicit bounded tee, push-to-stream bridges with declared overflow policy, and cancellation that closes producers. | M P2,P4 | 11 / V02,V04, G-22,23 |
| FR-51 | All product-owned UI, package, help, accessibility and diagnostic text is translatable with namespaced catalogues, named typed redacted arguments, plurals and defined fallback. Language switches preserve edit sessions and graph/run identity; palette searches localized labels and stable IDs. Unicode/RTL/layout checks pass; canonical graph values and default parse/format remain locale independent. User content is preserved. | M P1–P5 | 02,04,11,12,13 / V01,V03,V06,V09,V11, G-39, L-01–08 |

## Workflows and boundaries

1. **Calculation:** create typed constants → math/string transforms → inspect result → save → headless execution gives matching outputs using the same seed and lock.
2. **Credential-backed request:** create profile → enter credential in secure host dialog → test connection → select profile in a request node → branch on typed result. Export prompts to remap references, not to include plaintext secrets.
3. **Event automation:** OBS event → read scoped variable → switch → play random audio on selected endpoint → update state. Stop tears down subscriptions and playback owned by the run.
4. **Expansion:** install package → grant scoped host authority → preview contributions → activate → explicitly select replacement provider → run. Remove preserves definitions in saved documents as unresolved placeholders.
5. **Recovery:** crash during edit/save/update → reopen last valid document and registry generation → report lost in-flight operations without replaying remote effects automatically.

1.0 excludes arbitrary monkey-patching, application-provided Windows/macOS audio drivers, hard real-time certification, distributed graph execution, collaborative editing, marketplace operation, general native UI injection and a complete scene editor. These are exclusions, not assumptions that Bevy already implements them. A plugin may still extend supported node/editor/scene contracts without any of these features.

Graph-language requirements FR-44–50 are specified normatively in [11](11-graph-language.md) and [12](12-graph-data-structures.md) and are delivered first by WU-00. FR-51 is specified in [13](13-internationalization.md), with message schemas before SDK freeze and UI qualification before authoring preview. Traceability is completed by the [work-unit coverage matrix](07-delivery-plan.md) and [NFR-to-verification mapping](06-quality-and-validation.md). No M requirement can be silently deferred to claim 1.0 complete; previews label their subset.
