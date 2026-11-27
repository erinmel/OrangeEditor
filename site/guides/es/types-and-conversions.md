# Tipos y conversiones

## Sigue el tipo del puerto

Cada puerto de datos tiene un tipo. Por ejemplo, texto, entero, booleano u opcional de texto. Los tipos genéricos se resuelven a partir de las conexiones y declaraciones. Cuando un cable no encaja, revisa los dos tipos en la [referencia de nodos](../nodes/).

## Convierte explícitamente

Añade un nodo de la sección **Conversiones**. Por ejemplo, para usar el texto de un archivo en una operación numérica, analízalo primero y gestiona el resultado según sea válido o falle. La conversión queda visible para la siguiente persona que abra el grafo.

## Distingue ausencia y fallo

Un opcional representa un valor que puede faltar. Un resultado representa éxito o error. Consulta los [cables de datos y ejecución](../data-and-execution/) y el [contrato del lenguaje en inglés](@shared/reference/graph-language/).
