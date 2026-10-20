# Git workflow, branches and change rules

Status: **selected development policy**, 2026-10-20. Initial inspection found planning documents with no Git repository. The user subsequently selected public GitHub `erinmel/OrangeEditor` and the [monorepo layout](15-monorepo-layout.md), with `erinmel` as solo maintainer. Bootstrap adds local Git and the `docs`/`policy` CI definitions; remote execution/protection evidence is recorded in 10 and the setup status below. Git reports `2.55.0.windows.1`. [07](07-delivery-plan.md) owns delivery dependencies; Git merges do not pass engineering gates by themselves.

## 1. Branch model

Use one integration branch, `main`, plus short-lived topic branches. `main` contains reviewed work valid for the current phase: today that means consistent planning, later a buildable/tested implementation. No permanent `develop` branch. One branch/PR normally implements one executable subunit from 07 or a smaller independently reviewable part. A broad WU is not one giant PR. Prefer completing a small slice over leaving a branch open through several phases.

| Branch | Base and purpose | Example |
|---|---|---|
| `main` | Protected integration branch | `main` |
| `docs/<work-id>-<slug>` | `main`; planning/contracts/examples | `docs/wu-00a-graph-fixtures` |
| `feat/<work-id>-<slug>` | `main`; product behavior | `feat/wu-00b-document-model` |
| `fix/<work-id>-<slug>` | `main`; bug and regression fixture | `fix/wu-04d-stale-completion` |
| `chore/<slug>` | `main`; tooling, dependencies, maintenance | `chore/document-checks` |
| `exp/<probe-id>-<slug>` | `main`; bounded disposable feasibility experiment | `exp/b-p02-ime-locales` |
| `release/<major>.<minor>` | Qualified commit; temporary stabilization or explicitly supported older release only | `release/0.1` |
| `hotfix/<issue-id>-<slug>` | Affected release branch, or `main` if no older supported line | `hotfix/123-save-recovery` |

Use lowercase ASCII and hyphens; keep names descriptive. A ticket number is optional for ordinary work; the work/probe ID is the traceability anchor. `release/*` and `hotfix/*` are created only when a real release needs them. Preview tags do not imply G5 or full 1.0 scope.

Dependent branches may temporarily start from another topic branch only when necessary. Mark the PR draft, name the parent PR and list its dependency. After the parent squash-merges, transplant only the child's commits onto current `main`, retarget the PR, inspect its entire diff and rerun relevant checks. Do not merge the child's duplicate copy of its parent's commits. Shared branches are not rebased without coordination.

## 2. Normal change sequence

1. Inspect status/diffs and existing branches; preserve unrelated edits. Select the subunit and its prerequisite gates from 07. Documentation/research work may proceed before product implementation; a branch name does not waive those prerequisites.
2. Create a topic branch from current `main` after fetching the configured remote. Use a separate worktree for concurrent independent changes, with one owner per branch/checkout and explicit file ownership when collaboration is authorized. Do not switch a shared dirty checkout underneath another task. The worktree location is explicitly selected within authorized directories.
3. Implement the slice and update affected contracts, fixtures, FR/NFR traceability and ADRs together. Include translation keys/accessibility text for new product UI and structured diagnostics per [13](13-internationalization.md).
4. Run applicable checks; record exact commands, versions, results and limitations. Inspect the staged diff and whitespace. Stage explicit paths so unrelated work and generated files are not swept in.
5. Open a PR using the [template](../.github/PULL_REQUEST_TEMPLATE.md), with the work ID, dependencies, behavior, validation and remaining risk. Draft means incomplete work, not permission to merge failed checks.
6. Resolve review findings and conflicts. Synchronize with the target, rerun affected checks after changes, and merge through the configured PR mechanism once §4 is met.
7. Squash-merge, retain the PR/evidence link, then delete the merged topic branch when no dependent work needs it. Remove a worktree only after checking its uncommitted/untracked work and confirming its contents are safely retained. Begin the next slice from current `main`.

Examples below are **future commands**, assuming a configured `origin`, an existing `main` and a clean checkout. The branch name is an example, not an instruction to execute it now:

```powershell
git status --short
git fetch origin
git switch main
git pull --ff-only origin main
git switch -c docs/wu-00a-graph-fixtures
git diff --check
git diff --cached
```

## 3. Commits and history

Use `<type>(<scope>): <imperative summary>`, with types `docs`, `feat`, `fix`, `refactor`, `test`, `perf`, `build`, `ci`, `chore` or `revert`. Scope identifies the affected area. Example: `docs(i18n): define package catalogue fallback`. The commit body/PR records `Work: WU-00/00a` or the relevant probe, and affected requirement/fixture IDs. A commit changing a public contract explains compatibility and links its ADR/migration decision. `!` or `BREAKING CHANGE:` describes a breaking change; it does not authorize breaking frozen contracts.

Keep local commits coherent and reviewable. WIP commits are allowed on a topic branch; the final squash commit/title summarizes the final change, preserves contributor attribution and references the PR/work unit. Use one squash commit per PR on protected branches. Rebase only an owned topic branch; rewriting an already published owned topic branch uses `--force-with-lease` after checking collaborators and dependencies. Never force-push `main`, release branches or tags. Fix merged mistakes with a new revert/fix PR, preserving history. A revert cannot undo effects already performed outside Git.

Commit source, tests/fixtures, schemas, source catalogues, migrations, documentation and reproducible build inputs. Once Rust scaffolding exists, track the workspace `Cargo.lock` and selected `rust-toolchain.toml`; dependency changes include the updated lock/features/MSRV/license evidence. Do not commit credentials, tokens, local vaults, personal profiles, machine IDE state, `target/`, caches, raw secret-bearing traces or generated binaries. Deliberate binary fixtures need documented size/license/provenance. `.gitignore` is convenience, not secret protection; review staged content and scan it. A leaked real credential is revoked/rotated immediately, with removal/history remediation coordinated separately.

## 4. Review and merge rules

Every change to `main` or a release branch uses a PR after repository bootstrap. Require the latest applicable checks, resolved review conversations, a conflict-free diff against the current target, and complete work-unit evidence. Update the branch or use a validated merge queue so the tested integration matches the merge candidate. New commits/conflict resolutions invalidate affected test evidence; rerun it. Local green tests do not substitute for required remote checks once those are enabled.

**Solo maintainer mode:** require PRs/checks/conversation resolution and a recorded self-review, with zero required independent approvals. AI review is supplementary evidence, not an independent human approval. This avoids configuring a review requirement the only maintainer cannot satisfy. **Team mode:** when another human maintainer is available, require at least one non-author approval, dismiss stale approvals after new changes and require approval of the latest reviewable push by someone other than its pusher. Publish actual owner accounts before enabling CODEOWNERS; do not invent owners.

Public language/schema/SDK changes need contract review and matching fixtures/diagnostics/traceability/ADR; security-sensitive changes need review of authority/redaction; i18n changes need catalogue/argument coverage. Independent specialist evidence required by an engineering gate remains required in solo mode; an unavailable reviewer is recorded as a gate limitation, not an approval manufactured by another agent. Routine local work does not acquire a new user confirmation step from this policy.

If GitHub is selected, configure an active ruleset or equivalent branch protection for `main` and `release/*`: require PRs, block force pushes/deletion, require linear history, require applicable checks on the current candidate, and resolve conversations. Allow squash merging only for those branches. Apply §4's solo/team approval settings. Limit bypass to named emergency maintainers; document incident, reason, commit and omitted checks, then run them and open follow-up review promptly. A bypass never records a failed gate as passed. Confirm protection capabilities against the selected hosting plan before claiming enforcement.

Local hooks may provide fast feedback; CI/remote rules enforce merge policy. Hooks are not assumed installed or mandatory merely because a file exists. Commit signing is optional until maintainer key management is configured; release signing and artifact provenance are a separate WU-12 requirement. Do not enable an unusable signing requirement during bootstrap.

## 5. Checks by phase and change

| Change / phase | Required evidence before merge |
|---|---|
| Current documentation-only baseline | Markdown/link/fence/JSON checks; requirement/ADR/fixture references and roadmap coverage; consistency review for changed contracts; staged diff and secret review. No Cargo/GPU/OS test is reported as executed. |
| Rust implementation, after WU-01 toolchain/lock setup | Formatting/lints, applicable model/runtime/property/contract tests and headless build on the declared Windows/Linux/macOS CI matrix from [06](06-quality-and-validation.md), using the pinned toolchain/features and `--locked` where supported. |
| Graph/SDK/persistence contract | Relevant G-suite/schema/cross-language/migration/recovery fixtures; diagnostics and requirements updated in the same PR; G2 freeze evidence where applicable. |
| UI/text/catalogues | Text references/key and argument validation, relevant L-suite/IME/accessibility interaction checks; manual platform evidence at the named gate. |
| Dependency/toolchain/CI permissions | Lock/feature/MSRV/license/security review and applicable full matrix. CI must not expose repository secrets or write credentials to untrusted PR code. Workflow changes receive explicit permission-scope review. |
| Release | WU-12/G5 evidence, signed/provenance-recorded artifacts, supported capability/locale matrix, recovery/security review and changelog; an early preview explicitly lists its incomplete scope. |

Future required status names should be stable aggregate checks such as `docs`, `policy` and, once implemented, `rust`. Aggregate workflows run on every PR, dispatch applicable jobs and report why a job is inapplicable; do not require a status that path filtering never creates. A docs-only PR does not claim Rust tests ran when its aggregate correctly marks Rust as inapplicable. Changed public contracts and toolchain/CI changes trigger their full dependent tests. Slow hardware/soak/protocol checks run at their documented gates rather than on every prose edit. Never disable a failing required check just to merge.

## 6. Releases, hotfixes and compatibility

Use immutable annotated tags: `v0.1.0-alpha.1`, `v0.1.0-rc.1`, `v0.1.0` are naming examples, not scheduled releases. Tag only the exact qualified commit and record tag, full commit ID, toolchain, lockfile/features, artifact digests and gate evidence. A published tag is never moved or reused; corrections receive a new version. Restrict tag creation/update/deletion to the release role when hosting is configured.

Create `release/<major>.<minor>` only to stabilize a candidate or maintain a declared supported older line. Accept fixes/docs/version changes, with PRs and the same checks; new features target `main`. Fix a supported older version on its line, then forward-port the fix to `main` and any other affected supported line through separate PRs. Record cherry-pick/source PR and validate each target; do not merge an entire old release branch into `main`. Publish support/EOL decisions before maintaining multiple release lines. Package/SDK/graph-format versions remain separate axes from application Git tags.

## 7. Bootstrap and verification

Future bootstrap, when repository setup is requested: inspect current files, decide local/remote destination and access/visibility, initialize or connect the repository without replacing existing history, review ignore/line-ending policy, create a reviewed initial baseline on `main`, then activate PR/check rules. The initial commit is the sole bootstrap exception to PR-first. Enable required statuses only after their workflows exist and have run successfully. Add real maintainer ownership and verify rejection of an unauthorized direct push/force push, a failing-check merge and a merge with unresolved conversations in a disposable branch/test repository. Do not test destructive operations on the real protected branch.

Selected setup: public GitHub `erinmel/OrangeEditor`, `erinmel` as solo maintainer, `main` plus topic branches, `docs` and `policy` checks on a GitHub-hosted Ubuntu runner. Independent-approval count is zero until team mode is selected. The initial baseline is the one bootstrap exception to PR-first. Required statuses are enabled only after their first successful remote execution; later changes use PRs. Remaining choices include additional maintainers, artifact/tag signing identities and supported release lines. The user's repository-creation request supplies publishing authority for this bootstrap; the policy alone does not grant future remote authority.

Primary references checked 2026-10-20: [Git workflow guidance](https://git-scm.com/docs/gitworkflows), [GitHub ruleset controls](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets), and [GitHub merge methods](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/about-merge-methods-on-github). The branch names, squash default and review modes above are OrangeEditor policy choices. Hosted feature availability and account configuration still require setup evidence.
