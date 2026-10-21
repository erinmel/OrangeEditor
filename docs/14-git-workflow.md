# Git workflow, branches and change rules

Status: **selected development policy**, 2026-10-20. Initial inspection found planning documents with no Git repository. The user subsequently selected public GitHub `erinmel/OrangeEditor` and the [monorepo layout](15-monorepo-layout.md), with `erinmel` as solo maintainer. Bootstrap adds local Git and the `docs`/`policy` CI definitions; remote execution/protection evidence is recorded in 10 and the setup status below. Git reports `2.55.0.windows.1`. [07](07-delivery-plan.md) owns delivery dependencies; Git merges do not pass engineering gates by themselves.

## 1. Branch model

Use two long-lived branches plus short-lived topic branches. Every topic PR targets `dev`, the protected integration branch and repository default. `main` holds promoted work and changes only through a `dev` → `main` PR. Every PR is squash-merged and its branch deleted after merge. One branch/PR normally implements one executable subunit from 07 or a smaller independently reviewable part. A broad WU is not one giant PR. Prefer completing a small slice over leaving a branch open through several phases.

| Branch | Base and purpose | Example |
|---|---|---|
| `dev` | Protected integration branch; target of every topic PR | `dev` |
| `main` | Protected promoted branch; changes only through a `dev` → `main` PR | `main` |
| `docs/<work-id>-<slug>` | `dev`; planning/contracts/examples | `docs/wu-00a-graph-fixtures` |
| `feat/<work-id>-<slug>` | `dev`; product behavior | `feat/wu-00b-document-model` |
| `fix/<work-id>-<slug>` | `dev`; bug and regression fixture | `fix/wu-04d-stale-completion` |
| `chore/<slug>` | `dev`; tooling, dependencies, maintenance | `chore/document-checks` |
| `exp/<probe-id>-<slug>` | `dev`; bounded disposable feasibility experiment | `exp/b-p02-ime-locales` |
| `release/<major>.<minor>` | Qualified commit; temporary stabilization or explicitly supported older release only | `release/0.1` |
| `hotfix/<issue-id>-<slug>` | Affected release branch, or `dev` if no older supported line | `hotfix/123-save-recovery` |

Use lowercase ASCII and hyphens; keep names descriptive. A ticket number is optional for ordinary work; the work/probe ID is the traceability anchor. `release/*` and `hotfix/*` are created only when a real release needs them. Preview tags do not imply G5 or full 1.0 scope.

Dependent branches may temporarily start from another topic branch only when necessary. Mark the PR draft, name the parent PR and list its dependency. After the parent squash-merges, transplant only the child's commits onto current `dev`, retarget the PR, inspect its entire diff and rerun relevant checks. Do not merge the child's duplicate copy of its parent's commits. Shared branches are not rebased without coordination.

## 2. Normal change sequence

1. Inspect status/diffs and existing branches; preserve unrelated edits. Select the subunit and its prerequisite gates from 07. Documentation/research work may proceed before product implementation; a branch name does not waive those prerequisites.
2. Create a topic branch from current `dev` after fetching the configured remote. Use a separate worktree for concurrent independent changes, with one owner per branch/checkout and explicit file ownership when collaboration is authorized. Do not switch a shared dirty checkout underneath another task. The worktree location is explicitly selected within authorized directories.
3. Implement the slice and update affected contracts, fixtures, FR/NFR traceability and ADRs together. Include translation keys/accessibility text for new product UI and structured diagnostics per [13](13-internationalization.md).
4. Run applicable checks; record exact commands, versions, results and limitations. Inspect the staged diff and whitespace. Stage explicit paths so unrelated work and generated files are not swept in.
5. Open a PR using the [template](../.github/PULL_REQUEST_TEMPLATE.md), with the work ID, dependencies, behavior, validation and remaining risk. Draft means incomplete work, not permission to merge failed checks.
6. Resolve review findings and conflicts. Synchronize with the target, rerun affected checks after changes, and merge through the configured PR mechanism once §4 is met.
7. Squash-merge, retain the PR/evidence link, then delete the merged topic branch when no dependent work needs it. Remove a worktree only after checking its uncommitted/untracked work and confirming its contents are safely retained. Begin the next slice from current `dev`. A `dev` → `main` promotion is squash-merged too, so the two branches share content but not history; a later promotion PR can list already-promoted commits until history is reconciled.

Examples below are **future commands**, assuming a configured `origin`, an existing `dev` and a clean checkout. The branch name is an example, not an instruction to execute it now:

```powershell
git status --short
git fetch origin
git switch dev
git pull --ff-only origin dev
git switch -c docs/wu-00a-graph-fixtures
git diff --check
git diff --cached
```

## 3. Commits and history

Use `<type>(<scope>): <imperative summary>`, with types `docs`, `feat`, `fix`, `refactor`, `test`, `perf`, `build`, `ci`, `chore` or `revert`. Scope identifies the affected area. Example: `docs(i18n): define package catalogue fallback`. The commit body/PR records `Work: WU-00/00a` or the relevant probe, and affected requirement/fixture IDs. A commit changing a public contract explains compatibility and links its ADR/migration decision. `!` or `BREAKING CHANGE:` describes a breaking change; it does not authorize breaking frozen contracts.

Keep local commits coherent and reviewable. WIP commits are allowed on a topic branch; the final squash commit/title summarizes the final change, preserves contributor attribution and references the PR/work unit. Use one squash commit per PR on protected branches. Rebase only an owned topic branch; rewriting an already published owned topic branch uses `--force-with-lease` after checking collaborators and dependencies. Never force-push `dev`, `main`, release branches or tags. Fix merged mistakes with a new revert/fix PR, preserving history. A revert cannot undo effects already performed outside Git.

Commit source, tests/fixtures, schemas, source catalogues, migrations, documentation and reproducible build inputs. Once Rust scaffolding exists, track the workspace `Cargo.lock` and selected `rust-toolchain.toml`; dependency changes include the updated lock/features/MSRV/license evidence. Do not commit credentials, tokens, local vaults, personal profiles, machine IDE state, `target/`, caches, raw secret-bearing traces or generated binaries. Deliberate binary fixtures need documented size/license/provenance. `.gitignore` is convenience, not secret protection; review staged content and scan it. A leaked real credential is revoked/rotated immediately, with removal/history remediation coordinated separately.

User-selected attribution policy: use the configured human author and omit AI co-author trailers and generated-with commit/PR footers. The user explicitly authorized removing these trailers from the eight unpublished-to-main commits on `feat/wu-00b-document-model`. That cleanup preserves each commit's content tree and human author metadata, updates only the feature branch with an exact force-with-lease, and does not rewrite `main`.

## 4. Review and merge rules

Every change to `dev`, `main` or a release branch uses a PR after repository bootstrap. Require the latest applicable checks, resolved review conversations, a conflict-free diff against the current target, and complete work-unit evidence. Update the branch or use a validated merge queue so the tested integration matches the merge candidate. New commits/conflict resolutions invalidate affected test evidence; rerun it. Local green tests do not substitute for required remote checks once those are enabled.

**Solo maintainer mode:** require PRs/checks/conversation resolution and a recorded self-review, with zero required independent approvals. AI review is supplementary evidence, not an independent human approval. This avoids configuring a review requirement the only maintainer cannot satisfy. **Team mode:** when another human maintainer is available, require at least one non-author approval, dismiss stale approvals after new changes and require approval of the latest reviewable push by someone other than its pusher. Publish actual owner accounts before enabling CODEOWNERS; do not invent owners.

Public language/schema/SDK changes need contract review and matching fixtures/diagnostics/traceability/ADR; security-sensitive changes need review of authority/redaction; i18n changes need catalogue/argument coverage. Independent specialist evidence required by an engineering gate remains required in solo mode; an unavailable reviewer is recorded as a gate limitation, not an approval manufactured by another agent. Routine local work does not acquire a new user confirmation step from this policy.

If GitHub is selected, configure an active ruleset or equivalent branch protection for `dev`, `main` and `release/*`: require PRs, block force pushes/deletion, require linear history, require applicable checks on the current candidate, and resolve conversations. Allow squash merging only for those branches. Apply §4's solo/team approval settings. Limit bypass to named emergency maintainers; document incident, reason, commit and omitted checks, then run them and open follow-up review promptly. A bypass never records a failed gate as passed. Confirm protection capabilities against the selected hosting plan before claiming enforcement.

Local hooks may provide fast feedback; CI/remote rules enforce merge policy. Hooks are not assumed installed or mandatory merely because a file exists. Commit signing is optional until maintainer key management is configured; release signing and artifact provenance are a separate WU-12 requirement. Do not enable an unusable signing requirement during bootstrap.

## 5. Checks by phase and change

| Change / phase | Required evidence before merge |
|---|---|
| Documentation maintenance | Review consistency for changed contracts and the staged diff. No documentation or repository-hygiene scripts exist; this review is manual, outside required CI. |
| Rust implementation, after WU-01 toolchain/lock setup | Formatting/lints, applicable model/runtime/property/contract tests and headless build on the declared Windows/Linux/macOS CI matrix from [06](06-quality-and-validation.md), using the pinned toolchain/features and `--locked` where supported. |
| Graph/SDK/persistence contract | Relevant G-suite/schema/cross-language/migration/recovery fixtures; diagnostics and requirements updated in the same PR; G2 freeze evidence where applicable. |
| UI/text/catalogues | Text references/key and argument validation, relevant L-suite/IME/accessibility interaction checks; manual platform evidence at the named gate. |
| Dependency/toolchain/CI permissions | Lock/feature/MSRV/license/security review and applicable full matrix. CI must not expose repository secrets or write credentials to untrusted PR code. Workflow changes receive explicit permission-scope review. |
| Release | WU-12/G5 evidence, signed/provenance-recorded artifacts, supported capability/locale matrix, recovery/security review and changelog; an early preview explicitly lists its incomplete scope. |

The single required status is the stable `rust` aggregate. At the user's request, documentation and generic repository-policy jobs are removed from CI; their scripts were deleted. Remove obsolete `docs` and `policy` requirements from branch protection when adopting this workflow. This is a deliberate scope change, not a waiver for a failing check. The Rust workflow runs on every PR, including documentation-only PRs, with no path-filtering machinery; never describe an unexecuted check as passed. Changed public contracts and toolchain/CI changes trigger their full dependent tests. Slow hardware/soak/protocol checks run at their documented gates. Never disable a failing code check just to merge.

The configured workflow reports separate formatting, Clippy and rustdoc jobs on Ubuntu with matrix fail-fast disabled; Cargo architecture and declared-MSRV checks; pinned cargo-deny advisory/source checks; and locked core tests on Ubuntu x64, Windows x64 and macOS arm64. Formatting failure does not suppress other code-check results. A stable `rust` aggregate requires every Rust job to succeed; failure, cancellation or unexpected skipping cannot produce a green aggregate. It runs for each PR, pushes to `dev` and `main`, and manual dispatch; its weekly schedule becomes active once merged to the default branch. Dependabot proposes weekly Cargo and GitHub Actions updates after merge. Only the pinned cargo-deny installation is cached; the online advisory scan always executes. Checks use read-only permissions, do not persist checkout credentials and have bounded job durations. Deployment waits for a packaged application and the applicable release gates; CI configuration alone is not release evidence.

## 6. Releases, hotfixes and compatibility

Use immutable annotated tags: `v0.1.0-alpha.1`, `v0.1.0-rc.1`, `v0.1.0` are naming examples, not scheduled releases. Tag only the exact qualified commit and record tag, full commit ID, toolchain, lockfile/features, artifact digests and gate evidence. A published tag is never moved or reused; corrections receive a new version. Restrict tag creation/update/deletion to the release role when hosting is configured.

Create `release/<major>.<minor>` only to stabilize a candidate or maintain a declared supported older line. Accept fixes/docs/version changes, with PRs and the same checks; new features target `dev`. Fix a supported older version on its line, then forward-port the fix to `dev` and any other affected supported line through separate PRs. Record cherry-pick/source PR and validate each target; do not merge an entire old release branch into `dev` or `main`. Publish support/EOL decisions before maintaining multiple release lines. Package/SDK/graph-format versions remain separate axes from application Git tags.

## 7. Bootstrap and verification

Future bootstrap, when repository setup is requested: inspect current files, decide local/remote destination and access/visibility, initialize or connect the repository without replacing existing history, review ignore/line-ending policy, create a reviewed initial baseline on `main`, then activate PR/check rules. The initial commit is the sole bootstrap exception to PR-first. Enable required statuses only after their workflows exist and have run successfully. Add real maintainer ownership and verify rejection of an unauthorized direct push/force push, a failing-check merge and a merge with unresolved conversations in a disposable branch/test repository. Do not test destructive operations on the real protected branch.

Selected setup: public GitHub `erinmel/OrangeEditor`, `erinmel` as solo maintainer, `main` plus topic branches, `docs` and `policy` checks on a GitHub-hosted Ubuntu runner. Independent-approval count is zero until team mode is selected. The initial baseline is the one bootstrap exception to PR-first. Required statuses are enabled only after their first successful remote execution; later changes use PRs. Remaining choices include additional maintainers, artifact/tag signing identities and supported release lines. The user's repository-creation request supplies publishing authority for this bootstrap; the policy alone does not grant future remote authority.

## 8. Verified bootstrap status

On 2026-10-20, created public [erinmel/OrangeEditor](https://github.com/erinmel/OrangeEditor) with default branch `main` and initial commit `42efbe0f856a6e7ff956307d456ea7af5f7db4fd`. Local `origin` uses the credential-free SSH URL `git@github.com:erinmel/OrangeEditor.git`. The first [CI run](https://github.com/erinmel/OrangeEditor/actions/runs/39493589359) completed successfully for `docs` and `policy` before either status became mandatory.

GitHub API readback confirms: PRs required; strict/up-to-date `docs` and `policy` checks bound to GitHub Actions app 15368; zero independent approvals in solo mode; stale approvals dismissed; conversations resolved; linear history; force pushes/deletion disabled; administrator enforcement enabled. Repository settings allow squash merges only and delete merged topic branches. The default-branch creation/push was the documented bootstrap exception; subsequent evidence changes use a topic PR.

This verifies configured settings and the successful initial workflow, not negative-path enforcement tests. Rejection tests in a disposable target, release-branch/tag rules, signing identities and product/SDK/plugin checks remain future evidence. No production build or application gate passed through repository setup.

Quality-gate expansion on 2026-10-20: after all ten jobs passed for `1eb5721` in [run 39524649391](https://github.com/erinmel/OrangeEditor/actions/runs/39524649391), added `rust` through the required-status-check API. Readback confirms strict `docs`, `policy`, `rust`, each bound to GitHub Actions app 15368. All other inspected protection controls above remain unchanged; this readback is configuration evidence, not a negative-path enforcement test. The workflow subsequently refreshes its full-SHA Action pins to Node 24 releases; final-head validation is recorded in the PR.

Code-only scope revision on 2026-10-20: the user requested removing non-code checks. All ten code jobs passed for `9d8cbae` in [run 39526751351](https://github.com/erinmel/OrangeEditor/actions/runs/39526751351), including independent format/Clippy/rustdoc results and the scanner cache-miss path. The required-status API was then narrowed to strict `rust` only, bound to Actions app 15368. Readback confirms admin enforcement, zero solo-mode independent approvals, resolved conversations, linear history, and disabled force pushes/deletion are unchanged. This supersedes the earlier docs/policy required-status set; the local helper scripts remain available. Configuration readback is not a negative-path enforcement test.

Primary references checked 2026-10-20: [Git workflow guidance](https://git-scm.com/docs/gitworkflows), [GitHub ruleset controls](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets), and [GitHub merge methods](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/about-merge-methods-on-github). The branch names, squash default and review modes above are OrangeEditor policy choices; observed setup evidence is recorded above.
