const button = document.querySelector('#load-controls-demo');
const canvas = document.querySelector('#controls-demo');
const status = document.querySelector('#controls-demo-status');
const spanish = document.documentElement.lang === 'es';
let demo;

button.hidden = false;
button.addEventListener('click', async () => {
  button.disabled = true;
  status.textContent = spanish ? 'Cargando la demo de controles.' : 'Loading the controls demo.';
  try {
    const {default: init, ControlsDemo} = await import('./controls/orange_docs_web.js');
    await init();
    canvas.hidden = false;
    demo = await ControlsDemo.start(canvas, spanish);
    status.textContent = spanish ? 'Usa el deslizador o el dial para cambiar el nivel.' :
      'Use the slider or knob to change the shared level.';
    canvas.focus();
    button.hidden = true;
  } catch {
    canvas.hidden = true;
    button.disabled = false;
    status.textContent = spanish ? 'La demo no pudo arrancar. Activa WebGL o prueba la galería de escritorio.' :
      'The demo could not start. Try a browser with WebGL enabled, or run the desktop gallery.';
  }
});

window.addEventListener('pagehide', event => {
  if (!event.persisted) demo?.stop();
});
