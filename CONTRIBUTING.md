# Contributing to OrangeEditor

Follow the [Git workflow](docs/14-git-workflow.md) for branch names, commits, PRs, checks, review, merging and releases. Start with the [document map](README.md) and select a small subunit from the assigned task or local delivery plan. Keep delivery plans, WU task packets and working notes in ignored `.artifacts/`; do not commit them. Remove temporary reviews when their work is complete.

- Base ordinary work on `dev` and open every PR against `dev`; use `docs/`, `feat/`, `fix/`, `chore/` or `exp/` branches tied to a work/probe ID. `main` receives `dev` through a promotion PR. PRs are squash-merged and their branches deleted.
- Keep changes scoped. Update relevant contracts, fixtures, requirements, ADRs and translatable text together.
- Use commit subjects such as `docs(i18n): define package catalogue fallback` and record work/requirement IDs in the body or PR.
- Use the configured human Git author; omit AI co-author trailers and generated-with footers from commit messages and PR descriptions.
- Record exact validation commands/results and limitations in the [PR template](.github/PULL_REQUEST_TEMPLATE.md). Planned, inapplicable and unexecuted checks remain labeled.
- Merge through PRs with applicable checks and the workflow's solo/team review settings; squash-merge and preserve published history.

The [monorepo layout](docs/15-monorepo-layout.md) separates application source (`app/`), installable packages (`plugins/`) and their public contract (`sdk/`). PR validation runs in stages: formatting, Clippy, rustdoc, architecture/workflow/release-contract checks and dependency security first, then native tests on Linux, Windows and macOS plus minimum-version compilation. The run summary shows per-stage and per-OS results. [Release CD](.github/workflows/release.yml) is separate and blocks until a real executable and qualified packaging exist. Repository enforcement status is recorded in 14. Local edits and reviews follow the scope authorized for the task.
