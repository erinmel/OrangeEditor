# OrangeEditor documentation site

The public guides live in `site/pages/`. The builder renders them as static
HTML, generates the node reference from `orange nodes --json`, and includes
the public API from Cargo Rustdoc. The documents listed in `site/references.mjs`
are rendered directly from `docs/` by the pinned Rust toolchain's Markdown
renderer. Edit a contract in `docs/` and update any guide affected by the change.

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

The egui guide loads `orange-docs-web` when the visitor selects **Load controls
demo**. It runs shared Rust controls through eframe and WebGL, using sample
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
