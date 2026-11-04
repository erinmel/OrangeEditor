**Authoring, import and ownership verification — 2026-11-01**

This is durable verification evidence for the corrections to the merged hardening work. It records experiments and their practical limits, rather than declaring the core, UI or NFR gates complete. The fixed host example and importer are separate corrections; this record owns the formerly missing B0/B1/R6 measurement work.

The source base is dev `685456030e3553097154587fcffe4cd2c9de6990`, with the corrected import budget and host example integrated for qualification. Toolchain: Rust/Cargo 1.99.1, Windows x86_64 MSVC, default workspace features, unchanged lockfile SHA-256 `15E47ED4DCDA0C445F81D54E367A1291E509AF4937569F4AD694AFB8E122A1E4`. The declared MSRV is 1.97.1; exact-head CI supplies its compilation and native platform checks. Linux/macOS timing and reference-machine/RSS qualification were not run locally.

Measurement machine: AMD Ryzen 9 9900X3D, 12 physical cores / 24 logical processors; 33,439,916,032 bytes reported physical RAM; Windows 11 Home 10.0.26300 (build 26300). Runs used the release profile and the existing counting allocator. Thermal isolation, reference-class qualification and confidence intervals were not established; these are comparative development observations.

**Reproducible commands and method**

The source for the profiles lives in the development-only `orange-measure` crate: `index_measure.rs`, `import_measure.rs`, `lifecycle_measure.rs` and `authoring.rs`. Build before timing, then run profiles serially, without another build or benchmark running:

```sh
cargo build --release -p orange-measure --locked --offline
cargo run --release -p orange-measure --locked --offline -- --warmup 5 --runs 15 index-update
cargo run --release -p orange-measure --locked --offline -- --warmup 3 --runs 15 authoring-compare
cargo run --release -p orange-measure --locked --offline -- --warmup 3 --runs 15 import-profile
cargo run --release -p orange-measure --locked --offline -- --warmup 2 --runs 7 ownership-memory
cargo run --release -p orange-measure --locked --offline -- --warmup 2 --runs 7 session-edit
cargo run --release -p orange-measure --locked --offline -- --warmup 2 --runs 7 w9
```

`--offline` requires already cached exact dependencies. On an online fresh development setup, first populate the cache with the locked dependencies. The W9 profile always uses its fixed 1,000 seeded edits plus its fixed additional edit mix, independent of the generic sample-count arguments. No heavy profile runs as part of ordinary workspace unit tests.

For Windows whole-process working-set observations, PowerShell 7 can launch the already-built binary with [measure-memory.ps1](../tools/measure-memory.ps1), recording stdout/stderr, binary digest, exit code and sampled PeakWorkingSet64. Example:

```powershell
pwsh -NoProfile -File tools/measure-memory.ps1 -Profile authoring-compare -Runs 15 -Warmup 3 -Label pair1
```

Each invocation is a separate child process. The sampler polls every 10 ms, including fixture construction and teardown. This is observed Windows working set, not allocator bytes or Linux RSS; brief terminal peaks may escape observation. Nonzero child exit fails the wrapper. No release dependency or core process launcher is introduced.

Before/after index comparison uses the *same measurement source*, with only `orange-model/src/index.rs` replaced by its pre-membership version from e9aead5 (Git blob `9ee3c28b6cca0b1ce3fa9bc6ef9fd139e01ab253`) in an isolated checkout. To reproduce from a checkout containing these tools, create an owned baseline worktree at that checkout's HEAD, then `git -C .artifacts/index-before restore --source=e9aead5 -- app/crates/orange-model/src/index.rs` and build its release binary. Do not restore that file over a working checkout with unrelated edits. Import comparison restores only `codec.rs`, `error.rs` and `lib.rs` from 6854560 in its isolated baseline; both import variants run identical encoded fixtures.

Three independent paired processes were collected for index and import, alternating baseline/current order in the second pair. Authoring contains both candidates in each process; pair 2 sets `ORANGE_MEASURE_OWNER_FIRST=1` to reverse their order. Tables show the range of process p50 values and the worst observed process p95, rather than treating 45 samples from one process as three replications. Untimed oracle checking follows every index update; scenario full-analysis comparisons are outside the reported worker-update timer.

**Worker comparison and selected direction**

Both candidates retain AnalysisSession. One keeps the editable document on the caller and sends immutable document snapshots plus journal receipts; the other applies the same ordered commands under the background document owner. Both use capacity-one command/reply channels and preserve every accepted command. They do not silently coalesce commands or assume a skipped receipt is continuous.

Caller-work time includes document edits/snapshot construction and nonblocking admission attempts; it excludes explicit waits for backpressure, which remain inside total burst and newest-result latency. Newest latency starts before preparing the last requested edit. Worker time includes its own edit/analysis and snapshot retirement. Single-edit trials include the first edit after initialization, including lazy document-directory construction; they are not a warmed pure-validation microbenchmark. Every trial starts from an equivalent fresh graph.

The final comparison uses **zero-diagnostic chains**. The earlier chain experiment retained a literal under every wired input, producing 1,999/9,999 shadowed-literal warnings. Those graphs were valid with warnings; their approximately 105 MiB session observation was not a clean diagnostic baseline. The clean comparison removes those overridden literals. Warning-bearing and error-bearing memory are recorded separately below.

Times in microseconds; retained heap is the maximum additional requested heap relative to the original document/registry baseline (not process RSS). Both extra-retained figures include the session and thread/channel ownership; the snapshot candidate also retains its additional document image.

| Nodes | Burst | Candidate | Caller p50 range | Newest p50 range | Worst newest p95 | Whole-burst p50 range | Extra retained MiB |
|---:|---:|---|---:|---:|---:|---:|---:|
| 2000 | 1 | snapshot-session | 2218.2–2562.2 | 3387.1–4021.4 | 4575.6 | 3387.1–4021.4 | 22.58 |
| 2000 | 1 | document-owner | 5.8–5.9 | 465.7–615.8 | 996.9 | 465.8–615.8 | 15.58 |
| 2000 | 32 | snapshot-session | 55082.4–60605.9 | 2155.5–2610.7 | 4363.7 | 56379.8–62302.3 | 22.58 |
| 2000 | 32 | document-owner | 63.0–85.9 | 106.0–140.3 | 433.8 | 2208.2–2959.0 | 15.58 |
| 10000 | 1 | snapshot-session | 12702.1–14420.5 | 19973.9–22644.2 | 29277.7 | 19973.8–22644.2 | 108.54 |
| 10000 | 1 | document-owner | 5.7–5.9 | 2613.9–2887.9 | 3218.6 | 2613.8–2887.8 | 76.50 |
| 10000 | 32 | snapshot-session | 376446.2–407469.9 | 19184.8–21892.0 | 31377.9 | 389530.2–429321.2 | 108.54 |
| 10000 | 32 | document-owner | 99.0–109.9 | 253.2–263.3 | 434.6 | 6991.4–7037.1 | 76.50 |

In each measured 32-command burst, the host publication guard publishes only the final current requested sequence and suppresses the 31 older acknowledgements; worker-local freshness alone is insufficient. The functional scenario checks compare the retained session to full analysis across undo/redo, layout no-work, missing and foreign receipts, registry generation, host effects, cancellation and recovery. A deterministic pre-analysis gate also verifies an accepted job produces the actual AnalysisCancelled diagnostic during shutdown, then joins. This is pending-job cancellation evidence, not a worst-case interruption bound inside an arbitrary validation pass or trusted callback.

The measured direction is **a document-owning authoring worker with retained analysis and bounded publication** for command-driven editing. It removes repeated full clones and is materially cheaper for caller work and burst heap. Preserve the existing snapshot API for integrations that deliberately own/read immutable document images. Persistent/shared document internals are not justified by this experiment: the simpler owner alternative already removes the measured clone cost.

The experiment publishes small fixed metadata acknowledgements. A real editor still needs its visible graph/diagnostic projection, bounded history/bytes, nonblocking scheduling and renderer/accessibility qualification. The source example demonstrates 64 recent history entries and one active run; this record does not freeze a production host protocol or pass the UI gate. Session retirement/join can take milliseconds and belongs away from drawing/input.

**Function-index comparison**

Each indexed fixture contains actual function-chain edges. Shapes cover a small four-node function, 64/128 unrelated functions of 16 nodes, and one 1,024-node function. Root and function-local edits, wholesale replacement/removal/restoration are distinct operations. Transaction application plus DocIndex::update are timed; fixture/index setup, reset between remove/restore samples and fresh-build equality checks are excluded. Endpoint incidence remains ordered; provider/graph semantics are unchanged.

Times in microseconds. Heap is the additional index heap from membership bookkeeping, excluding the already-recorded +48-byte inline DocIndex change. The root fixture here contains nodes without unrelated root edges; it differs from the earlier temporary root-chain probe, so its speedup ratios should not be substituted for that historical shape.

| Case | Before p50 range | Membership p50 range | Worst p95 before / after | Added index heap bytes |
|---|---:|---:|---:|---:|
| root-10k-large-function-1024/function-local-edit | 2.7–2.8 | 2.6–2.6 | 6.9 / 6.4 | 70740 |
| root-10k-large-function-1024/function-remove | 294.9–298.9 | 331.7–336.0 | 340.3 / 405.6 | 70740 |
| root-10k-large-function-1024/function-replace | 1113.8–1130.1 | 1255.3–1261.6 | 1189.2 / 1327.7 | 70740 |
| root-10k-large-function-1024/function-restore | 829.7–839.7 | 919.5–925.3 | 1404.4 / 975.7 | 70740 |
| root-10k-large-function-1024/root-edit | 2.5–2.8 | 2.5–2.7 | 6.4 / 6.4 | 70740 |
| root-10k-many-functions-128x16/function-local-edit | 2.8–2.9 | 2.7–2.8 | 7.4 / 7.8 | 188688 |
| root-10k-many-functions-128x16/function-remove | 28.9–31.9 | 6.9–7.0 | 42.8 / 17.9 | 188688 |
| root-10k-many-functions-128x16/function-replace | 42.8–43.9 | 20.1–20.2 | 56.8 / 24.7 | 188688 |
| root-10k-many-functions-128x16/function-restore | 56.3–57.8 | 33.8–34.0 | 69.8 / 38.9 | 188688 |
| root-10k-many-functions-128x16/root-edit | 2.6–2.8 | 2.6–2.6 | 6.9 / 8.7 | 188688 |
| root-10k-small-function/function-local-edit | 2.4–2.6 | 2.3–2.4 | 2.9 / 4.7 | 660 |
| root-10k-small-function/function-remove | 6.6–6.7 | 1.6–1.7 | 19.4 / 1.9 | 660 |
| root-10k-small-function/function-replace | 9.5–10.1 | 4.7–4.8 | 15.3 / 8.7 | 660 |
| root-10k-small-function/function-restore | 8.6–8.9 | 3.6–3.8 | 11.6 / 5.9 | 660 |
| root-10k-small-function/root-edit | 2.5–2.6 | 2.4–2.5 | 2.8 / 7.7 | 660 |
| root-1k-many-functions-64x16/function-local-edit | 2.7–3.0 | 2.8–3.1 | 6.8 / 8.0 | 94352 |
| root-1k-many-functions-64x16/function-remove | 12.8–13.6 | 6.3–6.4 | 27.7 / 11.4 | 94352 |
| root-1k-many-functions-64x16/function-replace | 25.3–25.8 | 19.3–20.4 | 37.2 / 25.3 | 94352 |
| root-1k-many-functions-64x16/function-restore | 30.1–31.3 | 23.5–23.9 | 41.3 / 34.1 | 94352 |
| root-1k-many-functions-64x16/root-edit | 2.9–2.9 | 2.9–2.9 | 7.0 / 7.7 | 94352 |
| root-1k-small-function/function-local-edit | 2.1–2.2 | 2.1–2.2 | 2.3 / 7.5 | 660 |
| root-1k-small-function/function-remove | 1.7–1.8 | 1.4–1.4 | 1.9 / 4.2 | 660 |
| root-1k-small-function/function-replace | 4.5–4.7 | 4.4–4.4 | 4.8 / 4.5 | 660 |
| root-1k-small-function/function-restore | 3.7–3.8 | 3.3–3.4 | 3.9 / 3.6 | 660 |
| root-1k-small-function/root-edit | 2.1–2.2 | 2.2–2.2 | 2.3 / 2.3 | 660 |

Small-function work with unrelated contents improves, while bulk replacement/removal of the 1,024-node body is about 12–14% slower. The added membership bookkeeping is paid per member, and the original whole-map scan is comparatively cheap when the changed body itself is large. This meets the plan's requirement to investigate a repeated regression over 10%: a private unordered-set experiment was also measured and failed to remove the bulk cost, so it was rejected and the proven ordered representation retained.

The decision is an explicit local tradeoff: keep function-local lookup/update benefits without claiming universal speedup, and report the large-body cost. Ordinary root-edit p50 remains around a few microseconds in this fixture. The largest-body p95 is reported directly, not used to declare NFR-15 passed; real analysis/publication costs remain separate.

**Import comparison**

Encoded input exists before the measurement window. Import timing ends when parsing returns; allocator rows include result drop where named by the harness. The million-value fixture also contains 1,000 structurally valid measurement nodes; import does not imply those custom nodes have executable providers.

Times in milliseconds. Peak/live are *additional requested allocator bytes*, excluding input construction.

| Profile | Encoded bytes | Before p50 range | After p50 range | Before → after | Peak before / after bytes | After retained bytes |
|---|---:|---:|---:|---|---:|---:|
| node-heavy | 142143 | 1.458–1.467 | 1.447–1.518 | accepted → accepted | 1648244 / 1648284 | 1088340 |
| key-heavy | 292602 | 3.747–3.761 | 3.752–3.832 | accepted → accepted | 3951758 / 3951798 | 3165040 |
| value-heavy-escaped | 453482 | 2.172–2.249 | 2.077–2.196 | accepted → accepted | 2115444 / 2377628 | 1350874 |
| extension-array-262k | 666458 | 14.151–14.886 | 14.501–14.877 | accepted → accepted | 28352239 / 28352279 | 19963089 |
| extension-array-1m | 2142170 | 50.743–52.793 | 6.245–6.369 | accepted → rejected | 110141167 / 442 | 0 |
| rejected-node-quota | 142143 | 0.193–0.196 | 0.196–0.198 | rejected → rejected | 300 / 340 | 0 |

Default JSON budgets intentionally reject the million-element case before document materialization. A smaller array remains admitted and retains memory; hosts can tighten or explicitly raise profiles. Accepted shapes incur the small additional counting work shown by the paired results. The old importer, input bytes, node counts and decoded-JSON budgets are separate boundaries; this is not a hard process-memory ceiling or a claim that native providers are isolated. All measured parse results returned to their pre-import baseline after drop.

**Current session attribution**

Run [measure-session.ps1](../tools/measure-session.ps1) with PowerShell 7, for example `pwsh -NoProfile -File tools/measure-session.ps1 -Warmup 1 -Runs 3`. It creates a uniquely named ignored source copy, records source/lock/toolchain hashes, injects consuming partition helpers **only into the copied source**, and adds a bin to the copied development package. It never modifies the original model API. The short ignored target directory avoids Windows linker path limits. The helper Rust source is retained at [session-memory.rs](../tools/session-memory.rs); no new dependency or unsafe implementation is introduced.

The graph's lazy node directory is warmed before the document/registry baseline. Clean cases have no diagnostics; warning cases retain overridden literals; error cases add one isolated unknown definition to an otherwise clean graph. Three measured constructions follow one warmup for each case, and all partitions return to the warmed baseline after drop.

| Nodes | Mode | Diagnostics | Additional session live bytes | Peak additional bytes | Local-cache drop attribution bytes | Final residual bytes |
|---:|---|---:|---:|---:|---:|---:|
| 2000 | clean | 0 | 16336309 | 20217817 | 10143517 | 0 |
| 2000 | warnings | 1999 | 22301610 | 24041559 | 12426375 | 0 |
| 2000 | error-bearing | 1 | 16340753 | 20222045 | 10143517 | 0 |
| 10000 | clean | 0 | 80208349 | 99090425 | 50726365 | 0 |
| 10000 | warnings | 9999 | 110066834 | 119754895 | 62145223 | 0 |
| 10000 | error-bearing | 1 | 80216169 | 99098029 | 50729741 | 0 |

Partition figures are owner-drop attribution, not exclusive field sizes. Drop order is published signatures, published diagnostics, report remainder/freshness, retained arguments and diagnostic buckets, retained metadata, local validation, index, effects and session tracking. Shared storage is released by its last surviving owner. Temporary boxing/vector handles add 2,696 bytes; the container later releases 1,024 bytes, and individual partition releases include their box storage (particularly noticeable for empty diagnostic buckets). For 10k nodes:

| Partition | Clean bytes released | Warning-bearing bytes released | One-error bytes released |
|---|---:|---:|---:|
| published.signatures | 10998648 | 10998648 | 10998648 |
| retained.args | 6332568 | 6332568 | 6332568 |
| published.diagnostics | 24 | 3979626 | 1158 |
| retained.inputs | 24 | 12201426 | 24 |
| retained.metadata | 263960 | 263960 | 264152 |
| session.local_validation | 50726365 | 62145223 | 50729741 |
| session.index | 11705456 | 11705456 | 11705456 |
| session.tracking | 182536 | 2441159 | 183872 |

Local validation remains the largest retained category; warning retention materially changes the total and diagnostic buckets. These data identify a possible sharing/compaction target but do not justify deleting correct caches or introducing persistent document internals. That would require its own equivalent-behavior candidate and repeated measurements.

**Snapshot and wide-Fork ownership**

The independent 10k-node graph/registry fixture retains about 30.75 MB; cloning its document retains/reclaims 30,581,807 bytes. The independent-node session holds about 53.15 MB after construction, with about 63.70 MB peak additional heap. This is a different topology from the connected attribution graph; do not compare their totals as an optimization.

The lifecycle profile runs **100 races of 4,000 branches in the same Run**. Across the three processes, the measured stage counts are stable: 517,126,544 cumulative requested bytes per Run, 7,723,004 peak additional bytes and about 1,942,404 retained bytes once ordinary graph work is idle. Request-stop allocates one 8-byte terminal-history growth; draining stop and disposal allocate zero in this fixture. Disposal releases up to 1,942,401 bytes and dropping the disposed Run releases its remaining 8-byte history buffer. The harness verifies the total is 100 and no timers remain, then measures stop/drain/dispose/drop separately on that same run. Cumulative churn is not steady footprint or a demonstrated leak. Pool reuse remains a potential separately measured optimization.

**Windows whole-process observations**

The wrapper records the *whole child process*, including fixtures, all cases in that invocation and teardown. Authoring candidates execute in one process, so these working-set peaks cannot be assigned separately to one candidate.

| Profile family | Observed peak working set MiB |
|---|---:|
| clean authoring comparison | 225.70–229.12 |
| import, before | 111.70–112.62 |
| import, after | 33.79–33.86 |
| snapshot/session + wide Fork | 132.84–133.11 |
| edit matrix | 225.59 |
| W9 | 284.24 |

**Current edit matrix and full-scope costs**

These breadth checks are one current-source process per profile, not the three paired comparisons above. The session-edit invocation used two warmups and seven measured samples. Its p95 values in milliseconds are:

| Edit | 1,000 independent ten-node chains | One connected 10,000-node chain |
|---|---:|---:|
| literal | 0.118 | 0.080 |
| disconnect/connect | 0.122 | 0.103 |
| type pin | 0.125 | 0.083 |
| config | 0.125 | 0.088 |
| add/remove node | 0.487 | 109.939 |
| disconnect/connect beside an input error | 0.132 | 0.071 |
| disconnect/connect beside an incomplete node | 0.246 | 178.619 |
| disconnect/connect into an export | 0.142 | 4.758 |
| disconnect/connect after full analysis | 0.640 | 7.657 |
| disconnect/connect beside a zone | 0.127 | 0.062 |

For 1,000 Set nodes with nine-Add chains, exec-edge p95 is 5.628 ms with one activation and 0.253 ms with 100 activations; exec-node p95 is 5.721 ms and 0.592 ms respectively. Fast literal edits do not establish fast structural edits or error recovery on every topology.

W9 keeps its fixed 1,000 seeded mixed edits, plus separate 250-edit exec/body/reconnection categories and 20-edit cycle categories. The reconnection category also includes reconnect operations paired with disconnects; category counts are not a disjoint partition of the 1,000 seeded edits. Times below are milliseconds:

| Shape (nodes / edges) | Seeded p95 / p99 | Connect p95 / p99 (2026-11-03) | Exec-edge p95 | Body-literal p95 | Cycle-close / open p95 |
|---|---:|---:|---:|---:|---:|
| clustered (10,014 / 18,004) | 0.284 / 0.781 | 0.353 / 1.100 | 1.002 | 0.244 | 194.578 / 194.050 |
| connected (10,014 / 18,004) | 0.430 / 2.197 | 0.540 / 1.497 | 0.862 | 0.203 | 1,532.401 / 1,534.615 |
| clustered (11,014 / 20,004) | 0.354 / 0.641 | 0.400 / 1.072 | 1.008 | 0.344 | 175.957 / 173.400 |
| connected (11,014 / 20,004) | 0.488 / 2.325 | 0.957 / 2.164 | 0.927 | 0.196 | 1,799.361 / 1,749.163 |
| shared Get (10,024 / 20,004) | 0.290 / 0.637 | 0.440 / 1.260 | 0.986 | 0.232 | 189.191 / 171.752 |

The connect column is from a later run with the two-way data-order repair ([10](10-tooling-and-sources.md)); the other columns are from the earlier run. That run's connect maximum on connected 20k is still 16.6 ms: one reconnect whose two sides each span about 5,000 nodes, one of which a kept total order must move.

Node edits (2026-11-03): per four seeded edits, W9 also adds an add, alone (112 edits) or reading a random cluster add in the same transaction (138), removes a random added add (250), and removes a cluster add other than its last with its edges and restores them (250 each). p95 / p99 in milliseconds, before and after node edits kept the inference solution, in one session; every node edit was scoped in both runs, re-resolving up to 11,001 nodes before and at most 10 after. The node edits draw from the second seed, so the other non-seeded kinds now follow a different sequence than in the rows above; the 1,000 seeded edits are unchanged.

| Shape | Add alone | Add with an edge | Added remove | Chain remove | Chain restore |
|---|---:|---:|---:|---:|---:|
| clustered | 0.451 / 0.657 → 0.478 / 0.649 | 0.577 / 0.889 → 0.359 / 0.687 | 0.471 / 0.668 → 0.277 / 0.586 | 0.798 / 0.919 → 0.581 / 0.831 | 0.696 / 0.751 → 0.538 / 0.640 |
| connected | 0.458 / 0.505 → 0.492 / 0.515 | 178.6 / 198.9 → 0.559 / 1.193 | 173.2 / 192.2 → 0.201 / 0.549 | 180.2 / 194.2 → 0.477 / 0.610 | 181.8 / 198.7 → 0.384 / 0.919 |
| clustered 20k | 0.511 / 0.729 → 0.445 / 0.707 | 0.612 / 0.943 → 0.349 / 0.753 | 0.512 / 0.808 → 0.311 / 0.699 | 0.851 / 1.055 → 0.683 / 0.852 | 0.741 / 0.880 → 0.605 / 0.624 |
| connected 20k | 0.555 / 0.623 → 0.498 / 0.552 | 505.5 / 601.1 → 0.623 / 1.701 | 224.0 / 562.8 → 0.223 / 0.521 | 240.5 / 655.8 → 0.507 / 0.708 | 259.5 / 682.4 → 0.438 / 2.212 |
| shared Get 20k | 0.449 / 0.716 → 0.447 / 0.684 | 15.17 / 15.65 → 0.355 / 0.756 | 14.72 / 15.82 → 0.289 / 0.719 | 15.54 / 16.49 → 0.635 / 0.876 | 15.03 / 15.92 → 0.661 / 0.730 |

The seeded edits were scoped in all five shapes, with at most seven nodes re-resolved; every cycle close/open used full analysis. The connected cases' cycle tails reach about 1.8 seconds and need separate profiling before any latency guarantee. Work counters count re-resolved nodes rather than every validation visit, as documented in 16; zero re-resolved during a full rebuild does not mean zero work. These single-process breadth observations do not establish cross-run tail stability, pass NFR-05/15 or replace reference-machine and real-UI evidence.

Zone edits (2026-11-03): every W9 graph now also holds fifty Repeat zones of four body adds (10,314 to 11,314 nodes, 18,454 to 20,454 edges), and a third seed adds body literals, body edge toggles, body node adds and removals, capture toggles and capture retargets. Before re-checking reached zones ([16 §3](16-incremental-validation.md)), on the clustered graph every structure kind ran the full analysis: p95 249 to 371 ms, p99 623 to 726 ms; body literals were already scoped (p95 0.489 ms). After, on all five graphs every zone kind is scoped: p95 at most 0.931 ms (body node add) and at most 0.30 ms for the others, p99 at most 1.34 ms. The before run of the other four graphs was not completed.

Cycle edits (2026-11-04, development machine, not the R machine): closing a data cycle in a cluster and opening it again used to run the full analysis. The root body's data order now holds the edges a cycle runs through, and the session recomputes only the cycles among the nodes between them ([16 §3](16-incremental-validation.md)). Both runs used `cargo run --release -p orange-measure -- w9` on the same source but for this change, in one session per graph. p95 / p99 in milliseconds, before → after; with 20 edits of each kind, both are its slowest edit:

| Shape (nodes / edges) | Cycle close | Cycle open |
|---|---:|---:|
| clustered (10,314 / 18,454) | 285.9 / 285.9 → 0.264 / 0.264 | 271.8 / 271.8 → 0.270 / 0.270 |
| connected (10,314 / 18,454) | 1,785.6 / 1,785.6 → 0.234 / 0.234 | 1,901.0 / 1,901.0 → 0.224 / 0.224 |
| clustered 20k (11,314 / 20,454) | 337.4 / 337.4 → 0.360 / 0.360 | 295.9 / 295.9 → 0.292 / 0.292 |
| connected 20k (11,314 / 20,454) | 2,083.4 / 2,083.4 → 0.284 / 0.284 | 2,054.9 / 2,054.9 → 0.216 / 0.216 |
| shared Get 20k (10,324 / 20,454) | 337.0 / 337.0 → 0.261 / 0.261 | 311.9 / 311.9 → 0.329 / 0.329 |

Every cycle edit is now scoped, re-resolving at most 14 nodes, so no W9 edit of any kind runs the full analysis. No other kind's p95 rose; with no full analysis between edits, the seeded edits' p99 fell from 0.82–3.07 ms to 0.24–0.30 ms and every kind but exec edges (p95 0.91–1.05 ms, p99 at most 1.30 ms) stays under 0.8 ms p95. A cycle in a document with exports, in a function body or in an incomplete report still runs the full analysis.

Visit counters (2026-11-04, development machine, not the R machine): `orange-measure w9` now prints, per graph and edit kind, the p50/p95/max nodes, edges, ports, diagnostics and bodies the session's passes iterated per edit, summed over the passes (`EditStats::visits.total()`, defined per pass in [16 §3](16-incremental-validation.md)), and each graph's full analyses by `ScopeFallback` or rebuild reason. These replace the re-resolved-node proxy above. One `cargo run --release -p orange-measure -- w9` run; nodes iterated per edit, p50 / p95 / max, and diagnostics scanned per seeded edit at p95:

| Shape (nodes / edges) | Seeded edits | Seeded edges p95 | Most nodes, any non-exec kind | Exec edge | Seeded diagnostics p95 | Full analyses |
|---|---:|---:|---:|---:|---:|---:|
| clustered (10,314 / 18,454) | 19 / 32 / 37 | 27 | 83 (cycle close) | 22,167 / 24,223 / 24,572 | 17,152 | none |
| connected (10,314 / 18,454) | 22 / 34 / 42 | 29 | 103 (cycle close) | 22,167 / 24,223 / 24,572 | 17,152 | none |
| clustered 20k (11,314 / 20,454) | 19 / 33 / 34 | 28 | 87 (cycle close) | 24,167 / 26,223 / 26,572 | 19,150 | none |
| connected 20k (11,314 / 20,454) | 22 / 34 / 39 | 29 | 107 (cycle close) | 24,167 / 26,223 / 26,572 | 19,150 | none |
| shared Get 20k (10,324 / 20,454) | 19 / 33 / 34 | 28 | 87 (cycle close) | 22,187 / 24,243 / 24,592 | 19,150 | none |

Literal and type-argument edits iterate at most 19 nodes and scan no diagnostics; every kind but exec edges stays at or under 107 nodes and 299 edges, about 1% of each graph or less. Summed over passes, every non-exec kind ran in two to seven body-runs, all in the edit's own body. Two kinds of work still scale with the graph:

- Every edit that adds or removes an edge checks whether it carries a published diagnostic by scanning the published list, which on W9 holds about 17,000 to 19,000 warnings, so a seeded wire edit scans all of them and a chain-node removal with four edges scans up to 76,664.
- An exec edge edit reruns the state pass in full: it iterates all five bodies and every body's nodes twice, about 20,600 to 22,600 of its nodes. The rest, 1,500 to 4,000, is the activation's region, control-cycle and availability reruns and the merge with the retained region index, which scans the whole index. Its 19 body-runs are the state pass's five, five each for the region and control-cycle passes, which run over the exec subset once in every body, and one each for the exec walk, the edge check, resolution and inputs.

Neither cost has been profiled against the edits' time. Timing did not regress: in one session, `origin/dev`, this branch and `origin/dev` again ran W9 back to back, and no kind's p95 rose by more than 0.007 ms over the slower dev run; the largest p95 of any kind was 0.965 ms against 1.091 ms on dev.

**Qualification and remaining product gates**

Host example and importer behavior are qualified by focused regressions, the combined debug/release workspace suites, warnings-denied Clippy/Rustdoc, dependency policy and exact-head platform/MSRV/security CI. Authoring newest-publication/cancellation scenarios also have small ordinary unit tests, so CI does not rely on executing a heavy performance profile. Exact final counts and source/CI identities are recorded in the correction PRs and 10.

This closes the hardening review's missing comparative measurement work with a bounded, reproducible experiment and explicit tradeoffs. Real UI publication cost, general host services/packages, trusted-callback interruption and reference-machine NFR gates remain product work with their existing prerequisites; they are not inferred from these measurements.
