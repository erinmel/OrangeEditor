# Crea paquetes

<h2 id="export-a-function">Exporta una función del editor</h2>

Después de [crear una función](../custom-nodes/#graph-function), abre el menú contextual de su llamada y elige **Exportar función**. Revisa sus puertos y dependencias, usa un ID que controles y guarda el archivo `.orangeplug`. Instálalo en **Paquetes** del editor destinatario y configura las conexiones de ese grafo.

```sh
cargo run -p orange-cli --locked -- publish graph.json "Compute power" org.example.math 0.1.0 power power.orangeplug
```

<h2 id="pack-a-directory">Empaqueta un directorio</h2>

El directorio incluye `manifest.json` y las definiciones en `nodes/`. Guarda el grafo proveedor en `graphs/`, o el componente Wasm en la ruta de ejecución declarada. Añade los textos Fluent nombrados en el manifiesto. El [ejemplo Remember](@tree/app/crates/apps/orange-cli/packages/remember) muestra el conjunto mínimo.

```sh
cargo run -p orange-cli --locked -- pack app/crates/apps/orange-cli/packages/remember remember.orangeplug
cargo run -p orange-cli --locked -- check app/crates/apps/orange-cli/graphs/remember-on-trigger.json --package remember.orangeplug
```

<h2 id="permissions-and-updates">Actualiza un paquete</h2>

Revisa los permisos al instalar. Los grafos conservan las identidades de proveedor y los flujos activos pueden seguir usando el proveedor con el que empezaron. Instala la nueva versión junto a la anterior y revisa la vinculación antes de actualizar un grafo. Consulta el [contrato de paquetes en inglés](@shared/reference/packages/).
