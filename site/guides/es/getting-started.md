# Primeros pasos

<h2 id="open-the-editor">Abre el editor</h2>

Consulta las [versiones publicadas](https://github.com/erinmel/OrangeEditor/releases). Para compilar desde el repositorio, instala rustup y ejecuta este comando desde la raíz. El archivo `rust-toolchain.toml` selecciona la versión de Rust. En Linux también necesitas las librerías de desarrollo de ALSA.

```sh
cargo run -p orange-desktop --locked -- app/crates/apps/orange-cli/graphs/set-on-trigger.json
```

<h2 id="run-a-graph">Ejecuta tu primer grafo</h2>

1. Localiza el [disparador manual](../nodes/core/on-manual/1/) y sigue su cable de ejecución hasta la acción que asigna la variable.
2. Pulsa **Ejecutar** y luego dispara el primer nodo. Abre **Variables**. El valor de `total` debe ser `7`.
3. Detén el grafo, cambia el valor de la acción a `9`, ejecútalo y dispara de nuevo. Comprueba que `total` ahora vale `9`.
4. Usa **Guardar como** para conservar tu copia.

<h2 id="check-from-the-terminal">Comprueba el grafo desde la terminal</h2>

```sh
cargo run -p orange-cli --locked -- check app/crates/apps/orange-cli/graphs/set-on-trigger.json
cargo run -p orange-cli --locked -- run app/crates/apps/orange-cli/graphs/set-on-trigger.json --watch
```

Escribe `trigger 1` para activar el disparador y `quit` para salir. Para ver opciones, consulta la [referencia de la línea de comandos](../cli/).

<h2 id="example-graphs">Grafos de ejemplo</h2>

Abre la [galería de grafos](../examples/) para descargar el contador, la extracción de JSON, el manejo de errores HTTP y la puerta de ruido.
