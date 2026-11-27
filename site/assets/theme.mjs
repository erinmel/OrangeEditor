// The colour theme picker. The page's head applies a saved theme before it paints, so
// this only keeps the menu and the saved choice in step.
const key = 'orange-docs-theme';
const select = document.querySelector('#theme-select');
if (select) {
  const picker = select.closest('.theme-picker');
  const saved = (() => {
    try { return localStorage.getItem(key); } catch { return null; }
  })();
  select.value = [...select.options].some(option => option.value === saved) ? saved : 'auto';
  const label = picker.title;
  const updateTitle = () => { picker.title = label + ' · ' + select.selectedOptions[0].textContent; };
  updateTitle();
  picker.hidden = false;
  select.addEventListener('change', () => {
    const theme = select.value;
    if (theme === 'auto') delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = theme;
    try { localStorage.setItem(key, theme); } catch { /* Private windows may refuse storage. */ }
    updateTitle();
  });
}

const languageMenu = document.querySelector('.toolbar-menu');
if (languageMenu) {
  document.addEventListener('click', event => {
    if (languageMenu.open && !languageMenu.contains(event.target)) languageMenu.open = false;
  });
  languageMenu.addEventListener('keydown', event => {
    if (event.key === 'Escape' && languageMenu.open) {
      languageMenu.open = false;
      languageMenu.querySelector('summary').focus();
    }
  });
}
