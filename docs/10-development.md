# Development

## Toolchain

- Rust **1.99.1** through rustup, pinned in `rust-toolchain.toml` (rustfmt, clippy).
  The workspace declares MSRV **1.97.1**; CI derives and checks it from Cargo metadata.
- Edition 2024, resolver 3, one `Cargo.lock`. Every dependency is pinned to an exact
  version in the root `Cargo.toml`; check a new one against its versioned rustdoc or
  release-tag source, never a main-branch example.
- Node.js 24.18.0 runs the repository scripts in `tools/`.
- Builds go to `target/` inside the checkout. Incremental compilation is off by default
  (`.cargo/config.toml`) because synced folders such as OneDrive reject its hard links;
  set `CARGO_INCREMENTAL=1` on a local disk for faster rebuilds.
- Linux builds need the ALSA headers (`libasound2-dev`).

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node tools/check-cargo-metadata.mjs          # core stays free of UI, engine and service crates
node --test tools/*.test.mjs                 # repository scripts
node tools/check-automation.mjs --offline    # acceptance suite; --network adds the socket checks
```

CI (`.github/workflows/checks.yml`) runs, in order: formatting, Clippy and rustdoc;
repository and workflow rules; `cargo-deny` for advisories, licenses and sources; the
workspace tests on each native platform; optimized behavior and embedding examples;
and the Wasm guest probe built from `sdk/`. The required check is `rust`. Checks have no
path filters.

Static analysis runs with Qodana (`qodana.yaml`). Run it from a clean copy of the
repository, not the working checkout: its container rewrites the IDE settings in
`.idea/`.

```sh
git worktree add --detach ../orange-scan HEAD && cd ../orange-scan && qodana scan
```

## Tests

Tests grow with the code: a focused Rust test for each rule that is implemented and for
each regression found. There is no upfront fixture suite and no coverage target.

- Unit tests sit beside the code; integration tests in each crate's `tests/`.
- Property tests (`proptest`) cover invariants such as round trips and orderings.
- `orange-testkit` holds stable IDs and small graph builders for host and service tests.
- Hardware tests (audio devices, capture, global keys) are `#[ignore]`d and run by hand.
- Network tests that need brokers run against the [Docker network lab](../tools/README.md).

## Panic policy

`orange-model`, `orange-compiler`, `orange-runtime` and `orange-bundled` warn on
`clippy::unwrap_used` and `clippy::expect_used` in library code, and CI denies warnings.
Tests, examples and `orange-measure` are exempt. Input, lifecycle and every other path
that can fail returns a typed error. A panic remains only for an internal invariant,
written as `#[expect(clippy::expect_used, reason = "...")]` that says why it cannot fail.
Replacing such an invariant with a default would hide bugs.

## Performance

Pre-1.0, measure before and after a change; do not chase small regressions or add size
guards.

- `orange-measure` reports allocations, peak bytes and per-pump wall time for core
  workloads: `cargo run --release -p orange-measure --locked -- --help`.
- Set `ORANGE_EDIT_TIMING_PAIRS=2500` and run
  `cargo test --release -p orange-editor-ui edit_timing -- --nocapture` to time each
  step of an edit on a 5,000-node graph. The default test uses 64 pairs.
- Compare a baseline and a candidate built with the same compiler and lockfile, run
  serially, in at least three paired processes with the order reversed.

Current figures (Windows 11, release build, 5,000-node graph):

| Step of one edit | Time |
|---|---:|
| Whole edit, from input to previews | 22–26 ms |
| Graph view rebuild (only changed cards) | 9–11 ms |
| Previews of what the edit touched | 0.6–9 ms |

Host bounds: at most 4,096 effects wait for a worker (`HostOptions::max_queued_effects`;
past it a node fails with `host/Busy`); an observer that reads slowly loses telemetry,
counted, but never a flow's lifecycle events.

## Git workflow

- `dev` is the integration branch and every topic PR targets it; `main` receives `dev`
  through a promotion PR. PRs are squash-merged and their branches deleted.
- Branch names: `feat/`, `fix/`, `chore/`, `docs/`, `core/`, `polish/`, `exp/` and a short
  slug.
- Several focused, green commits per change. Use the configured human author; no AI
  co-author trailers or generated-with footers.
- Update the contracts in `docs/` in the same change as the behavior they describe.
- Plans, notes and logs go in the ignored `.artifacts/`, never in `docs/`.

## Releases

`tools/release.mjs` plans, verifies and collects release packages, and
`tools/release/<platform>.ps1` builds them ([contract](../tools/release/README.md)).
Publishing is disabled until a release is qualified.
