# OrangeEditor planning baseline

Status: architecture proposal, 2026-10-20. No product implementation, compilation, performance measurement, dependency installation or OS capability test has been performed by this planning assignment. Quantitative budgets are proposed release gates. Product name and current workspace folder are **OrangeEditor**; earlier environment evidence refers to the originally supplied **OraneEditor** path.

OrangeEditor is a Rust desktop and headless visual programming platform for typed calculations, state, events, files, connections and media. Its expansion plugins are installable product packages that add functionality or replace explicitly supported base providers while the application remains open. Bevy **=0.20.0** is the application/graphics pin; it does not define the graph language or external package ABI.

Public source: [erinmel/OrangeEditor](https://github.com/erinmel/OrangeEditor). The repository has `main` protection and passing documentation/policy CI; application and plugin implementation remains future work.

## Document map

| Document | Decision/output |
|---|---|
| [Assignment](AGENT_PROMPT.md) | Reusable, bounded architecture-agent prompt |
| [Agent instructions](AGENTS.md) | Persistent project version, contract and evidence rules |
| [01 Requirements](docs/01-product-requirements.md) | Product perspective, FR IDs, acceptance criteria, priorities and scope |
| [02 Architecture](docs/02-architecture.md) | Crates, ownership, dependency direction and editor decision |
| [03 Data and execution](docs/03-data-and-execution.md) | Value model, document examples, compilation and lifecycle semantics |
| [04 Expansion plugins](docs/04-expansion-plugins.md) | Installable SDK, isolation, overrides and hot lifecycle |
| [05 Services and platforms](docs/05-services-and-platforms.md) | Files, vault, protocols, audio, OBS and Twitch contracts |
| [06 Quality](docs/06-quality-and-validation.md) | NFR IDs, workloads, budgets, test and release evidence |
| [07 Delivery](docs/07-delivery-plan.md) | Dependency-ordered work, readiness gates and risks |
| [08 Decisions](docs/08-decisions.md) | ADRs, alternatives, falsifiable experiments and open decisions |
| [09 Bevy dossier](docs/09-bevy-0.20.md) | Exact-version evidence and feasibility gaps |
| [10 Tooling and sources](docs/10-tooling-and-sources.md) | Environment, dependency candidates, primary references and optional tools |
| [11 Graph language](docs/11-graph-language.md) | **Normative** node kinds, ports, types, conversions, regions, cycles, ordering, events, state, streams, diagnostics and G-suite |
| [12 Graph data structures](docs/12-graph-data-structures.md) | **Normative** document, registry, plan IR and run-state layouts; JSON formats and invariants |
| [13 Internationalization](docs/13-internationalization.md) | **Normative** translatable text, package catalogues, locale/fallback, diagnostics and localization acceptance |
| [14 Git workflow](docs/14-git-workflow.md) | Branches, commits, scoped checks, solo/team reviews, merge/release rules and verified bootstrap status |
| [15 Monorepo layout](docs/15-monorepo-layout.md) | Application/plugin source areas, SDK boundary and independent package builds/releases |
| [Contributing](CONTRIBUTING.md) | Short contributor entrypoint and PR template |

For node editor work, read 01 for product scope, 11 for language semantics, 12 for data layouts, 13 for presentation text, then 02/04 for editor and expansion boundaries and 06/07 for acceptance/dependencies. These are separate contract owners: examples and summaries must agree with them; conflicts are defects to fix, not permission to select a convenient rule. ADRs in 08 record decisions and evidence; 09/10 record research limits rather than proof of implementation.

Development follows [14](docs/14-git-workflow.md): protected `main`, short-lived branches per work subunit, PR checks/review and squash merging. Repository bootstrap/enforcement evidence is recorded there and in 10.

## Monorepo

One repository contains two main source areas: [app/](app/README.md) for the editor/engine and [plugins/](plugins/README.md) for installable expansion packages you create. The [sdk/](sdk/README.md) defines their public boundary; [examples/](examples/README.md) and [tools/](tools/README.md) hold shared examples and repository checks.

This lets a host/SDK/plugin contract change be reviewed and validated together. Each plugin still builds, versions and installs independently. Source sharing never changes hot installation into an application rebuild. These folders currently contain layout documentation; implementation follows the existing delivery gates. See [15](docs/15-monorepo-layout.md) for ownership and future Cargo workspace rules.

## Selected defaults

- Standalone graph model/compiler/runtime; Bevy observes editor/scene state through bounded messages.
- Hybrid execution: lazy memoized data pull, explicit exec-token control, cooperative async suspension, queued events that start activations, and reactive triggers at segment boundaries. One actor per run. Structured regions with a value-availability rule; explicit cycle classes; nominal constrained generics; visible conversions only ([11](docs/11-graph-language.md)).
- Immutable package registry snapshots and exact graph package locks; updates never silently rebind an existing graph.
- Capability-limited Wasm components as the normal expansion format; declarative UI and handle-based graphics. Explicit-trust worker processes are the native escape hatch. A process boundary alone is not an OS sandbox.
- Statically linked bundled kernels for cheap operations and real-time audio internals are a documented privilege exception. Public node, type, provider and package contracts remain shared.
- OS vault references, host-owned connection sessions, explicit error/control semantics and cooperative bounded execution.
- All product UI and expansion text is translatable through namespaced catalogues; UI locale never changes graph values, IDs, provider locks or execution. English is the source fallback; additional shipped languages remain a product choice.
- Windows/macOS existing audio endpoints; Linux application-owned virtual source subject to PipeWire feasibility and session policy. No bundled kernel audio driver.

## Assumptions and limits

Initial AMQ means **AMQP 0-9-1** with RabbitMQ compatibility; AMQP 1.0 is a separate later adapter. Public desktop Twitch authentication uses a supported public-client flow; no embedded confidential client secret. Marketplace, arbitrary custom native editor widgets and full scene authoring are later scope. The final 1.0 baseline includes all protocol families and streaming gRPC modes; early previews explicitly expose an incomplete capability list.

Readiness: **ready for feasibility work after G0 consistency/readiness checks**, with production architecture adoption conditional on the experiments in [delivery](docs/07-delivery-plan.md). Bevy toolchain, UI input/accessibility, component host costs, audio device behavior and plugin lifecycle are not proven by prose. These engineering gates add no mandatory user approval step to future authorized experiments. Read [09](docs/09-bevy-0.20.md) before using any Bevy API; older examples are not a substitute for the pin.

Next executable tasks: **00a, graph-language fixtures and semantic review within WU-00**, and, in parallel, **01a, Bevy evidence review within WU-01**, both in [07](docs/07-delivery-plan.md). The first implementation pass is pure logic: graph rules, execution semantics and data structures (WU-00), with no Bevy dependency. Node, port and type contracts must pass the G-suite before WU-03 freezes the expansion SDK. Bevy work then verifies the exact tag/toolchain/dependency set and builds disposable experiments for text/focus/accessibility and hot expansion before any large editor shell. No product code should precede the G0 consistency/readiness checks.
