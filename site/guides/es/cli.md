# Referencia de la línea de comandos

## Comprueba y ejecuta grafos

Desde la raíz del repositorio, ejecuta estos comandos. Reemplaza `graph.json` por tu archivo.

```sh
cargo run -p orange-cli --locked -- check graph.json
cargo run -p orange-cli --locked -- run graph.json --watch
```

Con `--watch`, usa `trigger 1` para disparar la primera entrada manual y `quit` para terminar. Usa `preview` para calcular salidas puras admitidas sin iniciar el flujo.

## Consulta nodos y paquetes

Usa `nodes --json` para exportar el catálogo y `pack` o `publish` para crear paquetes. La [guía de paquetes](../packages/) muestra los argumentos en ejemplos completos.

## Consulta todas las opciones

La [ayuda generada de la CLI en inglés](@shared/cli/) enumera los comandos y sus opciones. También puedes ejecutar `cargo run -p orange-cli --locked -- --help` para ver la ayuda de tu versión.
