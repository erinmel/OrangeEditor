# Application crates

Proposed members from [02 Architecture](../../docs/02-architecture.md), created incrementally by their delivery subunits:

| Future directory | Owns |
|---|---|
| `orange-model/` | Documents, stable IDs, types and transactions |
| `orange-contracts/` | Internal host/wire contracts and structured errors |
| `orange-compiler/` | Validation and immutable execution plans |
| `orange-runtime/` | Run actors, state and scheduling |
| `orange-services/` | File, vault, connection and audio services |
| `orange-packages/` | Installation, registry and provider lifecycle |
| `orange-host/` | Composition, async runtime, policy and locale service |
| `orange-bundled/` | Privileged statically linked base kernels/backend implementations |
| `orange-editor/` | Document commands/history and accessible view model |
| `orange-scene-bevy/` | Optional scene/render adapter |

These are reserved directories, not created Cargo packages. Core graph crates remain independent of Bevy. External plugin SDK artifacts are published under [sdk/](../../sdk/README.md); installing a plugin does not import private application crates. The bundled-native privilege exception is separate from the installable packages under [plugins/](../../plugins/README.md).
