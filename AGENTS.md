# OrangeEditor agent entrypoint

Product and current workspace: OrangeEditor. Earlier planning evidence may mention `OraneEditor`; do not rename directories implicitly.

Read `README.md`, then `docs/07-delivery-plan.md`. Use `AGENT_PROMPT.md` for the complete planning assignment. Read the contract/ADR documents relevant to the current work unit before changing a public interface.

## Implementation rules

- Rust; Bevy exactly `=0.20.0` until an explicit upgrade ADR. Verify APIs from versioned Rustdoc or `v0.20.0` release source. Reject main-branch or stale `latest` examples as version evidence. See `docs/09-bevy-0.20.md`.
- Release source requires Rust >=1.97.1. Record the selected toolchain, features and lockfile before build claims. The selected local toolchain is Rust 1.99.1 via rustup, recorded in `docs/10-tooling-and-sources.md`; it has not yet compiled Bevy.
- Current artifacts are specifications, not an implemented application. Feasibility probes and NFR numbers are unexecuted/proposed until evidence is recorded. Do not mark a gate passed from documentation alone.
- OrangeEditor expansion plugins are installed product packages. Bevy developer plugins and AI development MCP tools are separate concepts.
- Graph semantics are normative in `docs/11-graph-language.md` and layouts in `docs/12-graph-data-structures.md`. Change a rule only together with its diagnostics, G-suite fixture, FR trace and ADR. Packages extend the language only through 11 §16.
- Product text follows `docs/13-internationalization.md`: namespaced translatable messages, typed redacted arguments and host-owned presentation. UI language never changes graph semantics, values, IDs or provider locks.
- Keep graph/type/runtime contracts independent of Bevy, rendering and window updates. Resolve node/type/provider identities before execution. Preserve pinned running providers during package updates.
- Product extension changes use documented registry/override contracts. Never replace hot installation with application rebuild/restart without an explicit requirement change.
- Keep credentials in host-owned OS stores; graphs, package examples, traces and exports contain references and redacted data. Native workers are not OS-sandboxed merely because IPC is used.
- No generic plugin calls, blocking work or allocations in audio callbacks. Validate endpoint routing/virtual-source behavior per OS rather than inferring it from playback support.
- Update FR/NFR traceability, schemas, contracts and ADRs in the same work unit when behavior changes. Preserve user edits and unrelated files.
- Verify changed behavior with the work unit's acceptance checks. Record exact versions, commands, results and limitations; never claim planned tests were run.

The initial assignment ends after documentation and review. Later implementation instructions determine which work unit to execute; its applicable prerequisite gates remain part of that work.
