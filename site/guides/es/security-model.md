# Modelo de seguridad

OrangeEditor ejecuta grafos y extensiones que escriben otras personas, y esos grafos
llegan a archivos, a la red y a tus dispositivos. Esta página explica qué puede hacer
cada tipo de código, qué le impide hacer más y qué depende de ti.

<h2 id="who-can-do-what">Quién puede hacer qué</h2>

| Código | Se ejecuta | Lo limita |
|---|---|---|
| Nodos integrados | Dentro de la aplicación | Se revisan junto con la aplicación |
| Grafos | En el motor | Sus nodos y los límites de tiempo, memoria, colas y recursión |
| Extensiones Wasm | Dentro de la aplicación, en un entorno aislado | Los permisos que concedes al instalarlas y los límites de combustible, tiempo y memoria |
| Extensiones nativas | Dentro de la aplicación | Nada: actúan con todos tus permisos |

<h2 id="graphs">Grafos</h2>

Un grafo solo puede hacer lo que hacen sus nodos. Un grafo que abres no se ejecuta
hasta que pulsas **Ejecutar**, y solo llega al exterior a través de conexiones:

- **Las conexiones guardan direcciones y credenciales**, así que el archivo del grafo
  nunca las contiene. La primera vez que un grafo usa una conexión que sale del
  equipo, el editor te pide que apruebes ese grafo para ella.
- **Se conceden carpetas, no rutas.** Los nodos de archivos trabajan solo dentro de
  una conexión de carpeta. Se rechazan las rutas que salen de ella, los enlaces que
  apuntan fuera, los nombres de dispositivo de Windows como `CON` y los flujos de datos
  alternativos. Las carpetas de la ruta se mantienen abiertas para que nadie pueda
  cambiarlas durante una operación.
- **Las peticiones HTTP** solo llegan a la dirección base de su conexión. No se siguen
  redirecciones y el tamaño de las respuestas está limitado.

<h2 id="credentials">Credenciales</h2>

Los secretos, como claves de API y tokens, se guardan en el almacén de credenciales
del sistema operativo. Los grafos, las exportaciones, los registros y las trazas solo
contienen referencias `credential://` y valores ocultos, así que compartir un grafo
nunca comparte sus secretos.

<h2 id="wasm-plugins">Extensiones Wasm</h2>

Una extensión Wasm se ejecuta en un entorno aislado, sin acceso al equipo más allá de
lo que concedes al instalarla:

- **files**: su propia carpeta de datos, con una cuota de tamaño.
- **http**: direcciones públicas `https://`, opcionalmente solo los hosts que indiques.
  Nunca llega a este equipo ni a la red local: la dirección se vuelve a comprobar
  después de resolver el nombre, de modo que un nombre público no puede apuntar hacia
  dentro.
- **http-local**: este equipo y la red local, como un dispositivo en tu Wi-Fi.
- **compute**: más tiempo y memoria para trabajo pesado.

Puedes ver y retirar los permisos en **Paquetes**. Cada llamada se ejecuta con límites
de combustible, tiempo y memoria. Una extensión que los supera hace fallar su propia
activación y nunca bloquea el editor.

<h2 id="native-plugins">Extensiones nativas</h2>

Una extensión nativa es una biblioteca que se ejecuta con todos tus permisos: puede
leer y cambiar tus archivos, usar la red y los dispositivos, leer tus credenciales y
cerrar la aplicación. La aplicación de escritorio solo la carga después de que
concedas acceso completo a ese archivo exacto, y vuelve a preguntar si el archivo
cambia. Instala extensiones nativas solo de personas en las que confíes.

<h2 id="packages-and-updates">Paquetes y actualizaciones</h2>

- Los archivos de un paquete se comprueban con el resumen criptográfico que contiene,
  y el grafo registra el proveedor exacto con el que se construyó, de modo que un
  paquete no puede cambiar bajo un grafo guardado.
- Un canal de actualizaciones puede estar firmado. Un canal firmado se comprueba con
  la clave del editor antes de leerlo, y `orange updates --signed-only` rechaza los que
  no lo están.

<h2 id="what-is-up-to-you">Qué depende de ti</h2>

- Instala extensiones nativas solo de fuentes de confianza.
- Concede a las extensiones Wasm solo lo que necesitan y, para **http**, prefiere una
  lista de hosts.
- Mantén la aplicación actualizada y comprueba las descargas con el `SHA256SUMS`
  publicado y la atestación de procedencia.

<h2 id="see-also">Consulta también</h2>

- [Crea paquetes](../packages/)
- [Contrato de paquetes y permisos (en inglés)](@shared/reference/packages/)
