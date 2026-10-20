# Repository tooling

Run from the repository root with Node.js 24.18.0 and Git:

```sh
node tools/check-docs.mjs
node tools/check-repo-policy.mjs
```

The documentation check validates local file links, fences, JSON syntax, requirement/decision/fixture references, roadmap coverage, translated definition shapes and repeated example consistency. It does not validate external links, every schema rule or product behavior.

The policy check inspects tracked paths and high-confidence credential patterns without printing matches, and checks Git whitespace. Review the staged diff too; a clean scan is not proof that no secret exists. Run it after staging the files to be committed. Before the first commit it checks staged whitespace; CI checks the current commit. These scripts require no npm dependencies or credentials.

The [CI workflow](../.github/workflows/checks.yml) runs `docs` and `policy` on pushes to `main` and every PR. It has read-only repository permissions and disables persisted checkout credentials. Rust, SDK packaging, plugin execution and platform checks are introduced by the implementation work units; these two statuses make no claim that those tests ran.
