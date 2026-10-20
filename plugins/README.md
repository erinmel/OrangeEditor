# OrangeEditor expansion plugins

Place each maintained expansion in `plugins/<plugin-name>/`. This area is for the plugins you create for OrangeEditor, including the planned OBS and Twitch packages. It currently contains no implemented plugins.

Each future plugin owns its manifest, definitions/type schemas, component or explicitly trusted worker, localization resources, tests, README, license and changelog. Rust plugins have their own build workspace/toolchain/lockfile when needed; Wasm guest targets are not default members of the host Cargo workspace. A folder name is not package identity: manifests use the stable namespaced IDs from [04](../docs/04-expansion-plugins.md).

Example **future** plugin layout:

```text
plugins/my-plugin/
  README.md
  manifest.json
  definitions/
  locales/
  src/
  tests/
  CHANGELOG.md
```

Package manifests and payloads are added only after the relevant SDK work, with schema/compatibility evidence. Build outputs and local profiles/credentials stay out of Git. Shared fixtures/examples may live in [examples/](../examples/README.md), while a plugin's own tests remain with its source.

Plugins target the versioned public [SDK](../sdk/README.md), not private paths into the engine. They retain independent package versions and compatibility ranges. They must remain installable into a running application without rebuilding it, even when maintained in this monorepo. Host and plugin changes that evolve a shared contract may be reviewed in one PR, including compatibility/migration vectors.
