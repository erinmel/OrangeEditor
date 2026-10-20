# OrangeEditor application

This area holds the editor and engine. It currently contains layout documentation, with implementation following the [delivery plan](../docs/07-delivery-plan.md).

- [apps/](apps/README.md): desktop and headless entrypoints.
- [crates/](crates/README.md): graph model, compiler, runtime, editor, host and service crates.

The host's future Cargo workspace will live at the repository root and include these application members. Installable expansion packages live in [plugins/](../plugins/README.md); they use the public [SDK](../sdk/README.md) and independent builds/versions. Keeping both source areas in one repository does not link guest implementations into the application or replace hot installation with a rebuild.

No Cargo workspace or production code is scaffolded yet. Read the [monorepo contract](../docs/15-monorepo-layout.md) before adding members.
