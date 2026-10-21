import fs from 'node:fs';
import path from 'node:path';

const excluded = new Set(['.git', '.idea', '.vscode', 'node_modules', 'target', 'dist', '.artifacts']);
function walk(dir) {
  return fs.readdirSync(dir, {withFileTypes: true}).flatMap(entry => {
    if (excluded.has(entry.name)) return [];
    const name = path.join(dir, entry.name);
    return entry.isDirectory() ? walk(name) : entry.isFile() && name.endsWith('.md') ? [name.replaceAll('\\', '/')] : [];
  });
}
const files = walk('.').sort();
const docs = new Map(files.map(file => [file, fs.readFileSync(file, 'utf8')]));
const errors = [];
let jsonBlocks = 0, localLinks = 0;
const jsonPattern = /\x60{3}json\r?\n([\s\S]*?)\r?\n\x60{3}/g;
for (const [file, source] of docs) {
  if ((source.match(/^\x60{3}/gm) ?? []).length % 2) errors.push(file + ': unbalanced fences');
  for (const match of source.matchAll(jsonPattern)) {
    try { JSON.parse(match[1]); jsonBlocks++; }
    catch (error) { errors.push(file + ': invalid JSON: ' + error.message); }
  }
  for (const match of source.matchAll(/\[[^\]\n]*\]\(([^)\n]+)\)/g)) {
    if (/^(https?:|mailto:|app:|#)/.test(match[1])) continue;
    const target = match[1].split('#')[0].replace(/^<|>$/g, '');
    try {
      if (!fs.existsSync(path.resolve(path.dirname(file), decodeURIComponent(target)))) errors.push(file + ': missing link ' + target);
    } catch { errors.push(file + ': invalid link ' + target); }
    localLinks++;
  }
  if (/\n\n\| (?:FR-\d\d|NFR-\d\d|OPEN-\d\d|G-\d\d|L-\d\d|V\d\d)[^|\n]*\|/.test(source)) errors.push(file + ': detached catalogue row');
}
const definitions = {};
for (const [prefix, file, pattern] of [
  ['FR', 'docs/01-product-requirements.md', /^\| FR-(\d\d) \|/gm],
  ['NFR', 'docs/06-quality-and-validation.md', /^\| NFR-(\d\d) /gm],
  ['ADR', 'docs/08-decisions.md', /^## ADR-(\d\d) /gm],
  ['OPEN', 'docs/08-decisions.md', /^\| OPEN-(\d\d) /gm],
  ['G', 'docs/11-graph-language.md', /^\| G-(\d\d) \|/gm],
  ['L', 'docs/13-internationalization.md', /^\| L-(\d\d) \|/gm],
]) definitions[prefix] = [...docs.get(file).matchAll(pattern)].map(match => +match[1]);
for (const [prefix, ids] of Object.entries(definitions)) {
  if (ids.length !== new Set(ids).size) errors.push(prefix + ': duplicate definition');
  for (const [file, source] of docs) {
    const pattern = new RegExp('\\b' + prefix + '-(\\d{2})(?:[–-](?:' + prefix + '-)?(\\d{2}))?', 'g');
    for (const match of source.matchAll(pattern)) {
      const first = +match[1], last = +(match[2] ?? match[1]);
      if (first > last) errors.push(file + ': reversed range');
      for (let id = first; id <= last; id++) if (!ids.includes(id)) errors.push(file + ': undefined ' + prefix + '-' + id);
    }
  }
}
const matrix = docs.get('docs/07-delivery-plan.md').split('## Traceability matrix')[1].split('## Risk register')[0];
for (const [prefix, column] of [['FR', 2], ['NFR', 3]]) {
  const covered = new Set();
  for (const line of matrix.split(/\r?\n/)) {
    if (!line.startsWith('| WU-')) continue;
    for (const match of line.split('|')[column].matchAll(/(?:^|[^\d])(\d{2})(?:[–-](\d{2}))?(?!\d)/g))
      for (let id = +match[1]; id <= +(match[2] ?? match[1]); id++) covered.add(id);
  }
  for (const id of definitions[prefix]) if (!covered.has(id)) errors.push('Missing roadmap ' + prefix + '-' + id);
}
const repeated = [];
for (const file of ['docs/03-data-and-execution.md', 'docs/12-graph-data-structures.md']) {
  for (const match of docs.get(file).matchAll(jsonPattern)) {
    let o;
    try { o = JSON.parse(match[1]); } catch { continue; }
    if (!o.definition_id) continue;
    if (o.definition_id === 'org.orange.math/add') repeated.push(o);
    const refs = [o.label, o.editor.category, ...(o.editor.search_aliases ?? []), ...o.ports.map(port => port.label)];
    if (refs.some(ref => typeof ref?.key !== 'string' || typeof ref?.fallback !== 'string')) errors.push(file + ': invalid TextRef');
    if (!o.editor.category_id) errors.push(file + ': missing category identity');
  }
}
if (repeated.length !== 2 || JSON.stringify(repeated[0]) !== JSON.stringify(repeated[1])) errors.push('Repeated math definitions differ');
const catalogue = docs.get('docs/11-graph-language.md').split('## 17. Diagnostics catalog')[1].split('## 18.')[0];
const codes = new Set([...catalogue.matchAll(/\b[EW]-[A-Z][A-Z-]+\b/g)].map(match => match[0]));
for (const [file, source] of docs) for (const match of source.matchAll(/\b[EW]-[A-Z][A-Z-]+\b/g))
  if (!codes.has(match[0])) errors.push(file + ': uncatalogued diagnostic ' + match[0]);
console.log(JSON.stringify({status: errors.length ? 'FAIL' : 'PASS', markdownFiles: files.length, jsonBlocks, localLinks,
  definitions: Object.fromEntries(Object.entries(definitions).map(([key, ids]) => [key, ids.length])), errors}, null, 2));
if (errors.length) process.exitCode = 1;
