# AI context artifacts

Working material written by or for AI development agents: slice plans, session hand-offs and research notes. These files are **non-normative**. When they disagree with [docs/](../docs/), the docs win; fix the artifact or raise the conflict in a docs PR.

Start with [AGENTS.md](../AGENTS.md); it is the agent entrypoint and is not duplicated here.

| Path | Holds | Tracked |
|---|---|---|
| `plans/` | One plan per work slice, named after its work ID (for example `wu-00b-1-...`) | yes |
| `notes/` | Durable findings worth sharing between sessions (decisions still go to ADRs in [08](../docs/08-decisions.md)) | yes |
| `local/` | Scratch output, transcripts, per-user notes | no (ignored) |

Rules:

- No credentials, tokens or personal data, same as every tracked file ([14 §3](../docs/14-git-workflow.md)).
- A plan records intent, not evidence. Test results and gate status are recorded in the owning doc (07/10) with exact commands, never only here.
- Markdown here is checked by `tools/check-docs.mjs`: links must resolve and FR/NFR/G/diagnostic IDs must exist.
