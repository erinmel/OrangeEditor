# OrangeEditor expansion system

An expansion is installed into the running OrangeEditor application. It is not a Rust crate compiled into OrangeEditor and not a development MCP/plugin. Crates that a downstream product links against the core are a separate, build-time extension path (ADR-21). This design owns install, compatibility, permissions, contribution registration, lifetime barriers and removal.

Maintained expansion source lives under `plugins/<name>` in the [monorepo](15-monorepo-layout.md), with the versioned public SDK under `sdk`. Package versions/builds and application releases stay independent. Sharing a Git commit does not grant a package private host imports or bypass installation/permissions/provider locks.

ADR-22 narrows foundation delivery to generic capabilities. Specialized OBS/Twitch, TTS, webhook and browser-canvas integrations are independent plugins, not bundled release requirements. A plugin can add a typed event source (for example, a remote service notification) using owned bounded ingress, without adding scheduler rules. The SDK acceptance sample is synthetic and requires no live account. Plugins generating speech return an approved audio asset or bounded PCM stream to the existing selected-device playback service; they never write to an audio callback.

## Execution boundary decision

| Boundary | Hot addition / portability | Failure and authority | Contribution/latency limits | Decision |
|---|---|---|---|---|
| Native Rust dylib/trait objects | Platform/compiler/build sensitive | Host memory/credentials/process privileges; native fault can terminate host | Broad raw UI/ECS access; low call cost; no stable external Rust ABI | Rejected public format |
| Stable C ABI library | Possible per-target bundles; explicit ABI structs required | Same in-process crash/authority limits; unload needs every pointer/thread/callback barrier | Broad native access, ownership hazards, per-platform release testing | Not initial public format |
| Wasm Component Model | Portable validated component with versioned WIT-style host imports | Guest linear-memory isolation and checked imports; fuel/epoch/resource limits; host bug remains risk | Serialization/copy/call costs; no raw host pointers or native callback code | **Primary installable format** |
| External worker | Native runtime/language flexibility; bundle per target | Crash boundary and killable process; OS access remains user authority unless actual OS sandbox is implemented | IPC cost, versioned bounded frames, declarative UI/scene bridge only | **Explicit-trust secondary tier** |
| Statically linked bundled kernels/services | Shipped with app; cannot replace/unload code at runtime | Privileged host code; host crash not isolated | Cheap native kernels, audio callback/backend access | Documented bundled exception |

Rust ABI stability is not supplied by exported Rust symbols; stable foreign ABI contracts need explicit layout/ownership ([Rust Reference: ABI](https://doc.rust-lang.org/reference/items/functions.html#extern-function-qualifier), primary reference; exact boundary remains an Orange contract). Wasmtime documents Wasm isolation and capability imports ([security](https://docs.wasmtime.dev/security.html)); interruption limits only guest computation and does not automatically stop blocked host calls ([interruption](https://docs.wasmtime.dev/examples-interrupting-wasm.html), [configuration](https://docs.wasmtime.dev/api/wasmtime/struct.Config.html)). Retrieved 2026-10-20: the opened Config page resolved **51.0.0-dev**, not a selected stable dependency. Specific Wasmtime version, component async APIs, MSRV and features must be pinned after WU-01 review. No generated bindings in this plan are described as compiled.

Guest permissions are **enforced host-service authority**: profiles/destinations, approved file roots, scene resources, UI contribution IDs and quotas. Guests receive no unrestricted WASI filesystem/network/environment imports by default. Do not confuse a permission manifest with enforcement. Native bundled code and trusted workers can bypass in-app policy through OS APIs; worker mode displays that trust distinction and is disabled by default for downloaded packages. An OS sandbox for workers requires a separate per-OS design and validation; it is not a 1.0 promise. Host service operations validate handles, path identity and destination after redirects/reconnects. Compromised runtime/host, hostile kernel or other processes with the same OS-user authority are outside the guest isolation guarantee.

## Contribution surface

| Contribution / replaceable slot | Wasm component | Trusted worker | Bundled native |
|---|---|---|---|
| Node implementation, types, codecs, reusable graph | Yes; bounded values/calls | Yes; same contracts over IPC | Yes; shared definitions |
| Language surface ([11 §16](11-graph-language.md)): pure/volatile-entropy/action/async/entry/iterator definitions, constraint implementations for owned types, converters touching owned types | Yes; iterator `next` and async run as guest calls with host-mediated suspension | Yes | Yes |
| Port kinds, control/zone intrinsics, scheduling/reentrancy modes, cycle or availability rules | No; host language release only | No | Host language release only |
| Event-source provider | Typed payload and host-mediated owned subscription; no arbitrary OS hook or inline handler execution | Same ingress with native trust disclosure | Host input/timer/audio adapters; same event ownership |
| Connection adapter/service provider | Yes; host transport/credential imports and leases | Yes; registers versioned broker-facing service adapter; may own native connection with trust disclosure | Yes |
| File/audio/input-trigger backend provider | Non-real-time control/format policy through approved handles; cannot supply native callback | Yes; decoded PCM/control via bounded buffer bridge, never direct host callback | Yes; low-level OS/backend callback |
| Panels, settings, inspectors, value editors | Declarative widget tree, commands/actions, owned state | Same declarative contract | Same schema; host adapter privileged |
| Translation catalogues and translatable UI text | Data-only FTL resources and namespaced TextRef/MessageRef | Same contract; no executable formatter injection | Same public contract for bundled text |
| Scene objects/material parameters/behavior | Validated scene command batches/resource handles, P6 | Same bridge | Can implement renderer adapter, app release required |
| Raw host memory, custom render graph/shader pipeline/native widget code | No direct memory/system injection | No direct host internals; can supply validated assets or own external service | Host-reviewed release feature only |
| Credential provider | Can contribute broker workflow for its granted namespace; raw secret export absent by default | Trusted worker provider possible, explicitly discloses native authority | OS vault adapters privileged |

This supports extending or changing application behavior through stable contracts, including math node behavior, HTTP/connection policy providers, audio control/mixing provider choice and type editors. A custom renderer/backend that requires new host primitives first requires a compatible host release. "Expandable" does not mean arbitrary internal machine-code replacement.

Bundled math/string kernels use native dispatch to avoid guest overhead for tiny operations. Bundled audio engine/device adapters execute real-time/native code. They expose the same definition/provider IDs and behavioral contracts; a selected guest replacement runs as that replacement, not secretly as the bundled kernel. Native optimizer lowering is permitted only for the exact locked provider declaring that lowering. No generic installed guest is compiled into a trusted native host plugin. Component graph batching is optional and versioned; it must preserve individual node errors, debug/source mapping and selected semantics. WU-03 compares scalar/batched native/component/worker paths before API freeze.

## Package format and SDK

Local `.orangeplug` archive contains `manifest.json`, definitions/type schemas, component binary or target worker binaries, declarative UI assets, migration components and licenses. Import rejects archive traversal/reparse/symlinks, duplicate normalized paths, executable payload mismatch, malformed manifests, excessive compression/depth/size and digest mismatch. Workers never execute during validation. Native library payloads are refused in v1.

Packages contributing text include `localization` and catalogue files per [13 §3](13-internationalization.md). That section owns the manifest field shape, namespace/argument validation, integrity/lifetime and fallback contracts. The host resolves both rendered and accessible text; localized captions never replace stable option/action/provider IDs. Source catalogue validation precedes atomic activation. Translation bytes remain covered by exact package digests, so a translation update does not bypass explicit graph rebind rules.

```json
{
  "format": "orange.plugin", "format_version": 1,
  "id": "org.example.device", "version": "0.1.0", "publisher": "org.example",
  "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
  "localization": {"format_version": 1, "format": "fluent", "default_locale": "en", "catalogs": {"en": ["locales/en.ftl"]}, "arguments": {}},
  "execution": {"kind": "wasm-component", "entry": "component.wasm", "world": "orange:expansion/plugin@1.0.0"},
  "dependencies": [{"id": "org.orange.websocket", "version": "0.1.0"}],
  "contributions": {
    "nodes": ["org.example.device/get-status", "org.example.device/set-state", "org.example.device/on-state-changed"],
    "panels": ["org.example.device/session"],
    "providers": [{"slot": "connection:org.example.device/session@1", "id": "device-v1"}]
  },
  "permissions": [
    {"kind": "connection.use", "profiles": ["local-device"], "operations": ["websocket.send", "websocket.receive"]},
    {"kind": "auth.transform", "provider": "org.orange.auth/bearer@1", "profiles": ["local-device"]}
  ],
  "integrity": {"algorithm": "sha256", "payload_digest": "2222222222222222222222222222222222222222222222222222222222222222"},
  "license": "MIT"
}
```

Digest and package are synthetic. This sample device package implements a protocol over the granted WebSocket profile; raw send/receive grants do not enforce protocol operation restrictions. Its separate namespaced service validates allowed operations for other expansions. A consumer granted only typed device operations uses that checked service rather than receiving raw socket authority. Authentication uses host-supported broker operations; any new secret transform requires the explicit policy below and in 05. This sample is not a promised shipped device integration.

Archive digest excludes manifest signature/digest fields under a specified canonicalization rule; WU-03 must finalize this rule and publish cross-language vectors before distributing packages. Publisher names alone are not identity proof. Signed packages verify signatures against explicitly trusted signing identities; local unsigned packages show origin/hash and require explicit trust. Integrity verifies bytes, not benign behavior. Dependency locks record exact version+digest; a package cannot claim another publisher's reserved namespace. Cycles/incompatible major host API fail before activation. Offline local installation is baseline; registry/marketplace is P6.

SDK deliverables: versioned host/guest interfaces, JSON schemas, generated Rust guest bindings plus language-neutral wire documentation, conformance test runner, package validator, synthetic node/event-source/editor extension, migration sample, signing/bundling instructions and minimal supported toolchain. Guest toolchain may differ from the host if its output meets the pinned component contract; native worker binaries declare target OS/architecture and wire version. Every SDK release publishes a compatibility matrix against Orange host API and a supported deprecation window (proposed two minor host releases, confirmed at API freeze). No semver range alone overrides a graph's exact provider lock.

## Installation, activation and override

```mermaid
sequenceDiagram
  participant U as User
  participant P as Package manager
  participant S as Staging store
  participant R as Registry
  participant E as Editor
  U->>P: install local archive
  P->>S: bounded unpack + integrity/dependency validation
  P-->>U: origin, requested authority, conflicts, contribution preview
  U->>P: approve requested grants
  P->>P: instantiate with limits; run contract validation
  P->>R: atomic contribution transaction
  R-->>E: new immutable generation
  E-->>U: palette/panels available while app remains open
```

Approval is product UX for new package authority; it is not a request to stop this planning assignment. Installation state: Discovered → Staged → Validated → Granted → Activating → Active; failure → Quarantined with diagnostics and cleanup. Registry commit occurs only after contribution validation (IDs, versions, slots, schemas, quotas). Concurrent readers retain the prior snapshot; no partially installed palette. Activation has deadline and rollback cleanup. Activation failure cannot corrupt installed package bytes or modify a saved graph.

Replaceable slots include `node:<definition>@<contract>`, `convert:<from>-><to>` (contract version in the slot descriptor; [11 §6](11-graph-language.md)), `connection:<adapter>@<contract>`, `audio:playback-control@1`, `audio:device-backend@1` (native worker/bundled restrictions above), `editor:value-editor/<type>@1`, `editor:file-picker@1`, and approved credential namespaces. Slot contract includes capabilities, errors, concurrency, real-time restrictions, host API range and conformance tests. A replacement node provider must keep the definition's kind, ports, `suspends`, `cleanup_safe`, determinism class and outcome set. Changing any of them is a new definition version, not a provider swap. Core type rules, package signature verification, permission enforcement and graph effect ordering are not override slots. Publication also rejects namespace, constraint-coherence and converter-ownership violations ([12 §8](12-graph-data-structures.md)).

When providers compete: no automatic last-installed-wins. User selects workspace/app defaults for **new bindings**; existing graph locks retain provider ID, package version/digest and contract. Explicit rebind previews affected nodes/types/config/resource changes, runs migration on a copy and validates the graph before commit. Missing selected provider blocks affected execution and preserves placeholders. Uninstall may propose a compatible bundled fallback, but executing it requires explicit rebind; there is no silent substitution. Diagnostics expose selected provider/registry generation and override provenance.

Converter pair slots may contain distinct semantic entry contracts. An override implements the same converter ID/contract, preserving its endpoints, class, policies, errors and loss output. New-binding provider defaults are scoped to that entry contract; implementing an existing entry does not grant authority to register new foreign-to-foreign conversion semantics. Multiple providers for one entry do not increase the semantic conversion count used for auto-insertion ([11 §6](11-graph-language.md)).

## Hot update/removal semantics

Hot addition immediately publishes new contributions. **Hot replacement of running code is separate:** existing runs retain old snapshots/component instances; new graphs or explicitly rebound graphs select the new package version. A graph edit normally prepares a new run; live swapping an executing continuation is not supported in 1.0. Existing pending external side effects are never replayed to imitate migration.

Update sequence: stage new immutable version → dependency/API/authority validation → request only new grants → instantiate → atomic registry publication → optional explicit graph migration/rebind → old generation drains. State migration occurs only at a declared stopped/checkpoint boundary with schema/handle validation; arbitrary continuation stacks, raw pointers, sockets and audio callbacks do not migrate. New authority cannot be silently inherited from old grants.

Disable/removal hides package from new bindings and records a tombstone, then offers **drain**, **cancel affected runs**, or **leave pinned until they stop**. App stays open. Revocation is immediate policy enforcement: cancel prohibited I/O, revoke handles, drop pending callbacks, and fault affected activations even if old code remains pinned. Generation checks reject stale handles/messages. Delete files only after instance/task/session/UI/resource barriers report zero references. Trusted worker kill may be necessary after deadline; host-owned leases still clean up. Statically linked bundled machine code cannot physically unload; disabling its registrations is logical removal, communicated as such.

Rollback restores previous registry generation/default selections, reinstates old package availability and preserves old graph locks. It cannot undo already committed file/network effects or restore revoked credentials. Failed installer/migration leaves the previous registry usable. Crash journal records staged/committed generation and pending cleanup; restart selects the last fully committed generation and reports recoverable stale files, never executes a half-staged worker.

Resource limits (proposals for WU-03): 64 MiB component linear memory default, finite call fuel/deadline, max 1 MiB ordinary IPC frame, bounded host handles/tasks per grant. Streaming uses chunk handles rather than increasing every frame limit. Quotas are adjustable visible policy, tested with malformed/hostile packages. Guest trap faults its activation; repeated traps quarantine the instance/package and preserve the editor. Native host out-of-memory/crash cannot be promised recoverable by catching an ordinary Rust error.

Requirements: FR-09, FR-22, FR-34–38, FR-41–42, FR-44–46. Validation V03/V09 plus NFR-01/06/09 in [06](06-quality-and-validation.md). ADR-03/06/07/16 in [08](08-decisions.md).

The N1a orange-bundled scalar implementation uses the shipped native exception above. Ordinary org.orange.math definitions remain replaceable through provider contracts, but its native resolver binds only the exact selected native package/version/digest and supported scalar specialization. Selecting a guest/worker provider never falls back to native math when that provider cannot be resolved. The scalar implementation adds no installer, hot-loading mechanism or permission bypass for downloaded packages.