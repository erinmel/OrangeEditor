# Installable expansions

Each expansion under this directory has its own build and release lifecycle.
The root Cargo workspace excludes this directory so the application and its
installable expansions can be built independently.

See the [expansion contract](../docs/04-expansion-plugins.md) and the
[repository layout](../docs/15-monorepo-layout.md). The
[native development example](../app/crates/dev/orange-plugin-example) lives
in the application workspace for integration tests.
