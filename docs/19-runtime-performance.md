# Runtime storage and repeated input admission

Implemented optimization cohort, 2026-11-05. FR-12/14/16 and NFR-01/05;
ADR-16/21 boundaries remain unchanged. This is measured executor work, not a
passed reference-machine performance gate or a universal sub-1 ms guarantee.
The measurement window and qualification policy remain in [18](18-ci-and-performance.md).

## Evidence that selected the work

The [PR126 execution artifact](https://github.com/erinmel/OrangeEditor/actions/runs/42109192411)
contains timing, separate allocator measurements and build/machine provenance for
source `42fb77f0eafd7f457e5989d09f7bb34ece202a46`. Its source tree matches the
squash-merged baseline `fdec574c1b7ab43fa48f88c2a02b065dbcfb9268`.
The user-supplied timing.json matches the artifact byte-for-byte (SHA256
`5AE208DFD799D5F8BBF6BEA0892D262C07D4A4054C59A797668EEF1346B10362`).
The artifact's machine is a four-logical-CPU Linux EPYC 9V45 VM; the local
comparisons use the user's Ryzen 9900X3D Windows machine. Absolute times from
these two machines are not baseline/candidate speedup comparisons.

The widest Fork race does 100 iterations of 4,000 branches in one measured session,
not one 4,000-way operation. It requests 517,020,701 cumulative allocation bytes in
133,074 allocations, with 7,696,525 peak additional live bytes. That is churn,
not 517 MB simultaneously resident or proof of a leak. Large map CAS and host-map
input use the same allocation counts as their small-map variants; their higher
time points toward repeated content walks rather than copying whole maps.

## Implemented changes

- Store regions in reusable `Vec<Option<(RegionId, Region)>>` slots; hash only identities to
  small physical indices. Up to eight live regions use a bounded linear lookup
  without hashing. Once promoted, the directory stays active until clear/dispose,
  so sparse vacant storage after a wide Fork never becomes a long linear search.
  Logical identities remain monotonic and are never
  recycled. Removed regions drop their contents and their slots become reusable.
  The directory and slots follow peak live ownership, including repeated loops.
  Sorted root enumeration and ordered child sets preserve deterministic traversal.
- Reserve each already-admitted Fork's region batch without geometrically growing
  large region values, and reuse a drained child buffer between Forks. Pump-work
  charging, branch order, join semantics and cleanup ownership are unchanged.
  Disposal releases the slots, directory, free list and child buffer.
- Reuse the nominal type/size charge of an input map/list only if its fixed typed
  register still holds the exact same immutable allocation. No extra cache or
  value retention is added. A new allocation, different register or fault uses
  normal admission. Current limits and aggregate publication checks still run.

All changes use safe Rust and existing standard collections. No dependency,
lockfile, public API, saved graph, provider binding, quota or replay rule changes.

## Algorithms and alternatives considered

The implementation uses an identity directory plus reusable physical slots and
per-register immutable identity reuse. It does not implement a new allocator.
[Hanson's lifetime-based allocation paper](https://drhanson.s3.amazonaws.com/storage/documents/fastalloc.pdf)
motivates separating storage reuse from logical object lifetime.
[Berger, Zorn and McKinley's allocator study](https://people.cs.umass.edu/~emery/pubs/berger-oopsla2002.pdf)
also explains the memory-retention cost of arenas without individual reclamation.
The [2026 revisit](https://arxiv.org/abs/2605.17119v1) studies locality on modern
allocators; its results are motivation to measure, not predicted OrangeEditor gains.
[SwissTable design](https://abseil.io/about/design/swisstables) and the selected
[Rust 1.99.1 standard HashMap documentation](https://doc.rust-lang.org/std/collections/struct.HashMap.html)
support using the existing standard table for the small directory.

A direct `HashMap<RegionId, Region>` experiment reduced the initial local Fork
median from 129.8 to 107.1 ms but increased peak incremental heap from 7.7 to 16.1 MB;
it was rejected. Separating small directory entries from dense region storage
removed that large-bucket cost. A subsequent always-hashed dense version made
several small serial workloads slower in a complete paired comparison. The final
hybrid keeps bounded small trees free of hashing. Indexing a growing Vec directly by the monotonic
logical ID was also rejected: serial loops must not retain storage for every
region ever created. Faster custom hashers, unsafe arenas and new dependencies
were unnecessary for this cohort.

## Verification procedure

Local: Rust 1.99.1 (`58cf8f9c3`, LLVM 23.1.1), Cargo 1.99.1, Windows 11 Home
10.0.26300 x86_64 MSVC, Ryzen 9 9900X3D (12 cores/24 threads), approximately 31 GiB
visible RAM, High performance power plan. Lock SHA256 remains
`8BB91CAAFF5A7106E5752D49D9AB285D4566A1B69B55AD1CA81E10E1D70BB068`.
Portable builds use `RUSTFLAGS=-C target-cpu=x86-64`, `CARGO_INCREMENTAL=0`,
`--release --locked`; timing binaries use `--no-default-features` and allocation
binaries use the default counting feature. Executables are copied before building
the other mode. Compilation and tests finish before paired measurements.

Each pair runs independent serial baseline/candidate processes over all 12 unchanged
workloads with `--execution --warmup-seconds 10 --runs 100`. Three pairs reverse
process order in the middle pair. Each session checks its expected result. Raw
per-process samples, binary hashes, compiler/lock/machine/power provenance and
separate three-session allocation reports are retained locally in ignored
`.artifacts/runtime-optimization-2026-11-05/`. Normal desktop background activity
and scheduling are uncontrolled. These samples assess this optimization, not a
qualified tail estimate; the older physical x64 reference and >=1,000-session
tail procedure in 18 remain unexecuted. Independent processes and reversed order
help reveal variation; they do not eliminate code-layout or scheduler bias.
[Kalibera and Jones](https://kar.kent.ac.uk/33611/) discuss variation at multiple
measurement levels; [STABILIZER](https://people.cs.umass.edu/~emery/pubs/stabilizer-asplos13.pdf)
explains why repeated timings of one build do not remove layout bias. Neither
formal methodology is claimed as implemented by these exploratory comparisons.

Regression coverage includes stale region IDs after 10,000 physical-slot reuses,
another 1,000 wide batches across promotion, clear and directory-backed reuse,
sorted roots after removal/reuse, map/list input rejection without invalidating
the prior evidence, tighter resource limits, fault replacement, and copy-on-write
type changes. Equal-content copies are revalidated; a disposed wide Fork must
retain less than 1 KiB before the Run itself is dropped. Existing independent Fork/run oracles, budgets, disposal, and
provider-freshness tests remain. The 256-branch 100-iteration measurement now
requires less than 8 MiB cumulative allocation while still spending at least 200
pumps; the old implementation requested about 33 MB. This is a broad deterministic
regression bound, not a wall-time threshold.

Local commands (repository root, committed lockfile, test debug information set to
`line-tables-only`, `CARGO_INCREMENTAL=0`):

```sh
cargo test --workspace --locked
cargo test -p orange-runtime -p orange-measure -p orange-bundled --release --locked
cargo test -p orange-measure --no-default-features --release --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo fmt --all --check
git diff --check
```

PASS: 1,209 debug tests, 15 existing ignored; 485 selected release tests, 6 existing
ignored; 9 uninstrumented-feature-mode release tests; Clippy, formatting and whitespace.
Test builds retain allocator instrumentation even when default features are disabled.
These release tests cover the changed runtime and its bundled consumers, not a
claim that the entire optimized workspace suite ran locally. Native/MSRV CI is
recorded in the optimization PR. No older-computer qualification was run.

## Paired results

Each value below is the median of three per-process percentiles, each based on
100 complete sessions after ten seconds of warmup. Samples are not pooled across
processes. All 12 outputs and pump counts match baseline in every process.

| Complete workload | Before p50 ms | After p50 ms | Before p95 ms | After p95 ms | p50 change |
|---|---:|---:|---:|---:|---:|
| scalar chain (16 adds) (1000 iterations) | 1.4214 | 1.3746 | 1.4488 | 1.3952 | -3.3% |
| host inputs (384 gets per pull) (100 iterations) | 4.5803 | 4.5850 | 4.6833 | 4.6433 | 0.1% |
| function call + return (1000 calls) | 1.2057 | 1.0814 | 1.2421 | 1.1008 | -10.3% |
| nested for-each (32x32) (1024 inner items) | 0.3938 | 0.3542 | 0.4034 | 0.3621 | -10.1% |
| fork race + cancel region (1000 iterations) | 1.1630 | 1.0222 | 1.1891 | 1.0443 | -12.1% |
| fork race fan-out 256 (100 iterations) | 5.2207 | 5.0427 | 5.3432 | 5.1849 | -3.4% |
| fork race fan-out 4000 (100 iterations) | 130.4533 | 86.8900 | 132.2472 | 88.0792 | -33.4% |
| map CAS (16 entries) (1000 updates) | 0.5870 | 0.4841 | 0.6036 | 0.4960 | -17.5% |
| map CAS (1000 entries) (1000 updates) | 11.6206 | 8.4849 | 12.4821 | 8.6198 | -27.0% |
| host map input (16 entries) (1000 epochs) | 0.1009 | 0.0398 | 0.1039 | 0.0403 | -60.6% |
| host map input (1000 entries) (1000 epochs) | 3.7202 | 0.0436 | 3.8944 | 0.0450 | -98.8% |
| map zone (10000 items) (100000 items) | 14.6047 | 14.6297 | 14.8616 | 14.7492 | 0.2% |

The large-map input gain applies to repeated publication of the same immutable
map, not newly allocated or changed maps. Host-input chains (+0.1%) and map-zone
medians (+0.2%) are effectively flat in these observations; small shifts are not
presented as significant speedups. No formal confidence interval or tail gate is
claimed. The largest Fork session remains far above 1 ms.

Separate allocator windows (three sessions, first-session counts reported):

| Workload | Allocations before → after | Requested bytes before → after | Peak incremental live bytes before → after |
|---|---:|---:|---:|
| Scalar chain | 36 → 37 | 14,098 → 10,586 | 10,646 → 7,134 |
| Function calls | 43 → 44 | 11,160 → 7,648 | 10,332 → 6,820 |
| Nested for-each | 132 → 133 | 19,223 → 15,711 | 9,035 → 5,523 |
| Fork race/cancel | 3,034 → 2,038 | 813,545 → 141,961 | 14,029 → 11,693 |
| Fork 256 | 8,659 → 4,371 | 32,839,133 → 1,075,245 | 493,453 → 405,853 |
| Fork 4,000 | 133,074 → 66,589 | 517,020,701 → 16,385,197 | 7,696,525 → 6,319,021 |
| Map CAS (either size) | 32 → 33 | 7,978 → 4,466 | 7,662 → 4,150 |
| Host map (either size) | 13 → 13 | 585 → 585 | 361 → 361 |
| Map zone | 44 → 44 | 802,469 → 802,469 | 161,213 → 161,213 |

Small serial runs make one additional small allocation while requesting and
retaining fewer total bytes. Fork 4,000 reduces cumulative requested bytes by
96.8% and peak additional live heap by 17.9%. These are requested/live allocator
measurements, not resident-set measurements. Baseline Fork 256 counts varied
between samples; the first count is shown. Candidate counts did not vary in the
three observed allocation sessions. Hash placement can still vary in other runs.

Timing binary SHA256: baseline
`76A3314D35183739453633CA0D4239DFC072BC3D084513E11897A5DE3A0E6C19`, candidate
`BB6DDF8E5DE886B819E27DC7341E824815B11E67784928A329CE5F1172C2A9D8`.
A rebuild after the final test-only additions matched the candidate's machine-code
section byte-for-byte (SHA256
`5504BA8C8659A9E9EE1F37E86ABFDB68B5E2276EDC0B4589C63CCC62607411B5`);
full PE binaries include changing link metadata. Rustdoc with warnings denied also
passed. Benchmark raw files and per-process ranges remain with the local evidence.

Remaining investigation areas include ordered child-set allocation, repeated
variable/projection charge walks in large-map CAS, and capacity growth when live
ownership peaks repeatedly increase. This cohort does not claim those paths are
optimal or that every graph benefits equally.

## Second cohort: storage traded for repeated work

Implemented 2026-11-05 on top of the cohort above. FR-12/14/16 and NFR-01/05; no
public API, saved graph, provider, quota, replay or graph-language rule changes.
The user-supplied CI timing for the first cohort (same EPYC VM class) still showed
the 1000-entry map CAS at 12.3 ms against 0.68 ms for 16 entries and the 4,000-branch
Fork at 109 ms, so these two paths were selected.

- **Map check memo.** Each `MapRepr` allocation remembers its first complete shape
  check as two words: an exact preorder code of the key and value shapes with the
  depth in its top byte, and the entry charge (12 §6.1). Every five-bit tag is
  non-zero and fixes its arity, so equal codes are equal shapes, not a hash match.
  Entries are immutable, so the same shapes at the same or a smaller depth must reach
  the same verdict; the memo answers those checks and still applies the caller's byte
  limit. Failed or partial checks are never stored, other shapes walk again, and
  equality, hashing and debug output ignore the memo. Enum shapes and shapes over
  eleven nodes are not coded and keep walking. A map CAS update previously walked the
  whole map for the cell charge, both operands, projections and publication; now each
  allocation is walked once. This generalizes the first cohort's per-register reuse to
  every checked boundary, including nested maps, without address-keyed caches.
- **Tombstoned ownership spill.** Multi-member `IdSet`s (Fork children, multi-strand
  regions, retained frames) replaced the boxed `BTreeSet` with an ascending identity
  vector plus one removal bit per slot. Minted children append, removal is a binary
  search that sets the bit, and the vector compacts when fewer than half its slots are
  live, so it holds at most twice the live set. Bulk construction sorts once. An
  identity older than the newest member compacts and shifts the tail; ownership code
  inserts minted identities, so that path is rare. Iteration stays ascending.
- **Identity hasher.** Region slots, ready links and the timer directory are keyed by
  sequential run-minted identities that never come from graph, host or network input.
  They now use a fixed multiplicative hasher instead of SipHash. Flooding resistance
  is not needed for keys the run mints itself; external-input maps keep the standard
  hasher.
- **Out-of-line continuations.** A region's waiting continuation (a whole `Strand`,
  336 bytes) moved into a box drawn from a spare pool owned by the region tree.
  Branch, cleanup and detached-join regions never hold one, so every `Region` shrinks
  from 496 to 168 bytes. Resumed continuations return their box to the pool, so warm
  calls and loop bodies still make no allocation; the pool follows the peak number of
  simultaneously held continuations and is released on disposal.

An earlier draft of the memo cloned both `ValueShape`s (72 inline bytes per map and an
allocation for nested shapes); the exact code keeps the same reuse in 24 bytes without
allocating, at about 20 ns per hit for recomputing the probe's code. The first spill
draft stored `(id, bool)` pairs at 16 bytes per slot; the bitset halves that.

Rejected: moving timers into reusable slots so the B-tree stores indices. It did not
change the Fork workloads (the winning branch completes before losers park, so
cancellation only unlinks ready strands) and was reverted rather than kept unmeasured.
Container memos for lists were not attempted: `ListRepr` is a public enum used across
the workspace, and packed scalar lists already charge in O(1).

### Memory cost at expected graph sizes

Target sizes: ordinary graphs of about 30 nodes and 50–100 connections; the largest
expected graphs of about 10,000 nodes and 30,000 connections. NFR-05 allows 750 MiB in
total for the W4 editor case (10,000 nodes / 20,000 edges); the analysis session holds
about 124–129 MiB of heap there today (06). Runtime ownership is bounded by run limits
(default 4,096 strands, 8,192 regions, depth 1,024), not by node count.

| Change | Memory it adds or removes | 30 nodes / 100 connections | 10,000 nodes / 30,000 connections | Speed it buys |
|---|---|---:|---:|---|
| Map check memo | +24 bytes per map allocation (16 → 40) | under 1 KiB | ~240 KiB even with one map literal per node | map admission O(entries) → O(1) per repeat |
| Ownership spill | ~8 bytes per slot, at most 2 slots per live member | none (serial sets stay inline) | bounded by run limits: ≤ ~128 KiB | no per-node tree allocations; Fork 4,000 allocations −98% |
| Identity hasher | 0 | 0 | 0 | 5–9 cheaper lookups per strand |
| Out-of-line continuations | −328 bytes per region; spare boxes ≤ peak held continuations | net reduction | up to −2.6 MiB at 8,192 regions | smaller moves per region open/close |

All four trades are valid for these sizes: the only additive cost (24 bytes per map) is
proportional to live map values, not to history, and stays under 0.1% of the W4 budget
even in the largest expected graph; the other three reduce or bound memory.

Rule for later trades: memory that grows with live data by a few words per node, edge
or value (10,000 × 64 bytes = 640 KiB) is acceptable when it removes a repeated walk;
memory that grows with history (edits, epochs, minted identities) must be bounded or
released; quadratic storage is acceptable only as bits for the largest graphs
(10,000² bits ≈ 12 MiB), never as words (≈ 800 MB).

### Second cohort verification

Same local machine, toolchain, flags, lockfile and commands as above. PASS: workspace
debug tests, release tests for `orange-runtime`, `orange-measure`, `orange-bundled` and
`orange-model`, uninstrumented measure tests, Clippy with warnings denied, formatting
and whitespace. New tests cover memo reuse only after complete checks, limit
enforcement on a memo hit, shape and depth misses, exact shape codes, uncoded large
shapes, memo-blind equality/hashing, and spill compaction past the inline 64 bits,
re-insertion and sorted bulk construction against a `BTreeSet` oracle. Clippy
expectations are scoped to the duplicate-key set in `MapRepr::new` (the memo is
outside `Hash` and `Eq`), the held-continuation deref and the spare box pool.

Paired exploratory timing: baseline is commit `461ca53`, candidate is this working
tree, six independent processes in base/candidate/candidate/base/base/candidate order,
each `--execution --warmup-seconds 5 --runs 30`. Values are medians of the three
per-process percentiles; all outputs and pump counts match.

| Complete workload | Before p50 ms | After p50 ms | Before p95 ms | After p95 ms | p50 change |
|---|---:|---:|---:|---:|---:|
| scalar chain (16 adds) | 1.3798 | 1.3985 | 1.3942 | 1.4373 | 1.4% |
| host inputs (384 gets per pull) | 4.6351 | 4.6046 | 4.6876 | 4.6495 | -0.7% |
| function call + return | 1.0967 | 1.1275 | 1.1392 | 1.1519 | 2.8% |
| nested for-each (32x32) | 0.3605 | 0.3798 | 0.3655 | 0.3962 | 5.4% |
| fork race + cancel region | 1.0373 | 0.9477 | 1.0554 | 0.9724 | -8.6% |
| fork race fan-out 256 | 5.2911 | 2.8071 | 5.3788 | 2.8731 | -46.9% |
| fork race fan-out 4000 | 91.2978 | 44.4408 | 92.9606 | 45.5734 | -51.3% |
| map CAS (16 entries) | 0.4893 | 0.3912 | 0.4955 | 0.4124 | -20.0% |
| map CAS (1000 entries) | 8.5368 | 0.3783 | 8.5944 | 0.3897 | -95.6% |
| host map input (16 entries) | 0.0398 | 0.0400 | 0.0400 | 0.0402 | 0.5% |
| host map input (1000 entries) | 0.0437 | 0.0400 | 0.0438 | 0.0402 | -8.5% |
| map zone (10000 items) | 14.6560 | 14.5488 | 14.8865 | 15.0945 | -0.7% |

Serial workloads move by a few percent in either direction; earlier single-workload
reruns showed host-input differences inside process-to-process spread. These are
pre-v1 exploratory observations, not gated regressions.

Allocator windows (first session; counting build):

| Workload | Allocations before → after | Requested bytes before → after | Peak incremental live bytes before → after |
|---|---:|---:|---:|
| Scalar chain | 37 → 39 | 10,586 → 9,642 | 7,134 → 6,190 |
| Function calls | 44 → 47 | 7,648 → 7,040 | 6,820 → 6,212 |
| Nested for-each | 133 → 136 | 15,711 → 15,103 | 5,523 → 4,915 |
| Fork race/cancel | 2,038 → 2,041 | 141,961 → 91,713 | 11,693 → 10,709 |
| Fork 256 | 4,371 → 1,077 | 1,075,245 → 877,621 | 405,853 → 292,653 |
| Fork 4,000 | 66,589 → 1,899 | 16,385,197 → 13,404,789 | 6,319,021 → 4,545,037 |
| Map CAS (either size) | 33 → 35 | 4,466 → 3,522 | 4,150 → 3,206 |
| Host map, map zone | unchanged | unchanged | unchanged |

The two or three extra allocations in serial sessions are the one-time spare
continuation boxes and pool; warm iterations add none. Smaller regions lower requested
and peak live bytes in every Fork and serial workload; Fork 4,000 peaks 28% lower and
allocates 97% less often. Map memos are created when the plan checks its literals,
outside the measured window. These are requested/live allocator measurements, not
resident-set measurements, and no reference-machine or tail qualification is claimed.
