# Modelo de seguridad

## Permisos por conexión

Los grafos usan conexiones que el host controla. Una conexión a una carpeta limita las operaciones a ese directorio. El acceso externo puede requerir autorización por grafo y conexión. Guarda los secretos en el almacén del sistema. El grafo persiste sus referencias.

## Extensiones

Al instalar un paquete, revisa las capacidades solicitadas. Los componentes Wasm usan las importaciones que les proporciona el host. Las bibliotecas nativas corren con la autoridad del proceso y deben venir de un proveedor de confianza. Los grafos conservan las identidades de los proveedores con los que se guardaron.

## Comparte un ejemplo

Revisa las trazas y el grafo antes de compartirlo. Usa datos de prueba y referencias a credenciales. Consulta el [contrato de paquetes en inglés](@shared/reference/packages/) y la [guía de conexiones HTTP](../call-an-http-api/).
