// "Copy nodes" on the example graphs: puts an example on the clipboard in the editor's
// own format, so Paste in the editor places its nodes, wires and declarations. Each
// clip loads with the page, so the click writes at once: some browsers refuse a
// clipboard write that waits on the network after the click.
for (const button of document.querySelectorAll('button[data-clip]')) {
  const label = button.textContent;
  const clip = fetch(button.dataset.clip)
    .then(response => response.ok ? response.text() : Promise.reject(new Error(response.statusText)));
  clip.then(() => { button.hidden = false; }, () => {});
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(await clip);
      button.textContent = 'Copied: paste it in the editor';
    } catch {
      button.textContent = 'Could not copy: download the JSON instead';
    }
    setTimeout(() => { button.textContent = label; }, 2500);
  });
}
