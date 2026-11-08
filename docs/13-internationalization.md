# Internationalization and translatable text

C12b bridge messages (2026-11-07): fieldless SharedError uses
`core/shared-wrong-workspace`, `core/shared-wrong-run`, `core/shared-stale`,
`core/shared-not-running`, `core/shared-contract`, `core/shared-limit` and
`core/shared-snapshot-regression`, with static fallbacks and no arguments.
RunError::Shared preserves that key. Host actor failures retain validated native
faults; uninitialized reads/updates use `core/uninitialized-variable` with the
original graph source. Other actor failures use a static WorkspaceFailure envelope
and the existing workspace key. Host-supplied rejection faults undergo ordinary
effect envelope/type/byte validation. Debug shows identities/counts/lifetime
metadata, never operands, old/new values, names, config or provider formatting.
Locale cannot alter scope, snapshot version, ordering, ownership or cancellation.

C12a boundary messages (2026-11-07): fieldless host WorkspaceError uses
`core/workspace-contract`, `core/workspace-unknown-variable`,
`core/workspace-uninitialized`, `core/workspace-limit`,
`core/workspace-identity-exhausted`, `core/workspace-schema-mismatch` and
`core/workspace-domain-unsupported` with static
fallbacks and no arguments. Native update failures retain their validated static
KernelFault code/key; the host can inspect that envelope while Display stays the
generic "The workspace update failed." Debug omits values, labels, initializers,
registry/provider formatting and host-made copies. IDs, schema compatibility,
version/order and lease accounting are semantic metadata, independent of locale.
Graph Get error/provenance and cross-run envelopes remain subsequent integration.

C07b preview boundary (2026-11-07): updates and reads reuse the existing fieldless
CompileError/EvaluationError message keys, with typed source IDs and safe static
fallbacks. Graph faults retain original producer/ConstError provenance and host
origin. Session/snapshot Debug exposes only metadata and counts, never cached
strings, config, resolver/provider formatting or input/output buffers. Request,
source freshness, cache identity, reset reasons and reuse counters are semantic
metadata independent of locale. No new catalogue, argument or UI-language change.

C05 replay boundary messages (2026-11-07): fieldless `ReplayError` uses
`core/replay-full` ("The replay journal is full; the operation was not started."),
`core/replay-unsupported` ("This plan or option is outside the supported replay
profile."), `core/replay-incomplete` ("The recording is incomplete or was
interrupted."), `core/replay-diverged` ("Replay diverged from its recorded command
or outcome.") and `core/replay-payload-unavailable` (safe archive refusal).
`RunError::ReplayFailed` uses `core/replay-failed` ("Run recording or replay could
not continue safely."). Wrapped run/ingress/effect errors retain their existing
keys. Archive reasons are Excluded/Missing/Limit without payload text or arguments.
References, versions, seed, clock, command order and typed outcomes are semantic
metadata. Canonical replay never compares translated presentation or calls a
provider/archive formatter; Debug omits captured values and host context.

C10 boundary messages (2026-11-07): fieldless `StreamError::Full`, `Overflow` and
`Sequence` expose `core/stream-full`, `core/stream-queue-overflow` and
`core/stream-sequence`. Their static fallbacks are, respectively, "The stream queue
is full; retry when the consumer frees space.", "The stream queue overflowed." and
"The stream source sequence must be nonzero and strictly increasing."
Hosts render these keys without arguments. Graph
overflow faults as `core/QueueOverflow`; private terminal envelopes keep safe
static text without producer/item data. Buffer bounds, source sequences, counters,
branch names and overflow tokens are semantic metadata and never translated.

C09b graph streams (2026-11-07): refused stream acquisitions/results reuse
fieldless `StreamError`/`EffectError` keys. Graph `StreamError::Limit` faults as
`core/LimitExceeded`, `Contract` as `core/InternalFault`, `NotRunning` as
`core/RunStopping`, and stale/foreign/already-claimed consumption as
`core/StaleHandle`, with its `core/stream-*` key and static fallback "The stream is
unavailable or exceeds its configured bounds." Producer faults retain their safe
original kernel envelope. There are no message arguments or producer/item data in
boundary errors/debug. Node and port identities, max-items, item index, ownership,
claim keys, demand order and end/None semantics never depend on locale. Collect
config errors use existing config-schema diagnostics. No catalogue or UI language
change is introduced.

C08b boundary messages (2026-11-07): `ResourceError::WrongOwner` uses
`core/resource-wrong-owner` ("The resource is owned by another graph scope.") and
`Forged` uses `core/resource-forged` ("The resource value has no authentic issuing
evidence."). Graph consumption maps stale handles to `core/StaleHandle`, owner
violations to `core/ResourceOwner`, and other invalid references to
`core/InvalidHandle`, retaining the corresponding `ResourceError` message key and
the static fallback "The resource reference is unavailable in this scope."
There are no message arguments, resource contents or host credentials. Refused
resource completions reuse `EffectError` envelopes. Handle kind/identity,
ownership, cancellation and close reasons remain semantic metadata, independent
of UI language. Close Handle uses ordinary intrinsic port/label presentation; no
catalogue, locale or saved schema change is introduced.

C11a/C09a boundary messages (2026-11-06): fieldless `EffectError` exposes
`core/effect-wrong-run`, `core/effect-stale`, `core/effect-not-running`,
`core/effect-contract` and `core/effect-limit`. Fieldless `StreamError` exposes
`core/stream-wrong-run`, `core/stream-stale`, `core/stream-claimed`,
`core/stream-contract`, `core/stream-limit` and `core/stream-not-running`.
Their static English `Display` strings are fallback text; hosts present the
namespaced `message_key()` with no arguments. Error/debug envelopes contain no
input/result/config/producer/host-context data. Rising Watch type errors reuse
`E-TYPE-MISMATCH`/`core/graph-type-mismatch`; malformed config uses the existing
config-schema diagnostic. Watch faults retain the original typed producer and safe
kernel envelope. Route IDs, operation/demand/resource identities, Watch mode, Eq,
event order and counters are semantic metadata and never translated. No UI locale,
catalogue or saved-format change is introduced.

Status: **normative planning proposal**, 2026-10-20; no implementation or localization tests have run. This document owns presentation text, catalogues and locale selection. [11](11-graph-language.md) owns graph semantics; [12](12-graph-data-structures.md) owns persisted and wire layouts. Their contracts must agree. Requirement FR-51, NFR-16 and ADR-17 apply from feasibility through release, including expansion SDK design before G2.

## 1. Scope and ownership

Every product-owned user-facing string MUST be translatable: menus, commands, shortcuts descriptions, palette names/categories/aliases, nodes/ports/types/enum variants, configuration options, inspectors, settings, tooltips/help, empty/loading states, dialogs, notifications, remediation and diagnostic text, and accessibility names/actions/announcements. Bundled and third-party declarative UI use the same contract. User-authored graph names, comments, variable/event/function names, string values, paths, service responses and external device names remain content; display them safely without treating them as catalogue keys. OS-owned dialog strings follow OS capabilities, with that limitation recorded in platform evidence.

The host owns catalogue loading/validation, message formatting, font fallback, direction and accessibility projection. The editor and human-readable CLI request rendered text. Model/compiler/runtime emit stable IDs, codes and redacted typed message arguments; they do not load fonts or depend on the desktop locale. Formatting MUST NOT run in an audio callback.

## 2. Message references and schema

One `TextRef` identifies static presentation text:

```json
{"key": "org.orange.math/node-add-label", "fallback": "Add"}
```

The key is `<package_id>/<message-id>`; message IDs use `[a-z][a-z0-9-]*`. Only a package may publish its namespace; `org.orange.host` owns host UI, core diagnostics and implicit port names. `fallback` is required safe source-language plain text, never markup or a secret. The source language is English (`en`). IDs and keys are never translated. A key has one declared argument schema in its owning catalogue descriptor. Labels normally have none.

Presentation fields in [12 §4–5](12-graph-data-structures.md) use `TextRef`: `label`, optional `description`, port `label`/`description`, type/member `label`, `editor.category`, every `editor.search_aliases` entry and declarative UI text/accessibility fields. `editor.category_id` is the required stable namespaced grouping token when a category is supplied; `editor.category` is its localized caption. Config-schema and enum-option captions are presentation metadata keyed by stable property path/option value; translating a caption never rewrites that value. `control`, provider IDs, grouping IDs and action IDs stay machine tokens. A missing optional port label is derived visibly from its `port_id`; bundled baseline definitions must supply human labels. Legacy `label_key`/`doc_key` prose is replaced by this single shape before SDK freeze.

Dynamic messages use `MessageRef = {origin, key, args, fallback}`. `origin` identifies an exact package version/content digest or immutable host build, using [12 §6.2](12-graph-data-structures.md). Static TextRef origin is inherited from its enclosing definition/catalogue. Standalone diagnostic/trace records carry origin inline; compact streams may intern it in a retained header, but exported standalone records must expand it. Forwarded errors retain their original issuer. If origin bytes disappear, the safe fallback remains usable without downloading or choosing a newer package.

`args` is a bounded map of declared named values with canonical tagged wire encodings in 12 §6.2; templates may reorder them. `fallback` is a complete pre-rendered safe English message, so loss of a package cannot expose unresolved template syntax. Do not assemble sentences by concatenating translated fragments. Redaction happens **before** interpolation, fallback construction, logging or IPC, including translated error and recovery paths.

Declaration validation uses `core/graph-declaration-type` with an empty argument map and fallback "Declaration type is invalid or unavailable." The optional diagnostic subject `event` is a typed EventDeclId, alongside the existing `variable` VariableId; subject IDs are structured navigation data, never interpolated names or payloads. Admission and cancellation use `core/graph-analysis-limit` / "Graph analysis exceeds the host's admission limits." and `core/graph-analysis-cancelled` / "Graph analysis was cancelled." Both carry core origin, empty arguments and a global subject. Their stable codes are `E-DECLARATION-TYPE`, `E-ANALYSIS-LIMIT` and `E-ANALYSIS-CANCELLED`; all are errors independent of locale.

Bundled scalar failures (N1a) expose namespaced message keys and safe English Display fallbacks with no input/config/digest interpolation. Kernel keys are `core/kernel-numeric-overflow` ("Numeric arithmetic overflowed."), `core/kernel-divide-by-zero` ("Division requires a nonzero divisor."), `core/kernel-non-finite` ("Finite arithmetic requires finite inputs and results.") and `core/kernel-input-contract` ("Kernel inputs violate the resolved contract."). They map to existing runtime codes NumericOverflow, DivideByZero, NonFinite and InternalFault. Binding keys are `core/kernel-unavailable`, `core/kernel-unsupported-type`, `core/kernel-invalid-config`; construction keys are `core/kernel-build-identity-invalid` and `core/kernel-metadata-invalid`. Their typed errors provide safe complete fallbacks. The eventual host/runtime associates immutable origin, node/activation and an empty argument map when constructing a full MessageRef; the scalar crate does not manufacture runtime provenance or a completed error wire envelope.

Core conversion execution (N1b) returns safe typed failures with keys `core/conversion-out-of-range` (fallback "The value is outside the target conversion range.", code `core/OutOfRange`) and `core/conversion-input-contract` (fallback "Converter input violates the resolved contract.", code `core/InternalFault`). Neither carries/interpolates source values, bit patterns, policy text or loss payloads. Binding failures reuse the existing core kernel binding keys. The eventual runtime adds immutable origin/node/activation and an empty argument map; this scalar API supplies no forged provenance. The `lost` JSON flag is semantic data, so its key/value and conversion behavior never depend on UI language.

## 3. Catalogues and package contract

Selected planning format: UTF-8 Fluent FTL with named arguments, selectors and CLDR plural categories. Use complete messages and a required default selector branch. Exact Rust formatter/parser versions, Unicode/CLDR data, features, licenses and supported Fluent subset are finalized by WU-01/03 and tested before SDK freeze; this proposal claims no compatible library or compiled API. Custom executable formatter functions and cross-package message references are disallowed in v1. Host-provided number/date functions use recorded locale data and bounded execution.

The optional package manifest `localization` field is required when a package contributes presentation text. Its wire shape is:

```json
{
  "format_version": 1,
  "format": "fluent",
  "default_locale": "en",
  "catalogs": {"en": ["locales/en.ftl"], "es": ["locales/es.ftl"]},
  "arguments": {"selection-count": {"count": "u64"}}
}
```

Map keys in `arguments` are local message IDs; messages without arguments omit that entry. The complete v1 type vocabulary is `string`, `i64`, `u64`, `f64`, `timestamp`, `duration`, encoded as in 12 §6.2. `default_locale` MUST be `en`; catalogue locale tags are matched case-insensitively and duplicate equivalent tags are rejected. Since K01 (2026-11-06) `orange-packages` enforces this field shape when it decodes a manifest: tags must have the BCP 47 shape (a 2–3 or 5–8 letter language subtag, then 1–8 character alphanumeric subtags), each catalogue list is a non-empty set of distinct `.ftl` archive paths, a file serves one locale, message IDs are `[a-z][a-z0-9-]*`, argument names `[a-z][a-z0-9_]*`, and counts are bounded (256 locales, 64 files per locale, 4,096 messages, 32 arguments each; unmeasured starting points). Catalogue content validation below remains host work. Catalogue files use local IDs; the host adds the owning package prefix. Example source resource:

```ftl
node-add-label = Add
selection-count =
    { $count ->
        [one] One node selected
       *[other] { $count } nodes selected
    }
```

The host rejects malformed source catalogues, duplicate keys, namespace violations, invalid argument references/types, reference cycles and over-budget resources during package staging with `package/InvalidLocalization`. Optional translated entries that are missing or malformed fall back per §4 and produce bounded `host/LocalizationFallback` diagnostics. Declared catalogue paths missing from an archive are structural package errors. Quotas cover bytes, nesting/reference depth and formatted output length, finalized in 03a with hostile-input vectors. Catalogues are data; reading them never executes package components. Translation data cannot add permissions, actions or widgets.

Catalogue bytes participate in the package integrity digest and immutable generation. Install/update/removal publishes definitions and their catalogues atomically. A translation fix shipped as a package update still creates a new exact digest; existing graph/provider locks never silently move. Running diagnostics resolve against their pinned origin catalogue, and the editor can display multiple package versions by origin. If origin bytes are unavailable after import/removal, use safe fallback text or stable IDs. There is no separate downloadable language-pack override contract in 1.0.

## 4. Locale choice, fallback and switching

Locale tags use BCP 47. Desktop precedence: explicit saved per-user language setting, OS preferred languages, then `en`. CLI precedence: explicit `--locale`, OS preference, then `en`; an invalid explicit tag is a CLI usage error. Graphs and workspace exports do not store the user's UI language. An unsupported well-formed locale falls back visibly rather than failing graph execution.

For each requested locale in preference order, try its exact catalogue then successively less-specific tags using RFC 4647 lookup; then the owning package's `en`, then the reference's safe fallback, finally its stable key/ID for damaged or unknown metadata. Do not guess a different regional dialect. Message resolution and argument errors use the same fallback path and bounded diagnostic, never a UI panic. A shipped locale requires full baseline host/bundled coverage; third-party coverage is reported separately and may fall back to English.

Changing language while open updates visible text, palette search and accessibility together in a view transaction. Keep document IDs, graph bytes/revision, undo history, selection, viewport and running plans unchanged. Retain an active edit buffer, caret and IME composition; defer replacing its displayed value until composition/commit ends. Keyboard commands bind action IDs; translated mnemonics are checked for collisions. Rebuild only affected text metrics/search indexes off the main thread as needed; never rebuild the graph model. Search includes translated labels/aliases, source-language labels and stable IDs; locale-dependent search order has no execution meaning.

## 5. Locale and graph data

UI text language, display formatting, time zone and **graph computation** are separate settings. Persisted literals/codecs, IDs, JSON keys, protocol tokens, error codes, sort/equality/hash operations and default graph parse/format nodes stay invariant under [03](03-data-and-execution.md). Derived graph `display` uses stable type/member IDs, not translated labels or mutable display names. Human inspector previews may show localized numbers/dates alongside an exact canonical value. Invariant literal editors visibly use invariant syntax; any localized entry mode must be explicit and commit the same canonical typed value, without ambiguous separator guessing.

Locale-sensitive graph parse/format/collation is allowed only through explicitly configured versioned nodes/providers with locale, time zone where applicable, and locale-data version recorded in graph/provider configuration. It never reads the editor's language or machine defaults. A future locale-data change must obey the same explicit package rebind/migration rules. A locale toggle alone performs zero validation, compilation, effects or preview evaluation.

Structured CLI/error/trace output retains codes, subject IDs, `message_key`, typed redacted `message_args` and safe fallback. Localized human text is an optional presentation projection; golden replay compares canonical structured records, excluding rendered text. Compile diagnostics and suggested-fix labels use this envelope too; fix commands bind stable action IDs. An external service's raw message is safe content inside a translated host wrapper, not trusted translation syntax.

## 6. Layout, fonts and accessibility

Support text expansion, CJK shaping/font fallback and bidirectional text. RTL shell controls follow locale direction; **graph port roles, input/output sides, saved coordinates and wire direction do not mirror**. Use bidi isolation for interpolated IDs, paths and user content. Text shaping, selection, caret movement and screen-reader order need native platform evidence. Do not infer them from UTF-8 storage or rendering a sample string. Bundled fonts require license review; downloads are not required to read a graph offline.

Labels may wrap or ellipsize with accessible full text. Required controls, errors and actions remain reachable at 200% scale with expanded text. Locale changes relayout only affected view elements; measured W4 budgets apply. English plus expansion/RTL pseudo locales and CJK/Arabic mixed-text fixtures are mandatory validation inputs; pseudo locales are not advertised as shipped translations. Additional release languages are decided in OPEN-13, without deferring translatability.

## 7. Acceptance fixtures (all NOT RUN)

| ID | Future evidence / owner / gate |
|---|---|
| L-01 | Static extraction/schema audit covers 100% of product-owned UI text and catalogue keys, including accessible text, help, config options and bundled packages; explicit content/OS exceptions reviewed. Editor + SDK leads / G2 schema, G3 coverage. |
| L-02 | Locale precedence, exact/parent/source/fallback lookup, unsupported/invalid tag, missing key and argument mismatch; no crash and bounded diagnostics. Host lead / WU-03, G2. |
| L-03 | Singular/plural/select messages with 0/1/2 and languages with additional plural categories; reordered named arguments and malicious/oversized catalogues; secrets absent from fallback and rendered text. SDK + security leads / G2, G4. |
| L-04 | Hot install/update/remove with two pinned versions and competing namespaces; missing package keeps IDs/ports/fallback readable; translation change never silently rebinds a provider. SDK lead / G2. |
| L-05 | Switch locale during text edit, IME composition, async completion and run inspection; focus/caret/history retained, no graph revision or execution change. Integrate the G-39 core invariants with real host catalogues and rendering. Editor + language leads / G1 prototype, G3 qualification. |
| L-06 | 40% expanded pseudo text, CJK and mixed Arabic/Latin at 100/150/200% DPI; RTL traversal, selected text, bidi isolation and screen readers on each declared OS. Required controls accessible, W4/NFR-03 met. Editor/platform leads / P-02, G3. |
| L-07 | Every advertised release locale has complete host/bundled key coverage and no unexpected fallback in baseline workflows; translator context and human review recorded. Product/editor leads / G5. |
| L-08 | Headless structured diagnostics retain code/key/typed args across locales, human output translates independently; invariant numeric literals round-trip and explicit locale-node configuration replays. Runtime/CLI leads / G2, G5. |

## 8. Sources and limits

Primary sources read 2026-10-20: [BCP 47 language tags, RFC 5646](https://www.rfc-editor.org/rfc/rfc5646), [lookup, RFC 4647 §3.4](https://www.rfc-editor.org/rfc/rfc4647#section-3.4), [Fluent variables](https://projectfluent.org/fluent/guide/variables.html) and [selectors](https://projectfluent.org/fluent/guide/selectors.html), [Unicode bidi algorithm](https://www.unicode.org/reports/tr9/) and [LDML numbers/plurals](https://www.unicode.org/reports/tr35/tr35-numbers.html). These support language tags, interpolation/plural design and text-direction requirements. They establish neither UI toolkit capability nor Rust dependency compatibility. Exact parser/data pins and platform demonstrations remain future evidence; all budgets and fixtures above are proposed.

E1a compiler messages: CompileError exposes core/compile-* message keys with typed node/port IDs and static safe fallbacks. ConstantFault carries a bounded static namespaced code/key/fallback from trusted code, plus original producer NodeId. W-CONST-ERROR uses the shared Diagnostic envelope with host origin, core/graph-const-error and no raw value/config arguments. Compiler callback envelopes are checked for identifier/message bounds and control characters. Translation stays host-owned; neither locale nor translated text participates in lowering, provider selection or fault order.

E2b evaluation entry/access errors expose core/evaluate-stale-document, core/evaluate-stale-registry, core/evaluate-resource-limit and core/evaluate-cancelled message keys and static safe fallbacks. They contain no graph values, configuration text or credentials. A graph's ConstantFault remains the original producer's namespaced envelope; EvaluatedExport identifies the requested output port separately. Presentation remains host-owned and does not affect snapshot selection or result order.

Headless control messages (2026-10-24): RunError exposes `core/run-invalid-state`, `core/run-resource-limit`, `core/run-identity-exhausted`, `core/run-clock-went-backwards`, `core/run-interrupted`, or the underlying typed DataError key. Resumable pulls add `core/data-pull-in-progress`. Graph faults use `core/uninitialized-variable`, `core/count-out-of-range`, `core/wait-out-of-range`, `core/loop-budget-exceeded` and `core/cleanup-unsafe-node`, with their original producer ID and safe static fallbacks. RunEvent carries consumer/activation/outcome identity separately; status and variable-write inspection expose typed IDs. Default Run/ControlPlan formatting never renders values, names, config, registry contents or callbacks. Explicit variable inspection remains host-owned. These runtime enums and keys do not localize graph semantics or qualify an editor catalogue/UI.

Aggregate run budget (2026-11-05): `RunError::AggregateLimit` exposes `core/run-aggregate-limit` and `DataError::AggregateLimit` exposes `core/data-aggregate-limit`, with static safe fallbacks and no value arguments.

Package messages and redaction (2026-11-05; `orange-packages`): `ManifestError` and `TransactionError` expose `message_key()` and are presented through it, never `Display`. Rejected text (format, package ID, version, publisher, slot, provider ID) is held as `error::Rejected`, which formats as its length only; the host calls `reveal` when it chooses to show it. Decode errors keep the failure kind and line/column only, not serde's message. `PackageManifest`, `ProviderSlot`, `PackageContents`, `PackageRecord` and `PackageStore` have manual bounded `Debug` summaries (counts, versions, state, message keys); opaque `execution`/`permissions`/`localization`/`panels` data is never formatted, and the accessors are the authorized read path. Manifest keys: `packages/manifest-too-large`, `-decode`, `-format`, `-format-version`, `-package-id`, `-version`, `-publisher`, `-publisher-namespace`, `-host-api-range`, `-digest-algorithm`, `-digest`, `-self-dependency`, `-duplicate-dependency`, `-node-id`, `-node-namespace`, `-duplicate-node`, `-slot`, `-provider-id`, `-duplicate-provider`. Transaction keys: `packages/transaction-already-active`, `-not-installed`, `-wrong-state`, `-update-target`, `-revoked`, `-digest-conflict`, `-host-api`, `-missing-dependency`, `-undeclared-definition`, `-missing-definition`, `-definition-changed`, `-unknown-slot-definition`, `-undeclared-specialization`, `-unsupported-slot`, `-depended-on`, `-nothing-to-roll-back`, `-rollback-window-exceeded`, `-recovery-mismatch`, `-recovery-default-unavailable`, `-registry`, `-journal`. Limitations: `RegistryError` and `JournalError` each map to the one generic `-registry` or `-journal` key, and `JournalError.reason` is a host-supplied string that `TransactionError::Journal` still shows in `Display`, so hosts must keep it free of secrets. These keys are not yet in a catalogue.

Import and identifier messages (2026-10-28): `CodecError::message_key` and `IdError::message_key` expose stable keys (`core/import-json`, `core/import-unsupported-format`, `core/import-unsupported-version`, `core/import-duplicate-id`, `core/import-duplicate-key`, `core/import-duplicate-lock`, `core/import-limit-exceeded`, `core/import-nesting-limit`, and `core/id-*` per identifier rule). Text that failed a check is arbitrary input, so errors hold it as `Rejected`, whose `Display` and `Debug` show only its length; a host that decides to show it calls `Rejected::reveal`. JSON failures keep their kind, line and column, and the parser's own message only as `Rejected` detail, because it can quote document text. Validated identifiers, such as a duplicated node ID, are still named.

Definition file messages (2026-11-03): `DefinitionError::message_key` exposes `core/definition-json`, `core/definition-limit-exceeded`, `core/definition-duplicate-key`, `core/definition-nesting-limit` and `core/definition-too-many-ports`, with the same `Rejected` and JSON-position rules as graph import.

Literal decoding messages (2026-10-28): `LiteralError::message_key` exposes `core/literal-*` keys, one per variant (for example `core/literal-unknown-field` and `core/literal-unknown-variant`); an invalid type expression reuses its `core/id-*` key. Unknown record field and enum variant names come from the rejected literal, so they are `Rejected` text: Display and Debug show only the length, and the host reads the text through `reveal`. Type names and declared field names come from the registry and stay in the fallback text.

Control review corrections (2026-10-24): demand-driven host input publication, shared constant-budget admission and separate shutdown reservation reuse existing typed ResourceLimit, MissingInput and cancellation diagnostics. No new user message keys or arguments are introduced; host-owned presentation and redaction remain unchanged.

Diagnostic declaration reuse (2026-10-24): one internal declaration now generates DiagnosticCode variants/serde names, stable codes, translation keys, severity and safe fallback text. Existing enum ordering, wire identifiers, fallbacks, typed/redacted arguments and host-owned presentation remain unchanged; no catalogue/schema migration is required.

Structured Fork execution (2026-10-25) adds a typed Forked trace outcome and metadata-only owner counts, reusing the existing run/data limit and cancellation message keys. Branch order, joins, error provenance and provider identities do not depend on locale or display labels. No new catalogue or persisted graph fields are introduced.

Background analysis worker (2026-10-28): `AnalysisWorkerError` exposes `core/analysis-worker-closed` ("the analysis worker has shut down") after the host requested shutdown and `core/analysis-worker-panicked` ("the analysis worker stopped unexpectedly") when its thread stopped on its own. Neither carries arguments, graph content or diagnostics; presentation stays host-owned.

Standalone data interruption (2026-10-31): `DataError::Interrupted` exposes `core/data-interrupted` with a static safe fallback explaining that a provider callback panicked and the activation must be replaced. It carries no message arguments or graph values. It is an infrastructure failure, not a graph `core/error@1` value captured by Try, and presentation remains host-owned.

JSON budget resources (2026-11-01 UTC): `JsonValues`, `JsonKeyBytes` and `JsonStringBytes` extend the typed `ImportResource` vocabulary carried by `CodecError::ImportLimitExceeded`. The key remains `core/import-limit-exceeded`; resource kind and numeric maximum/observed count are safe metadata, with no rejected document value or object key included. Host presentation translates the resource kind rather than interpreting payload strings as messages.

Run-scope event faults (2026-11-02): core/LimitExceeded uses core/event-limit-exceeded (safe fallback "The event exceeds a run delivery limit."); core/RunStopping uses core/event-run-stopping ("The run is stopping and cannot accept an event."); core/RunawayEventChain uses core/event-cause-depth ("The event causal depth limit was reached."). They carry no payload or interpolation arguments. Trace provenance retains the original emitter or upstream payload-fault node; source scope/IDs, numeric counters, queue ordering and virtual time are semantic metadata and never translated. Host-owned presentation localizes messages; EventMetadata/RunEventStats contain no graph values or credentials.

Bounded host ingress errors (C04a contract, 2026-11-06; implementation evidence pending) are fieldless, expose `message_key()`, and retain no event payload, rejected content or host text. `WrongRun` maps to `core/ingress-wrong-run` ("The event source belongs to another run."); `NotRunning` reuses `core/event-run-stopping`; `UnknownEvent` maps to `core/ingress-unknown-event` ("The event is not present in the checked run plan."); `SourceLimit` to `core/ingress-source-limit` ("The run has reached its active host source limit."); `IdentityExhausted` to `core/ingress-identity-exhausted` ("The host source identity space is exhausted."); `RevokedSource` to `core/ingress-source-revoked` ("The host event source is revoked or unknown."); `SequenceNotIncreasing` to `core/ingress-sequence-invalid` ("The source sequence must be nonzero and strictly increasing."); and `InvalidPayload` to `core/ingress-payload-invalid` ("The event payload does not match its checked type."). `DeliveryLimit` reuses `core/event-limit-exceeded`. `UnknownEntry` (C04b `Run::trigger`) maps to `core/ingress-unknown-entry` ("The node is not a manual entry in the checked run plan."). The C04b validation diagnostic `E-MANUAL-PAYLOAD` uses `core/graph-manual-payload` ("A manual entry payload must name a concrete registered type, and no other field is allowed."), `E-VARIABLE-CHANGED` uses `core/graph-variable-changed` ("A variable-changed entry needs a run variable with an initial value and a type with eq."), and `E-TIMER-CONFIG` uses `core/graph-timer-config` ("A timer entry needs a positive interval or a non-negative delay in nanoseconds, and only its documented fields."). A `core/now` read past the duration range faults with `core/OutOfRange` and key `core/clock-out-of-range` ("The run clock is beyond the duration range."). Fieldless `ResourceError` values (C08a) map to `core/resource-wrong-run`, `core/resource-not-running`, `core/resource-unknown-kind`, `core/resource-limit`, `core/resource-identity-exhausted`, `core/resource-stale` and `core/resource-wrong-kind`, with their English `Display` text as fallback; they carry no resource content. All fallbacks are static and have no arguments. These keys are host-presented; no catalogue or saved schema change is implied. EventMetadata contains typed event/source/cause identity and timestamps but never payload or credentials; source IDs, sequences, counters, queue order and time remain semantic metadata, not translated strings.
