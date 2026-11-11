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

Binding request (2026-10-27): host kernel resolvers should implement `PureKernelResolver::resolve_binding` and `EntropyKernelResolver::resolve_entropy_binding`. They receive a `BindingRequest` holding only the node identity, the exact definition, the locked provider, the config version, the config, the signature and the pinned registry snapshot, built by the compiler after its exact-lock checks. Since 2026-10-28 these are the only methods a resolver implements; the document-wide `resolve` and `resolve_entropy` are now `ResolveInDocument` and `ResolveEntropyInDocument`, blanket-implemented conveniences that build the request first, so no resolver receives the whole document. This is a breaking change for resolvers that implemented only the old methods. Configuration is still interpreted at compile time by the adapter, and execution traits are unchanged.

## Contribution surface

The table below is the selected extension contract. Current installed guest
execution implements Pure, Action and Async nodes with concrete port types and empty
config (K02c; effects 2026-11-10). The compiler and host also support package Entry
nodes fed by trusted native service sources, including incoming WebSocket text.
Installed Wasm guests still lack host service imports, iterators and resource contributions. Trusted host type registration implements records, enums and handle kinds;
arbitrary package scalar representations/codecs and complete archive contribution
activation remain open. Metadata registration alone does not prove execution support.

An ordinary node declares ports, types, configuration and effects within an existing
execution class. Special control rules belong to the host language release; a
separate evaluation model belongs to a trusted downstream domain. Typed domain
registration/dispatch and its external consumer probe remain the ADR-21 pre-G2 gate.
Backend replacement uses an existing slot contract and grants no scheduler authority.

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
  "dependencies": [{"id": "org.orange.websocket", "version": "0.1.0", "content_digest": "sha256:3333333333333333333333333333333333333333333333333333333333333333"}],
  "contributions": {
    "nodes": ["org.example.device/get-status", "org.example.device/set-state", "org.example.device/on-state-changed"],
    "panels": ["org.example.device/session"],
    "providers": [{"slot": "connection:org.example.device/session@1", "id": "device-v1"}]
  },
  "permissions": [
    {"kind": "connection.use", "profiles": ["local-device"], "operations": ["websocket.send", "websocket.receive"]},
    {"kind": "auth.transform", "provider": "org.orange.auth/bearer@1", "profiles": ["local-device"]}
  ],
  "integrity": {"algorithm": "sha256", "content_digest": "2222222222222222222222222222222222222222222222222222222222222222"},
  "license": "MIT"
}
```

Digest and package are synthetic. This sample device package implements a protocol over the granted WebSocket profile; raw send/receive grants do not enforce protocol operation restrictions. Its separate namespaced service validates allowed operations for other expansions. A consumer granted only typed device operations uses that checked service rather than receiving raw socket authority. Authentication uses host-supported broker operations; any new secret transform requires the explicit policy below and in 05. This sample is not a promised shipped device integration.

Installed packages between sessions (2026-11-11; `orange-host`): a host given a
package directory keeps each installed archive and the capabilities granted to it, and
installs them again when it next starts (02). The `orange` command takes the directory
as `--store <dir>`.

Native plugins in Rust (2026-11-11; `orange-sdk`): a plugin crate depends on
`orange-sdk` and returns a `ServicePackage` holding its definition files, a
`ServiceNode` per asynchronous node, a `ServiceSource` per trigger and a
`ConnectionAdapter` per connection kind. The application passes it to
`Host::with_services`. This tier is compiled into the application: it is trusted, not
sandboxed and not installed at run time. A plugin that must be installed into a running
application is a Wasm component or a graph package.

Manifest `about` and `update` (2026-11-11; `orange-packages`): two optional top-level
sections. `about` holds `name` (≤ 100 characters), `description` (≤ 2,000), `authors`
(≤ 16), `homepage` and `repository` (`https://` URLs) and `keywords` (≤ 16 distinct
lowercase `[a-z0-9-]` terms). Text has no control characters and no surrounding space;
`name` and `description` are fallbacks for a host without the package's localization
catalogue (13 §3). `update` is `{"feed": "https://…", "channel": "stable"}`: where the
publisher announces versions of this package and which channel this build belongs to
(default `stable`). A feed URL must be `https://`, at most 2,048 printable ASCII bytes,
with a host and no user information. Both sections are covered by the content digest
like the rest of the manifest. They are descriptive only: installation never contacts
a URL, and the feed's own format, its signature and the host's update check are not
defined yet. A bad field is `packages/manifest-metadata` and names the field.

Manifest schema (K01, 2026-11-06; `orange-packages`): the manifest is strict JSON. Duplicate object keys and numbers other than integers within ±(2^53 − 1) are rejected at any depth, including uninterpreted sections, because readers disagree on them. `execution` is absent for a data-only package, `{"kind": "wasm-component", "entry", "world"}` with `world` = `orange:expansion/<name>@<x.y.z>`, or `{"kind": "worker", "wire_version": 1, "targets": [{"os": "windows"|"macos"|"linux", "arch": "x86_64"|"aarch64", "entry"}]}` with 1–16 distinct targets; any other kind, including native libraries, is refused. `contributions.panels` lists up to 256 distinct panel IDs `<package>/<[a-z][a-z0-9-]*>` in the package's own namespace; the declarative widget descriptor format is later UI work (ADR-08), so panels are identities only. `localization` follows 13 §3 exactly: format version 1, `fluent`, default locale `en` with an `en` catalogue, BCP 47-shaped tags unique ignoring case, non-empty lists of distinct `.ftl` files, `[a-z][a-z0-9-]*` message IDs and `[a-z][a-z0-9_]*` argument names typed from the 13 §3 vocabulary, with bounded counts. Every file reference is an archive path (below). Dependency locks are `{"id", "version", "content_digest": "sha256:<64 hex>"}`; installation refuses an active dependency whose content digest differs from the lock. `integrity.content_digest` (formerly `payload_digest`; the unreleased name has no reader) is the package's own content digest. `permissions` stays uninterpreted until grants (K03). Errors carry stable `packages/manifest-*` keys and redacted text.

Canonical archive (K01, 2026-11-06; `orange-packages`): a `.orangeplug` file is the header `\x89OPKG\r\n\x1a`, layout version `u16 = 1`, flags `u16 = 0` and entry count `u32`, then per entry `path_len u16 | path | kind u8 = 0 | mode u8 = 0 | size u64 | bytes`, little-endian. `manifest.json` is the first entry; the others follow in strictly ascending path byte order and nothing follows the last. There is no compression: deflate has many valid encodings, so a canonical byte form would need a pinned encoder, and without it unpacked bytes never exceed archive bytes. Archive paths are relative ASCII `[A-Za-z0-9._-]` components separated by `/`, at most 240 bytes and 16 components, without `.`/`..`, empty components, trailing dots or Windows device names; two paths equal ignoring ASCII case, or a file that is also another entry's directory, are rejected. Kinds other than a regular file (directories, symbolic or hard links) and nonzero modes are rejected. The content digest is SHA-256 over `"orange.plugin/content/v1\0" | u64 len(M) | M | u32 n | n × (u16 len(path) | path | u64 size | SHA-256(bytes))`, where `M` is the RFC 8785 canonical JSON of the manifest without its `integrity` member and the entries are the archive's other files in order; it therefore covers every metadata byte, including dependency locks. `read_archive` streams entries through an `ArchiveSink`, checks every quota (`ArchiveLimits`: archive bytes, entries, per-file and total file bytes; defaults 256 MiB, 4,096, 128 MiB and 256 MiB, unmeasured) against declared sizes before reading an entry's bytes, never allocates by a declared size, requires each manifest-referenced file under its exact path, requires a component entry to start with the WebAssembly magic, refuses PE/ELF/Mach-O bytes outside declared worker targets, and finally compares the content digest; any failure aborts the sink. `DirectorySink` stages into a new directory under a plain (not linked or reparse-point) parent, creates every directory itself, opens files with `create_new`, flushes each, and deletes the directory on abort. `VerifiedArchive::require_digest` checks a document or dependency lock, and `into_contents` is the archive route to `PackageContents`. Cross-language vectors: `app/crates/orange-packages/tests/fixtures/archive-v1.json`, reproduced by the Rust reader and by the independent Node encoder `tools/package-archive.mjs`. Definition and specialization file locations inside the archive, signatures and actual code loading remain open.

The content digest excludes only the manifest's own `integrity.content_digest` value under the canonical rule below, which K01 fixes together with cross-language vectors; signatures, when added, are detached over that digest. Publisher names alone are not identity proof. Signed packages verify signatures against explicitly trusted signing identities; local unsigned packages show origin/hash and require explicit trust. Integrity verifies bytes, not benign behavior. Dependency locks record exact version+digest; a package cannot claim another publisher's reserved namespace. Cycles/incompatible major host API fail before activation. Offline local installation is baseline; registry/marketplace is P6.

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

The durable package store has one cooperating writer per directory. `FileJournal`
acquires a nonblocking exclusive OS lock on the stable sibling
`packages.journal.lock` before reading recovery state or removing stale compaction
files. It retains that ownership through writes, compaction and failed reopen paths
until the journal is dropped. Another owner refuses with a redacted journal error.
Retirement explicitly unlocks after closing the append handle, so an inherited
duplicate descriptor cannot prolong the writer's ownership beyond that retirement.
The lock file remains on disk; OS ownership releases on process exit, so its presence
is not a stale-lock indicator. Hosts must keep the store directory under their control.
The record format remains 2 with no historical reader.

Hot addition immediately publishes new contributions. **Hot replacement of running code is separate:** existing runs retain old snapshots/component instances; new graphs or explicitly rebound graphs select the new package version. A graph edit normally prepares a new run; live swapping an executing continuation is not supported in 1.0. Existing pending external side effects are never replayed to imitate migration.

Update sequence: stage new immutable version → dependency/API/authority validation → request only new grants → instantiate → atomic registry publication → optional explicit graph migration/rebind → old generation drains. State migration occurs only at a declared stopped/checkpoint boundary with schema/handle validation; arbitrary continuation stacks, raw pointers, sockets and audio callbacks do not migrate. New authority cannot be silently inherited from old grants.

Disable/removal hides package from new bindings and records a tombstone, then offers **drain**, **cancel affected runs**, or **leave pinned until they stop**. App stays open. Revocation is immediate policy enforcement: cancel prohibited I/O, revoke handles, drop pending callbacks, and fault affected activations even if old code remains pinned. Generation checks reject stale handles/messages. Delete files only after instance/task/session/UI/resource barriers report zero references. Trusted worker kill may be necessary after deadline; host-owned leases still clean up. Statically linked bundled machine code cannot physically unload; disabling its registrations is logical removal, communicated as such.

Rollback restores previous registry generation/default selections, reinstates old package availability and preserves old graph locks. It cannot undo already committed file/network effects or restore revoked credentials. Failed installer/migration leaves the previous registry usable. Crash journal records staged/committed generation and pending cleanup; restart selects the last fully committed generation and reports recoverable stale files, never executes a half-staged worker.

Resource limits (proposals for WU-03): 64 MiB component linear memory default, finite call fuel/deadline, max 1 MiB ordinary IPC frame, bounded host handles/tasks per grant. Streaming uses chunk handles rather than increasing every frame limit. Quotas are adjustable visible policy, tested with malformed/hostile packages. Guest trap faults its activation; repeated traps quarantine the instance/package and preserve the editor. Native host out-of-memory/crash cannot be promised recoverable by catching an ordinary Rust error.

Implementation status (2026-11-03; `orange-packages`): `PackageManifest::from_json` validates the format 1 fields above (exact version, sha256 payload digest, publisher-owned ID, own-namespace nodes, host API minor range, exact dependencies, typed `node:`/`convert:`/`connection:`/`audio:`/`editor:` slots) and rejects unknown fields; `execution`, `permissions`, `localization` and `panels` are retained uninterpreted (superseded for all but `permissions` by the K01 manifest schema above). `PackageStore` walks Discovered → Staged → Validated → Granted → Activating → Active, or Quarantined with the typed failure, and publishes each install, update, removal, revocation or rollback as a new `RegistrySnapshot` built from the current one; a failed step publishes nothing. Update keeps the previous version published beside the new one, so existing locks keep resolving, and moves new-binding defaults to the same provider ID; retiring the old version is a separate removal. Removal leaves a tombstone, keeps definitions as passive metadata and is refused while an active package depends on it; locked nodes report provider-unavailable. A published version keeps its digest for good: rollback leaves a version it un-installs as a Removed tombstone, so the same version cannot return with other bytes. Revocation applies to the digest: every version published with it is deactivated, the digest is blocked and rollback history is discarded. A `Journal` trait (in-memory implementation only) records steps; each commit record checkpoints the active set, published digests and revoked digests. `Recovery` reports that state plus abandoned half-staged versions, and `PackageStore::restore` rebuilds a store from it and the staged contents of the active versions. Not implemented: archives, grants, code instantiation, draining/barriers and non-node slot publication. Explicit rebind is `orange_model::rebind` ([12 §8](12-graph-data-structures.md)).

Recovery and retention update (2026-11-05; `orange-packages`): `PackageStore::restore` stages the whole candidate against a private in-memory journal in dependency order, then makes exactly one atomic `Committed` append to the host journal. A failed append writes nothing and returns `TransactionError::Journal`, so the host may retry; a crash leaves either the old or the new complete checkpoint, never a prefix. Hosts must append atomically. `Journal::compact(checkpoint)` replaces earlier entries with one complete `Committed`; its default is a no-op and an implementation must be atomic (all earlier entries or exactly the checkpoint). Compaction drops per-step transitions, so a half-staged version is no longer reported abandoned: compact when no transaction is in flight. `PackageStore::compact_journal` writes the current state through it. `RetentionPolicy { max_rollback_generations }` (default 16, an unmeasured starting point; `0` disables rollback) bounds in-memory rollback history. Rollback beyond it is `RollbackWindowExceeded`; retirement releases only the store's reference, so a snapshot a plan still holds stays usable. Published digests and revocations are identity state and survive retirement. Re-selecting the current default is a no-op and creates no generation. Defaults are checkpointed ([12 §8](12-graph-data-structures.md)); package error text is redacted ([13 §8](13-internationalization.md)).

Default-absence recovery (P01, 2026-11-05): a checkpoint records an exact selected provider **or explicit absence** for each controlled definition ([12 §8](12-graph-data-structures.md)). Removing or revoking a selected package provider therefore remains unselected after restart, even when the shipped base registry contains another default. Rollback restores its prior selection state and compaction retains explicit clears; unrelated base definitions keep their choices. This is new-binding policy only: existing graph locks and running provider snapshots never change. The current pre-beta checkpoint representation replaces its earlier unreleased form without compatibility readers or a migration layer. Durable journal implementations, executable instances and draining still require their own evidence.
Durable journal (K01, corrected 2026-11-09; `orange-packages`): `FileJournal` implements `Journal` over `packages.journal` in a host-chosen directory. Current header is `OJOURNAL | version u16 = 2`; each record is `length u32 | CRC-32(length) u32 | CRC-32(payload) u32 | JSON`, little-endian. Protecting the length prevents accidental header corruption from hiding later identity/revocation commits as an apparent short tail. `append` writes and flushes a whole record; failure truncates its partial bytes, and failure of that cleanup closes the journal to further appends. `open` truncates only a physically short final header or payload and reports dropped bytes. A complete header/payload checksum mismatch, over-limit length or unsupported shape refuses open and preserves the log. CRC detects accidental corruption, not malicious rewriting. Recovery includes every complete record, including a durable commit whose acknowledgement was not observed before a crash; an interrupted install recovers the previous or new complete committed set and reports half-staged versions. `compact` writes and flushes one checkpoint to `packages.journal.tmp`, then renames it over the log; open deletes a stale temporary file. Format 2 replaces unreleased format 1 without a historical reader; other versions are refused without truncation. Error reasons contain operation/I/O kind only. Windows rename durability relies on NTFS metadata journaling because std provides no directory flush; Linux/macOS attempt a directory flush. Power-loss/per-filesystem qualification remains open.

SDK surface (2026-11-03): `HOST_API_VERSION` (1.0) is the host API this foundation build offers; hosts pass it to `PackageStore::new`, which refuses a manifest whose range excludes it. Definition files decode with `NodeDefinition::from_json` (default 1 MiB, 1,024 ports) or `from_json_with_limits` ([12 §4](12-graph-data-structures.md)). `PackageContents::with_specializations(provider, definition, supported)` lists the argument sets a generic provider supports ([11 §5.5](11-graph-language.md)); the pair must be a declared `node:` contribution (`TransactionError::UndeclaredSpecialization` otherwise), and the list is published on that exact provider, so other instantiations report `E-NO-SPECIALIZATION`. Without a list the provider delegates to constraints. Which archive file carries the list is not specified yet; the host supplies it with the staged contents.

Requirements: FR-09, FR-22, FR-34–38, FR-41–42, FR-44–46. Validation V03/V09 plus NFR-01/06/09 in [06](06-quality-and-validation.md). ADR-03/06/07/16 in [08](08-decisions.md).

The N1a orange-bundled scalar implementation uses the shipped native exception above. Ordinary org.orange.math definitions remain replaceable through provider contracts, but its native resolver binds only the exact selected native package/version/digest and supported scalar specialization. Selecting a guest/worker provider never falls back to native math when that provider cannot be resolved. The scalar implementation adds no installer, hot-loading mechanism or permission bypass for downloaded packages.

Package interface, first world (K02b, 2026-11-08; `sdk/wit/orange-expansion.wit`):
WIT package `orange:expansion@1.0.0`, world `plugin`. It imports `host` (always
linked: `api-version() -> (u16, u16)`, currently (1, 0)) and `log` (`write(message)`,
linked only with the `log` grant), and exports `nodes.evaluate(definition, inputs) ->
result<list<value>, node-error>`. A `value` is one Orange literal in the
`orange.value.v1` codec: the JSON text of `{"type", "value"}` (or `"bits"` for floats),
so literal-capable types, aggregates included, cross with exact float bits, map order and
None. `node-error` carries a namespaced `code` and `message-key` only. The host
(`orange_guest::GuestPlugin`) encodes inputs with their declared types, checks the
output count, each output's exact type and both error identifiers, and reports any
breach as `GuestError::OutputContract` without poisoning the instance; a value that
does not have its declared type never reaches the guest (`InputContract`). A
guest-reported node error is data (`GuestNodeError`), not a trap. Each instance links
only granted interfaces, so a component importing `log` without the grant fails to
instantiate. Log entries are bounded (64 per instance, 1 KiB each). Guests are built
with the Rust toolchain in `sdk/`: `wasm32-unknown-unknown`, `wit-bindgen` 0.63.0 for
guest bindings and `orange-componentize` (`wit-component` 0.262.0) to produce the
component; wit-component keeps only the imports a guest actually uses. Resources,
streams, async, events, panels and service imports are later interface versions.

Binary values and effects (2026-11-10, replacing the first unreleased world in place):
`value` is a variant. `literal(string)` is the `orange.value.v1` text above and can
carry any value. `bytes(list<u8>)` carries `core/bytes@1`, and `i32-list`, `i64-list`,
`u32-list`, `u64-list`, `f32-list` and `f64-list` carry `core/list@1<T>` of that
scalar, so audio blocks and pixel data cross without text conversion. The host sends
these types in their binary case whenever it holds them densely and as a literal
otherwise; a guest may return either form. A binary case is accepted only for exactly
its own type; any other use is `OutputContract`. The world also exports
`effects.execute(definition, inputs) -> result<list<value>, node-error>` for Action
and Async nodes. Unlike `evaluate` it is never folded, cached or repeated, so a guest
may keep state in its instance. `GuestPlugin::execute` shares the value, error, fuel,
transfer and cancellation contract of `evaluate`.

Graph-implemented package nodes (2026-11-10; `orange-host`): a node made in the editor
is published without code. Its manifest provider has the reserved ID `graph`, and the
archive carries `graphs/<name>.json` beside `nodes/<name>.json`:
`{"format": "orange.graph-node", "format_version": 1, "function": <function
declaration, 11 §14>, "variables": [<local variable declarations>]}`. The package needs
no `execution` section, and one package may mix graph and component providers. The
function's kind and data ports must equal the node definition's; the body may use core
nodes and any package nodes installed before it, and may declare local variables only,
so each call has its own state and nothing leaks into the calling document.

Installation decodes every graph, then validates them together, linked, against the
registry including the package's own definitions; a contract mismatch is `Graph` and a
failing body is `GraphInvalid` with its diagnostics, and neither is published. A
document uses such a node like any other, locked to its exact provider, and the saved
document never contains the body. When the host checks or opens a document it links an
in-memory copy: each graph-implemented node becomes a `core/call` of the package's
function with the same node ID, literals and edges, and the function and its local
variables are added once. From there validation, compilation and execution are those
of graph functions; there is no boundary to cross and no per-call copying.

Bodies built on other packages, and connections (2026-11-11): a graph file may carry
`"signatures"` and `"package_lock"`, copied from the document its body was authored in.
Linking adds them where the calling document lacks them, which is what lets a body use
another package's nodes; without them such a body does not validate. A node whose
definition sets `"connection": true` is implemented by a graph file with
`"connection": "<id>"`: a stand-in ID that the body's own nodes name in their
`connection` field. The two must agree or the package is refused. Each use of the node
names a real connection of its document, and linking gives each distinct connection its
own copy of the function, with the stand-in replaced and with function, node and edge
IDs derived from the original ID and the connection, so one diagram can use the node on
several connections. At installation the body is validated against a stand-in
declaration. When the host validates a document it states that package nodes inside
function bodies write no graph variable, which holds for every execution tier because
package code has no access to a run's variables.

Trigger nodes made as graphs (2026-11-11): a node of kind `entry` with a `payload`
output is implemented by a graph file with a `"trigger"` object instead of a function:
`{"event": "<stand-in id>", "nodes": [...], "edges": [...]}`. The nodes are root nodes
that start at some other entry (a timer, a manual entry, another package's trigger)
and end in `core/emit` on the stand-in event. Linking gives each use of the node its
own copy of the fragment and its own run-scope event, with node, edge and event IDs
derived from the using node, replaces the stand-in connection with the one the use
names, and turns the using node into `core/on-event` of that event; it keeps its ID,
its `then` outcome and its `payload` output, so its edges stay. A fragment is
validated at installation against stand-in declarations. A trigger without a payload
cannot be made this way, since an event carries one.

Limits: function and body node IDs are the publisher's, so two versions of
one graph package cannot be linked into the same document; trace events name body
nodes that the calling document does not contain.

`orange_host::pack` builds an archive from a manifest and files, computing the content
digest; `orange pack <dir> <out.orangeplug>` does the same from a directory.

Definition files and installation (2026-11-10; `orange-host`): each node a manifest
contributes, `<package>/<name>`, is described by the archive file `nodes/<name>.json`
in the `NodeDefinition` format. `Host::install` reads and digest-checks those files,
refuses a file that defines another ID, prepares the component for each provider ID
before publishing anything, then installs through `PackageStore`. A package provider
becomes the choice for new nodes of a definition only when no provider is chosen yet;
an existing choice is never replaced by installing. Flows already running keep the
providers they were compiled with. The host's store is in memory for now.

Package effect nodes in graphs (2026-11-10): `WithGuests<R>` also implements
`EffectResolver`. An Action node of a loaded package gets an inline action kernel and
runs on the run's own thread inside a pump. An Async node gets no kernel: the run
issues an ordinary effect request whose `context` is that node's `GuestEffect`. The
host recovers it with `guest_effect(&request.context)`, calls `GuestEffect::run` off
the run's thread and passes the result to `Run::complete_effect`; `cancellation()`
interrupts it when the effect's lease is cancelled. Each node binding owns one guest
instance for the life of its plan and calls on it are serialized, so every request of
one node reaches the same state. A guest-reported error becomes the node's own fault
and takes its `failed` route. Exec ports carry no value and are not passed to the
guest; event ports and non-empty config are refused at binding.

Package nodes in graphs (K02c, 2026-11-08; `orange_guest`): `GuestPackage::from_archive`
takes a `VerifiedArchive` and the files its `MemorySink` received, requires a
`wasm-component` execution section whose world is `orange:expansion/plugin@1.x`, a
declared provider ID and the entry file, compiles the component, and records the exact
provider key (package ID, version, verified content digest, provider ID) the package
store publishes for it. `WithGuests<R>` wraps a host resolver: a binding whose locked
provider is a loaded package's key gets a guest kernel; every other binding goes to
`R` (for example `MathPackage`), and a loaded package's provider never falls back to
`R`. Pure guest bindings (effect bindings are described above) need empty config and concrete port types; each
binding owns one guest instance, so a trap faults that node's binding and later folds
report `core/PackageFault` / `core/guest-poisoned`. A guest node error becomes the
kernel fault with the guest's own validated namespaced code and message key, each
at most 128 bytes. Identifiers use shared owned text, released when the last fault,
plan, result or journal owner drops; there is no process-wide identifier table.
Invalid or overlong identifiers map to `core/PackageFault`; a guest fault is
`core/PackageFault` with the `GuestError`
key. Constant folding and data demands run guest kernels like bundled ones. Guest
entropy, actions, async, events and generic (non-concrete) package nodes are later
work; grants come from the host until manifest permissions are interpreted (K03).

Verified payload correction (2026-11-09; FR-34/36/38, NFR-08/09):
`VerifiedArchive::verify_file` checks independently stored bytes against the exact
entry's size and SHA-256. `GuestPackage::from_archive` checks before compilation
and refuses a mismatched sink with `GuestLoadError::Integrity`. Reusing a
`MemorySink` replaces its previous archive at the next manifest, removes old-only
paths and keeps one archive's storage; failure clears that staging set. Verified
metadata and arbitrary payload bytes cannot be mixed under a provider lock.

Scoped guest cancellation (K02d1, 2026-11-09): `GuestInstance::cancellation()` and
`GuestPlugin::cancellation()` return a cloneable `GuestCancellation` scoped to that
instance's entire lifetime, not a reusable operation ID. `cancel()` sets a monotonic
flag and wakes epoch checks. A cancellation observed before entry or in running
guest code reports `GuestTrap::Interrupted`, poisons the instance, and later calls
report `GuestError::Poisoned`. Discard that instance; handles never reset or retarget.
An old handle may keep its engine alive but cannot cancel another store. Merely
dropping a handle does not cancel work. `is_cancelled()` reports the local request;
an engine-wide interruption is separately reported by the executing instance.

The host must periodically call `GuestEngine::advance_epoch()` while guest workers
run. A single wakeup can race Wasmtime's deadline renewal after a callback, so it
is not a reliable ticker. The per-store callback interrupts only a cancelled store
and lets peers continue. `GuestEngine::interrupt()` remains an explicit broadcast
to calls already running; idle instances renew their broadcast baseline before a
later call. Epoch cadence and worker ownership are host policy; there is no
wall-time latency qualification. Fuel is still charged independently. Epochs cannot
preempt a blocked host import. Service-import cancellation, C11 lease integration,
manifest authority enforcement and package revocation remain separate gates.

Guest resource correction (2026-11-09; review F2/F3/F4/F7, FR-35/38, NFR-05/06/08):
both instance constructors use one store-owned limiter. `memory_bytes` is the total
of initial linear memories and successful growth; `table_elements` is the total
initial/grown table entries. Denial reserves no capacity; allocation failure refunds
its reservation. Defaults are64MiB memory,65,536 table elements,64 memories/tables/
core instances,10million execution fuel and512KiB Wasm stack. These are adjustable
unqualified policy values, not process RSS bounds. The selected no-threads Wasmtime
feature profile rejects shared memory, whose growth bypasses the synchronous limiter.

`GuestLimits::transfer_bytes` defaults to1MiB of Wasmtime hostcall fuel for each
guest-to-host import or result. Canonical list/string storage is admitted before
copying, independently of execution fuel and linear memory. Exhaustion traps and
poisons that instance under the existing redacted `GuestTrap::Other` mapping.
Wasmtime48.0.8 already defaults to128MiB per transfer; the review's original claim
of an unbounded copy was incorrect. OrangeEditor now selects an explicit limit.
This is nominal runtime accounting, including encoded text and list descriptors,
not an exact allocator/transcoding peak or an aggregate of all imports in a call.
Retained logs remain64entries of1KiB, separately. Decoded plugin outputs also share
one `transfer_bytes` nominal value budget; decoding checks minimum container slots
before reserve and includes record identities and absent field slots. Decoded-output
refusal reports `OutputContract` without poisoning or publication. Added Rust limit
fields are unfrozen API changes; WIT and saved formats stay unchanged. Cancellation
verification uses observed live callbacks and host ticks, replacing the redundant
sleep-based interruption test.
