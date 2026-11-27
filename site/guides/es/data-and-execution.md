# Cables de datos y de ejecución

## Cables de ejecución

Indican qué ocurre después. Un [disparador manual](../nodes/core/on-manual/1/) inicia una activación. Cada acción sigue por una salida, por ejemplo **Verdadero** o **Falso** en [Rama](../nodes/core/branch/1/). Si la ejecución no llega a una acción, esa acción no produce efectos.

## Cables de datos

Llevan un valor de un puerto de salida a una entrada compatible. La acción lee el valor al llegar a ese paso. Un nodo puro se calcula bajo demanda y comparte el resultado entre las entradas que lo leen durante esa demanda. Algunas salidas solo existen tras recorrer su ruta de ejecución. Por ejemplo, el estado de una [petición HTTP](../nodes/org.orange.net/http-request/2/) está disponible tras recibir una respuesta.

## Juntos en un grafo

En el [contador](../count-button-presses/), un disparador activa el nodo que actualiza la variable. Si conectas la variable a la condición de una rama, se lee el valor que tenga cuando se ejecute esa rama.

Consulta [tipos y conversiones](../types-and-conversions/) y el [contrato del lenguaje de grafos en inglés](@shared/reference/graph-language/).
