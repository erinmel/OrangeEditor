# WU-00/00a review — 2026-10-21

The fixture baseline is reviewed and structurally checked. It is **not a passing behavioral G-suite**: OrangeEditor has a document model, but no graph validator, compiler or interpreter. G0/00a acceptance remains open where the rule is represented only by prose or an unspecified harness. This report replaces the incomplete session checklist; [README](README.md) defines fixture conventions and open-question owners.

## Findings and corrections

| Finding | Resolution and verification boundary |
|---|---|
| Structural loading accepted stale package snapshots | Added definition → type substitution → snapshot comparison for every test-package node in root/function bodies. It found the missing `numbers.fail_at` input in G-48; corrected its snapshot and option type lock. G-29's intentional drift is checked on its exact node, with no file-wide exemption |
| Four G-77 traces expected events after the script's final time | Added explicit clock advances for iteration, next-item and collect cases. The script check now rejects out-of-order timestamps or timestamps beyond the final virtual clock |
| Replay eligibility was used without requesting a replay | G-32/G-59 now request replay of the recorded run, compare complete canonical traces and assert zero real remote sends during replay. G-59 separately expects one real send during recording. This defines an acceptance check; it does not execute replay |
| Coverage could silently drift between fixture and rule inventory | Fixture rule arrays are the sorted inverse of `rules[].cases`; the test checks both directions and rejects duplicate rule IDs. An absent rule inventory now fails instead of becoming an empty inventory |
| Prose and empty diagnostic lists could look like executable coverage | All 32 prose-only cases now name `deferred.owner` and `deferred.reason`; the structural test requires both. Runtime traces with supplementary prose still need that supplementary assertion implemented by their owner |
| G-78/G-80 placeholders had no precise blocker | Added error-injection/catalogue harness questions 23/24 for G-78; G-80's package-function case is blocked by question 10's package definition format. Removed the nonexistent 00g owner from the debounce question |
| Numeric default preference changed during review | ADR-20, FR-45, 11 §5.5 and six G-49 vectors now specify Float Single/f32 and Double/f64 alongside integer math. Double is the untyped default; explicit/resolved types win. Large integers parse directly from text |
| Duplicate descriptions disagreed on subscription diagnostics | G-81 descriptions now agree with §14 and their fixture: Entry/Watch use `E-ENTRY-IN-FUNCTION`, subscriptions use `E-SUBSCRIPTION-IN-FUNCTION` |

The remaining second-pass resume items were already present on inspection: G-43 warning edge IDs, generic stream inputs and default/optional cases; G-50 wrapping/saturating/remainder/shift graphs; G-52 consumer-end diagnostic subject; G-59's send definition; G-63's multi-run reset; and G-77's failed-item close case. These were retained. Checks also found no wrong consumer-end diagnostic subjects in the remaining review ranges, no absent trace node references there and no stream-consuming intrinsic missing its item type argument. These are data audits, not semantic execution.

## Code quality and reading flow

The existing model separates persistence, edit transactions, indexes, signatures and runtime values. The review found no reason to merge those responsibilities or add semantic validation to JSON loading. Public transaction state is protected by commands; undo uses content identities; derived indexes live outside persisted documents. Existing property tests exercise rollback/history, index rebuilding, paste remapping and literal round trips. This is evidence for those model operations, not for future execution or NFR timing.

Added [the model reading guide](../app/crates/orange-model/README.md): load → commands → journal/patches → committed change set/inverse → derived index → save, plus the signature/literal/paste side paths. Each conformance test has one purpose: structure, package snapshots, inverse mapping, script/replay shape or documentation examples. Test failures identify fixture, case and node. They reuse the real model projection instead of implementing a second signature algorithm.

Standalone fixture documents deliberately repeat snapshots so missing-provider cases are portable. Shared package definitions and the projection check guard that duplication. Further abstraction into a generated fixture DSL would make individual golden graphs harder to inspect; no such generator or second executor was added to product code.

## Remaining coverage limits

The inventory has 272 rule entries mapped to 83 fixture files containing 356 named cases. This is file-level traceability, not 272 proven semantic rules.

- `R-4.5-declarative-port-templates` (G-45) has only a prose assertion: question 10, owners 00c/WU-03. It is the sole mapped fixture file without diagnostics, a trace, registry results or structured editor document expectations.
- A strict diagnostic/trace/registry audit also lists `R-4.5-variadic-stable-keys` (G-31) and `R-15-layout-edits-inert` (G-36). Those files instead contain meaningful structured edit/document/change-set expectations; the editor/compiler still must execute them.
- `R-5.5-no-specialization` maps to G-49 but has no actual `E-NO-SPECIALIZATION` diagnostic vector. The package specialization-list wire contract is missing (question 10). Its prose expectation is explicitly deferred.
- The diagnostic-catalog audit also found no golden `E-DOC-DUPLICATE-ID` or `E-SCOPE-EDGE` expectation. Existing model tests reject duplicate IDs and cross-body transaction edges, but those tests do not establish a host diagnostic mapping. The fixture loader currently requires codec-loadable documents; 00d must define negative-load vectors and the precedence of scope errors before claiming full diagnostic conformance (question 25).
- Expected traces are not all `exact`. A `subsequence` checks only the listed observations and cannot exclude extra effects. Replay's complete-trace comparison helps only after a runner exists. Future runners must implement all expectation fields, including supplemental behavior/invariants, or report unsupported cases; silently ignoring them must not count as a pass.

The 32 prose-only cases are grouped below; their individual names, rules and reasons remain in the JSON files:

| Fixture area | Owner and prerequisite |
|---|---|
| G-45, G-46, G-48, G-49, G-51 | 00c/WU-03: type-resolution errors, constraints, templates, specialization, converter metadata/override wire formats (questions 9/10/12) |
| G-54 | 00e: forced offload and concurrent-write hooks (19) |
| G-59 | 00e/00f: entropy metadata/index inspection (17) |
| G-63 | 00f: inspection/checkpoint hooks; a separate multi-run graph specifies reset |
| G-66 | 00f: wall-time overlap integration probe (22) |
| G-77 | 00d: required-config failure diagnostic/shape (6) |
| G-78 | 00e/00f: synthetic nested errors and runtime-code introspection (23/24) |
| G-79 | WU-04: actual bundled kernel inventory plus property/fuzz qualification |
| G-80 | WU-03: graph-implemented package definition/update contract (10) |
| G-82, G-83 | 00e/WU-05: debounce, preview source/seed/invalidation and editor hooks (20) |

Declarations-only or registry cases may legitimately have no nodes. An empty graph is not itself a defect; a prose assertion without executable inputs/observations is the limitation. No completed diagnostic or runtime gate is inferred from successful parsing.

## Product decisions and comparison sources

Reviewed official documentation on 2026-10-21; no competitor application was run. These systems have different execution models, so their choices inform OrangeEditor's design rather than prove its behavior.

| System | Verified behavior and limit |
|---|---|
| [Node-RED subflows](https://nodered.org/docs/user-guide/editor/workspace/subflows) | Prohibits direct and indirect self-containing subflows. This is distinct from messages circulating in a flow |
| [Blender 5.0 node groups](https://docs.blender.org/manual/id/5.0/interface/controls/nodes/groups.html) | Prohibits direct and indirect recursive groups. The indexed official localized page supplied the text; the English page fetch was unavailable |
| [Unity Visual Scripting 1.9.12 Recursion API](https://docs.unity3d.com/Packages/com.unity.visualscripting@1.9/api/Unity.VisualScripting.Recursion.html) | Documents `safeMode`, `defaultMaxDepth` and guarded entry APIs. This alone does not establish which recursive user graph patterns execute or their limits; no numeric depth default is assumed |
| [Warudo camera tutorial](https://docs.warudo.app/docs/blueprints/tutorials/camera) | Rejects loop wiring to prevent freezes; demonstrates repetition using function triggers. This is not proof of arbitrary recursive return-value functions |
| [Node-RED messages](https://nodered.org/docs/user-guide/messages), [ECMAScript Number](https://tc39.es/ecma262/multipage/ecmascript-data-types-and-values.html#sec-ecmascript-language-types-number-type) | Ordinary numeric messages use JavaScript Number, based on binary64. This does not make every payload numeric or establish compatibility of arbitrary-precision extension values |
| [Blender 5.0 geometry nodes](https://docs.blender.org/manual/en/5.0/modeling/geometry_nodes/index.html) | Includes Integer Math and Float To Integer alongside Math. Float and integer workflows coexist |
| [Unreal Engine 5 Float precision](https://dev.epicgames.com/documentation/en-us/unreal-engine/large-world-coordinates-project-conversion-guidelines-in-unreal-engine-5) | Presents Float with single/double precision subtypes and documents conversions. OrangeEditor adopts the family/precision presentation; its existing visible-converter contract remains authoritative |
| [Rust f64](https://doc.rust-lang.org/stable/core/primitive.f64.html), [f128 feature status](https://doc.rust-lang.org/stable/unstable-book/language-features/f128.html) | f64 has 53 binary significant digits; wider binary floating precision does not make arbitrary integers or decimal fractions exact. A local Rust 1.99.1 probe confirms f128 is unstable |

The user confirmed retaining v1's synchronous recursion rejection and selected both Float precisions and integer math. The recommendation is Double for new untyped numeric input and direct integer parsing for exact inputs; G-49 includes `9007199254740993` to exclude a lossy float intermediate. Arbitrary-precision numbers remain a separate package type/codec/resource-policy proposal. No dependency was installed. Package-defined control intrinsics retain the current v1 boundary; the user received the explanation, but no separate confirmation of OPEN-12 is claimed.

## Validation evidence

Exact environment, commands and results are recorded in [10](../docs/10-tooling-and-sources.md#wu-0000a-fixture-review-2026-10-21). Local model tests and fixture structural checks pass. The test package contains **test definition data**, not installed product providers or implemented kernels; its definitions include deterministic, seeded and external behaviors. Behavioral G-suite, NFR-15/W9, Bevy, component isolation, platform UI and runtime replay remain NOT RUN. Remote CI is separate evidence; this report does not predeclare its result.

Next core work is 00c's missing contracts and type/converter design, followed by implementation under its prerequisite checks. Resolve the listed contract gaps and rerun affected vectors before claiming full 00a acceptance or freezing WU-03. WU-05 implements the numeric authoring UI after its own dependencies; the editor labels and precision selector do not exist yet.
