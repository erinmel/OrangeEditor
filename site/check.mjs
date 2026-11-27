import assert from 'node:assert/strict';
import {readFile, stat} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {prepareIndex, searchIndex} from './assets/search-index.mjs';
import {references} from './references.mjs';
import {pages as guidePages} from './pages.mjs';
import {demoAssets} from './live-demo.mjs';

const root = new URL('./dist/', import.meta.url);
const read = path => readFile(new URL(path, root), 'utf8');
const readJson = async path => JSON.parse(await read(path));
const catalogue = await readJson('nodes.json');
const search = await readJson('search.json');
const spanishSearch = await readJson('es/search.json');
const sitemap = await read('sitemap.xml');
const home = await read('index.html');
const canonical = html => html.match(/rel="canonical" href="([^"]+)"/)?.[1];
const base = new URL(canonical(home));
const requireNativePreviews = process.argv.includes('--require-native-previews');
if (requireNativePreviews) {
  assert.match(home, /href="\.\/assets\/editor-theme\.css"/);
  assert.match(await read('assets/editor-theme.css'), /:root\[data-theme="dark"\]/);
}
const pages = new Map();
const paths = new Set();

for (const entry of [...search, ...spanishSearch]) {
  const [path] = entry.path.split('#');
  paths.add(path);
}
assert.equal(paths.size, 2 * (catalogue.nodes.length + guidePages.length) + references.length);
const egui = await read('egui/index.html');
const controls = await read('controls/index.html');
assert.match(egui, /href="\.\.\/controls\/#live-controls"/);
const usage = controls.indexOf('orange_ui_controls::continuous_slider');
const expected = controls.indexOf('The level starts at 55%');
const source = controls.indexOf('Read the Rust demo source');
assert.ok(usage >= 0 && usage < expected && expected < source, 'Show the code and expected result alongside the demo source');
const hasDemo = controls.includes('id="controls-demo"');
if (process.argv.includes('--require-demo')) assert.ok(hasDemo, 'The Pages artifact must include the live demo');
if (hasDemo) {
  assert.ok(source < controls.indexOf('id="load-controls-demo"'));
  assert.match(controls, /src="\.\.\/assets\/controls-demo\.mjs"/);
  const [glue, wasm] = demoAssets;
  assert.match(await read('assets/controls/' + glue), /class ControlsDemo/);
  const bytes = await readFile(new URL('assets/controls/' + wasm, root));
  assert.ok(WebAssembly.validate(bytes), 'Invalid browser Wasm');
}
for (const [file, slug] of references) {
  const html = await read('reference/' + slug + '/index.html');
  assert.ok(html.includes('/docs/' + file + '">View Markdown source'));
}

for (const path of paths) {
  const html = await read(path + 'index.html');
  const address = new URL(path, base);
  assert.equal(canonical(html), address.href);
  assert.ok(sitemap.includes('<loc>' + address.href + '</loc>'));
  assert.equal([...html.matchAll(/<h1>/g)].length, 1, path);
  assert.match(html, /<main id="main"><article>/);
  assert.match(html, /class="search-trigger"[^>]*aria-haspopup="dialog"/);
  assert.match(html, /class="toolbar-menu"/);
  assert.match(html, /class="theme-picker"/);
  assert.match(html, /data-search-filter="nodes"/);
  const spanish = path.startsWith('es/');
  assert.ok(html.includes('<html lang="' + (spanish ? 'es' : 'en') + '">'), path);
  if (!path.startsWith('reference/')) {
    const alternate = new URL(spanish ? path.slice(3) : 'es/' + path, base);
    assert.ok(html.includes('hreflang="' + (spanish ? 'en' : 'es') + '" href="' + alternate.href + '"'), path);
    for (const [, href] of html.matchAll(/rel="(?:prev|next)" href="([^"]+)"/g)) {
      const neighbor = new URL(href, address).href.slice(base.href.length);
      assert.equal(neighbor.startsWith('es/'), spanish, href + ' from ' + path);
      assert.ok(paths.has(neighbor) && neighbor !== path, href + ' from ' + path);
    }
  }
  const ids = [...html.matchAll(/\sid="([^"]+)"/g)].map(([, id]) => id);
  assert.equal(new Set(ids).size, ids.length, 'Duplicate IDs in ' + path);
  pages.set(address.href, new Set(ids));
}

for (const path of paths) {
  const html = await read(path + 'index.html');
  const address = new URL(path, base);
  for (const [, href] of html.matchAll(/\b(?:href|src)="([^"]+)"/g)) {
    const link = new URL(href, address);
    const sourcePath = link.pathname.match(new RegExp(
      '^/erinmel/OrangeEditor/(blob|tree)/' + catalogue.revision + '/(.+)$'));
    if (link.hostname === 'github.com' && sourcePath) {
      const [, kind, path] = sourcePath;
      const source = await stat(new URL('../' + path, import.meta.url));
      assert.ok(kind === 'tree' ? source.isDirectory() : source.isFile(), href);
    }
    if (!link.href.startsWith(base.href)) continue;
    const anchor = decodeURIComponent(link.hash.slice(1));
    link.hash = '';
    const target = new URL(link.href.slice(base.href.length), root);
    if (link.pathname.endsWith('/')) target.pathname += 'index.html';
    assert.ok((await stat(fileURLToPath(target))).isFile(), link.href);
    if (anchor && pages.has(link.href)) assert.ok(pages.get(link.href).has(anchor), href + ' in ' + path);
  }
}

for (const item of [...search, ...spanishSearch]) {
  const [path, anchor] = item.path.split('#');
  if (anchor) assert.ok(pages.get(new URL(path, base).href).has(anchor), item.path);
}
for (const node of catalogue.nodes) {
  const route = 'nodes/' + node.id + '/' + node.version + '/';
  assert.ok(paths.has(route), 'Missing node page ' + node.id);
  const html = await read(route + 'index.html');
  const es = await read('es/' + route + 'index.html');
  assert.match(html, /class="node-stage"><div class="node-card"/);
  assert.match(es, /class="node-stage"><div class="node-card"/);
  assert.match(html, /<h2 id="ports">Ports<\/h2>/);
  assert.match(es, /<h2 id="ports">Puertos<\/h2>/);
  assert.ok(es.includes('id="identity"'), route);
  if (requireNativePreviews) {
    assert.match(html, /class="native-light" src="[^"]+\/en\/[^"]+\/light\.png"/);
    assert.match(es, /class="native-dark" src="[^"]+\/es\/[^"]+\/dark\.png"/);
  }
  assert.ok(search.some(item => item.path === route && item.title === node.name.en && item.alias === node.name.es));
  assert.ok(spanishSearch.some(item => item.path === 'es/' + route && item.title === node.name.es && item.alias === node.name.en));
}
assert.ok((await read('llms.txt')).includes(new URL('sdk/', base).href));
const index = prepareIndex(search);
assert.equal(searchIndex(index, 'puénte-salida')[0]?.path, 'nodes/core/bridge-out/1/');
assert.equal(searchIndex(index, 'core/emit@1')[0]?.path, 'nodes/core/emit/1/');
assert.equal(searchIndex(index, 'brnch')[0]?.path, 'nodes/core/branch/1/');
assert.equal(searchIndex(index, 'embed and customize the egui editor')[0]?.path, 'egui/');
assert.deepEqual(searchIndex(index, '<>{}'), []);
const tutorial = await read('count-button-presses/index.html');
assert.match(tutorial, /href="\.\.\/examples\/#counter-with-reset"/);
assert.match(tutorial, /<details class="toc-inline">[\s\S]*?href="#run-it"/);
const examplePage = await read('examples/index.html');
assert.match(examplePage, /id="counter-with-reset"[\s\S]*?href="graphs\/counter-with-reset\.json" download/);
const counterGraph = await readJson('examples/graphs/counter-with-reset.json');
assert.ok(counterGraph.variables.some(variable => variable.name === 'Counter'));
assert.match(await read('assets/lucide-license.txt'), /Lucide Icons and Contributors/);
const branchPage = await read('nodes/core/branch/1/index.html');
const rama = await read('es/nodes/core/branch/1/index.html');
assert.match(rama, /<h1>Rama<\/h1>/);
assert.match(rama, /href="#port-condition">condición<\/a>/i);
assert.match(rama, /El booleano que elige la rama/);
assert.equal(searchIndex(prepareIndex(spanishSearch), 'Branch')[0]?.path, 'es/nodes/core/branch/1/');
assert.equal(searchIndex(prepareIndex(spanishSearch), 'puene salida')[0]?.path, 'es/nodes/core/bridge-out/1/');
assert.equal(searchIndex(prepareIndex(spanishSearch), 'cuenta las pulsaciones de un botón')[0]?.path, 'es/count-button-presses/');
const spanishTutorial = await read('es/count-button-presses/index.html');
const loopTutorial = await read('repeat-with-a-loop/index.html');
const spanishLoop = await read('es/repeat-with-a-loop/index.html');
assert.match(spanishTutorial, /<code>Counter = 3<\/code>/);
for (const page of [tutorial, spanishTutorial]) {
  assert.match(page, /class="node-stage graph-wire"/);
  assert.match(page, /nodes\/core\/on-manual\/1\/#port-then/);
  assert.match(page, /nodes\/core\/increment-counter\/1\/#port-in/);
  assert.match(page, /nodes\/org\.orange\.json\/parse\/1\//);
  assert.match(page, /nodes\/org\.orange\.json\/read-text\/1\//);
  assert.match(page, /<code>Studio mic<\/code>/);
}
if (requireNativePreviews) {
  // Each guide shows the editor pictures it was written around, in both languages.
  const pictures = {
    'count-button-presses': ['editor-counter-properties', 'editor-counter-variables'],
    'repeat-with-a-loop': ['power-loop-start'],
    'run-a-graph-from-rust': ['counter-with-reset'],
    'call-an-http-api': ['editor-http-connections', 'http-with-failure-handling', 'http-with-bounded-retry'],
    'react-to-file-changes': ['editor-file-trigger', 'file-change-journal'],
    'share-between-graphs': ['shared-score'],
    'data-and-execution': ['json-extraction'],
  };
  for (const [slug, graphs] of Object.entries(pictures)) {
    for (const language of ['en', 'es']) {
      const page = await read((language === 'es' ? 'es/' : '') + slug + '/index.html');
      for (const graph of graphs) {
        assert.ok(page.includes('assets/tutorial-graphs/' + language + '/' + graph + '/light.png'), slug + ' ' + language + ' ' + graph);
      }
    }
  }
  for (const [language, page] of [['en', loopTutorial], ['es', spanishLoop]]) {
    for (const theme of ['light', 'dark']) {
      assert.ok(page.includes('assets/tutorial-graphs/' + language + '/power-with-count/' + theme + '.png'));
    }
  }
  for (const [language, page] of [['en', tutorial], ['es', spanishTutorial]]) {
    for (const graph of ['counter-first-step', 'counter-with-reset', 'json-extraction']) {
      for (const theme of ['light', 'dark']) {
        const file = 'assets/tutorial-graphs/' + language + '/' + graph + '/' + theme + '.png';
        assert.ok(page.includes(file), file);
        const bytes = await readFile(new URL(file, root));
        assert.ok(bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])), file);
      }
    }
  }
}
assert.match(spanishTutorial, /rel="prev" href="\.\.\/getting-started\//);
assert.match(spanishTutorial, /rel="next" href="\.\.\/repeat-with-a-loop\//);
for (const page of [loopTutorial, spanishLoop]) {
  assert.match(page, /nodes\/core\/count\/1\/#port-body/);
  assert.match(page, /<code>power = 81<\/code>/);
}
for (const page of [await read('run-a-graph-from-rust/index.html'), await read('es/run-a-graph-from-rust/index.html')]) {
  assert.match(page, /examples\/run_counter\.rs/);
  assert.match(page, /Counter = Some\(I64\(3\)\)/);
}
assert.match(branchPage, /Boolean that chooses the branch/);
assert.match(branchPage, /<ul class="port-legend">/);
assert.ok(branchPage.indexOf('<h2 id="try-it">') < branchPage.indexOf('<h2 id="ports">'));
assert.match(branchPage, /Route reads matched/);
assert.match(branchPage, /examples\/#http-with-failure-handling/);
const trigger = catalogue.nodes.find(node => node.id === 'core/on-manual');
assert.ok(trigger?.preview_ports?.some(port => port.id === 'then' && port.execution));
const branch = catalogue.nodes.find(node => node.id === 'core/branch');
assert.ok(branch?.purpose.en && branch?.purpose.es && Array.isArray(branch.aliases.en));
// Every language the editor ships is exported, and a real Japanese name reaches the page.
assert.ok(catalogue.nodes.every(node => ['en', 'es', 'ja'].every(language => language in node.name)));
assert.ok(branch.name.ja && branch.name.ja !== branch.name.en, 'core/branch has a Japanese name');
assert.ok(branchPage.includes('lang="ja"'));
console.log('Verified ' + paths.size + ' static pages, ' + catalogue.nodes.length + ' registry identities, local links, and search anchors.');
