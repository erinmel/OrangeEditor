# OrangeEditor documentation site

The public guides live in `site/pages/`. A dependency-free Node builder renders the
HTML, exports the default CLI node catalogue, and includes Rustdoc. The published
site is static and uses client-side JavaScript for search. To edit a product
contract, update the corresponding document under `docs/` and update any guide
affected by the change.

## Build and check locally

Run from the repository root with the pinned Rust toolchain and Node 24.

```sh
cargo doc --no-deps --locked -p orange-host -p orange-editor-ui -p orange-ui-controls -p orange-sdk
node site/build.mjs
node site/check.mjs
```

The builder runs `orange nodes --json` from the current checkout. For iterative
editing, it also accepts `--catalogue <file>` and `--rustdoc <directory>`. Both
inputs must come from the same checkout as the guides. Serve `site/dist/` using
a local HTTP server to try search.

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
