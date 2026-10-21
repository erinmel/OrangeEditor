# Graph-language conformance fixtures

Golden fixtures for the G-suite in [11 §18](../docs/11-graph-language.md). Each fixture is data, not code: a graph document plus the diagnostics or deterministic trace a conforming implementation must produce. The validator (00d), compiler and reference interpreter (00e), run actor (00f) and later the optimized executor (WU-04) all run the same files. Status: **defined in WU-00/00a; NOT RUN**: no validator or interpreter exists yet.

| Path | Holds |
|---|---|
| `rules.json` | Every testable rule of 11 §4–§15, each mapped to at least one G case |
| `packages/org.orange.test.json` | The one test package every fixture uses: node definitions per [12 §4](../docs/12-graph-data-structures.md) with deterministic test behaviour |
| `g-suite/G-NN-<slug>.json` | One file per G case; a file may hold several cases |

## Fixture file

```json
{
  "format": "orange.conformance",
  "format_version": 1,
  "id": "G-04",
  "title": "Action output read after a Branch merge",
  "rules": ["R-7.2-dominance"],
  "cases": [
    {
      "name": "read-after-merge",
      "document": {"format": "orange.graph", "format_version": 1, "graph_id": "a0000000-0000-4000-8000-000000000004", "revision": 0},
      "expect": {"diagnostics": [{"code": "E-UNAVAILABLE-VALUE", "subject": {"node": "10000000-0000-4000-8000-000000000005", "port": "value"}}]}
    }
  ]
}
```

- `id` is the G case; `rules` lists the `rules.json` IDs the file covers. Every case in the file shares them.
- `document` is a complete `orange.graph` format v1 document ([12 §6](../docs/12-graph-data-structures.md)) that loads with the `orange-model` codec. The example above is abbreviated; real fixtures include `nodes`, `edges`, `signatures` and `package_lock`. Intrinsic nodes persist `type_args` when they are generic, like any node.
- `case.registry.absent` lists definition, type or provider IDs the host registry does NOT contain for this case (missing-provider handling).
- `case.presentation` is host-side presentation input (locale, labels) for i18n and presentation cases. It is never part of the document.
- Exactly one of `expect.diagnostics` (compile-time) or a `run` + `expect` trace pair (run time) is the point of a case; a run-time case also asserts `diagnostics: []` implicitly unless it lists warnings.

### IDs

Use readable fixed v4 UUIDs, the same pattern as the model tests, with the last block identifying the item:

| Kind | Pattern | Example |
|---|---|---|
| graph | `a0000000-0000-4000-8000-0000000000NN` (NN = G number) | `a0000000-0000-4000-8000-000000000004` |
| node | `10000000-0000-4000-8000-0000000000NN` | `10000000-0000-4000-8000-000000000003` |
| edge | `20000000-0000-4000-8000-0000000000NN` | |
| variable | `30000000-0000-4000-8000-0000000000NN` | |
| event | `40000000-0000-4000-8000-0000000000NN` | |
| function | `50000000-0000-4000-8000-0000000000NN` | |

### Test package and signatures

Every non-intrinsic node uses a definition from `org.orange.test` 0.1.0 and needs, per 12 I-4:

- `"provider": {"package": "org.orange.test", "version": "0.1.0", "id": "reference"}`;
- a `signature` key whose snapshot is in the document's `signatures` map. Copy the snapshot from the definition's ports with `$T` substituted from the node's `type_args` and presentation fields removed (12 §3.1). Use the readable key `s-<definition-name>-<type>` (for example `s-add-i64`);
- the lock entry `{"id": "org.orange.test", "version": "0.1.0", "content_digest": "sha256:<digest>"}` in `package_lock`, where the digest is SHA-256 of the exact bytes of `packages/org.orange.test.json` (the fixture test recomputes and compares it).

`core/` intrinsics have no provider, signature or lock entry.

- **Core converters** for core types (11 §6 examples, such as `lossless-total` i32→i64 and `checked-partial` i64→i32) are host-provided and always registered; fixtures use them unbound, so they need no declaration or lock entry (11 §6.4).
- **Extra packages:** a case that needs a definition, type or converter the shared package must not contain (for example a converter the registry must reject, G-21) puts an inline package in `case.packages`, using the same format with its own `id`. Use this sparingly; prefer the shared package.
- **Signature snapshots** list only declared ports. Implicit kind ports (`in`, `then`, `failed`, `error`, `cancelled`, `body`, `completed`) are never listed (11 §4.1).

## Expected diagnostics

```json
{"code": "E-CYCLE-DATA", "subject": {"node": "…", "port": "…", "edge": "…", "decl": "…"}, "args": {"path": ["…", "…"]}}
```

`code` is from the 11 §17 catalog. `subject` holds only the IDs the diagnostic is about. `args` holds structured arguments (no prose); omit it when the rule defines none. The list is compared as a set; an implementation may not add or omit diagnostics.

Subject conventions:

- Port-level codes set `node` + `port`. Edge-level codes (an edge is the cause) also set `edge`, and `node` + `port` name the consumer (`to`) end. Exception: `E-PORT-UNKNOWN` names the end whose port does not exist.
- Node-level codes set `node` only, the offending node (for example `E-REGION-AMBIGUOUS`, `E-SUBSCRIPTION-IN-FUNCTION`, `W-DETACH-IN-FUNCTION` and `E-REGION-ESCAPE` on a Return inside a Fork branch: the Return node).
- `E-CYCLE-SYNC-CONTROL`, `W-BUSY-LOOP`: `node` + `edge`, the closing back edge and its consumer node; no port.
- A rejected `connect` that is never committed (for example `E-TYPE-MISMATCH`): the subject `edge` is the attempted edge's ID.
- An export has no edge: `E-CROSS-ACTIVATION` and `E-STREAM-STORE` name the exported `node` + `port`.
- Function-interface ports (`functions[].inputs`/`outputs`; `E-PORT-DIRECTION`, `E-HANDLE-LITERAL`): `decl` (the function ID) + `port`.
- `E-CYCLE-DATA`: `args.path` lists node IDs starting at the smallest node ID.
- `E-RECURSION`: one diagnostic per call node on the cycle.
- `E-FUNCTION-KIND`, `E-MISSING-RETURN`: subject `decl` (the function ID).
- `E-WRITE-CONFLICT`: subject `decl` (the variable ID) and `args.nodes` (writer node IDs, sorted). A register has no declaration ID: subject `node` (the register) and `args.nodes`.
- `W-STALE-CAPTURE`: subject `node` + `port` of the captured output; `args.variable` (the variable ID) and `args.suggest` (the suggested definition ID).
- Rules are independent: a case lists every code its graph violates (a stream export gets `E-STREAM-STORE` and `E-CROSS-ACTIVATION`), but a subscription in a function body gets only `E-SUBSCRIPTION-IN-FUNCTION`, because `E-ENTRY-IN-FUNCTION` covers entry and Watch nodes.

Registry and definition checks (package publication, definition validity, converter ownership) have no graph diagnostic code. Such a case puts the package under test in `case.packages` and asserts `"registry": {"result": "rejected", "reason": "<rule ID or short reason>"}` (or `"accepted"`) next to `"diagnostics": []`.

Runtime behaviour that has no trace event of its own (for example a definition-level property) is asserted with `"behaviour": {"rule": "<rule ID>", "expect": "<what must hold>"}` next to `"diagnostics": []`; keep these rare and prefer a trace. `behaviour` may also appear next to a trace.

A case with only a prose `behaviour` expectation must also declare `deferred: {"owner": "<subunit>", "reason": "<missing contract or harness>"}`. An empty diagnostic list does not establish that prose assertion. These cases remain specifications until the named owner replaces the note with executable assertions. [REVIEW](REVIEW.md) records the remaining gaps and what the structural checks actually prove.

## Run script and expected trace

```json
"run": {
  "seed": 1,
  "max_cause_depth": 64,
  "steps": [
    {"trigger": {"node": "10000000-…-000000000001", "payload": {"type": "core/i64@1", "value": "5"}}},
    {"advance": "1000000000"},
    {"cancel": {"activation": 1}},
    {"edit": {"op": "set-literal", "port": {"node": "…", "port": "b"}, "literal": null}},
    {"stop": "graceful"}
  ]
},
"expect": {
  "trace": {"match": "exact", "events": [{"t": "0", "event": "activation-start", "activation": 1, "entry": "…"}]},
  "variables": {"30000000-…-000000000001": {"type": "core/i64@1", "value": "5"}},
  "counters": {"queue_dropped": 0}
}
```

- Time is a virtual clock in nanoseconds, as decimal strings; it starts at `"0"` and moves only through `advance`. Steps run in order; after each step the run executes until nothing is ready.
- `trigger` enqueues a manual trigger on an entry node. `cancel` cancels an activation; `stop` is `graceful` or `cancel`. `edit` applies one document edit (below) between steps. `{"run": N}` starts run N (1-based) of a multi-run case; runs share workspace state, and activation numbers restart at 1 per run. Later steps target run N until the next `{"run": …}` step. All runs share one virtual clock; the trace lists every run's events in the order they happen (same-time ties in run order), with no boundary event.
- `match` is `exact` (the whole trace, in order) or `subsequence` (these events appear in this order; others may interleave). Prefer `exact` for small graphs.
- Activations are numbered 1, 2, … in dispatch order within the run. Values use the tagged literal form of 12 §6.1.
- A case may request `"replay": {"source": "recorded-run"}` beside `run`. The harness first executes and records that run, then starts a fresh run from the same initial document/state, plan identity, seed and entropy metadata, feeding its recorded ingestion log at the recorded virtual times. `expect.replay_run.trace: "identical"` compares the complete canonical trace from both executions, even when the golden trace uses `subsequence`. `expect.replay_run.counters.remote_sends: 0` counts real transport sends during replay; simulated completions still produce their recorded semantic trace events. This is a harness contract for 00e/00f, not an implemented replay runner.
- An activation starts with `activation-start` followed by `exec` for its entry node with outcome `then`. Events at the same virtual time keep the order in which they happen.

| Event | Fields | Emitted when |
|---|---|---|
| `activation-start` | `activation`, `entry`, `cause_depth` | An activation is dispatched |
| `activation-end` | `activation`, `outcome`: `completed`, `faulted`, `cancelled` | It finishes |
| `activation-dropped` | `entry`, `reason`: `reentrancy-drop`, `queue-overflow`, `causal-depth` | An envelope never becomes an activation |
| `exec` | `activation`, `node`, `outcome` (the fired outcome port ID) | An exec node finishes one execution |
| `kernel` | `node` | A pure or entropy kernel is invoked (lets fixtures count memoization) |
| `log` | `activation`, `node`, `value` | `org.orange.test/log` executes |
| `emit` | `activation`, `decl` (the event declaration ID), `value` | `core/emit` enqueues |
| `notify` | `variable`, `value`, `previous` | A variable-change notification is enqueued |
| `watch` | `node`, `value` | A Watch fires |
| `fault` | `activation`, `node`, `error` (code) | A strand faults |
| `cancelled` | `activation`, `node` | A suspended node is cancelled |

Trace conventions:

- Control intrinsics emit one `exec` per outcome they fire: Sequence `then#k` and Fork `branch#k` one per start; Fork `completed` once; loops `body` once per iteration, then `completed` or `failed`. Variadic `#k` indices start at 0. Break, Continue and Return emit no `exec`.
- Fork emits every `branch#k` `exec` (port order) before any branch body event. `detach` fires `completed` right after them; `race` emits the losers' `cancelled` before `completed`.
- `core/function-entry` and `core/function-outputs` emit no `exec`. A pure `core/call` emits none; an action `core/call` emits one `exec` `then` after the callee returns, with the body's events (caller's activation number) before it.
- A node's own effect events (`log`, `emit`) come before its `exec`.
- A cancelled suspended node emits `cancelled`, then `exec` with outcome `cancelled` only if that port is connected.
- A node that fires `failed` emits `exec` with outcome `failed`; if `failed` is unconnected, a `fault` follows. `fault.node` is the node whose `failed` fired (for a pure error, the exec consumer that pulled it).
- `kernel` is emitted only for non-core definitions, including invocations that fail. A constant cone is folded at plan time and emits no `kernel`, even when the fold is an error (`W-CONST-ERROR`).
- `notify` is emitted only when an On Variable Changed entry for that variable exists, and is enqueued at segment end.
- `advance` jumps the virtual clock (a wait, not a busy wait); work that becomes due, timer ticks included, runs at its own scheduled time. `{"stall": "<ns>"}` jumps the clock the same way but leaves the run actor unserviced: every timer tick due in the jump is delivered at the end time and handled by its missed-tick policy (11 §10.2). Ties at the same virtual time are processed in start order.
- In `replace` activation mode, the old activation's `activation-end` (outcome `cancelled`) comes before the new one's `activation-start`.
- A Watch baseline is computed at run start and is not traced.
- A fault raised inside cleanup mode ends the activation `cancelled`.
- Nodes inside a function body emit events like any node, with the caller's activation number.

Other expectations:

| Field | Meaning |
|---|---|
| `exports` | Exported values after the run (same tagged literal form). A case with no `run` evaluates the exported volatile cone once from the declared initial values |
| `replay` | `full` or `partial` (11 §7.6). Deterministic cases with no `replay` key are implicitly `full` |
| `counters` | Names from the table below; unlisted counters are not asserted |
| `committed` | Whether an edit transaction committed (bool) |
| `choice` | The option the host offers or applies (string), for editor cases |
| `wire_path` | Route assertion for an editor wire |
| `invariant` | A named property that must hold (string); use sparingly, like `behaviour` |

### Counters

| Counter | Counts |
|---|---|
| `accepted` | Envelopes that became activations |
| `queue_dropped` | Envelopes lost to entry capacity: `drop-oldest`, `drop-newest`, `coalesce-latest` replacements and `fault` rejections, each with an `activation-dropped` `queue-overflow`. Capacity counts pending envelopes only, not the running activation |
| `reentrancy_dropped` | Triggers ignored by `drop` mode (`reentrancy-drop`) |
| `depth_dropped` | Notifications and Watch firings suppressed above `max_cause_depth` (`causal-depth`); an Emit instead fails with `core/RunawayEventChain` |
| `cancelled` | Activations that ended `cancelled` |
| `recompiles`, `preview_invalidations` | Plan compilations and preview-cone invalidations |
| `open_streams` | Streams still open at the end of the run |
| `tee_dropped` | Items a `core/tee` side lost to overflow |
| `timer_missed` | Timer ticks dropped or folded by a missed-tick policy |
| `remote_sends` | Real sends performed by `org.orange.test/send`; simulated replay completions do not increment it |

### Edits

For editor-level cases (G-19, G-20, G-30, G-31, G-36), `case.edits` is a list applied in order, as one transaction, before `expect`. A case may combine `edits` and `run`; edits apply first. Each edit is one `DocCommand` ([12 §3.3](../docs/12-graph-data-structures.md)) in kebab case: `{"op": "connect", "edge": {…}}`, `{"op": "disconnect", "edge": "…"}`, `{"op": "set-literal", "port": {…}, "literal": {…}|null}`, `{"op": "insert-variadic", "node": "…", "port": "item", "at": 1}`, `remove-variadic`, `{"op": "set-type-args", "node": "…", "args": {"T": "core/i64@1"}}`, `{"op": "layout", "path": ["nodes", "…"], "value": {…}}`. `expect` may then assert `document` fragments (`{"nodes": {"<id>": {…}}}`), `change_set` (`layout_only`, added/removed IDs) or `diagnostics`.

G-49 also uses `case.numeric_input: {"port": {"node": "…", "port": "x"}, "text": "3", "type": "core/i64@1"}` to exercise the editor's creation of a typed numeric literal. `type` is an optional explicit type selection; otherwise the resolved port type wins, falling back to f64 when unresolved (11 §5.5). Apply this once before `edits`/`run`, resolve the affected type arguments and signature, then compare the expected document fragment. Integer parsing must not pass through f64. This is an editor harness action, not a `DocCommand` or a new graph wire format; no such editor is implemented here.

## Core intrinsic ports

11 §8.4, §10.2, §11.1, §12 and §14 name most intrinsic ports. Where they do not, fixtures use the names below; 00d adopts or revises them in doc 11 together with these fixtures.

| Intrinsic | Inputs | Outputs | Config |
|---|---|---|---|
| `core/on-start`, `core/on-stop` | — | `then` | — |
| `core/on-manual` | — | `then`, `payload` | `payload` (type) |
| `core/on-event` | — | `then`, `payload` | `event` |
| `core/on-timer` | — | `then`, `scheduled_at`, `missed` | `interval` (ns string), `first`, `missed` |
| `core/on-timeout` | — | as `core/on-timer` (`missed` is always 0) | `interval`: fires once, `interval` after run start |
| `core/on-variable-changed` | — | `then`, `value`, `previous` | `variable` |
| `core/on-watch` | `source` (pure cone) | `then`, `value` | `mode`: `changed` or `rising` |
| `core/get-variable` | — | `value` | `variable` |
| `core/set-variable` | `in`, `value` | `then` | `variable` |
| `core/update-variable` | `in`, `operand` | `then`, `old`, `new` | `variable`, `op` |
| `core/set-shared`, `core/update-shared` | as set/update-variable | as set/update-variable | as set/update-variable |
| `core/emit` | `in`, `payload` | `then` | `event` |
| `core/convert` | `value` | `result` | `converter` (slot ID) |
| `core/select` | `condition`, `when_true`, `when_false` | `result` | — |
| `core/and`, `core/or` | `a`, `b` | `result` | — |
| `core/coalesce` | `value#k` | `result` | — |
| `core/call` | the function's inputs (+ `in` for action functions) | the function's outputs (+ `then`) | `function` |
| `core/function-entry` | — | the function's inputs (+ `then`) | — |
| `core/return`, `core/function-outputs` | the function's outputs (+ `in` for return) | — | — |
| `core/repeat-begin` | `count`, `carry` | `index`, `carry` | `end` (node ID) |
| `core/repeat-end` | `carry` | `result` | — |
| `core/for-each-item` | `in`, `stream` | `body`, `completed`, `failed`, `item` | — |
| `core/next-item` | `in`, `stream` | `then`, `item` | — |
| `core/collect` | `in`, `stream` | `then`, `items` | `max_items` |
| `core/close-stream` | `in`, `stream` | `then` | — |
| `core/tee` | `in`, `stream` | `then`, `left`, `right` | `capacity`, `overflow` |

`core/switch` config `cases` maps the key `k` of each `case#k` port to an enum variant: `{"<k>": {"variant": "<variant_id>"}}`. `scheduled_at` is a `core/timestamp@1` in nanoseconds on the virtual clock.

Generic intrinsics persist `type_args` under these parameter names:

| Intrinsic | Type params |
|---|---|
| `core/select`, `core/coalesce`, `core/try`, `core/match-result`, `core/register`, `core/repeat-begin`, `core/repeat-end` | `T` |
| `core/for-each` | `T` (list) or `K`, `V` (map) |
| `core/switch` | `K` (the key type) |
| `core/for-each-item`, `core/next-item`, `core/collect`, `core/close-stream`, `core/tee` | `T` (the item type of `core/stream@1<T>`) |
| `core/call` | the called function's `type_params` |

Control intrinsics use exactly the 11 §8.4 port names (`condition`, `true`, `false`, `then#k`, `branch#k`, `completed`, `body`, `element`, `index`, …). Implicit ports follow 11 §4.1.

## Open questions

The language spec leaves these undecided. Fixtures touching them keep the current choice or a `behaviour` note until the owner work unit decides; deciding one means updating doc 11/12 and the affected fixtures together.

| # | Question | Spec | Owner WU |
|---|---|---|---|
| 1 | `core/coalesce` and `core/select`: input types, which input wins, type-parameter names and signatures | 11 §8.4 | 00d |
| 2 | Flip Flop start state and `is_a`; Do Once second `in`; Gate `open`/`close`/`toggle` outcomes and `enter` while closed | 11 §8.4 | 00d |
| 3 | `update-variable`: `append` on a string; outcome of a `compare-and-set` mismatch | 11 §11.1 | 00d |
| 4 | `E-INPUT-REQUIRED` on a pure node that no consumption site reaches | 11 §4.4 | 00d |
| 5 | Zone-body membership of input-less volatile nodes (Get vs Random) | 11 §8.4, §7.4 | 00d |
| 6 | Missing required config (for example `collect.max_items`) has no code (proposal `E-CONFIG-INVALID`) | 11 §17 | 00d |
| 7 | Missing provider: no §17 code or subject (G-29 asserts none; doc 03 says "unresolved diagnostic") | 11 §17, 03 | 00c, 00d |
| 8 | Reactive diamond R-7.3 W1/W2/W3 shape: Watch has no Watch inputs | 11 §7.3 | 00d |
| 9 | Unresolved signature snapshots (`$T` in `signatures`) vs "`$T` outside a definition is rejected" | 12 §3.1 | 00c |
| 10 | Port templates, specialization lists (`E-NO-SPECIALIZATION`), package JSON for constraint implementations, override-slot providers, push-source `stream_source`, package entry and graph-implemented definitions | 12 §4, §5, §8 | 00c, WU-03 |
| 11 | `core/error@1` trace and literal encoding; logging a missing field vs `None` (G-18) | 12 §6.1 | 00c |
| 12 | Load-time type-text errors (unknown types, type lock with arguments, `$T` outside a definition) have no §17 code; they stay `behaviour` (G-46) | 11 §17 | 00c |
| 13 | Dispatch tie-break between two entry queues free in the same scheduler pass (proposal: head envelope `ingestion_seq`) | 11 §7.5 O5, §9.2 | 00f |
| 14 | Beyond `max_parallel`: queue or drop; `accepted` for dropped envelopes; `replace` "latest pending" under a true burst | 11 §10.1 | 00f |
| 15 | `missed` for `skip` (all dropped ticks at the next fire) and `coalesce` (folded count) | 11 §10.2 | 00f |
| 16 | Tee lagging-side buffering and `tee_dropped` | 11 §12 | 00f |
| 17 | `invocation_in_activation` indexing | 11 §7.6 | 00e |
| 18 | Activation outcome when a Return cancels a detached strand (G-81 assumes `completed`) | 11 §14 | 00e |
| 19 | Harness hooks for heavy offload and a concurrent write during a pull (G-54) | 11 §7.4 | 00e |
| 20 | `case.edits` cannot express several quick commits; recompile debounce interval unspecified (G-82) | 11 §15 | 00e, WU-05 |
| 21 | Display text of derived enum and record values from stable IDs (G-39) | 13 | WU-05 |
| 22 | Wall-time overlap of two runs on the compute executor is not observable in a virtual-time trace (G-66) | 11 §9.1 | 00f |
| 23 | No synthetic nested-error injection hook for G-78 cause truncation; errors have no literal codec. Define a harness value builder without allowing error literals in saved graphs | 11 §13, 12 §6.1 | 00e, 00f |
| 24 | Runtime-code catalogue introspection for G-78 has no harness API; the prose catalogue assertion does not exercise registration | 11 §13 | 00e |
| 25 | Negative-load fixtures and mapping codec duplicate-ID errors to `E-DOC-DUPLICATE-ID`; precedence/suppression of `E-SCOPE-EDGE` against availability errors. Both diagnostic codes still lack golden expectations | 11 §17, 12 §6 | 00d |

## Reading and running the checks

Start with a fixture's named case, follow its `document.nodes` and `edges`, then its `run.steps` and `expect`. Non-core nodes resolve to `packages/org.orange.test.json`; `signature` selects the saved semantic ports, and `type_args` resolves generic parameters. Intrinsic behavior comes from doc 11. `rules.json` links the normative rule to fixture files; file-level mapping is not proof that every aspect of a rule has an executable assertion.

`cargo test -p orange-model --locked --test conformance_fixtures` runs structural checks: codec loading, package digests, semantic snapshot projection (with exact-node intentional-drift assertions), inverse rule mappings, script clocks, replay expectation shape and documentation JSON agreement. It executes no graph, kernel, validator, scheduler or remote effect. The same suite is part of the workspace tests.
