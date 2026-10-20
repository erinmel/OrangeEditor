# Application entrypoints

Future workspace members:

| Path | Responsibility |
|---|---|
| `orange-desktop/` | Bevy/window/input adapter and desktop composition |
| `orange-cli/` | Headless graph loading/execution and structured diagnostics |

These paths are reserved by the layout, not implemented binaries. Neither entrypoint defines graph semantics. The CLI must remain usable without initializing a GPU/window. Shared implementation belongs in [app/crates](../crates/README.md), under the dependency boundaries in [02 Architecture](../../docs/02-architecture.md).
