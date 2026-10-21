# Repository tooling

Run from the repository root with Node.js 24.18.0 and Git:

```sh
node tools/check-docs.mjs
node tools/check-repo-policy.mjs
```

The documentation check validates local file links, fences, JSON syntax, requirement/decision/fixture references, roadmap coverage, translated definition shapes and repeated example consistency. It does not validate external links, every schema rule or product behavior.

The policy check inspects tracked paths and high-confidence credential patterns without printing matches, and checks Git whitespace. Review the staged diff too; a clean scan is not proof that no secret exists. Run it after staging the files to be committed. Before the first commit it checks staged whitespace; CI checks the current commit. These scripts require no npm dependencies or credentials.

The [CI workflow](../.github/workflows/checks.yml) runs on pushes to `main`, every PR and manual dispatch. Stable statuses are `docs`, `policy` and `rust`. Formatting and Clippy run in `rust-quality`; the locked workspace tests run on Linux x64 (`ubuntu-24.04`), Windows x64 (`windows-2022`) and macOS arm64 (`macos-15`). Every Rust job uses the shared toolchain pin. The `rust` aggregate runs even after dependency failure and succeeds only if quality and the entire test matrix succeeded. Matrix fail-fast is disabled so each OS reports its result; each job has a timeout.

Permissions remain read-only, checkout credentials are not persisted, and action references remain pinned to full commits. There is no deployment workflow yet: an application executable, packaging contract and release qualification belong to later delivery work. Passing these core tests establishes no Bevy, SDK, plugin-host or hardware capability.
