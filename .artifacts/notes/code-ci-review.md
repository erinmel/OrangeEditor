# Code-focused CI review

User direction, 2026-10-20: remove non-code pipeline steps and investigate formatting failures hiding later checks. Three Luna agents reviewed scope, failure behavior and caching; the parent consolidated and implemented the decisions below. This note is working context, not normative acceptance evidence.

## Findings and decisions

- Remove the standalone Markdown `docs` and generic repository-hygiene `policy` CI jobs. Keep their scripts as optional local helpers. Change the required-status set to the already established `rust` aggregate after the revised workflow succeeds. Update the policy owner in docs/14 alongside the workflow; prior bootstrap records remain historical evidence.
- The former `rust-quality` job ran fmt, Clippy and rustdoc sequentially. GitHub's default step condition is success, so fmt failure skipped the next two commands. Independent tests/security continued. This was a visibility problem within that job, not a reason to ignore formatting failures.
- Use one static matrix for `rust-format`, `rust-clippy` and `rustdoc`, with fail-fast disabled. This gives separate check cards and avoids continued-error/outcome bookkeeping. It repeats checkout/toolchain setup and separates build outputs, a small current cost accepted for clear results. The `rust` aggregate waits for all variants and rejects failure, cancellation and unexpected skip.
- Rename Cargo metadata checking to `rust-architecture`: it enforces dependency direction and exact Bevy pin declarations, then derives the workspace MSRV for its separate test job. This checks only the coded Cargo boundary, not every architecture rule. Keep current three-OS native tests and RustSec/source checks; these are code/dependency validation.
- Cache only cargo-deny 0.20.2's installation. Measured run 39525101049 spent 135 seconds installing it and one second scanning. Native compile/test steps took 18–34 seconds, so defer Cargo registry/build caches until there is material benefit. Cache key: OS, architecture, toolchain-file hash, CLI version and explicit key-schema version. No fallback restore keys, credential paths or advisory database cache. Check CLI version and refresh online advisory data on hits and misses.
- Verify official actions/cache v6.1.0 (`55cc8345863c7cc4c66a329aec7e433d2d1c52a9`) by the GitHub release/commit API and tagged action.yml; it uses Node 24. Preserve normal PR cache scope, read-only workflow token, no secrets, no elevated checkout trigger, full-SHA Actions and existing timeouts. Cache restoration trusts GitHub's scoped cache contents; the CLI version check is not a cryptographic integrity guarantee.

The reviewers confirmed the matrix/aggregate dependencies and cache implementation. An agent preferred keeping a single quality job with continued-error steps plus final outcome aggregation to reduce setup duplication; the parent selected independent matrix jobs for explicit result names. An agent initially preferred retaining documentation checks based on old repository prose; the user's explicit code-only scope supersedes that policy, which is updated in the same work unit.

## Sources and validation boundaries

- [GitHub step/job conditions](https://docs.github.com/en/actions/reference/workflows-and-actions/expressions), [step outcomes](https://docs.github.com/en/actions/reference/workflows-and-actions/contexts), [matrix fail-fast](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/run-job-variations).
- [Clippy CI](https://doc.rust-lang.org/clippy/continuous_integration/index.html), [Cargo doc](https://doc.rust-lang.org/cargo/commands/cargo-doc.html), [workspace commands](https://doc.rust-lang.org/cargo/reference/workspaces.html).
- [GitHub cache scope/key behavior](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching), [cache v6.1.0](https://github.com/actions/cache/releases/tag/v6.1.0), [tagged action runtime](https://github.com/actions/cache/blob/v6.1.0/action.yml), [Cargo build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html).

No Rust application source, dependency feature, manifest, lockfile or selected toolchain changes. Remote cache miss/hit and final-head outcomes belong to the PR evidence; no intentional failing-format CI run is claimed. Bevy UI, hardware, packaging and release acceptance remain later work.
