# OrangeEditor agent entrypoint

Product: OrangeEditor. Read `README.md` and [docs/00-overview.md](docs/00-overview.md) first, then the contract and decision documents relevant to the change before changing a public interface.

Plans, task notes, logs and reviews belong in the ignored `.artifacts/`, never in `docs/`, and are not committed. Remove a review once its work is done. `docs/` holds durable contracts and decisions only.

## Implementation rules

- Rust only; no game engine (ADR-02). OrangeEditor is a foundation: an embeddable core plus a separate egui editor UI (ADR-21). Pin exact dependency versions and verify APIs from versioned rustdoc or release-tag source, never main-branch or `latest` examples.
- Toolchain: Rust 1.99.1 via rustup; MSRV 1.97.1 ([docs/10-development.md](docs/10-development.md)).
- Expansion plugins are installed product packages. Crates that downstream products build on the core, and AI development tools, are separate concepts.
- Graph semantics are normative in `docs/11-graph-language.md` and the document format in `docs/12-graph-data-structures.md`. Keep rules, diagnostics, tests and ADRs consistent. Packages extend the language only through 11 §16.
- Tests grow with the code: focused Rust behavior or property tests for each rule implemented and each regression found. No exhaustive JSON suites, no coverage target.
- Product text follows `docs/13-internationalization.md`: namespaced translatable messages in English and Spanish, typed redacted arguments, host-owned presentation. Interface language never changes graph semantics, values, IDs or provider locks.
- Keep graph, type and runtime contracts independent of UI, rendering, windows and engines; public core APIs are a product surface for downstream crates. Resolve node, type and provider identities before execution. Preserve pinned running providers during package updates.
- Never replace hot installation with an application rebuild or restart.
- Credentials stay in host-owned OS stores; graphs, examples, traces and exports hold references and redacted data. Native plugins are trusted code, not sandboxed.
- No generic plugin calls, blocking work or allocations in audio callbacks. Check device routing per OS rather than inferring it from playback.
- Update contracts, schemas and ADRs in the same change as the behavior. Preserve user edits and unrelated files.
- Verify changed behavior with focused checks; report exact commands and results, and never claim a check that did not run.
- Use the configured human Git author. Do not append AI co-author trailers or generated-with footers to commits or pull requests.
