# Expansion SDK

This area is reserved for the versioned public expansion contract: component/WIT interfaces, manifest/node/type schemas, packaging conventions, language bindings and conformance vectors. The actual artifacts are delivered by WU-00/WU-03 after the graph contracts are validated; no SDK has been generated yet.

The SDK is the boundary between [app/](../app/README.md) and [plugins/](../plugins/README.md). It exposes no private engine internals, raw Bevy pointers or native Rust ABI. Internal `orange-contracts` implementation types remain in `app/crates`; public definitions are explicitly exported as versioned wire/schema artifacts here.

Source may be developed together, but SDK/host API versions, application releases and individual plugin package versions are independent. A plugin declares its supported host API range; conformance fixtures verify it. Read [04 Expansion plugins](../docs/04-expansion-plugins.md), [11 Graph language](../docs/11-graph-language.md), [12 Data layouts](../docs/12-graph-data-structures.md) and [13 Internationalization](../docs/13-internationalization.md).
