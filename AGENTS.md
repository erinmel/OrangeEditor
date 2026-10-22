# OrangeEditor agent entrypoint

Product and current workspace: OrangeEditor. Earlier planning evidence may mention `OraneEditor`; do not rename directories implicitly.

Read `README.md`, then `.artifacts/07-delivery-plan.md` when available. Read the contract/ADR documents relevant to the current work unit before changing a public interface.

Delivery plans, WU task packets and agent working notes belong in ignored `.artifacts/`, not `docs/`, and must not be committed. Reviews are temporary; remove them when their work is complete. Keep durable product contracts, decisions and verification evidence in `docs/`.

## Implementation rules

- Rust only; no game engine (ADR-02). OrangeEditor is a foundation: an embeddable core plus a separate egui editor UI (ADR-21). Pin exact versions of UI and other dependencies when adopted, and verify APIs from versioned Rustdoc or release-tag source; reject main-branch or stale `latest` examples as version evidence. Feasibility probes are in `docs/09-feasibility-probes.md`.
- MSRV is Rust 1.97.1. Record the selected toolchain, features and lockfile before build claims. The selected local toolchain is Rust 1.99.1 via rustup, recorded in `docs/10-tooling-and-sources.md`.
- Current artifacts are specifications, not an implemented application. Feasibility probes and NFR numbers are unexecuted/proposed until evidence is recorded. Do not mark a gate passed from documentation alone.
- OrangeEditor expansion plugins are installed product packages. Crates that downstream products build on the core and AI development MCP tools are separate concepts.
- Graph semantics are normative in `docs/11-graph-language.md` and layouts in `docs/12-graph-data-structures.md`. Keep changed rules, diagnostics, focused Rust behavior/property tests, FR trace and ADR consistent. G IDs are reference scenarios, not a mandatory fixture inventory. Do not build exhaustive JSON suites or target 100% coverage; test foundational invariants and actual regressions as behavior is implemented. Packages extend the language only through 11 §16.
- Product text follows `docs/13-internationalization.md`: namespaced translatable messages, typed redacted arguments and host-owned presentation. UI language never changes graph semantics, values, IDs or provider locks.
- Keep graph/type/runtime contracts independent of UI, rendering, windows and engines; public core APIs are a product surface for downstream crates. Resolve node/type/provider identities before execution. Preserve pinned running providers during package updates.
- Product extension changes use documented registry/override contracts. Never replace hot installation with application rebuild/restart without an explicit requirement change.
- Keep credentials in host-owned OS stores; graphs, package examples, traces and exports contain references and redacted data. Native workers are not OS-sandboxed merely because IPC is used.
- No generic plugin calls, blocking work or allocations in audio callbacks. Validate endpoint routing/virtual-source behavior per OS rather than inferring it from playback support.
- Update FR/NFR traceability, schemas, contracts and ADRs in the same work unit when behavior changes. Preserve user edits and unrelated files.
- Verify changed behavior with the work unit's acceptance checks. Record exact versions, commands, results and limitations; never claim planned tests were run.
- Use the configured human Git author. Do not append AI co-author trailers or generated-with attribution footers to commits or pull requests.

The initial assignment ends after documentation and review. Later implementation instructions determine which work unit to execute; its applicable prerequisite gates remain part of that work.
