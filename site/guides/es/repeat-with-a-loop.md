# Repite una acción con un bucle

En este tutorial calculas 3 elevado a 4 multiplicando una variable por 3, cuatro veces.
Conoces el bucle [**Contar**](../nodes/core/count/1/), que ejecuta los mismos pasos un
número fijo de veces, y ves cómo trabajan juntos los cables de datos y de ejecución en
un mismo flujo. Lleva unos diez minutos.

Debes saber colocar nodos, conectarlos y disparar un disparador. Si no, empieza por
[Cuenta las pulsaciones de un botón](../count-button-presses/).

<h2 id="declare-the-variable">Declara la variable</h2>

1. Abre el editor con un grafo vacío:

   ```sh
   cargo run -p orange-desktop --locked
   ```

2. Abre **Variables** y añade una variable llamada `power` con el tipo **Entero**.
   Mantén el ámbito de ejecución.

<h2 id="start-from-one">Empieza desde uno</h2>

Cada ejecución debe partir del mismo valor; si no, una segunda pulsación seguiría
multiplicando el último resultado.

1. Coloca [**Manual**](../nodes/core/on-manual/1/) y llámalo `Calculate`.
2. Coloca [**Escribir variable**](../nodes/core/set-variable/1/) a su derecha. En
   **Propiedades**, elige `power` y pon el valor `1`.
3. Conecta la **Salida** del disparador con la **Entrada** de la acción.

<h2 id="add-the-loop">Añade el bucle</h2>

1. Coloca [**Contar**](../nodes/core/count/1/) a la derecha de **Escribir variable**.
2. En **Propiedades**, pon [**cantidad**](../nodes/core/count/1/#port-count) en `4`.
3. Conecta la **Salida** de **Escribir variable** con la **Entrada** de **Contar**.

**Contar** tiene dos salidas de ejecución. [**cuerpo**](../nodes/core/count/1/#port-body)
se ejecuta en cada vuelta y [**terminado**](../nodes/core/count/1/#port-completed)
después de la última. Su salida [**índice**](../nodes/core/count/1/#port-index) indica
qué vuelta se está ejecutando, empezando en `0`.

<!-- tutorial-graph: power-loop-start power-with-count -->

<h2 id="multiply-on-each-pass">Multiplica en cada vuelta</h2>

El paso que se ejecuta en cada vuelta lee `power`, lo multiplica por 3 y lo vuelve a
escribir.

1. Coloca [**Leer variable**](../nodes/core/get-variable/1/) debajo del bucle y elige
   `power` en **Propiedades**. No tiene puertos de ejecución: se lee cuando un paso
   necesita su valor.
2. Coloca [**Multiplicar**](../nodes/org.orange.math/multiply/1/). Conecta
   [**valor**](../nodes/core/get-variable/1/#port-value) de **Leer variable** con su
   [**a**](../nodes/org.orange.math/multiply/1/#port-a), y pon
   [**b**](../nodes/org.orange.math/multiply/1/#port-b) en `3` en **Propiedades**.
3. Coloca otro **Escribir variable** con `power` y conecta
   [**producto**](../nodes/org.orange.math/multiply/1/#port-product) de **Multiplicar**
   con su **valor**.
4. Conecta **cuerpo** de **Contar** con la **Entrada** de este **Escribir variable**.

<!-- tutorial-graph: power-with-count -->

El grafo de ejemplo guarda la base y el exponente en nodos **Entero** para cambiarlos en
un solo lugar, y guarda la respuesta en otra variable desde **terminado**.

<h2 id="run-it">Ejecútalo</h2>

1. Comprueba que **Problemas** está vacío y pulsa **Ejecutar**.
2. Pulsa **Disparar** en `Calculate`.
3. Abre **Variables** y comprueba **`power = 81`**.

Pulsa **Disparar** otra vez: la respuesta sigue siendo 81, porque el primer **Escribir
variable** empieza cada activación desde 1.

<h2 id="what-you-built">Qué construiste</h2>

- **Un bucle es una acción con más de una salida.** **cuerpo** se ejecuta en cada vuelta y
  **terminado** una vez al final, así que los pasos posteriores esperan al bucle.
- **Los datos se leen cuando se necesitan.** **Leer variable** y **Multiplicar** se
  ejecutan en cada vuelta, cuando el **Escribir variable** de **cuerpo** pide su valor, así
  que siempre ven el último `power`.
- **Los literales llenan las entradas sin conectar.** Una entrada que fijas en
  **Propiedades** conserva su valor hasta que le conectas algo.

Después, [ejecuta este tipo de grafo desde tu propio programa en Rust](../run-a-graph-from-rust/),
o lee por qué los grafos tienen [dos tipos de cables](../data-and-execution/).
