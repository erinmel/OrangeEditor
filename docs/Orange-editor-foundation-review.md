# OrangeEditor Foundation Review

Oct 23, 2026 · @erin

## Verdict

The rules are strong; the code that carries them is not yet. The graph language (docs/11) already rejects the failure modes mature editors are known for. Unreal re-runs a pure node for every consumer, Blender converts socket types implicitly and loses data, and Houdini picks asset versions by precedence rules. OrangeEditor memoizes pure values, makes every conversion a visible node and pins exact providers.

The weakness is structural. Rules live in prose and in scattered match tables, the domain seam that ADR-21 promises downstream crates has no code, and the public Rust types cannot grow without breaking those crates.

| Dimension | Spec (docs/11–12) | orange-model today | Verdict |
| --- | --- | --- | --- |
| Node rules: kinds, ports, cardinality, connection matrix | Complete and strict | Structure, kind matrix, fan-in/out and inputs checked; regions, availability, cycles and write conflicts not checked | Strong rules, about a third enforced |
| Propagation: data pull, control tokens, queued events | Complete (ADR-13) | Edit-time type propagation only; no compiler or runtime | Sound design, not built |
| Data types | Nominal, versioned, no `any`, no implicit conversion | Types, constraints, generics and converters implemented | Strong |
| Nature of data | Value, state, event, stream, handle and error kept apart | `Value` covers all of them in 16 bytes | Strong; no tuples, closures or units |
| Expand through packages | 11 §16 surface | `RegistrySnapshot` with a subset of `NodeDefinition` | Partial: no port templates, event sources or labels |
| Expand through crates (domains) | ADR-21 `domain` seam | No `domain` field, no domain interface | Weak: the main seam is missing |
| Override implementations | Slots, explicit rebind, no last-installed-wins | Only converters have provider alternatives and defaults | Weak in code |
| Evolve the public API | ADR-21: a semantically versioned product surface | Exhaustive public enums, all-public struct fields | Weak: every addition breaks downstream code |
| Edit-time speed | NFR-15: ≤2 ms per edit on 10,000 nodes | Whole-document `validate`: 189 ms on 10,000 nodes | Weak: about 100× over budget |

Fix the structure before adding more passes: make the rules data, add the domain seam and close the semver gaps. The Recommendations section orders the work.

## Code findings

One real bug blocks a core feature: a Get or Set Variable node with any edge never validates, and the report gives no reason. fmt, clippy, all tests and rustdoc pass on branch `refactor/typed-port-descriptor`; the bugs below were confirmed with temporary probe tests that were then deleted.

| Finding | Evidence | Effect |
| --- | --- | --- |
| Edges on Get/Set never validate | `lookup_port` reads a static intrinsic copy that drops variable-typed ports (`validation/ports.rs:25`, `:105`) | Variables cannot be used in a valid graph; a string-to-i64 edge gets no type error |
| Incomplete with no diagnostic | Same path marks the node incomplete silently | Breaks “understandable errors” |
| A self data cycle reports valid | No cycle pass yet; `is_valid()` returns true | The name claims more than the check proves |
| `core/call` and `core/on-event` report E-PROVIDER-UNAVAILABLE | Neither has a contract yet | Reusable graphs and event triggers cannot validate; the message misleads |
| Intrinsic knowledge in six places | ID list, catalog definitions and reference table, static port copy and break/continue special case, function kind table, implicit ports by kind, reserved port list | Rules drift; adding an intrinsic means editing six files |
| Connection pass ignores fresh resolution | `validate_connections` uses the registry only for `origin` and checks stored signatures | Two answers for one node's ports |
| Module cycles | catalog ↔ validation (`HostOrigin`), inference ↔ validation (`implicit_port`), catalog ↔ conversion | Blocks the planned split into compiler and runtime crates |
| Invalid states representable | `PortDescriptor` stores `kind` beside optional `ty` and `source`; kind/suspends checked per node, not at registration | Every pass re-checks what the type could forbid |
| Hot-path allocation | `definition()` clones the ID string per lookup; `contract()` always clones all ports, four or more times per node | Linear but slow: 18 ms at 1,000 nodes, 189 ms at 10,000, 803 ms at 40,000 (release, one run, this machine) |
| Docs drift | docs/12 §4 note still says `NodeDefinition` holds a `NodeContract`; README calls Get/Set projection “next” | Readers trust stale contracts |

## Node rules in mature editors

Mature editors trade strictness for convenience, and each convenience has a documented cost. OrangeEditor's spec takes the strict side of almost every trade below, which is the right call for a foundation others build on.

| Editor | Execution and propagation | Typing and generics | Conversion | Versions and overrides | Lesson for OrangeEditor |
| --- | --- | --- | --- | --- | --- |
| [Unreal Blueprints](https://dev.epicgames.com/documentation/en-us/unreal-engine/functions-in-unreal-engine) | Exec wires drive impure nodes; a pure function “will be called one time for each node it is connected to” | [Wildcard pins](https://dev.epicgames.com/documentation/unreal-engine/API/Editor/BlueprintGraph/UK2Node_MacroInstance/InferWildcards) are inferred when a pin is connected | Explicit conversion nodes, some auto-inserted | [Core Redirects](https://dev.epicgames.com/documentation/en-us/unreal-engine/core-redirects-in-unreal-engine) remap renamed classes, enums, functions, properties and structs at load | Keep pure memoization (11 §7.4); add a declarative redirect table for renamed definitions and ports |
| [Blender nodes](https://docs.blender.org/manual/en/5.1/interface/controls/nodes/parts.html) | Field sockets hold per-element values; single values broadcast | Socket shapes: single, field, dynamic, grid | Implicit and lossy: “data may be lost and cannot be retrieved later”; angle sockets assume radians | Node groups; [5.0 adds bundles and closures](https://developer.blender.org/docs/release_notes/5.0/geometry_nodes/) | Keep “no implicit conversion”; consider bundles and closures (OPEN-15); add unit metadata |
| [Houdini assets](https://www.sidefx.com/docs/houdini/assets/namespaces.html) | Cook on demand | Typed parameters and geometry attributes | Node-specific | `namespace::name::version`; old instances keep the old implementation; precedence rules pick the default | Keep exact versions per instance; never resolve by precedence, offer an explicit upgrade |
| [Unity Shader Graph](https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/Data-Types.html) | Compiled to a shader | Dynamic Vector takes the lowest connected dimension; one concrete type per node | Vectors truncate or pad with (0, 0, 0, 1) | Package versions | Generic unification is the stronger form; never truncate silently |
| [Grasshopper](https://developer.rhino3d.com/guides/grasshopper/gh-algorithms-and-data-structures/advanced-data-structures/) | Every value is a data tree; inputs declare item, list or tree access | Loose | Implicit casts | Plugin assemblies | Collection shape is a real data property; keep it explicit (Map/Fold zones) but offer a “wrap in Map” editor action |
| [TouchDesigner](https://derivative.ca/UserGuide/Cook) | Pull: a node cooks only with a request and a reason; time-dependent nodes cook every frame | Operator families by data kind | Family converters | Built-in operators | Matches the data-pull plane; dirty tracking is the model for incremental validation |
| [Max/MSP](https://docs.cycling74.com/legacy/max8/tutorials/basicchapter06) | Hot inlets fire, cold inlets store; trigger emits right to left | Message types | Implicit | Externals | Shows why control order must be structural, as Sequence does (O2) |
| [Node-RED](https://nodered.org/docs/user-guide/messages) | Untyped message objects with a `payload` convention and `_msgid` | None | Manual | npm packages | Typed event payloads and causal depth (11 §10) are the stronger form |
| [ComfyUI v3](https://docs.comfy.org/custom-nodes/v3_migration) | Queue of node executions | `MatchType` links input and output types | Manual | Versioned API module; `NodeReplace` maps old node IDs; `is_deprecated` flag | Ship deprecation and replacement metadata in `NodeDefinition` |

The common lesson: strong editors fix identity (versioned IDs), keep old instances working, and provide a declarative way to move data forward. OrangeEditor has the first two in its format and none of the third in code.

## Propagation and execution

The runtime design combines the best-documented models: typed queued events (Node-RED, but typed), exec-token control (Unreal exec pins) and lazy memoized pull (TouchDesigner cooking). Only the edit-time half exists, and only type propagation is incremental.

&#91;embedded content: propagation · edit time in code, run time specified\]

Reactivity is a layer on top: variable changes and watches are evaluated at segment end and re-enter as queued events, so diamonds never glitch (11 §2). Unreal re-runs a pure node per consumer; here a pure value is pulled once per epoch and shared.

## Nature of data

The data model is the strongest part of the foundation: values, state, events, streams, resources and errors are separate by construction, and `Value` stays within 16 bytes. The gaps are three missing shapes (grouped values, behaviour as a value, units) and one broken path (variable edges).

| Nature | Mature editors | OrangeEditor spec | orange-model today | Gap |
| --- | --- | --- | --- | --- |
| Immutable value | Copied along wires; Unreal re-computes pure values per consumer | Immutable typed `Value`, pulled and memoized once per epoch (11 §7.4) | `Value` enum, `Arc`-shared, size asserted at compile time | None |
| Absence and errors | Usually null or a default | `option`, `result`, `error` and JSON null kept distinct; only `core/try` turns an error into data | Implemented in `Value` and the literal codec | None |
| Collection shape | Grasshopper data trees with implicit list matching; Blender broadcasts a single value to a field | `list`, `map`, explicit Map/Fold/Filter zones; no broadcasting | Types and literals exist; zones not validated | Strict by design; needs an editor “wrap in Map” action |
| Per-element (field) values | Blender field sockets, evaluated in a geometry context | Not in the `control` domain; a downstream domain (ADR-21) | No domain seam to add one | Blocked by the missing domain interface |
| Grouped values | Blender 5.0 bundles; Unreal structs | Nominal records with stable field IDs; no tuples | Records registered through the type registry | Record split/join ports need `port_template`, which is not implemented |
| Behaviour as a value | Blender 5.0 closures | Pure closures proposed (OPEN-15) | None | Open decision; affects node groups and package nodes |
| State | Max cold inlets store; Node-RED context | Variables (local, run, workspace), registers, node state; only actions write | Declarations and Get/Set contracts; Update and Shared not registered | Edges on Get/Set broken |
| Events | Node-RED untyped messages | Typed `EventDecl` payload, queued envelopes, causal depth limit | Declarations only; `on-event` has no contract | Trigger nodes cannot validate |
| Streams | Rare | Affine `stream<T>`, pull iterators, one consumer | Type exists; no stream rules checked | Fan-out check exists; storage and literal rules not |
| Resources | Engine object references | Generation-checked `handle<K>` of a declared kind | Type and `HandleId` exist | None in the model |
| Units | Blender angle sockets assume radians | Duration in ns; dBFS named for audio levels in ADR-22 | No unit metadata | Add units as nominal types or port metadata before audio and time nodes ship |

The rule set for data is complete enough to keep. What must change is how the code exposes it: shapes that depend on config or types (records, schemas) need declarative port templates, and new natures (fields, signals) need the domain seam rather than new core types.

## Node kinds and data types

The kind taxonomy is richer than any editor above, but it is a closed Rust enum, so a downstream domain cannot add its own kinds. Unreal splits nodes into pure and impure; OrangeEditor's six kinds also fix memoization, suspension, implicit ports and who may write state.

| Concept | OrangeEditor | Assessment |
| --- | --- | --- |
| Node kinds | `pure`, `volatile`, `action`, `async`, `entry`, `iterator`, plus control and zone intrinsics, function calls and editor-only nodes (11 §3) | Strong semantics. In code `NodeKind` has six variants and no `#[non_exhaustive]`; domain kinds (ADR-21) have nowhere to go |
| Implicit ports | Derived from the kind: `in`, `then`, `failed`, `error`, `cancelled`, `body`, `completed` (11 §4.1) | Good rule; the derivation lives in three files (`signature.rs`, `validation/ports.rs`, `catalog/snapshot.rs`) |
| Port kinds | `data`, `exec`, `event`; event is output-only | Strong and closed on purpose (P7) |
| Port descriptor | `type`, `source`, `available_on`, `default`, `optional`, `variadic`, `lazy`, `sensitivity`, presentation | Complete in the spec; the Rust struct is flat, so an exec port with a type is representable |
| Data sources | `computed`, `execution`, `state`, `payload`, `iteration` | The key idea behind availability and cycle rules; not yet used by any pass |
| Type grammar | `namespace/name@major<args>`, `$T` parameters, no `any`, no subtyping | Strong; `TypeExpr` is still text re-parsed in every pass |
| Generics | Unification over the touched component, persisted `type_args`, user `type_pins` | Stronger than Unreal wildcards or Shader Graph dynamic types: deterministic, saved, and re-inferred from evidence, never from a cached binding |
| Constraints | `eq`, `ord`, `hash`, `num`, `int`, `float`, `display`, `serialize`, `default`; coherence by type owner | Strong; `TypeParam.constraints` is a `Vec<String>` in the document model |
| Conversions | Visible `core/convert` nodes naming a registry entry; lossless-total auto-insert only when unique | Strongest part of the code: registry, provider alternatives, defaults and connection plans exist |
| Definition identity | `definition_id@definition_version` plus `config_version` | Right shape; no migration or redirect when either changes |

Two structural changes would make this taxonomy expandable without loosening it: let a domain register its kinds and implicit-port rules through the catalog, and model ports as a typed shape (exec, data with type and source, event with payload) behind the unchanged JSON format.

## Expandable and overridable

The extension model is designed correctly: semantics are closed, implementations are replaceable, and every graph pins what it used. Code exists for the pins and part of the package layer; the domain seam and any upgrade path are missing.

&#91;embedded content: extension and override layers · 6 layers, 1 cross-cutting upgrade path\]

Override means replacing an implementation under the same contract, never changing what a node means. That matches the best practice above: Houdini keeps old instances on their old definition, and Unreal and ComfyUI move saved data forward with declarative redirects. OrangeEditor needs that redirect layer before the first package renames anything.

## How the editor should work

The editor should never accept an edit the model cannot explain: every refused or incomplete state needs a stable diagnostic on a node, port or edge. Connection planning already works this way; versioning, renames and refactors do not exist yet.

| Editor behaviour | Rule | Status |
| --- | --- | --- |
| Dragging a wire highlights valid targets | Filter by the kind matrix and a dry-run of type unification | Model support exists (`plan_connection`); no editor |
| Dropping a wire | Direct, replace or insert one visible lossless converter; offer a choice for partial or lossy entries; refuse conflicts | Implemented in `conversion::plan` |
| Generic nodes | Resolve on connect, keep pins, re-infer on disconnect; never keep a stale binding (the [Unreal wildcard revert bug](https://forums.unrealengine.com/t/391015) shows the failure) | Implemented in inference |
| Errors | Shown on the subject with a translatable message key; nothing is incomplete without a reason | Message keys implemented; Get/Set edges break the second half |
| Missing package | Show the stored signature, keep edges as orphans, block compile | Implemented in the model |
| New version available | Palette shows the latest; existing instances keep theirs; upgrade is an explicit command with a preview, as in Houdini | Spec only (04); no rebind or upgrade command |
| Renamed definition or port | Declarative redirect applied on load and saved forward, like Unreal Core Redirects and ComfyUI `NodeReplace` | Missing from spec and code |
| Provider override | User picks the default for new bindings; existing locks never change silently | Converters only |
| Collapse to function | Preview interface, effects and variable use; one undoable transaction | Spec only (11 §14.1) |
| Wrap in Map | Turn an item-typed chain into a Map zone over a list | Not specified; recommended |
| Dynamic ports | Ports from config or types come from a declarative template, never from running plugin code | Spec only (11 §4.5) |
| Live preview | Pull pinned pure outputs; re-evaluate only the downstream cone | Spec only (11 §15) |
| Layout edits | Never touch validation, plans or preview | Layout is a separate command; holds |

Rename handling is the one rule missing from the spec itself. Without it, a package that renames a port turns every saved edge into an orphan.

## Recommendations

Start with the Get/Set fix and one rules table, then build the two seams (domains and migrations) before adding more validation passes. Items 1 and 2 fit the current branch; items 3 to 7 change public contracts and need docs/11–12 updated in the same work unit.

| # | Change | Fixes | Size |
| --- | --- | --- | --- |
| 1 | Look ports up through `registry.contract()` and the fresh node resolution; give every incomplete node a diagnostic | Get/Set edge bug, silent incomplete states, two answers for one node | Small |
| 2 | One language-rules table in `catalog`: every intrinsic ID (registered, or reserved with its own “not supported yet” code), implicit ports by kind, reserved IDs, function-kind facts | Six copies of intrinsic knowledge; misleading E-PROVIDER-UNAVAILABLE | Medium |
| 3 | Domain seam: typed `domain` on `GraphBody`; a `Domain` interface (kinds, implicit ports, body validation) registered in `RegistryBuilder`; an unknown domain loads, is preserved and reports its own diagnostic | ADR-21 expansion through crates | Medium to large |
| 4 | Semver-safe surface: `#[non_exhaustive]` on enums and structs that will grow, constructors for `NodeDefinition` and `PortDescriptor`, and no module cycles | Downstream crates can upgrade without breaking | Medium |
| 5 | Complete `NodeDefinition`: `port_template`, `event_source`, `errors`, `determinism`, `cleanup_safe`, `capabilities`, TextRef labels, deprecation and replacement; check kind and suspension at `build` | Triggers, record/schema ports, plugin text, early metadata errors | Medium |
| 6 | Migration contract: redirects for renamed definitions, ports and enum variants; version upgrades with a port map; rebind with preview | Override and upgrade without orphaned edges | Medium; spec first |
| 7 | Override slots in the registry: slot IDs, providers per slot, new-binding defaults for nodes (converters already have them) | The 04 override model | Medium |
| 8 | Incremental validation keyed by document revision and registry generation, driven by `ChangeSet`, with unchanged results reused ([Salsa backdating](https://salsa-rs.github.io/salsa/reference/algorithm.html)) | NFR-15 (≤2 ms per edit) | Large; design first |
| 9 | Decide closures and bundles (OPEN-15); add unit metadata | Missing data shapes | Decision first |

What to keep unchanged: no implicit conversion, pure memoization, exact provider locks, nominal versioned types, the closed language intrinsics (P7) and the availability rule. These are the parts of the design that make it stronger than the editors it learned from.

## Sources

Project: docs/04, 08 (ADR-13, ADR-21, ADR-22), 11 and 12 on branch `refactor/typed-port-descriptor`; the orange-model source and probe tests run on 2026-10-23.

- [Unreal Engine: Functions](https://dev.epicgames.com/documentation/en-us/unreal-engine/functions-in-unreal-engine)
- [Unreal Engine: UK2Node\_MacroInstance::InferWildcards](https://dev.epicgames.com/documentation/unreal-engine/API/Editor/BlueprintGraph/UK2Node_MacroInstance/InferWildcards)
- [Unreal Engine: Core Redirects](https://dev.epicgames.com/documentation/en-us/unreal-engine/core-redirects-in-unreal-engine)
- [Unreal forum: wildcard nodes not reverting](https://forums.unrealengine.com/t/391015)
- [Blender 5.1 Manual: Node Parts](https://docs.blender.org/manual/en/5.1/interface/controls/nodes/parts.html)
- [Blender 5.0 release notes: Geometry Nodes](https://developer.blender.org/docs/release_notes/5.0/geometry_nodes/)
- [Houdini: Asset namespaces and versions](https://www.sidefx.com/docs/houdini/assets/namespaces.html)
- [Unity Shader Graph 17.0: Data Types](https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/Data-Types.html)
- [Rhino developer guide: Grasshopper advanced data structures](https://developer.rhino3d.com/guides/grasshopper/gh-algorithms-and-data-structures/advanced-data-structures/)
- [TouchDesigner: Cook](https://derivative.ca/UserGuide/Cook)
- [Max 8 tutorial: Simple Math](https://docs.cycling74.com/legacy/max8/tutorials/basicchapter06)
- [Node-RED: Working with messages](https://nodered.org/docs/user-guide/messages)
- [ComfyUI: V3 migration](https://docs.comfy.org/custom-nodes/v3_migration)
- [Salsa: the red-green algorithm](https://salsa-rs.github.io/salsa/reference/algorithm.html)
