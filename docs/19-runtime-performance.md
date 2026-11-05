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
