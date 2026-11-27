import {execFileSync} from 'node:child_process';
import {mkdir, readFile} from 'node:fs/promises';
import {basename, join} from 'node:path';

// Source filenames and public routes. Rustdoc supplies the titles and headings.
export const references = [
  ['11-graph-language.md', 'graph-language', 'Types, execution, functions and graph semantics.'],
  ['12-graph-data-structures.md', 'graph-file-format', 'The versioned graph document and value formats.'],
  ['04-expansion-plugins.md', 'packages', 'Manifests, providers, permissions and package lifecycle.'],
  ['13-internationalization.md', 'internationalization', 'Fluent catalogues, namespaced text and locale fallback.'],
];

/// Renders one Markdown file with the pinned toolchain's rustdoc, without its title:
/// the page supplies its own heading. Links stay as written.
export async function renderMarkdown(root, target, file) {
  const output = join(target, 'orange-docs-markdown', 'guides');
  await mkdir(output, {recursive: true});
  execFileSync('rustdoc', ['--edition', '2024', '--markdown-no-toc', '--output', output, file],
    {cwd: root, stdio: 'inherit'});
  const html = await readFile(join(output, basename(file).replace(/\.md$/, '.html')), 'utf8');
  const article = html.match(/<h1 class="title">[^<]+<\/h1>([\s\S]+)<\/body>/);
  if (!article) throw new Error('Could not read the Rustdoc Markdown output for ' + file);
  return article[1].replace(/<a class="doc-anchor" href="#[^"]+">[^<]*<\/a>/g, '');
}

export async function loadReferences(root, target, source, tree) {
  const output = join(target, 'orange-docs-markdown');
  await mkdir(output, {recursive: true});
  const rendered = new Map();
  for (const [file, slug, description] of references) {
    execFileSync('rustdoc', ['--edition', '2024', '--markdown-no-toc', '--output', output, 'docs/' + file],
      {cwd: root, stdio: 'inherit'});
    const html = await readFile(join(output, file.replace(/\.md$/, '.html')), 'utf8');
    const article = html.match(/<h1 class="title">([^<]+)<\/h1>([\s\S]+)<\/body>/);
    if (!article) throw new Error('Could not read the Rustdoc Markdown output for ' + file);
    const [, title, section] = article;
    const body = section.replace(/<a class="doc-anchor" href="#[^"]+">[^<]*<\/a>/g, '');
    const ids = new Set([...body.matchAll(/\sid="([^"]+)"/g)].map(([, id]) => id));
    rendered.set('docs/' + file, {file, slug, title, description, body, ids});
  }

  return [...rendered.values()].map(({file, slug, title, description, body}) => ({
    file, slug, title, description,
    body: '<p><a href="' + source + 'docs/' + file + '">View Markdown source</a></p>' +
      body.replace(/\bhref="([^"]+)"/g, (attribute, href) => {
        if (href.startsWith('#') || /^(?:[a-z][a-z\d+.-]*:|\/)/i.test(href)) return attribute;
        const resolved = new URL(href, 'https://repo.invalid/docs/' + file);
        const path = resolved.pathname.slice(1);
        const linked = rendered.get(path);
        if (linked && (!resolved.hash || linked.ids.has(decodeURIComponent(resolved.hash.slice(1))))) {
          return 'href="../' + linked.slug + '/' + resolved.hash + '"';
        }
        const base = path.endsWith('/') || !path.split('/').at(-1).includes('.') ? tree : source;
        return 'href="' + base + path + resolved.search + resolved.hash + '"';
      }),
  }));
}
