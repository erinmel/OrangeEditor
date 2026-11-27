# Cuenta las pulsaciones de un botón

Construye un contador con tres botones. El primero suma uno, el segundo resta uno y el tercero restablece el valor. Puedes comparar tu trabajo con el [grafo terminado](../examples/#counter-with-reset).

<h2 id="open-an-empty-graph">Abre un grafo vacío</h2>

```sh
cargo run -p orange-desktop --locked
```

Deja abiertos **Nodos** y **Propiedades**.

<h2 id="declare-the-counter">Declara el contador</h2>

En **Variables**, crea `Counter` con tipo **Entero** y valor inicial `0`. Elige el ámbito de ejecución para que el contador conserve su valor entre activaciones y vuelva a cero al iniciar otra ejecución.

<h2 id="add-the-first-button">Añade el primer botón</h2>

1. Añade [**Manual**](../nodes/core/on-manual/1/) y ponle el nombre `Increment`.
2. Abre el grupo **Variables** en **Nodos** y añade [**Incrementar contador**](../nodes/core/increment-counter/1/). Llámalo `Add one` y, en **Propiedades**, elige `Counter` como variable. Su **Paso** es `1`, así que cada ejecución suma uno.
3. Conecta la [**Salida**](../nodes/core/on-manual/1/#port-then) del disparador con la [**Entrada**](../nodes/core/increment-counter/1/#port-in) de la acción.

<!-- counter-wire -->

Selecciona una tarjeta para ver en **Propiedades** todo lo que puedes ajustar:

<!-- tutorial-graph: editor-counter-properties counter-with-reset -->

<h2 id="run-it">Ejecuta el grafo</h2>

Revisa **Problemas** y pulsa **Ejecutar**. Dispara `Increment` tres veces. En **Variables** debe aparecer **`Counter = 3`**. Cada pulsación inicia una activación. El disparador guarda hasta 16 pulsaciones en espera mientras se ejecuta otra. Si llega una más, el disparador falla.

<!-- tutorial-graph: editor-counter-variables counter-with-reset -->

<h2 id="add-the-other-two-buttons">Añade los otros dos botones</h2>

Detén el grafo. Conecta un disparador `Decrement` a [**Decrementar contador**](../nodes/core/decrement-counter/1/), llamado `Subtract one`, con la variable `Counter`. Conecta `Reset` a [**Escribir variable**](../nodes/core/set-variable/1/) y usa `0`. Las dos acciones usan `Counter`. Ejecuta el grafo, dispara `Increment` tres veces y luego `Decrement` y `Reset`. Comprueba los valores `3`, `2` y `0`. Guarda tu grafo.

<!-- tutorial-graph: counter-with-reset -->

<h2 id="what-you-built">Qué construiste</h2>

Los disparadores inician el trabajo, los cables de ejecución ordenan los pasos y la variable guarda el estado entre pulsaciones. Consulta [datos y ejecución](../data-and-execution/) para ver cómo se combinan con los cables de datos, o sigue con [Repite una acción con un bucle](../repeat-with-a-loop/).

<h2 id="try-a-graph-with-data-wires">Prueba un grafo con cables de datos</h2>

Abre [Extracción de JSON](../examples/#json-extraction) y dispara **Read sample JSON**. [**Leer JSON**](../nodes/org.orange.json/parse/1/) lee la muestra. [**Leer texto JSON**](../nodes/org.orange.json/read-text/1/) entrega el nombre, o su valor alternativo, a la acción que guarda la variable. En **Variables**, comprueba que `extracted_text` vale `Studio mic`.

<!-- tutorial-graph: json-extraction -->
