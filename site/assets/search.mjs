import {prepareIndex, searchIndex} from './search-index.mjs';

const open = document.querySelector('#search-open');
const dialog = document.querySelector('#search-dialog');
const input = document.querySelector('#search-query');
const status = document.querySelector('#search-status');
const results = document.querySelector('#search-results');
const filters = [...dialog.querySelectorAll('[data-search-filter]')];
const root = new URL('../', import.meta.url);
const spanish = document.documentElement.lang === 'es';
const unavailable = spanish ? 'La búsqueda no está disponible. Usa los enlaces de la documentación.' :
  'Search is unavailable. Browse the documentation links instead.';
const starterPaths = ['getting-started/', 'nodes/', 'examples/', 'controls/', 'sdk/'];
const starterNodes = ['nodes/core/on-manual/1/', 'nodes/core/branch/1/', 'nodes/core/update-variable/1/'];
const isNode = entry => /^(es\/)?nodes\/.+/.test(entry.path);
let request;
let pending = 0;
let filter = 'all';

function loadIndex() {
  return request ??= fetch(new URL(spanish ? 'es/search.json' : 'search.json', root)).then(response => {
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
  void update();
}

async function update() {
  const current = ++pending;
  const query = input.value.trim();
  results.replaceChildren();
  status.textContent = '';
  try {
    const index = await loadIndex();
    if (current !== pending) return;
    if (!query) {
      status.textContent = spanish ? 'Para empezar' : 'Suggestions';
      for (const path of filter === 'nodes' ? starterNodes : starterPaths) {
        const entry = index.find(entry => entry.path === (spanish ? 'es/' : '') + path);
        if (entry) addResult(entry);
      }
      return;
    }
    const matches = searchIndex(index, query).filter(entry =>
      filter === 'all' || (filter === 'nodes' ? isNode(entry) : !isNode(entry)));
    const count = Math.min(12, matches.length);
    status.textContent = spanish
      ? (count ? 'Mostrando ' + count + ' de ' + matches.length + ' resultados.' : 'Sin resultados.')
      : (count ? 'Showing ' + count + ' of ' + matches.length + ' results.' : 'No results.');
    for (const entry of matches.slice(0, 12)) addResult(entry);
  } catch {
    if (current === pending) status.textContent = unavailable;
  }
}

function addResult(entry) {
  const item = document.createElement('li');
  const link = document.createElement('a');
  const summary = document.createElement('span');
  const kind = document.createElement('small');
  link.href = new URL(entry.path, root).href;
  link.textContent = entry.title;
  kind.textContent = isNode(entry) ? (spanish ? 'Nodo' : 'Node') :
    (entry.path.includes('#') ? (spanish ? 'Sección' : 'Section') : (spanish ? 'Página' : 'Page'));
  summary.textContent = entry.alias ? entry.alias + ' · ' + entry.description : entry.description;
  link.append(kind, summary);
  item.append(link);
  results.append(item);
}

if (typeof dialog.showModal === 'function') {
  open.hidden = false;
  open.addEventListener('click', showSearch);
  input.addEventListener('input', update);
  for (const button of filters) {
    button.addEventListener('click', () => {
      filter = button.dataset.searchFilter;
      for (const option of filters) option.setAttribute('aria-pressed', String(option === button));
      input.focus();
      void update();
    });
  }
  dialog.addEventListener('keydown', event => {
    const links = [...results.querySelectorAll('a')];
    if (event.target === input && event.key === 'Enter' && links.length) {
      event.preventDefault();
      links[0].click();
      return;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const position = links.indexOf(event.target);
    if (event.target !== input && position < 0) return;
    const destination = position + (event.key === 'ArrowDown' ? 1 : -1);
    if (destination < 0) input.focus();
    else links[destination]?.focus();
    event.preventDefault();
  });
  results.addEventListener('click', event => {
    if (event.target.closest('a')) dialog.close();
  });
  dialog.addEventListener('click', event => {
    if (event.target === dialog) {
      const bounds = dialog.getBoundingClientRect();
      if (event.clientX < bounds.left || event.clientX > bounds.right ||
          event.clientY < bounds.top || event.clientY > bounds.bottom) dialog.close();
    }
  });
  document.addEventListener('keydown', event => {
    const command = (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k';
    const slash = event.key === '/' && !event.ctrlKey && !event.metaKey &&
      !event.target.closest('input, textarea, select, [contenteditable]');
    if ((!command && !slash) || event.altKey) return;
    event.preventDefault();
    showSearch();
  });
}
