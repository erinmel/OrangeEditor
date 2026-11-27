import {execFileSync} from 'node:child_process';
import {copyFile, cp, lstat, mkdir, readFile, realpath, rm, stat, writeFile} from 'node:fs/promises';
import {join, relative, resolve, sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import {parseArgs} from 'node:util';
import {compileDemo, demoAssets} from './live-demo.mjs';
import {loadReferences, renderMarkdown} from './references.mjs';
import {pages, sections} from './pages.mjs';
import {locales} from './locales/index.mjs';
import {icon} from './icons.mjs';
import {loadCliUsage, renderCliReference} from './cli-reference.mjs';
import {loadNodeExamples, renderCounterWire, renderGraphExamples, renderNodeReference} from './node-reference.mjs';

// The colour themes, in the picker's order; each locale's ui.themes names them.
const themes = ['auto', 'light', 'dark', 'contrast'];
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
  'node-previews': {type: 'string'},
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
const search = {en: [], es: []};
const sitemap = [];
const editions = {
  en: pages,
  es: pages.map(([slug, , , section]) => {
    const [title, description, label] = locales.es.pages[slug];
    return [slug, title, description, section, label];
  }),
};

async function write(path, contents) {
  const file = join(output, path);
  await mkdir(resolve(file, '..'), {recursive: true});
  await writeFile(file, contents);
}

async function render(route, title, description, body, selected, authored = false, language = 'en', neighbors = []) {
  const path = language === 'es' ? 'es/' + route : route;
  if (paths.has(path)) throw new Error('Duplicate page: ' + path);
  paths.add(path);
  const back = '../'.repeat(route.split('/').length - 1) || './';
  const assets = '../'.repeat(path.split('/').length - 1) || './';
  const ui = locales[language].ui;
  const edition = editions[language];
  // Only the current page's section starts open, so the list stays short.
  const current = edition.find(([slug]) => slug === selected)?.[3] ?? 'start';
  const nav = sections.map(([section, heading]) => '<details class="nav-group"' +
    (section === current ? ' open' : '') + '><summary>' + icon('chevron-right') +
    escape(language === 'es' ? locales.es.sections[section] : heading) + '</summary>' +
    edition.filter(page => page[3] === section).map(([slug, name, , , label]) => '<a href="' + back + pathOf(slug) + '"' +
      (slug === selected ? ' aria-current="page"' : '') + '>' + escape(label ?? name) + '</a>').join('\n') +
    '</details>').join('\n');
  const anchors = [...body.matchAll(/<h2 id="([^"]+)">([\s\S]+?)<\/h2>/g)];
  const tocLinks = anchors.map(([, id, label]) => '<a href="#' + id + '">' + plain(label) + '</a>').join('');
  const toc = anchors.length > 1 ? '<aside class="toc"><nav aria-label="' + ui.contents + '"><strong>' + ui.contents + '</strong>' +
    tocLinks + '</nav></aside>' : '';
  const mobileBody = toc ? body.replace('</h1>', '</h1><details class="toc-inline"><summary>' +
    icon('chevron-right') + ui.contents + '</summary><nav aria-label="' + ui.contents + '">' + tocLinks + '</nav></details>') : body;
  const canonical = new URL(path, base).href;
  const shared = route.startsWith('reference/');
  const alternate = shared ? 'es/sdk/' : (language === 'es' ? route : 'es/' + route);
  const otherLanguage = language === 'es' ? 'en' : 'es';
  const pageLinks = neighbors.some(Boolean) ? '<nav class="page-links" aria-label="' + ui.sequence + '">' +
    neighbors.map((neighbor, index) => {
      if (!neighbor) return '<span></span>';
      const [href, name] = neighbor;
      return '<a rel="' + (index === 0 ? 'prev' : 'next') + '" href="' + back + (href || './') + '"><span>' +
        (index === 0 ? '← ' + ui.previous : ui.next + ' →') + '</span><strong>' + escape(name) + '</strong></a>';
    }).join('') + '</nav>' : '';
  sitemap.push(canonical);
  if (authored) {
    search[language].push({path, title, description, terms: authored === true ? plain(body) : ''});
    for (const [index, match] of anchors.entries()) {
      search[language].push({path: path + '#' + match[1], title: plain(match[2]), description: title,
        terms: authored === true ? plain(body.slice(match.index + match[0].length, anchors[index + 1]?.index)) : ''});
    }
  }
  await write(path + 'index.html', `<!doctype html>
<html lang="${language}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escape(title)} · OrangeEditor</title>
  <meta name="description" content="${escape(description)}">
  <link rel="canonical" href="${escape(canonical)}">
  ${shared ? '' : '<link rel="alternate" hreflang="' + otherLanguage +
    '" href="' + escape(new URL(alternate, base).href) + '">'}
  <link rel="stylesheet" href="${assets}assets/docs.css">
  ${values['node-previews'] ? '<link rel="stylesheet" href="' + assets + 'assets/editor-theme.css">' : ''}
  <link rel="icon" href="${assets}assets/favicon.svg" type="image/svg+xml">
  <script>try { const t = localStorage.getItem('orange-docs-theme'); if (["light","dark","contrast"].includes(t)) document.documentElement.dataset.theme = t; } catch {}</script>
  <script type="module" src="${assets}assets/theme.mjs"></script>
  <script type="module" src="${assets}assets/search.mjs"></script>
  ${selected === 'controls' && demo ? '<script type="module" src="' + assets + 'assets/controls-demo.mjs"></script>' : ''}
  ${selected === 'examples' ? '<script type="module" src="' + assets + 'assets/copy-graph.mjs"></script>' : ''}
</head>
<body>
  <a class="skip" href="#main">${ui.skip}</a>
  <header class="top"><a class="brand" href="${back}"><span aria-hidden="true"></span>OrangeEditor</a>
    <button type="button" id="search-open" class="search-trigger" aria-label="${ui.search}" aria-haspopup="dialog" hidden>${icon('search')}<span>${ui.searchShort}</span><kbd aria-hidden="true">Ctrl K</kbd></button>
    <details class="toolbar-menu">
      <summary title="${ui.language}" aria-label="${ui.language}">${icon('globe')} <span>${language.toUpperCase()}</span> ${icon('chevron-down')}</summary>
      <div class="toolbar-options">
        ${['en', 'es'].map(locale => {
          const name = locale === 'en' ? 'English' : 'Español';
          return locale === language
            ? '<span lang="' + locale + '" aria-current="true">' + name + '</span>'
            : '<a lang="' + locale + '" href="' + assets + (alternate || './') + '">' + name + '</a>';
        }).join('')}
      </div>
    </details>
    <label class="theme-picker" title="${ui.theme}" hidden>
      ${icon('palette')} ${icon('chevron-down')}<span class="visually-hidden">${ui.theme}</span>
      <select id="theme-select">${themes.map((value, index) =>
        '<option value="' + value + '">' + ui.themes[index] + '</option>').join('')}</select>
    </label>
    <a class="source icon-action" href="${repository}">GitHub ${icon('external-link')}</a>
  </header>
  <div class="layout">
    <nav class="sections" aria-label="${ui.docs}">${nav}</nav>
    <main id="main"><article>${mobileBody}</article>
      ${pageLinks}
      <footer><a href="${repository}/tree/${revision}">${ui.revision} ${revision.slice(0, 8)}</a> · <a href="${back}llms.txt">${ui.index}</a></footer>
    </main>
    ${toc}
  </div>
  <dialog id="search-dialog" aria-labelledby="search-title">
    <div class="dialog-head"><h2 id="search-title">${ui.search}</h2><form method="dialog"><button>${ui.close}</button></form></div>
    <label for="search-query" class="visually-hidden">${ui.search}</label>
    <input type="search" id="search-query" placeholder="${ui.placeholder}" maxlength="160" autocomplete="off">
    <div class="search-filters" role="group" aria-label="${ui.search}">
      <button type="button" data-search-filter="all" aria-pressed="true">${ui.searchAll}</button>
      <button type="button" data-search-filter="guides" aria-pressed="false">${ui.searchGuides}</button>
      <button type="button" data-search-filter="nodes" aria-pressed="false">${ui.searchNodes}</button>
    </div>
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

for (const [language, edition] of Object.entries(editions)) {
  const readingOrder = sections.flatMap(([section]) => edition.filter(page => page[3] === section));
  for (const [index, page] of readingOrder.entries()) {
    const neighbors = [readingOrder[index - 1], readingOrder[index + 1]]
      .map(item => item && [pathOf(item[0]), item[1]]);
    await renderGuide(page, language, neighbors);
  }
}

async function renderGuide([slug, title, description], language, neighbors) {
  const es = language === 'es';
  let body;
  if (slug === 'nodes') {
    body = es
      ? '<p>Consulta las ' + sorted.length + ' definiciones del registro. Las tarjetas muestran los puertos al colocar cada nodo. El tipo y la configuración pueden cambiar sus puertos. <a href="../../nodes.json">Descarga el catálogo JSON</a>.</p>'
      : '<p>The ' + sorted.length + ' definitions below come from the CLI\'s active default registry. Each card shows the ports you see when placing the node. Configuration and inferred types can change the card. See <a href="../nodes.json">the full JSON export</a>.</p>';
    for (const [category, members] of [...groups].sort(([a], [b]) => compare(a, b))) {
      const categoryId = category.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
      const ordered = es ? [...members].sort((a, b) => a.name.es.localeCompare(b.name.es, 'es')) : members;
      const other = es ? 'en' : 'es';
      body += '<h2 id="category-' + categoryId + '">' + escape(members[0].category[language]) + '</h2><ul class="node-list">' +
        ordered.map(node => '<li><a href="../' + nodePath(node) + '">' + escape(node.name[language]) + '</a>' +
          '<span lang="' + other + '">' + escape(node.name[other]) + '</span>' +
          (translated(node.name, 'ja') ? '<span lang="ja">' + escape(node.name.ja) + '</span>' : '') + '</li>').join('') + '</ul>';
    }
  } else if (slug === 'cli' && !es) {
    body = renderCliReference(loadCliUsage(root), escape);
  } else if (slug === 'examples') {
    body = renderGraphExamples(examples, sorted, source, escape, nodePath, language);
  } else {
    const guide = join(site, 'guides', ...(es ? ['es'] : []), (slug || 'index') + '.md');
    body = es ? await renderMarkdown(root, target, guide) :
      await stat(guide).then(() => renderMarkdown(root, target, guide), () =>
        readFile(join(site, 'pages', (slug || 'index') + '.html'), 'utf8'));
    const back = slug ? '../' : './';
    const shared = es ? back + '../' : back;
    for (const contract of contracts) {
      body = body.replaceAll('@source/docs/' + contract.file, shared + 'reference/' + contract.slug + '/');
    }
    body = body.replaceAll('@shared/', shared).replaceAll('@source/', source)
      .replaceAll('@tree/', tree).replaceAll('@revision@', revision);
    if (slug === 'count-button-presses') {
      const counter = examples.find(example => example.slug === 'counter-with-reset');
      const diagram = renderCounterWire(counter, sorted, nodePath, escape, language);
      body = body.replace('<!-- counter-wire -->', values['node-previews']
        ? (await renderTutorialGraph('counter-first-step', language, counter.slug)) +
          '<details class="port-diagram"><summary>' + escape(locales[language].tutorial.linkedPorts) +
          '</summary>' + diagram + '</details>'
        : diagram);
    }
    // <!-- tutorial-graph: <picture> [<example>] --> places an editor picture, linked to
    // the example graph it shows when the picture's name is not the example's.
    for (const match of body.matchAll(/<!-- tutorial-graph: ([a-z0-9-]+)(?: ([a-z0-9-]+))? -->/g)) {
      const [marker, graph, example] = match;
      body = body.replace(marker, await renderTutorialGraph(graph, language, example ?? graph));
    }
    if (slug === 'controls') body = body.replace('<!-- controls-demo -->', demo
      ? '<button type="button" id="load-controls-demo" class="icon-action" hidden>' +
        icon('play') + (es ? 'Pruébalo' : 'Try it') + '</button>' +
        '<p id="controls-demo-status" role="status"></p>' +
        '<canvas id="controls-demo" tabindex="0" aria-label="' +
        (es ? 'Demo del deslizador y el dial de OrangeEditor' : 'OrangeEditor slider and knob demo') + '" hidden></canvas>' +
        '<noscript><p>' + (es ? 'Activa JavaScript para usar la demo.' : 'Enable JavaScript to try the controls demo.') + '</p></noscript>'
      : es ? '<p>Prueba los controles en la galería de escritorio. La versión de GitHub Pages incluye la demo web.</p>'
      : '<p>Run the gallery command above to try the controls locally. The GitHub Pages build includes the browser demo.</p>');
  }
  if (slug || es) body = '<h1>' + escape(title) + '</h1><p class="lead">' + escape(description) + '</p>' + body;
  await render(pathOf(slug), title, description, body, slug, slug !== 'nodes', language, neighbors);
  if (slug === 'nodes') search[language].push({path: es ? 'es/nodes/' : 'nodes/', title, description,
    terms: 'catalogue catálogo converter conversión registry'});
}

async function renderTutorialGraph(graph, language, exampleSlug = graph) {
  const messages = locales[language].tutorial;
  const [alt, caption] = messages[graph];
  const example = '<a href="../examples/#' + exampleSlug + '">' + escape(messages.graph) + '</a>';
  if (!values['node-previews']) return '<p>' + escape(caption) + ' ' + example + '.</p>';

  const folder = 'assets/tutorial-graphs/' + language + '/' + graph + '/';
  const back = language === 'es' ? '../../' : '../';
  await mkdir(join(output, folder), {recursive: true});
  const images = await Promise.all(['light', 'dark'].map(async theme => {
    const file = theme + '.png';
    await copyFile(join(values['node-previews'], language, 'graphs', graph, file), join(output, folder, file));
    // Each picture opens at full size, where the editor's text is easy to read.
    return '<a class="native-' + theme + '" href="' + back + folder + file + '"><img src="' + back + folder + file +
      '" alt="' + escape(alt) + '" loading="lazy" decoding="async"></a>';
  }));
  return '<figure class="node-preview native-preview tutorial-graph">' + images.join('') +
    '<figcaption>' + escape(caption) + ' ' + example + '.</figcaption></figure>';
}

for (const contract of contracts) {
  await render('reference/' + contract.slug + '/', contract.title, contract.description,
    '<h1>' + escape(contract.title) + '</h1>' + contract.body, 'sdk', 'reference');
}

for (const language of ['en', 'es']) {
  const ordered = language === 'en' ? sorted : [...sorted].sort((a, b) =>
    a.name.es.localeCompare(b.name.es, 'es', {numeric: true}) || compare(nodePath(a), nodePath(b)));
  for (const [index, node] of ordered.entries()) {
    const neighbors = [ordered[index - 1], ordered[index + 1]]
      .map(item => item && [nodePath(item), item.name[language]]);
    await renderNode(node, language, neighbors);
  }
}

async function renderNode(node, language, neighbors) {
  const es = language === 'es';
  const path = nodePath(node);
  const identity = node.id + '@' + node.version;
  const other = es ? 'en' : 'es';
  const description = identity + ' · ' + (node.purpose[language] || node.category[language]);
  let images;
  if (values['node-previews']) {
    const folder = 'assets/node-cards/' + language + '/' + node.id + '/' + node.version + '/';
    await mkdir(join(output, folder), {recursive: true});
    images = {};
    for (const theme of ['light', 'dark']) {
      const filename = theme + '.png';
      await copyFile(join(values['node-previews'], language, node.id, String(node.version), filename),
        join(output, folder, filename));
      images[theme] = (es ? '../../../../../' : '../../../../') + folder + filename;
    }
  }
  const body = '<p><a href="../../../">← ' + (es ? 'Referencia de nodos' : 'Node reference') + '</a></p><h1>' +
    escape(node.name[language]) + '</h1><p class="lead">' + escape(node.purpose[language]) + '</p>' +
    renderNodeReference(node, examples, escape, language, images) +
    '<h2 id="identity">' + (es ? 'Identidad' : 'Identity') + '</h2><dl><dt>' +
    (es ? 'Identificador' : 'Identifier') + '</dt><dd><code>' + escape(identity) + '</code></dd><dt>' +
    (es ? 'Nombre en inglés' : 'Spanish name') + '</dt><dd lang="' + other + '">' + escape(node.name[other]) + '</dd>' +
    (translated(node.name, 'ja') ? '<dt>' + (es ? 'Nombre en japonés' : 'Japanese name') +
      '</dt><dd lang="ja">' + escape(node.name.ja) + '</dd>' : '') +
    '<dt>' + (es ? 'Categoría' : 'Category') + '</dt><dd>' + escape(node.category[language]) +
    '</dd><dt>' + (es ? 'Proveedor' : 'Provider') + '</dt><dd>' + escape(node.provider ?? (es ? 'Núcleo' : 'Core')) + '</dd></dl>' +
    (node.config_schema ? '<h2 id="configuration">' + (es ? 'Esquema de configuración' : 'Configuration schema') +
      '</h2><pre><code>' + escape(JSON.stringify(node.config_schema, null, 2)) + '</code></pre>' : '');
  await render(path, node.name[language], description, body, 'nodes', false, language, neighbors);
  search[language].push({path: es ? 'es/' + path : path, title: node.name[language], description, alias: node.name[other],
    terms: [node.category.en, node.category.es, node.category.ja, node.provider, node.purpose[other],
      node.name.ja, node.purpose.ja, ...node.aliases.en, ...node.aliases.es, ...(node.aliases.ja ?? []),
      ...[...(node.preview_ports ?? []), ...(node.preview_ports_es ?? [])].flatMap(port => [port.id, port.label, port.description]),
      ...(node.ports ?? []).map(port => port.port_id + ' ' + (port.type ? JSON.stringify(port.type) : ''))]
      .filter(Boolean).join(' ')});
}

await mkdir(join(output, 'assets'), {recursive: true});
if (values['node-previews']) {
  await copyFile(join(values['node-previews'], 'editor-theme.css'), join(output, 'assets', 'editor-theme.css'));
}
for (const asset of ['docs.css', 'favicon.svg', 'search.mjs', 'search-index.mjs', 'theme.mjs', 'copy-graph.mjs']) {
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
// The editor's clipboard format, read from the host so a copied example always pastes.
const clipFormat = (await readFile(join(root, 'app/crates/platform/orange-host/src/clip.rs'), 'utf8'))
  .match(/const CLIP_FORMAT: &str = "([^"]+)";/)?.[1];
if (!clipFormat) throw new Error('Could not read the editor clipboard format');
for (const example of examples) {
  await copyFile(join(root, example.path), join(output, 'examples', 'graphs', example.slug + '.json'));
  await write('examples/graphs/' + example.slug + '.clip.json', JSON.stringify({
    format: clipFormat, version: 1, document: example.graph, nodes: example.graph.nodes.map(node => node.id),
  }));
}
await copyFile(join(root, 'app/crates/apps/orange-editor-ui/licenses/egui-lucide-0.2.0-artwork.txt'),
  join(output, 'assets', 'lucide-license.txt'));
await write('.nojekyll', '');
await write('nodes.json', JSON.stringify({revision, nodes}));
await write('search.json', JSON.stringify(search.en));
await write('es/search.json', JSON.stringify(search.es));
await write('es/llms.txt', '# Documentación de OrangeEditor\n\n' +
  editions.es.map(([slug, title, description]) =>
    '- [' + title + '](' + new URL('es/' + pathOf(slug), base).href + '): ' + description).join('\n') +
  '\n\n- [Catálogo de nodos JSON](' + new URL('nodes.json', base).href + ')\n');
await write('sitemap.xml', '<?xml version="1.0" encoding="UTF-8"?>\n' +
  '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n' +
  [...sitemap, ...apiCrates.map(crate => new URL('api/' + crate + '/', base).href)]
    .map(url => '  <url><loc>' + escape(url) + '</loc></url>').join('\n') + '\n</urlset>\n');
await write('llms.txt', '# OrangeEditor documentation\n\n' +
  '> OrangeEditor is a Rust foundation for typed node graphs with an egui editor and headless host.\n\n' +
  '[Documentación en español](' + new URL('es/llms.txt', base).href + ')\n\n' +
  'Source revision: ' + revision + '\n\n## Guides\n\n' +
  pages.map(([slug, title, description]) =>
    '- [' + title + '](' + new URL(pathOf(slug), base).href + '): ' + description).join('\n') +
  '\n\n## Reference\n\n- [Node catalogue JSON](' + new URL('nodes.json', base).href + ')\n' +
  apiCrates.map(crate => '- [' + crate + ' Rustdoc](' + new URL('api/' + crate + '/', base).href + ')').join('\n') +
  '\n- [Wasm WIT interface](' + source + 'sdk/wit/orange-expansion.wit)\n' +
  contracts.map(contract => '- [' + contract.title + '](' +
    new URL('reference/' + contract.slug + '/', base).href + ')').join('\n') + '\n');
console.log('Built ' + paths.size + ' pages in ' + output);
