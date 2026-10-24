# Internationalization and translatable text

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

Map keys in `arguments` are local message IDs; messages without arguments omit that entry. The complete v1 type vocabulary is `string`, `i64`, `u64`, `f64`, `timestamp`, `duration`, encoded as in 12 §6.2. `default_locale` MUST be `en`; catalogue locale tags are matched case-insensitively and duplicate equivalent tags are rejected. Catalogue files use local IDs; the host adds the owning package prefix. Example source resource:

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
