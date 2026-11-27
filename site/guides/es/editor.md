# Usa el editor

## Coloca y conecta nodos

1. Busca por nombre o tarea en **Nodos** y coloca una tarjeta en el lienzo. Para convertir tipos, busca en **Conversiones**.
2. Arrastra entre puertos compatibles. Soltar un cable en el lienzo muestra sugerencias. Lee [cómo funcionan los cables](../data-and-execution/).
3. Selecciona la tarjeta y abre **Propiedades** para configurar valores, declaraciones y puertos dinámicos.
4. Revisa **Problemas**, ejecuta el grafo, dispara un evento y observa las variables, los cables o el registro.

Selecciona un solo nodo integrado y pulsa **F1** para abrir su referencia. Prueba con [Rama](../nodes/core/branch/1/). Puedes cambiar el idioma en **Configuración** y buscar por nombres de otros idiomas.

## Comparte valores y eventos

Las variables guardan estado. **Despachar** y **Escuchar** comparten un evento, con o sin un valor tipado. **Puente salida** y **Puente entrada** comparten un valor que se lee bajo demanda. Consulta la guía para [compartir entre grafos](../share-between-graphs/).

## Conecta un servicio y revisa sus fallos

Crea una conexión y selecciónala en el nodo correspondiente. Guarda las credenciales con los controles del sistema y aprueba la conexión cuando se solicite. Para HTTP, distingue una respuesta del servidor de un fallo de conexión. Sigue el ejemplo [Llama a una API HTTP](../call-an-http-api/).

Si el grafo no se ejecuta, sigue la [guía de solución de problemas](../troubleshooting/).
