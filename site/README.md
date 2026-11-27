# OrangeEditor documentation site

The public guides are listed in `site/pages.mjs`, grouped as tutorials, how-to guides, concepts and reference (the Diátaxis split). Write a new guide in Markdown as `site/guides/<slug>.md`, starting with a `# Title`; the builder renders it with the pinned toolchain's rustdoc. Older guides are HTML in `site/pages/`. The builder renders them as static
HTML, generates the node reference from `orange nodes --json`, and includes
the public API from Cargo Rustdoc. The documents listed in `site/references.mjs`
are rendered directly from `docs/` by the pinned Rust toolchain's Markdown
renderer. Node diagrams use `preview_ports` from the host's fresh authoring
view. The configuration-specific notes and links to checked-in graphs live in
`site/node-reference.mjs`. Edit a contract in `docs/` and update any guide affected by the change.

## Translate the guides

English routes stay at the site root. Spanish guides in `site/guides/es/`
publish under `/es/` with the same route suffixes. The locale catalogues under
`site/locales/` hold navigation labels, example text, tutorial captions, and
node help added by the site. The page order and English guide metadata are in
`site/pages.mjs`. Register new catalogues in `site/locales/index.mjs`. A new
published language also needs a route, a search index, and renderer coverage.
Use `@shared/`
to link to generated Rustdoc and the English contracts. Match explicit heading IDs
where the two guides discuss the same topic. Node cards and localized port
descriptions come from the CLI catalogue.

## Build and check locally

Run from the repository root with the pinned Rust toolchain and Node 24.

```sh
cargo doc --no-deps --locked -p orange-host -p orange-editor-ui -p orange-ui-controls -p orange-sdk
node site/build.mjs
node site/check.mjs
```

The builder runs `orange nodes --json` from the current checkout. For iterative
editing, it also accepts `--catalogue <file>` and `--rustdoc <directory>`. Both
inputs must come from the same checkout as the guides. Rustdoc and generated
intermediates use the Cargo target directory reported by `cargo metadata`,
including `CARGO_TARGET_DIR` overrides. For cross-target Rustdoc, point
`--rustdoc` at that target's `doc/` directory. Serve `site/dist/` over HTTP to
try search.

## Build the live controls demo

The controls reference loads `orange-docs-web` when the visitor selects **Try it**. It runs shared Rust controls through eframe and WebGL, using sample
values. The page and its reference links work without loading the demo.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version '=0.2.129' --locked
node site/build.mjs --wasm-bindgen wasm-bindgen
node site/check.mjs --require-demo
```

The CLI version must match the workspace's `wasm-bindgen` dependency. The Pages
workflow runs this build and publishes only `site/dist/`. The checkout's ignore
rules cover that directory and Cargo's default `target/`. Keep generated
Rustdoc, JavaScript bindings, and Wasm out of Git.

## Render editor node previews

Use the desktop renderer to generate previews with its node palette geometry,
fonts, icons, and colors in light and dark. The site also takes its light and
dark colors from the installed editor style. Render once per source revision.
This needs a wgpu adapter, but it runs without creating a window. Pages uses a
software Vulkan adapter on its Linux runner.

```sh
cargo run --locked -p orange-desktop --features documentation --example docs_node_cards -- .artifacts/native-docs-cards
node site/build.mjs --node-previews .artifacts/native-docs-cards
node site/check.mjs --require-native-previews
```

The same exporter opens the checked-in counter and JSON extraction graphs in the
editor and renders their canvas for the tutorial. The site keeps linked port
diagrams and tables for keyboard navigation. A build without `--node-previews`
displays the diagrams by default.

## Publish with GitHub Pages

Set the repository's **Settings → Pages → Build and deployment → Source** to
**GitHub Actions**. The Documentation Pages workflow publishes from `main`.
To preview before merging, run the workflow on the documentation branch once
the workflow is available on the default branch, or open a PR that changes the
site. The run uploads a `documentation-preview` artifact for non-main builds.

The site uses relative navigation so it works under the project prefix. The
Pages configuration supplies the canonical URL used by the sitemap and
`llms.txt`. The expected project URL is
[erinmel.github.io/OrangeEditor](https://erinmel.github.io/OrangeEditor/).

## Maintain stable links

Use explicit IDs on guide headings. Keep routes stable when changing a heading or
node display name. Each generated node page is addressed as
`nodes/<namespace>/<name>/<definition-version>/`; the JSON catalogue carries that
same identity. Guide source links point to the Git revision used for the build.
