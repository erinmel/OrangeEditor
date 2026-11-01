# Development measurements

The Rust `orange-measure` binary is development tooling. It is not a core dependency, and heavy measurement profiles are not invoked by ordinary workspace tests.

Build the exact locked source before timing:

```sh
cargo build --release -p orange-measure --locked --offline
cargo run --release -p orange-measure --locked --offline -- --warmup 5 --runs 15 index-update
cargo run --release -p orange-measure --locked --offline -- --warmup 3 --runs 15 authoring-compare
cargo run --release -p orange-measure --locked --offline -- --warmup 3 --runs 15 import-profile
cargo run --release -p orange-measure --locked --offline -- --warmup 2 --runs 7 ownership-memory
```

Offline mode requires cached dependencies; otherwise fetch the locked dependencies first. Run comparisons serially after compilation, using at least three independent paired processes. Set `ORANGE_MEASURE_OWNER_FIRST=1` for the second authoring process to reverse candidate order. `session-edit` and `w9` cover the wider edit matrix; W9 keeps its fixed seeded workload regardless of the generic run-count argument.

- `index-update`: root/function-local edits and wholesale function replacement/removal/restoration, including many-function and large-body cases. Transaction application and index update are timed; reset and fresh-build oracle checks are outside the timer.
- `authoring-compare`: equivalent snapshot-plus-session and document-owning workers on zero-diagnostic graphs. Caller work excludes explicit backpressure waits; newest/burst latency includes them. The profile verifies newest-request publication, history, missing/foreign receipts, registry/effects, cancellation and shutdown.
- `import-profile`: encoded inputs are built outside the window. Node/key/value-heavy shapes, wide extension arrays and node-quota refusal report allocator bytes separately from timing. It prints accepted/rejected outcomes so the same source works against the older importer.
- `ownership-memory`: document clone/drop, session construction/update/drop and 100 wide Fork races in one Run, with explicit stop/drain/dispose/drop stages.

On Windows, PowerShell 7 can observe the already-built child process:

```powershell
pwsh -NoProfile -File tools/measure-memory.ps1 -Profile authoring-compare -Runs 15 -Warmup 3 -Label pair1
```

The wrapper accepts `-Binary` and `-OutputDirectory` for an isolated comparator. It runs a hidden child, records stdout/stderr and executable SHA-256, fails on nonzero exit, and polls Windows PeakWorkingSet64 every 10 ms. This is whole-process observation including fixture construction and all cases, not per-candidate allocator memory or a Linux RSS measurement.

For internal session attribution:

```powershell
pwsh -NoProfile -File tools/measure-session.ps1 -Warmup 1 -Runs 3
```

That command creates a new ignored source copy under `.artifacts`, records source/toolchain/lock digests, injects consuming ownership helpers only into the copy, and builds an extra bin in the copied development package. The original source and public model API are unchanged. The injected helpers exhaustively destructure AnalysisSession, ValidationAnalysis and ValidationReport, so added fields there fail compilation rather than silently escaping attribution. Remaining retained-cache fields stay in the named metadata partition. Output distinguishes clean, warning-bearing and one-error chains; shared allocations are attributed to their last owner. Temporary boxing overhead is reported. Scratch copies and a short `.artifacts/sm-target` directory are retained for inspection; no recursive deletion is performed.

For the pre-membership index comparator, create an owned worktree at the checkout containing this harness, restore only `app/crates/orange-model/src/index.rs` from `e9aead5` in that worktree, then build its binary and run the same `index-update` command. For pre-budget import, restore only `codec.rs`, `error.rs` and `lib.rs` from `6854560` in the isolated comparator. Never restore over unrelated work.

Method, measured results, selected ownership direction and limitations are in [the verification record](../docs/17-authoring-verification.md). These profiles do not declare UI, security-isolation or reference-machine NFR gates passed.
