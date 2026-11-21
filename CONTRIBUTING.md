# Contributing to OrangeEditor

Start with the [README](README.md) and the [overview](docs/00-overview.md).
[Development](docs/10-development.md) covers the toolchain, checks, tests and the Git
workflow.

- Branch from `dev` and open every PR against `dev`; `main` receives `dev` through a
  promotion PR. PRs are squash-merged and their branches deleted.
- Keep a change scoped, in several focused commits that each pass the checks. Update the
  affected contracts in `docs/`, ADRs and translated text in the same change.
- Use the configured human Git author; no AI co-author trailers or generated-with
  footers in commits or PR descriptions.
- Fill in the [PR template](.github/PULL_REQUEST_TEMPLATE.md) with the checks you ran
  and their results.
- Plans, notes and logs stay in the ignored `.artifacts/`.

Application source is in `app/`, installable expansions in `plugins/`, and the Wasm guest
interface in `sdk/` ([architecture](docs/02-architecture.md)). Releases are built by
[release.yml](.github/workflows/release.yml), which stays disabled until a release is
qualified.
