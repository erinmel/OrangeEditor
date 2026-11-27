# Llama a una API HTTP

Abre el [grafo terminado](../examples/#http-with-failure-handling) para seguir sus nodos y probar el resultado.

## Crea la conexión

En **Conexiones**, añade una conexión HTTP. Define la URL base (`http://` o `https://`, sin usuario en la dirección) y el tiempo de espera. Guarda la clave en el campo de secreto. El grafo conserva una referencia `credential://`. Aprueba el acceso cuando el editor lo solicite.

## Envía la petición

Conecta un [disparador manual](../nodes/core/on-manual/1/) a [Petición HTTP](../nodes/org.orange.net/http-request/2/) y elige la conexión en **Propiedades**. Configura `method` y una ruta `path` que empiece con `/`. Usa `content` y `content_type` para el cuerpo, `header_fields` para un mapa de cabeceras o cabeceras como texto, y `query` para los parámetros. Las redirecciones no se siguen.

## Gestiona el resultado

- **then** significa que el servidor respondió. Lee `status`, `success` (estado `2xx`), `response` y `response_headers`.
- **failed** significa que la petición no obtuvo respuesta, por ejemplo por un plazo vencido. Lee `error` en esta ruta.

Conecta [Rama](../nodes/core/branch/1/) después de `then` y usa `success` como condición. Conecta una acción distinta a `failed`.

## Reintenta una vez

Para operaciones que se pueden repetir, conecta `failed` a una espera de 250 ms y luego a una segunda petición. Trata el fallo de esa segunda petición como definitivo. Un `POST` repetido puede crear dos registros.
