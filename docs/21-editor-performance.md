# Editor performance and representation costs

## Preview and value observation review (2026-11-17)

The former 64-output cap controlled coverage, not latency. The compiler admits
at most 100,000 graph items and 20,000 potential fold operations per request by
default. A compile has a 16 MiB nominal constant budget and the preview cache
has its own 16 MiB nominal retention budget. Those budgets do not bound the
editor's cumulative cost across retries or cap process memory.

The editor runs preview synchronously on open and after semantic changes. When
the full graph cannot compile, it projects upstream nodes for a batch of outputs.
If every batch fails, bisection can attempt up to `2P - 1` projections for `P`
outputs, in addition to the full-graph attempt. Every projection clones and
scans the saved graph, and every attempt invokes host parsing and validation.
The next performance work should move that evaluation off the UI thread, bound
work across retries, cancel obsolete edit revisions and publish staged results.
There is no large-graph frame-time qualification for this path.

Run observation still scans the current output ports on each observed turn. The
editor's per-poll event limit bounds the number of events, while a single event
can carry many port changes. Wired input presentation is a projection of the
latest producer sample through the current wire, not an execution-time record of
the consumer's input.

Port cards and Properties now retain freshness per output across a run end or
graph replacement. Receiving one fresh output leaves older outputs marked
previous. Formulas use fresh connected values and use saved literals for unwired
inputs. Value summaries and detailed inspection stop formatting at their text
budgets on UTF-8 boundaries. This bounds formatted text per value; it does not
limit the number of observed values retained by the editor.

Windows verification used Rust 1.99.1 (`58cf8f9c3`), Cargo 1.99.1
(`5f94df478`), and unchanged Cargo.lock SHA-256
`79f3aa431e51ba221d3ff8a4af718187202f5f7364d3d4e8546ebe1a6a287f08`.
`cargo test --workspace --all-features --locked --offline --no-fail-fast`
reported 1,948 passed, zero failed and 24 ignored. The regressions cover
freshness after one output updates, stopping UTF-8 formatting at the budget,
preserving the byte inspector's truncation marker, and the 71-Absolute chain.
The workspace format check reported differences. The Clippy command did not
launch through the IDE runner.

Work: PERF-EDITOR, 2026-11-14 (America/Mexico_City). Baseline:
`af99e1d6a2356357970ed89a1eadb36b128041a3` (PR179). Scope: the desktop editor,
standalone canvas and host graph-view projection. Runtime execution, audio callbacks,
package permissions and language semantics are unchanged.

## Findings and implementation

Let N be nodes, E wires, P total ports and D registered definitions. Hash lookups below
are expected constant time, using the standard randomized hasher. These are source
complexity descriptions, not measured worst-case latency guarantees.

| Path | Baseline cost/problem | Implemented change and memory tradeoff |
|---|---|---|
| `canvas::auto_layout` | Up to N full wire-relaxation passes; reverse-ordered chains and cycles are expensive. It runs even when every position is saved. | Dense adjacency, incoming counts and a topological queue visit nodes/edges once, plus construction of the public ordered position map. O(N+E) scratch space. Fully saved layouts skip topology. |
| `Canvas::update` | Computes a complete fallback layout after every semantic edit, even when no node needs a position. | Retains existing position entries; computes fallback only when adding an unsaved node. O(N) temporary membership set. |
| Canvas wires | Each endpoint and data-wire color searches the entire node list; port rows are repeatedly searched. Offscreen wires still produce paint shapes. | One O(N+P) borrowed geometry index per frame; lookups replace global scans. Conservative Bezier control-point bounds cull offscreen wires. Positions are read after dragging, preserving same-frame wire motion. |
| `Host::view` | Each port scans all edges for connectivity; each wire searches nodes again for its kind. Literal lookup formats every candidate key. | Endpoint connectivity and output-kind indexes replace those scans; typed literal lookup. O(E+P) temporary index storage, including owned endpoint strings. |
| Node palette | Rebuilds labels, lowercased search text, grouping and ordering every frame. | Retains one result keyed by process-unique registry generation and exact search text. Stores O(D) presentation data; replaces the previous search result, retaining no old provider snapshot. |
| Event polling | Drains until the channel happens to become empty; a continuously refilled queue can prevent painting. | Consumes at most 256 events per call, including discarded events. Keeps FIFO order and queued terminal events; desktop requests another frame after receiving any batch. This bounds event count, not time spent formatting a large event. |
| Names and multi-selection | Name extraction deep-clones entire JSON node/variable arrays; dragging clones the selected-ID tree. | Borrows parsed arrays and iterates disjoint selection/position fields directly. |

The canvas now separates topology (`canvas/layout.rs`) and frame geometry/culling
(`canvas/geometry.rs`) from gesture handling and drawing. Palette projection/cache
has its own module. Existing public function paths remain available by re-export;
no extra crate or dependency was added. These are targeted separations around
performance ownership, not a claim that all remaining large modules need rewriting.

## Behavior and compatibility

Saved node positions remain exact. Acyclic unsaved nodes retain longest-predecessor
columns independent of edge order. Cyclic nodes and descendants blocked by cycles use
the columns reached from acyclic roots and stack in document order. This intentionally
replaces the old edge-order-dependent cyclic layout; dangling wires contribute no
column. It affects fallback presentation only, never validation or execution.

Geometry is borrowed from the supplied view for one frame. It never retains stale
ports or type colors across graph replacement; standalone callers have no new cache
invalidation obligation. The index stores offsets/types, not cloned graph payloads.
Wire culling tests all four control points with stroke margin, so a wire crossing the
viewport or bending into it is retained even when both endpoints are outside.

`Editor::take_events` retains its signature and boolean meaning (any event consumed),
but callers must poll subsequent frames while it returns true. `MAX_EVENTS_PER_POLL`
documents the cap. Existing runtime/provider ownership and graph formats are unchanged.
No schema, translation key, credential or package-authority change is introduced.

Trace: FR-01/02/03/04/07; NFR-03/05/12/15; ADR-02/21. This is partial performance
evidence, not completion of W4, NFR-03 or incremental-validation qualification.

## Local measurement method

Windows x64, AMD Ryzen 9 9900X3D 12-Core Processor, Rust 1.99.1
(`58cf8f9c3`, LLVM 23.1.1), Cargo 1.99.1 (`5f94df478`). Same committed dependencies,
default feature selection, optimized Cargo release profile; egui 0.37.0 with
`default_fonts`. Root `Cargo.lock` SHA-256:
`79F3AA431E51BA221D3FF8A4AF718187202F5F7364D3D4E8546EBE1A6A287F08`.
MSRV remains 1.97.1; no dependency, feature or lock change.

Command: `cargo run -p orange-editor-ui --example performance --release --locked --offline`.
The identical example was compiled against baseline and candidate. Setup is excluded;
one warm-up precedes each sample. Layout/view samples average three operations and
canvas samples average twenty frames. The graph is a chain with reverse edge order;
saved nodes occupy a 100-column grid. Canvas uses a 1280x720 egui headless frame with
automatic fit (minimum zoom 0.2). Host-view samples decode and project Set-variable
nodes and their execution wires. The public uncached palette measurement is a control,
not a measurement of the editor's cache.

After compilation and correctness checks finished, three fresh baseline processes and
three fresh candidate processes ran sequentially, alternating order (B/C, C/B, B/C).
The table shows medians of those process averages, in milliseconds per operation.

| Workload | Nodes | Baseline ms | Candidate ms |
|---|---:|---:|---:|
| Unsaved layout, reverse chain | 100 | 0.157 | 0.010 |
| Unsaved layout, reverse chain | 1,000 | 23.990 | 0.145 |
| Unsaved layout, reverse chain | 5,000 | 1,254.105 | 0.832 |
| Already saved layout | 100 | 0.155 | 0.002 |
| Already saved layout | 1,000 | 24.069 | 0.008 |
| Already saved layout | 5,000 | 1,255.959 | 0.099 |
| Headless canvas frame | 100 | 0.027 | 0.032 |
| Headless canvas frame | 1,000 | 0.654 | 0.464 |
| Headless canvas frame | 5,000 | 10.903 | 2.394 |
| Host view, including decoding | 100 | 1.152 | 1.038 |
| Host view, including decoding | 1,000 | 12.664 | 10.147 |
| Host view, including decoding | 5,000 | 105.713 | 55.663 |
| Public uncached palette control | catalogue | 0.053 | 0.053 |

At 5,000 nodes the canvas improved about 4.55x and host view about 1.90x. The
reverse-chain layout improvement is specific to the removed repeated-pass worst
case: it must not be generalized to all graphs. The 100-node canvas costs an extra
0.005 ms in this sample, consistent with index construction overhead; small canvases
are not claimed faster. The palette cache avoids repeated projection in the editor,
but the public pure `palette` function intentionally remains uncached.

Across the three 5,000-node samples: canvas baseline 10.882–10.972 ms, candidate
2.329–2.438 ms; host view baseline 104.852–106.636 ms, candidate 55.229–56.599 ms;
unsaved layout baseline 1,251.797–1,254.273 ms, candidate 0.799–0.839 ms.
Initial exploratory runs under a different machine load were slower on both sides;
the repeated alternating runs above are the comparison used here.

No GPU submission, desktop input latency, allocation counting, peak RSS, power-plan
control, physical audio or cross-platform performance qualification is claimed.
The example is opt-in; CI only compiles it and runs correctness tests, per [18](18-ci-and-performance.md).

## Verification

All commands below used Windows x64, the pinned Rust/Cargo versions and unchanged
lock above. Node helpers used Node.js 24.18.0. Local evidence/logs are retained under
ignored `.artifacts/performance/`; they are not product source or a hosted timing gate.

| Command/check | Result |
|---|---|
| `cargo test --workspace --all-features --locked --offline` | FAIL: seven HTTP integration tests timed out; all earlier test binaries passed. This command stops at that failed test binary, so it is not a full-workspace pass. |
| Original-host reproduction: `cargo test -p orange-net --test http a_node_sends_a_request_on_the_documents_connection --locked --offline -- --exact` | FAIL with the same 20-second reply timeout after restoring the two changed host production files byte-for-byte from `af99e1d`; candidate files restored afterward. The failure predates the optimization. Network access is restricted in this execution environment; no transport test was weakened or marked passed. |
| `cargo test --workspace --exclude orange-net --all-features --locked --offline` | PASS: 1,820 reported test passes, zero failures, 18 ignored (aggregate includes doctest/subprocess results). All remaining workspace crates executed. |
| `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings` | PASS |
| `cargo doc --workspace --no-deps --all-features --locked --offline` with `RUSTDOCFLAGS=-D warnings` | PASS |
| `cargo fmt --all --check`; `git diff --check` | PASS |
| `node tools/check-cargo-metadata.mjs --self-test`; `node tools/check-cargo-metadata.mjs` | PASS: 20 policy fixtures and core/UI dependency boundary |
| `node --test tools/*.test.mjs`; `node tools/release.mjs check` | PASS: 17 tests; release configuration remains intentionally disabled |
| Linux/macOS native tests, MSRV and refreshed dependency audit | NOT RUN locally; hosted PR checks remain required before merge |
| Physical rendering, pointer/IME/accessibility and memory qualification | NOT RUN; these remain separate gates |

Focused regressions verify saved/moved layout retention, node add/remove selection,
edge-order-independent layout, cycles/dangling endpoints, port side/type/row lookup,
curve crossing/bending/stroke culling and actual painted-wire movement, cache search
and registry replacement, connect/disconnect view equivalence, and bounded event
polling with a queued Finished event. Self-review checked the index lifetimes,
deterministic fallback, culling conservatism, public re-exports and unchanged graph
authority/format. No independent review or passed hosted check is claimed here.

## Editing through a retained draft (2026-11-14; MAINT-CORE)

The editor and the host now edit one retained `orange_host::Draft` instead of passing
document text through every operation. A draft owns the parsed `GraphDocument`, model
transaction inverses for undo/redo, and the `AnalysisSession` of [16](16-incremental-validation.md):
`Host::problems` hands the session every committed change set, including undo and redo,
so validation follows the edit instead of reparsing and reanalysing the document. Text
is produced only to save, copy, or hand the graph to a flow. Each host operation is one
transaction and one undo step; a node move is its own undoable layout edit. A refused
wire is withdrawn without a history entry (its revision advance stays). When the host
has graph-implemented nodes installed, the draft is linked and checked in full, as
before, because the linked form is not an edit of the draft.

`cargo run --release -p orange-editor-ui --example performance` on the local Windows
machine (Rust 1.99.1), one value edit followed by its problems and view, mean of 3:

| Nodes | Text in/out (parse, edit, write, check, view) | Draft (edit, follow problems, view) |
|------:|---------------------------:|-----------------:|
| 100   | 3.27 ms  | 0.41 ms |
| 1,000 | 31.2 ms  | 3.33 ms |
| 5,000 | 156 ms   | 20.3 ms |

The draft column still includes a whole-graph view projection (`draft_view`), which is
now the largest remaining cost of an edit. Host tests check after each edit, undo and
redo step that the followed problems equal a from-scratch `check` of the draft's text.

## Remaining costs identified by source inspection

1. Each edit still rebuilds the whole `GraphView` and the names of every node. A view
   that follows the draft's change sets would remove the last whole-graph step of an edit.
2. Undo keeps up to 200 model inverses, bounded by count rather than bytes.
3. Canvas index construction and node/wire traversal still scale with the whole graph.
   A retained spatial index and revision-aware geometry cache could reduce pan-only
   work further. The current public mutable `GraphView` has no revision contract, so
   silently caching by pointer or node count would render stale ports after edits.
4. Palette and properties widgets are not fully virtualized; the bounded log still
   lays out its retained lines. Measure expanded large catalogues before changing
   widget/focus/accessibility behavior.
5. Observed flows in `orange-host/src/flow.rs` compare all visible ports and variables
   after each turn; variable values are cloned before equality checks. Dirty/version
   tracking is a separate runtime/observation change requiring ownership and update
   regressions. No runtime throughput improvement is claimed from this editor work.

These remaining items are analysis findings, not completed optimizations or measured
regressions. The implementation does not relax validation, change connection rules,
or remove bounded runtime resource checks to obtain the gains above.
