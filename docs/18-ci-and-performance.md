# CI coverage and complete-execution measurements

Selected policy, 2026-11-05. This implements development checks, not a new graph/runtime
contract or a passed product performance gate. Branch and release policy remains in
[14](14-git-workflow.md); NFR-01/NFR-05 and reference workloads remain in
[06](06-quality-and-validation.md).

## Required validation and architecture coverage

| Lane | Trigger | Checks |
|---|---|---|
| Repository rules | Every PR, merge group, dev/main push, manual, weekly; reusable by CD | Format; first-party/core dependency policy and its fixtures; actionlint 1.7.12 including shellcheck on Linux; zizmor 1.30.1; platform-selection and release-contract tests; disabled-release check |
| Rust quality | Same | All-target/all-feature Clippy; rustdoc with warnings denied; measurement binary Clippy without allocator instrumentation |
| Dependencies | Same | Pinned cargo-deny 0.20.2; refreshed advisories, bans, licenses and sources |
| Native behavior | After repository rules | Workspace unit/integration/property/doctests on Linux x64, Windows x64 and macOS ARM64 |
| Optimized behavior | After repository rules, parallel with native tests | Linux release runtime/measurement regressions, both measurement feature modes, twelve execution workloads, example tests and all three headless example programs; release builds reused throughout. Extended weekly/manual runs also execute the full optimized workspace suite |
| Compatibility | After repository rules | MSRV 1.97.1, all workspace targets; weekly and extended manual native tests also cover Linux ARM64 and macOS Intel x64 |
| Required result | After all required jobs | Existing `rust` context fails on failure, cancellation or skipping, including any failed native matrix member |

The selected compiler remains 1.99.1 from `rust-toolchain.toml`. Builds use the committed
lockfile. No dependency or compiler update is part of this change. Ordinary PR validation
has nine jobs rather than ten: format shares the rules runner, Clippy/rustdoc share a
quality runner, and optimized behavior gets a parallel job. Tests wait for rules,
allowing them to overlap quality checks. PRs avoid compiling every integration test twice:
they retain full debug workspace coverage on three native platforms and exercise the
runtime, twelve real execution fixtures and embedding examples in release mode. Full
optimized workspace tests run on extended weekly/manual validation. Full
test debug symbols are replaced with `line-tables-only` in native CI; debug assertions
and overflow checks remain enabled. Dependency caches retain their architecture/toolchain
boundaries and save only on dev. Clean release builds still do not restore build caches.

Each crate's integration tests build as one binary (`tests/main.rs`, `autotests = false`),
with every former test file as a module and shared helpers such as `common` declared
once. 99 separately linked test executables became three, and the harness runs their
tests on one thread pool instead of one binary after another. Locally, with dependencies
built, the workspace test build fell from 19 s to 15 s and the run from about 18–33 s to
9–10 s, with the same 1,211 passing and 15 ignored tests. A new test file must be added as
a `mod` in its crate's `tests/main.rs`; Cargo no longer discovers it on its own.

`tools/ci-platforms.mjs` selects explicit runner labels and Rust host triples. Native jobs
assert the compiler host before testing, so a runner-label architecture change fails
visibly. A successful cross-compilation alone is not execution evidence. Using x64 on the
development machine does not test ARM64; native macOS ARM64 already belongs to the product
matrix. Additional Linux ARM64/macOS Intel checks expand compatibility evidence without
adding shipping targets. Windows ARM64 and other target combinations remain unqualified.
Manual validation's **extended** option defaults to true; ordinary PRs keep three platforms.
Reusable validation defaults to ordinary coverage; Release CD explicitly requests extended
coverage, including the full optimized workspace suite, before building shipping artifacts.

Release CD groups identity/qualification, reusable validation, clean builds, native
signing/install checks and attestation/publication in order. Read-only validation,
non-persisted checkout credentials, SHA-pinned actions, checksum-verified tools and
timeouts remain. Signing/publication authority stays in release environments. The
unqualified application configuration still blocks publication; no signing/packaging
gate has been passed. Branch protections continue to require the existing `rust` context.

## Performance checks selected for the current core

Keep deterministic output, budget, allocation-refusal and disposal regressions in the
required workspace suite. They catch concrete behavior/resource regressions without
depending on VM speed. Do not gate ordinary PRs on an absolute millisecond limit, RSS
sample or a coverage percentage. Named rule boundaries, property tests and regressions
remain the coverage policy; unimplemented services/UI and ignored tests are not covered
merely because this workflow succeeds.

`Execution measurements` runs weekly, manually, and on PR changes to its workflow or
the measurement crate. It builds and runs the existing twelve execution workloads in
release mode on Linux x64. Each workload warms for at least ten seconds and three runs,
then records thirty complete sessions. Assertions still check expected results after
each session. A failed workload fails the measurement job. This workflow is evidence
separate from the required `rust` gate; it has path-filtered PR triggers and must not be
made a required check for every PR without changing that selection.

The measured interval includes session construction, all budget-64 pumps, stop/dispose
and session destruction. Documents and compiled plans are prepared beforehand; returned
outcome checks/destruction are afterward. Waits use virtual time, so external I/O and
physical timer latency are excluded. Workload names, units and pump counts accompany
each result: e.g. a chain with 1,000 loop traversals is one measured session, not a claim
that each traversal takes the session's time. These fixtures are not automatically the
normative W1/W2 workloads or all possible user graphs.

Two separately built feature modes prevent allocator counters from distorting latency:

```sh
# Timing: platform allocator, no per-pump stopwatches, raw complete-session samples.
cargo build --release -p orange-measure --no-default-features --locked
target/release/orange-measure --execution --warmup-seconds 10 --runs 30 > timing.json

# Memory: counting allocator; timing in this output is instrumented.
cargo build --release -p orange-measure --locked
target/release/orange-measure --execution --warmup-seconds 10 --runs 30 > allocations.json
```

On Windows, use `target/release/orange-measure.exe`. An optional final substring selects
a workload, for example `scalar-chain` or `fork-fanout-4000`. A filter that matches no
execution workload fails. Memory-only diagnostic profiles still require default features;
timing-only builds reject those profiles rather than reporting fake zero allocations.
Unit tests retain the counting allocator in both feature modes so allocation regressions
actually execute. No production crate feature or runtime behavior changes.

JSON schema 1 contains per-workload complete-session p50/p95/p99/max and all samples in
milliseconds, plus units, pump counts, warmup/budget/run settings and architecture/OS.
`allocation_counts` is null in the timing binary; the memory binary reports first-run
allocation count, requested bytes, peak incremental live heap and whether counts vary.
Requested bytes include reallocations; the peak excludes documents/plans already owned
before the window. Neither is process RSS or a hard resident-memory bound. Thirty
samples make nearest-rank p99 equal to the maximum; this is descriptive evidence, not
a qualified tail-latency estimate.

The workflow uploads both JSON files, exact checked-out commit, compiler/Cargo versions,
lockfile digest, each binary digest, CPU/OS/RAM and build flags for thirty days. Code uses
portable x64 instructions, not `target-cpu=native`. Partial provenance is retained after
a failure. Cache/toolchain/build changes and differing machines invalidate direct
absolute comparisons; compilation occurs before measurement.

## Testing the 1 ms goal and older computers

Treat 1 ms as an exploratory complete-execution goal for an explicitly named graph and
input size, not a bound for arbitrary graphs, I/O or virtual waits. The current NFR-01
contract is unchanged. Select a physical older x64 reference machine with CPU, RAM and
OS recorded, in addition to the development X3D machine. Reducing cores or throttling
an X3D CPU does not reproduce an older cache hierarchy or memory subsystem.

On each machine, compare baseline and candidate portable release binaries built with
the same compiler, lockfile and flags, serially after compilation. Use at least three
independent paired processes, reverse candidate order, warm each workload for ten
seconds and collect at least 1,000 sessions per process for tail investigation. Keep
raw samples and results; record background load/power state and investigate outliers
instead of deleting them. Compare complete-session p95/p99/max and allocation/peak
changes separately. Compilation and end-to-end host/I/O latency need separate windows.

Before turning results into a gate, agree on the graph/size, machine, percentile and
allowed memory budget, then measure normal run-to-run variation. A dedicated controlled
runner can enforce the resulting threshold; hosted VMs remain trend/evidence runners.
No older-machine result or universal sub-1 ms result is claimed here. Improving the
executor's data structures/ownership costs is a separate measured runtime change.

Implemented runtime optimization evidence, including paired comparisons, allocation
costs, research and rejected alternatives, is recorded in [19](19-runtime-performance.md).

Sources checked 2026-11-05: [GitHub native runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
[Cargo test profiles/debug settings](https://doc.rust-lang.org/cargo/reference/profiles.html),
[pinned rust-cache behavior](https://github.com/Swatinem/rust-cache/blob/d1527c8f67ef9af357290475d4af4b0fab8f12e0/README.md).
