# Reacciona a cambios en archivos

## Autoriza una carpeta

En **Conexiones**, añade una conexión de carpeta y elige el directorio que vas a vigilar. El grafo solo puede acceder al contenido dentro de esa carpeta.

## Dispara un flujo

Coloca el disparador de cambio de archivo y elige la conexión en **Propiedades**. Usa `folder` para limitar la observación a una subcarpeta como `incoming`. Lleva la salida de ejecución a las acciones que deben reaccionar.

Cada cambio inicia una activación. `payload` contiene el tipo de cambio, la ruta relativa, la ruta previa si cambió el nombre y si el elemento es una carpeta. Los archivos presentes al inicio sirven como punto de partida.

## Espera a una carpeta que aún no existe

Si otro programa creará la carpeta más adelante, activa `wait_for_folder`. Con el ajuste predeterminado, una ruta ausente produce un fallo. Usa la misma conexión para leer o mover el archivo. Consulta el [catálogo de nodos](../nodes/) y el [modelo de permisos](../security-model/).
