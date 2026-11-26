import assert from 'node:assert/strict';
import {readFile, stat} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {prepareIndex, searchIndex} from './assets/search-index.mjs';

const root = new URL('./dist/', import.meta.url);
const read = path => readFile(new URL(path, root), 'utf8');
const readJson = async path => JSON.parse(await read(path));
const catalogue = await readJson('nodes.json');
const search = await readJson('search.json');
const sitemap = await read('sitemap.xml');
const home = await read('index.html');
const canonical = html => html.match(/rel="canonical" href="([^"]+)"/)?.[1];
const base = new URL(canonical(home));
const pages = new Map();
const paths = new Set();

for (const entry of search) {
  const [path] = entry.path.split('#');
  paths.add(path);
}
assert.equal(paths.size, catalogue.nodes.length + 9);

for (const path of paths) {
  const html = await read(path + 'index.html');
  const address = new URL(path, base);
  assert.equal(canonical(html), address.href);
  assert.ok(sitemap.includes('<loc>' + address.href + '</loc>'));
  assert.equal([...html.matchAll(/<h1>/g)].length, 1, path);
  assert.match(html, /<main id="main"><article>/);
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
    const anchor = link.hash.slice(1);
    link.hash = '';
    const target = new URL(link.href.slice(base.href.length), root);
    if (link.pathname.endsWith('/')) target.pathname += 'index.html';
    assert.ok((await stat(fileURLToPath(target))).isFile(), link.href);
    if (anchor && pages.has(link.href)) assert.ok(pages.get(link.href).has(anchor), href + ' in ' + path);
  }
}

for (const item of search) {
  const [path, anchor] = item.path.split('#');
  if (anchor) assert.ok(pages.get(new URL(path, base).href).has(anchor), item.path);
}
for (const node of catalogue.nodes) {
  const route = 'nodes/' + node.id + '/' + node.version + '/';
  assert.ok(paths.has(route), 'Missing node page ' + node.id);
  assert.ok(search.some(item => item.path === route && item.title === node.name.en && item.alias === node.name.es));
}
assert.ok((await read('llms.txt')).includes(new URL('sdk/', base).href));
const index = prepareIndex(search);
assert.equal(searchIndex(index, 'puénte-salida')[0]?.path, 'nodes/core/bridge-out/1/');
assert.equal(searchIndex(index, 'core/emit@1')[0]?.path, 'nodes/core/emit/1/');
assert.equal(searchIndex(index, 'embed and customize the egui editor')[0]?.path, 'egui/');
assert.deepEqual(searchIndex(index, '<>{}'), []);
const branch = catalogue.nodes.find(node => node.id === 'core/branch');
assert.ok(branch?.purpose.en && branch?.purpose.es && Array.isArray(branch.aliases.en));
console.log('Verified ' + paths.size + ' static pages, ' + catalogue.nodes.length + ' registry identities, local links, and search anchors.');
