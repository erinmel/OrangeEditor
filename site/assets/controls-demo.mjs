const button = document.querySelector('#load-controls-demo');
const canvas = document.querySelector('#controls-demo');
const status = document.querySelector('#controls-demo-status');
let demo;

button.hidden = false;
button.addEventListener('click', async () => {
  button.disabled = true;
  status.textContent = 'Loading the controls demo…';
  try {
    const {default: init, ControlsDemo} = await import('./controls/orange_docs_web.js');
    await init();
    canvas.hidden = false;
    demo = await ControlsDemo.start(canvas);
    status.textContent = 'Use the slider or knob to change the shared level.';
    canvas.focus();
    button.hidden = true;
  } catch {
    canvas.hidden = true;
    button.disabled = false;
    status.textContent = 'The demo needs WebGL. You can still read the controls guide below.';
  }
});

window.addEventListener('pagehide', event => {
  if (!event.persisted) demo?.stop();
});
