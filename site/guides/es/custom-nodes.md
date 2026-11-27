# Crea nodos personalizados

<h2 id="graph-function">Extrae una función del grafo</h2>

Abre el [ejemplo de potencia](../examples/#power-with-count). Selecciona los nodos que calculan el resultado y deja fuera el disparador, la base, el exponente y la acción que guarda el resultado. Elige **Crear función**, revisa las entradas `a` y `count` y la salida `value`, y confirma. Cuando dispares Calculate power con los valores predeterminados, comprueba que el resultado sea `81`. Puedes reutilizar la misma función con varias llamadas.

<h2 id="wasm-node">Implementa un nodo Wasm</h2>

Usa el [ejemplo del componente](@tree/sdk/guests/probe) y la [interfaz WIT](@source/sdk/wit/orange-expansion.wit). Elige un identificador propio como `org.example.text/normalize`. Declara tipos y puertos versionados en `nodes/`, registra el proveedor en `manifest.json` y añade etiquetas Fluent en español e inglés.

Implementa la función invitada respetando el orden declarado. Elige Pure para cálculo determinista, Action para un paso de ejecución o Async para E/S soportada por el host. Compila a `wasm32-unknown-unknown` con `wit-bindgen`, convierte el módulo con `orange-componentize` y apunta el manifiesto al componente. [Empaqueta e instala](../packages/#pack-a-directory) y prueba las entradas válidas y los fallos esperados.

<h2 id="trusted-services">Añade un servicio Rust</h2>

Registra un `ServicePackage` con `Host::with_services`. Implementa `ServiceNode` para un nodo, `ServiceSource` para un disparador o `ConnectionAdapter` para una conexión. Consulta [el ejemplo de Shout](@source/app/crates/dev/orange-plugin-example/src/lib.rs) y la [referencia del SDK](../sdk/).
