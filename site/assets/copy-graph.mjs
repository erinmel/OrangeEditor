// "Copy nodes" on the example graphs: puts an example on the clipboard in the editor's
// own format, so Paste in the editor places its nodes, wires and declarations. Each
// clip loads with the page, so the click writes at once: some browsers refuse a
// clipboard write that waits on the network after the click.
const spanish = document.documentElement.lang === 'es';
for (const button of document.querySelectorAll('button[data-clip]')) {
  const caption = button.querySelector('.copy-label');
  const label = caption.textContent;
  const clip = fetch(button.dataset.clip)
    .then(response => response.ok ? response.text() : Promise.reject(new Error(response.statusText)));
  clip.then(() => { button.hidden = false; }, () => {});
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(await clip);
      caption.textContent = spanish ? 'Copia lista para pegar en el editor' : 'Copied: paste it in the editor';
    } catch {
      caption.textContent = spanish ? 'No se pudo copiar. Descarga el JSON.' : 'Could not copy: download the JSON instead';
    }
    setTimeout(() => { caption.textContent = label; }, 2500);
  });
}
