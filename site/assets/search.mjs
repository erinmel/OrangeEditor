import {prepareIndex, searchIndex} from './search-index.mjs';

const open = document.querySelector('#search-open');
const dialog = document.querySelector('#search-dialog');
const input = document.querySelector('#search-query');
const status = document.querySelector('#search-status');
const results = document.querySelector('#search-results');
const root = new URL('../', import.meta.url);
let request;
let pending = 0;

function loadIndex() {
  return request ??= fetch(new URL('search.json', root)).then(response => {
    if (!response.ok) throw new Error('Search index unavailable');
    return response.json();
  }).then(prepareIndex).catch(error => {
    request = undefined;
    throw error;
  });
}

function showSearch() {
  if (dialog.open) return;
  dialog.showModal();
  input.focus();
  loadIndex().catch(() => { status.textContent = 'Search is unavailable. Browse the documentation links instead.'; });
}

async function update() {
  const current = ++pending;
  const query = input.value.trim();
  results.replaceChildren();
  status.textContent = '';
  if (!query) return;
  try {
    const index = await loadIndex();
    if (current !== pending) return;
    const matches = searchIndex(index, query);
    status.textContent = matches.length ? 'Showing ' + Math.min(12, matches.length) + ' of ' +
      matches.length + ' results.' : 'No results.';
    for (const entry of matches.slice(0, 12)) {
      const item = document.createElement('li');
      const link = document.createElement('a');
      const summary = document.createElement('span');
      link.href = new URL(entry.path, root).href;
      link.textContent = entry.title;
      summary.textContent = entry.alias ? entry.alias + ' · ' + entry.description : entry.description;
      link.append(summary);
      item.append(link);
      results.append(item);
    }
  } catch {
    if (current === pending) status.textContent = 'Search is unavailable. Browse the documentation links instead.';
  }
}

if (typeof dialog.showModal === 'function') {
  open.hidden = false;
  open.addEventListener('click', showSearch);
  input.addEventListener('input', update);
  input.addEventListener('keydown', event => {
    if (event.key === 'ArrowDown') results.querySelector('a')?.focus();
  });
  results.addEventListener('click', event => {
    if (event.target.closest('a')) dialog.close();
  });
  document.addEventListener('keydown', event => {
    if (event.key !== '/' || event.ctrlKey || event.metaKey || event.altKey ||
        event.target.closest('input, textarea, select, [contenteditable]')) return;
    event.preventDefault();
    showSearch();
  });
}
