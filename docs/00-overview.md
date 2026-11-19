# Overview

## Vision

OrangeEditor is a node editor foundation: a strong, typed graph language and the
pieces to build products on it.

- **People** use the desktop editor to automate and react: a stream overlay that
  flashes on each kick drum, a variable that counts MQTT messages, a request fired by a
  hotkey. Cards edit in place, values show on the wires while a graph runs, and nothing
  needs a rebuild or a restart.
- **Plugin authors** add nodes, triggers and connection kinds as installed packages
  (Wasm components or native libraries) that load while the editor is open.
- **Product builders** embed the headless core (`orange-host` and below) in their own
  Rust application, with or without the egui editor.

The language never guesses: types are explicit, every conversion is a visible node,
pure values are computed once per demand, effects run where the control flow says, and
a package node is pinned to the exact provider a graph was built with.

## Scope of 1.0

| Area | In 1.0 |
|---|---|
| Language | Typed math and logic, text, lists, maps, JSON, options and results, user enums and records, variables, branching, loops, time, errors and cancellation, graph-local functions |
| Triggers | Manual, start, timers, keys, custom events, conditions on values ("When"), file changes, network messages, audio levels and hits |
| Services | HTTP/HTTPS, WebSocket, MQTT, AMQP 0-9-1, files in a granted folder, credentials in the OS store |
| Audio | Play and control sound files, input devices, system and per-application capture, levels, frequency bands, transients, and stream effects: noise gate, compressor, limiter, gain and filters |
| Editor | Canvas with editable cards, palette search in English and Spanish, properties, declarations, connections, packages, problems, log, undo |
| Extension | Installed packages with hot install, update and removal; native plugin libraries |

Specialized integrations (OBS, Twitch, TTS, browser sources) belong in plugins, not in
the foundation.

## Status

| Area | State |
|---|---|
| Graph model, validation, inference, compiler, runtime | Working; normative rules in [11](11-graph-language.md) and [12](12-graph-data-structures.md) |
| Host: drafts, flows, effects, previews, packages | Working; an edit on a 5,000-node graph takes about 25 ms ([10](10-development.md#performance)) |
| Editor and desktop application | Working on Windows; Linux and macOS build in CI but are not used day to day |
| Network, files, credentials | Working, with a Docker lab for HTTP, WebSocket, MQTT and AMQP |
| Audio | Working on Windows; Linux per-application capture needs PipeWire and is not verified on hardware; macOS per-application capture is not implemented |
| Wasm packages | Pure, action and async nodes run; host service imports and resources are missing |
| Native plugins | Load at start; not sandboxed |
| File format | Version 1, the first release; later versions upgrade it on open (12 §6.3) |
| Releases | Packaging scripts exist; publishing is disabled until a release is qualified |

## What is missing

These are the gaps between the vision and the repository today, most important first.

1. **Documentation for people who use it.** No getting-started guide, no tutorial and
   no node reference. A reference could be generated from the node catalogue, which
   already carries names and purposes in English and Spanish.
2. **A threat model.** Credentials, package grants, native plugins (not sandboxed) and
   network access are each described, but nowhere together with what an attacker could
   do and what stops it.
3. **Plugin and embedding guides.** [04](04-expansion-plugins.md) is a contract, not a
   walkthrough, and the public Rust API has no stability policy that says what may change
   in a minor release.
4. **Flow-started subscriptions.** Event outputs on actions (11 §10.4) are specified but
   not implemented; triggers are always armed while a graph runs.
5. **Audio tuning surface.** Band and transient detection work, but there is no spectrum
   view with hit markers for tuning them, and When thresholds cannot be wired.
6. **Platform qualification.** Frame time, memory and latency targets are measured on
   one Windows machine only; macOS and Linux audio, input and capture need real runs.
7. **Wasm guest services.** Installed Wasm packages cannot yet call host services or own
   resources, so service plugins must be native libraries today.
8. **Accessibility.** Keyboard use works for shortcuts and search; screen reader output
   from the canvas has not been checked.

## Glossary

| Term | Meaning |
|---|---|
| Graph, document | A saved `.json` file of nodes, wires and declarations |
| Draft | A document being edited in a host, with undo and incremental checking |
| Flow | One running document in a host, on its own thread |
| Node, card | A unit of the language; a card is how the editor draws it |
| Trigger, entry | A node that starts control flow: manual, timer, key, message, audio |
| Package | An installable bundle of node definitions and their providers |
| Connection | A declared service profile (a broker, a base URL, a folder) nodes refer to |
