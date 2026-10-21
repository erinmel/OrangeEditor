# Contributing to OrangeEditor

Follow the [Git workflow](docs/14-git-workflow.md) for branch names, commits, PRs, checks, review, merging and releases. Start with the [document map](README.md) and select a small subunit from the [delivery plan](docs/07-delivery-plan.md).

- Base ordinary work on `main`; use `docs/`, `feat/`, `fix/`, `chore/` or `exp/` branches tied to a work/probe ID.
- Keep changes scoped. Update relevant contracts, fixtures, requirements, ADRs and translatable text together.
- Use commit subjects such as `docs(i18n): define package catalogue fallback` and record work/requirement IDs in the body or PR.
- Use the configured human Git author; omit AI co-author trailers and generated-with footers from commit messages and PR descriptions.
- Record exact validation commands/results and limitations in the [PR template](.github/PULL_REQUEST_TEMPLATE.md). Planned, inapplicable and unexecuted checks remain labeled.
- Merge through PRs with applicable checks and the workflow's solo/team review settings; squash-merge and preserve published history.

The [monorepo layout](docs/15-monorepo-layout.md) separates application source (`app/`), installable packages (`plugins/`) and their public contract (`sdk/`). The required code pipeline runs formatting, Clippy, rustdoc, architecture checks, minimum-version tests, dependency security and native tests. Repository enforcement status is recorded in 14. Local edits and reviews follow the scope authorized for the task.
