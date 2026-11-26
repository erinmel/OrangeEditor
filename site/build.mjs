import {execFileSync} from 'node:child_process';
import {copyFile, cp, lstat, mkdir, readFile, realpath, rm, stat, writeFile} from 'node:fs/promises';
import {join, relative, resolve, sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import {parseArgs} from 'node:util';
import {compileDemo, demoAssets} from './live-demo.mjs';
import {loadReferences, renderMarkdown} from './references.mjs';
import {pages, sections} from './pages.mjs';
import {loadCliUsage, renderCliReference} from './cli-reference.mjs';
import {loadNodeExamples, renderGraphExamples, renderNodeReference} from './node-reference.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const site = join(root, 'site');
const output = join(site, 'dist');
const repository = 'https://github.com/erinmel/OrangeEditor';
const revision = execFileSync('git', ['rev-parse', 'HEAD'], {cwd: root, encoding: 'utf8'}).trim();
const source = repository + '/blob/' + revision + '/';
const tree = repository + '/tree/' + revision + '/';
const {values} = parseArgs({options: {
  catalogue: {type: 'string'},
  rustdoc: {type: 'string'},
  'wasm-bindgen': {type: 'string'},
  'base-url': {type: 'string', default: 'https://erinmel.github.io/OrangeEditor/'},
}});
const base = new URL(values['base-url'].replace(/\/?$/, '/'));
if (!['http:', 'https:'].includes(base.protocol) || base.search || base.hash) {
  throw new Error('The base URL must be an HTTP(S) directory URL.');
}
const {target_directory: target} = JSON.parse(execFileSync('cargo',
  ['metadata', '--format-version=1', '--no-deps', '--locked'], {cwd: root, encoding: 'utf8'}));
const rustdoc = resolve(values.rustdoc ?? join(target, 'doc'));
const contracts = await loadReferences(root, target, source, tree);
const examples = await loadNodeExamples(root);
const demo = values['wasm-bindgen'] ? await compileDemo(root, target, values['wasm-bindgen']) : null;
const apiCrates = ['orange_host', 'orange_editor_ui', 'orange_ui_controls', 'orange_sdk'];
for (const crate of apiCrates) await stat(join(rustdoc, crate, 'index.html'));

const catalogue = values.catalogue
  ? await readFile(values.catalogue, 'utf8')
  : execFileSync('cargo', ['run', '-q', '-p', 'orange-cli', '--locked', '--', 'nodes', '--json'],
    {cwd: root, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024});
const {nodes} = JSON.parse(catalogue.replace(/^\uFEFF/, ''));
if (!Array.isArray(nodes) || nodes.length === 0) throw new Error('The node catalogue is empty.');

const pathOf = slug => slug ? slug + '/' : '';
const escape = value => String(value).replace(/[&<>"']/g, char =>
  ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}[char]));
const plain = html => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim();
const nodePath = node => 'nodes/' + node.id + '/' + node.version + '/';
// Japanese is a preview: a message it lacks falls back to English, which adds nothing.
const translated = (texts, language) => texts?.[language] && texts[language] !== texts.en ? texts[language] : '';
const compare = new Intl.Collator('en', {numeric: true}).compare;
const sorted = [...nodes].sort((a, b) => compare(a.name.en, b.name.en) || compare(nodePath(a), nodePath(b)));
const paths = new Set();
const search = [];
const sitemap = [];

async function write(path, contents) {
  const file = join(output, path);
  await mkdir(resolve(file, '..'), {recursive: true});
  await writeFile(file, contents);
}

async function render(path, title, description, body, selected, authored = false) {
  if (paths.has(path)) throw new Error('Duplicate page: ' + path);
  paths.add(path);
  const back = '../'.repeat(path.split('/').length - 1) || './';
  // Only the current page's section starts open, so the list stays short.
  const current = pages.find(([slug]) => slug === selected)?.[3] ?? 'start';
  const nav = sections.map(([section, heading]) => '<details class="nav-group"' +
    (section === current ? ' open' : '') + '><summary>' + escape(heading) + '</summary>' +
    pages.filter(page => page[3] === section).map(([slug, name, , , label]) => '<a href="' + back + pathOf(slug) + '"' +
      (slug === selected ? ' aria-current="page"' : '') + '>' + escape(label ?? name) + '</a>').join('\n') +
    '</details>').join('\n');
  const anchors = [...body.matchAll(/<h2 id="([^"]+)">([\s\S]+?)<\/h2>/g)];
  const toc = anchors.length > 1 ? '<aside class="toc"><nav aria-label="On this page"><strong>On this page</strong>' +
    anchors.map(([, id, label]) => '<a href="#' + id + '">' + plain(label) + '</a>').join('') + '</nav></aside>' : '';
  const canonical = new URL(path, base).href;
  sitemap.push(canonical);
  if (authored) {
    search.push({path, title, description, terms: authored === true ? plain(body) : ''});
    for (const [index, match] of anchors.entries()) {
      search.push({path: path + '#' + match[1], title: plain(match[2]), description: title,
        terms: authored === true ? plain(body.slice(match.index + match[0].length, anchors[index + 1]?.index)) : ''});
    }
  }
  await write(path + 'index.html', `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escape(title)} · OrangeEditor</title>
  <meta name="description" content="${escape(description)}">
  <link rel="canonical" href="${escape(canonical)}">
  <link rel="stylesheet" href="${back}assets/docs.css">
  <link rel="icon" href="${back}assets/favicon.svg" type="image/svg+xml">
  <script>try { const t = localStorage.getItem('orange-docs-theme'); if (t && t !== 'auto') document.documentElement.dataset.theme = t; } catch {}</script>
  <script type="module" src="${back}assets/theme.mjs"></script>
  <script type="module" src="${back}assets/search.mjs"></script>
  ${selected === 'egui' && demo ? '<script type="module" src="../assets/controls-demo.mjs"></script>' : ''}
</head>
<body>
  <a class="skip" href="#main">Skip to content</a>
  <header class="top"><a class="brand" href="${back}"><span aria-hidden="true"></span>OrangeEditor</a>
    <button type="button" id="search-open" hidden>Search docs <kbd>/</kbd></button>
    <label class="visually-hidden" for="theme-select">Colour theme</label>
    <select id="theme-select" hidden>
      <option value="auto">Automatic</option>
      <option value="light">Light</option>
      <option value="dark">Dark</option>
      <option value="contrast">High contrast</option>
      <option value="comfort">Comfort</option>
    </select>
    <a class="source" href="${repository}">GitHub ↗</a>
  </header>
  <div class="layout">
    <nav class="sections" aria-label="Documentation">${nav}</nav>
    <main id="main"><article>${body}</article>
      <footer><a href="${repository}/tree/${revision}">Source revision ${revision.slice(0, 8)}</a> · <a href="${back}llms.txt">Plain-text index</a></footer>
    </main>
    ${toc}
  </div>
  <dialog id="search-dialog" aria-labelledby="search-title">
    <div class="dialog-head"><h2 id="search-title">Search documentation</h2><form method="dialog"><button aria-label="Close search">Close</button></form></div>
    <label for="search-query" class="visually-hidden">Search documentation</label>
    <input type="search" id="search-query" placeholder="Search pages, nodes, or IDs…" autocomplete="off">
    <p id="search-status" role="status"></p><ul id="search-results"></ul>
  </dialog>
</body>
</html>
`);
}

const groups = new Map();
for (const node of sorted) {
  if (!/^[a-z0-9][a-z0-9.-]*\/[a-z0-9][a-z0-9.-]*$/.test(node.id) ||
      !Number.isSafeInteger(node.version) || node.version < 1) throw new Error('Invalid ID: ' + node.id);
  const group = node.category.en;
  if (!groups.has(group)) groups.set(group, []);
  groups.get(group).push(node);
}

// Remove only the generated output, and refuse a redirected directory.
const localOutput = join(await realpath(site), 'dist');
const existingOutput = await realpath(output).catch(error => {
  if (error.code !== 'ENOENT') throw error;
});
if (existingOutput && existingOutput !== localOutput) throw new Error('Unexpected output location: ' + existingOutput);
if (existingOutput) await rm(localOutput, {recursive: true});

for (const [slug, title, description] of pages) {
  let body;
  if (slug === 'nodes') {
    body = '<p>The ' + sorted.length + ' definitions below come from the CLI’s active default registry. ' +
      'Each node page explains its behavior and shows the ports from its fresh authoring view. Configuration and inferred types can change the card. ' +
      'See <a href="../nodes.json">the full JSON export</a>.</p>';
    for (const [category, members] of [...groups].sort(([a], [b]) => compare(a, b))) {
      const categoryId = category.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
      body += '<h2 id="category-' + categoryId + '">' + escape(category) + '</h2><ul class="node-list">' +
        members.map(node => '<li><a href="../' + nodePath(node) + '">' + escape(node.name.en) + '</a>' +
          '<span lang="es">' + escape(node.name.es) + '</span>' +
          (translated(node.name, 'ja') ? '<span lang="ja">' + escape(node.name.ja) + '</span>' : '') +
          '</li>').join('') + '</ul>';
    }
  } else if (slug === 'cli') {
    body = renderCliReference(loadCliUsage(root), escape);
  } else if (slug === 'examples') {
    body = renderGraphExamples(examples, sorted, source, escape, nodePath);
  } else {
    const guide = join(site, 'guides', (slug || 'index') + '.md');
    body = await stat(guide).then(() => renderMarkdown(root, target, guide), () =>
      readFile(join(site, 'pages', (slug || 'index') + '.html'), 'utf8'));
    const back = slug ? '../' : './';
    for (const contract of contracts) {
      body = body.replaceAll('@source/docs/' + contract.file, back + 'reference/' + contract.slug + '/');
    }
    body = body.replaceAll('@source/', source).replaceAll('@tree/', tree).replaceAll('@revision@', revision);
    if (slug === 'egui') body = body.replace('<!-- controls-demo -->', demo
      ? '<button type="button" id="load-controls-demo" hidden>Load controls demo</button>' +
        '<p id="controls-demo-status" role="status"></p>' +
        '<canvas id="controls-demo" tabindex="0" aria-label="OrangeEditor slider and knob demo" hidden></canvas>' +
        '<noscript><p>Enable JavaScript to try the controls demo.</p></noscript>'
      : '<p>This build contains the written controls guide. The Pages build includes the interactive demo.</p>');
  }
  if (slug) body = '<h1>' + escape(title) + '</h1><p class="lead">' + escape(description) + '</p>' + body;
  await render(pathOf(slug), title, description, body, slug, slug !== 'nodes');
}
search.push({path: 'nodes/', title: 'Node reference', description: pages.find(([slug]) => slug === 'nodes')[2], terms: 'catalogue converter multilingual registry'});

for (const contract of contracts) {
  await render('reference/' + contract.slug + '/', contract.title, contract.description,
    '<h1>' + escape(contract.title) + '</h1>' + contract.body, 'sdk', 'reference');
}

for (const node of sorted) {
  const path = nodePath(node);
  const identity = node.id + '@' + node.version;
  const description = identity + ' · ' + (node.purpose.en || node.category.en + ' · ' + node.kind);
  const body = '<p><a href="../../../">← Node reference</a></p><h1>' + escape(node.name.en) + '</h1>' +
    '<p class="lead"><code>' + escape(identity) + '</code></p>' +
    (node.purpose.en ? '<p>' + escape(node.purpose.en) + '</p>' : '') +
    '<dl><dt>Spanish name</dt><dd lang="es">' +
    escape(node.name.es) + '</dd>' +
    (node.purpose.es ? '<dt>Spanish description</dt><dd lang="es">' + escape(node.purpose.es) + '</dd>' : '') +
    (translated(node.name, 'ja') ? '<dt>Japanese name</dt><dd lang="ja">' + escape(node.name.ja) + '</dd>' : '') +
    (translated(node.purpose, 'ja') ? '<dt>Japanese description</dt><dd lang="ja">' + escape(node.purpose.ja) + '</dd>' : '') +
    '<dt>Category</dt><dd>' + escape(node.category.en) +
    '</dd><dt>Execution kind</dt><dd>' + escape(node.kind) +
    '</dd><dt>Provider</dt><dd>' + escape(node.provider ?? 'Core') + '</dd></dl>' +
    renderNodeReference(node, examples, escape) +
    (node.config_schema ? '<h2 id="configuration">Configuration schema</h2><pre><code>' +
      escape(JSON.stringify(node.config_schema, null, 2)) + '</code></pre>' : '');
  await render(path, node.name.en, description, body, 'nodes');
  search.push({path, title: node.name.en, description, alias: node.name.es,
    terms: [node.category.en, node.category.es, node.category.ja, node.provider, node.purpose.es,
      node.name.ja, node.purpose.ja,
      ...node.aliases.en, ...node.aliases.es, ...(node.aliases.ja ?? []),
      ...(node.preview_ports ?? []).flatMap(port => [port.id, port.label, port.description]),
      ...(node.ports ?? []).map(port =>
      port.port_id + ' ' + (port.type ? JSON.stringify(port.type) : ''))].filter(Boolean).join(' ')});
}

await mkdir(join(output, 'assets'), {recursive: true});
for (const asset of ['docs.css', 'favicon.svg', 'search.mjs', 'search-index.mjs', 'theme.mjs']) {
  await copyFile(join(site, 'assets', asset), join(output, 'assets', asset));
}
if (demo) {
  await copyFile(join(site, 'assets', 'controls-demo.mjs'), join(output, 'assets', 'controls-demo.mjs'));
  await mkdir(join(output, 'assets', 'controls'), {recursive: true});
  for (const asset of demoAssets) {
    await copyFile(join(demo, asset), join(output, 'assets', 'controls', asset));
  }
}

const rustdocEntries = new Set([
  ...apiCrates, 'crates.js', 'help.html', 'settings.html', 'src-files.js', 'src',
  'search.index', 'static.files', 'trait.impl', 'type.impl',
]);
await cp(rustdoc, join(output, 'api'), {recursive: true, filter: async file => {
  if (file === rustdoc) return true;
  const parts = relative(rustdoc, file).split(sep);
  if (!rustdocEntries.has(parts[0]) || parts.some(part => part.startsWith('.'))) return false;
  const info = await lstat(file);
  return info.isDirectory() || (info.isFile() && /\.(css|html|js|json|png|svg|txt|woff2?)$/.test(file));
}});
await mkdir(join(output, 'examples', 'graphs'), {recursive: true});
for (const example of examples) {
  await copyFile(join(root, example.path), join(output, 'examples', 'graphs', example.slug + '.json'));
}
await write('.nojekyll', '');
await write('nodes.json', JSON.stringify({revision, nodes}));
await write('search.json', JSON.stringify(search));
await write('sitemap.xml', '<?xml version="1.0" encoding="UTF-8"?>\n' +
  '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n' +
  [...sitemap, ...apiCrates.map(crate => new URL('api/' + crate + '/', base).href)]
    .map(url => '  <url><loc>' + escape(url) + '</loc></url>').join('\n') + '\n</urlset>\n');
await write('llms.txt', '# OrangeEditor documentation\n\n' +
  '> OrangeEditor is a Rust foundation for typed node graphs with an egui editor and headless host.\n\n' +
  'Source revision: ' + revision + '\n\n## Guides\n\n' +
  pages.map(([slug, title, description]) =>
    '- [' + title + '](' + new URL(pathOf(slug), base).href + '): ' + description).join('\n') +
  '\n\n## Reference\n\n- [Node catalogue JSON](' + new URL('nodes.json', base).href + ')\n' +
  apiCrates.map(crate => '- [' + crate + ' Rustdoc](' + new URL('api/' + crate + '/', base).href + ')').join('\n') +
  '\n- [Wasm WIT interface](' + source + 'sdk/wit/orange-expansion.wit)\n' +
  contracts.map(contract => '- [' + contract.title + '](' +
    new URL('reference/' + contract.slug + '/', base).href + ')').join('\n') + '\n');
console.log('Built ' + paths.size + ' pages in ' + output);
