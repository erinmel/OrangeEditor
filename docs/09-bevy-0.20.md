# Bevy 0.20 integration dossier

Research date: 2026-10-20. Status: source-backed planning; no Bevy program has been compiled or run in this assignment. This document is an integration guide, not a mirror of Rustdoc. Decisions and APIs below must be checked again against the pinned release when their work unit begins.

## Version baseline

| Item | Verified evidence | Project rule |
|---|---|---|
| Release | 0.20.0, released 2026-10-08 ([S-B01](https://bevy.org/news/bevy-0-20/)) | Plan for exactly `=0.20.0`; no automatic minor update. |
| Source | Tag `v0.20.0` exists; manifest identifies version 0.20.0 ([S-B02](https://github.com/bevyengine/bevy/blob/v0.20.0/Cargo.toml)) | Release-tag source wins over main-branch examples. Record commit and package checksum when dependencies are actually fetched. |
| Rust | Release manifest declares Rust 1.97.1 and edition 2024 (S-B02) | Pin a tested toolchain at least 1.97.1 in the implementation phase. The selected local toolchain, Rust 1.99.1, meets the minimum; see [10](10-tooling-and-sources.md#workspace-and-environment). No Bevy build has used it yet (B-P01). |
| License | Release manifest declares MIT OR Apache-2.0 (S-B02) | Audit transitive and optional dependencies separately. |
| Features | Release manifest includes `bevy_feathers` depending on UI widgets (S-B02) | Select explicit app features; keep graph-core crates free of renderer/window dependencies. Resolve the complete feature set in B-P01. |

No Cargo manifest or toolchain file has been generated. The pin above is a requirement for future implementation, not an installed dependency.

## Evidence integrity

Direct requests for `https://docs.rs/bevy/0.20.0/bevy/`, its app/UI/schedule pages, and `https://docs.rs/crate/bevy/0.20.0` failed in the research tool. This does not prove the pages are unavailable in a normal browser. Use release source as the fallback.

The opened `https://docs.rs/bevy/latest/bevy/prelude/` returned a **0.19.1** heading, despite search results describing 0.20.0. It is excluded as 0.20 API evidence. Other opened latest pages explicitly identified 0.20.0; those are recorded with that resolved version, but their URLs remain mutable. Replace mutable references with downloaded, version-verified local Rustdoc or release-tag source during B-P01. A search snippet alone is insufficient to establish an API signature.

## Integration map

The OrangeEditor architecture choices in this table are proposed contracts; the cited framework behavior is verified source evidence.

| Area | Verified 0.20 evidence | OrangeEditor integration rule | Validation |
|---|---|---|---|
| App and developer plugins | `Plugin` configures `App`; `build`, `ready`, `finish`, `cleanup` form startup lifecycle. `cleanup` runs before schedules, not on product-package uninstall. [S-B03](https://docs.rs/bevy/latest/bevy/app/trait.Plugin.html), [release source](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_app/src/plugin.rs) | Install Orange's registries/bridges during app setup. Implement expansion activation/deactivation separately; never treat Bevy cleanup as a dynamic unload hook. | B-P01, B-P03 |
| Schedules | Schedule module exposes ordering and execution on `World`. [S-B04](https://docs.rs/bevy/latest/bevy/ecs/schedule/index.html) | Use ECS scheduling for presentation/scene work. Graph side effects and variables follow Orange runtime semantics, not incidental ECS execution order. | B-P03 |
| Buffered messages | `MessageReader` tracks unread messages; `Messages` retention spans two update calls. [S-B05](https://docs.rs/bevy/latest/bevy/ecs/message/index.html) | Use an owned bounded channel for runtime completions. Bevy presentation messages are not a durable event queue; define consumption when a window pauses or stops updating. | B-P03 |
| Events and observers | Event definitions and observer triggering are present in [S-B06 release source](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_ecs/src/event/mod.rs) | Use observers for editor interaction. Do not derive graph delivery, replay or inter-node ordering guarantees from them. | B-P03 |
| Tasks/threads | Source distinguishes frame-bound compute, multi-frame async compute, and I/O-oriented pools. [S-B07](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_tasks/src/usages.rs) | Network runtime ownership and reactor compatibility must be explicit. No assumption that a Bevy pool supplies a Tokio reactor; no blocking file/network work on the presentation thread. | B-P03 |
| UI/Feathers | Feathers is feature-gated editor/inspector widget infrastructure. [S-B08](https://docs.rs/bevy/latest/bevy/feathers/index.html) | Preferred candidate is Bevy UI plus an Orange theme and schema-driven controls. A node canvas, accessible interactions and plugin panels remain application work. | B-P02 |
| Text/focus/input | Release examples index includes IME, multiline input, tab navigation and UI drag/drop. [S-B09](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/README.md) | Example existence is not proof of platform completeness. Test text editing, focus preservation, shortcut routing and clipboard with real OS sessions. | B-P02, B-P06 |
| Picking | Flat pointer event types are defined in [S-B10 release source](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_picking/src/events.rs) | Node, socket and wire hit testing need consistent world/screen coordinates, clipping and spatial indexing. Do not spawn a full widget tree for every offscreen item by default. | B-P02 |
| Rendering/assets | Release examples cover 2D/3D, assets and scenes (S-B09) | Expose typed generation-checked scene/asset handles. Only host-owned adapters touch Bevy entities or render resources. Saved graph IDs survive scene rebuilds. | B-P05 |
| Audio | Public AudioPlugin exposes playback configuration. [S-B11](https://docs.rs/bevy/latest/bevy/audio/struct.AudioPlugin.html). Its release output implementation opens the default sink using an internal resource. [S-B12](https://github.com/bevyengine/bevy/blob/v0.20.0/crates/bevy_audio/src/audio_output.rs) | These sources do not establish per-player device routing or virtual-source support. A separate audio service/backend is the default plan; do not promise these requirements through Bevy audio alone. | B-P04 |
| Headless | Release example disables default features and configures the schedule runner. [S-B13](https://github.com/bevyengine/bevy/blob/v0.20.0/examples/app/headless.rs) | Pure graph execution must not require Bevy at all. Optional Bevy scene execution may use a separate headless host; rendering without a window is a different capability. | B-P05 |
| Linux dependencies | Release document lists distro-specific build packages. [S-B14](https://github.com/bevyengine/bevy/blob/v0.20.0/docs/linux_dependencies.md) | Derive dependency installation from selected features and supported distributions, including Wayland/X11 and audio. Build prerequisites are not proof of portable packaging. | B-P06 |

## Migration traps to keep visible

The official [0.19-to-0.20 migration guide (S-B15)](https://bevy.org/learn/migration-guides/0-19-to-0-20/) verifies these changes; this list is deliberately limited to Orange's integration risks:

- Pointer events use flat names; reject older generic-pointer examples.
- Editable text state and the text input widget are separate components.
- Lifecycle observer bundle generics moved into event types.
- Some built-in system-set ordering is weak: channel/atomic dependencies outside tracked ECS access need explicit strict ordering.
- UI rendering is retained; old extraction assumptions require review.
- Sprites use the updated mesh/material representation; do not copy older scene bridge snippets.

This dossier intentionally contains no runnable API snippets. Each implementation example must compile against the exact pin before becoming reference material.

## Planned feasibility probes

All probes below are **not run**. Deliver evidence into a future `docs/evidence/` directory: source versions, exact commands, feature set, OS/hardware, output, pass/fail and resulting ADR change. Toolchain installation and prototype code belong to the later feasibility phase, outside this documentation assignment.

| ID | Minimal experiment | Acceptance / architectural consequence |
|---|---|---|
| B-P01 | Install/select a supported pinned Rust toolchain when implementation begins; verify release metadata and retrieve versioned local docs. Compile the smallest exact-0.20 app with a proposed minimal feature set. Use `cargo metadata --locked`, `cargo check --locked`, `cargo tree -e features` after a manifest/lockfile actually exists. | Exact Bevy resolution and MSRV verified; no accidental full-engine dependency in graph-core crates; all referenced symbols backed by 0.20 local docs. Failure blocks Bevy integration, not requirements work. |
| B-P02 | Build a canvas slice with numeric/text/enum settings, plugin-declared panel, wire drag, 10,000 total nodes / approximately 500 visible, focus-preserving edits, IME and keyboard navigation, expanded/RTL pseudo locales, CJK shaping/font fallback, plural/fallback and language switching during composition; add 1,000-visible-node stress measurements separately. | Meet the W4 budgets in [06](06-quality-and-validation.md), screen-reader/keyboard requirements, [13](13-internationalization.md) L-05/06 prototypes and platform input checks. Record missing shaping, bidi, font or accessible-text capabilities explicitly. Missing accessibility or unacceptable interaction requires an explicit UI ADR change, not silently dropping features. |
| B-P03 | Run headless async actions through a bounded bridge while Bevy updates, pauses presentation and resumes; inject task failure, queue saturation, cancel/stop, and a stale generation result. Randomize independent ECS order. | No lost accepted graph completion, no dependence on message retention, deterministic graph state under declared ordering, bounded shutdown and no stale write. |
| B-P04 | Decode and route two simultaneous players to different real/virtual endpoints; remove one; create and remove an app-owned PipeWire virtual source on Linux. | Verify channel/rate mapping, cleanup and failure policy on actual supported OS/backend combinations. Demonstrate requirements independently of Bevy AudioPlugin. Default-sink playback alone fails this probe. |
| B-P05 | Run a saved graph without display/GPU, then the same logic with a Bevy scene bridge; delete/recreate assets and scene entities while handles exist. | Core succeeds with no Bevy/window. Bridge rejects stale handles and reports missing capabilities; scene rendering stays optional. |
| B-P06 | Build and launch the minimal app on Windows, macOS, Linux; exercise Wayland/X11 where supported, DPI, native dialogs, clipboard and OS secret-store availability. Inventory runtime libraries. | Explicit supported-target matrix, install/runtime prerequisites, reproducible CI builds and real-machine evidence; cross-compilation alone is not execution verification. |

The product readiness gate must resolve source/toolchain/UI/plugin-boundary risks. Full Bevy integration is not approved merely because 0.20 is released.
