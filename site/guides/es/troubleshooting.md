# Solución de problemas

Busca el síntoma y sigue la solución.

<h2 id="the-editor-or-download">El editor o la descarga</h2>

<h3 id="windows-says-it-protected-your-pc">Windows dice que protegió tu equipo.</h3>

Las versiones preliminares aún no tienen firma de código. Elige **Más información** y
después **Ejecutar de todas formas**. Antes, comprueba la descarga con `SHA256SUMS` en
la página de la versión.

<h3 id="macos-says-it-cannot-check-the-app">macOS dice que no puede comprobar la aplicación.</h3>

Haz clic derecho en OrangeEditor y elige **Abrir**, o permítela en **Ajustes del
Sistema → Privacidad y seguridad**.

<h3 id="a-linux-source-build-fails-on-alsa">La compilación en Linux falla por ALSA.</h3>

Instala los archivos de desarrollo de ALSA, como `libasound2-dev` en Ubuntu y Debian.

<h2 id="running-a-graph">Ejecutar un grafo</h2>

<h3 id="run-is-blocked">No se puede ejecutar.</h3>

Abre **Problemas** y sigue el primer diagnóstico hasta la tarjeta, el puerto o el
paquete que nombra.

<h3 id="nothing-happens-after-run">No pasa nada al ejecutar.</h3>

Un grafo solo empieza a trabajar en una entrada. Pulsa **Disparar** en un nodo
**Manual**, o provoca el evento que espera su entrada, y sigue sus salidas de
ejecución.

<h3 id="an-action-never-runs">Una acción nunca se ejecuta.</h3>

Comprueba que un cable de ejecución llega a su puerto **Entrada**. Una acción sin él
nunca se ejecuta; consulta [cables de datos y de ejecución](../data-and-execution/).

<h3 id="a-trigger-faults-when-pressed-quickly">Un disparador falla al pulsarlo rápido.</h3>

Su cola está llena. Un disparador guarda hasta 16 activaciones en espera mientras se
ejecuta otra. Haz que cada activación trabaje menos, o cambia `capacity` y `overflow`
en el `activation` del nodo en el archivo del grafo; consulta el
[formato de archivo de grafo (en inglés)](@shared/reference/graph-file-format/).

<h3 id="two-ports-will-not-connect">Dos puertos no se conectan.</h3>

Sus tipos son distintos. Suelta el cable sobre la entrada para ver las conversiones, o
busca **Conversiones** en **Nodos**. Consulta [tipos y conversiones](../types-and-conversions/).

<h2 id="connections-files-and-the-network">Conexiones, archivos y red</h2>

<h3 id="the-editor-asks-to-approve-a-connection">El editor pide aprobar una conexión.</h3>

Un grafo usa por primera vez una conexión que sale del equipo. Apruébala si confías en
el grafo.

<h3 id="an-http-request-goes-to-failed">Una petición HTTP sale por failed.</h3>

No llegó respuesta: la dirección es incorrecta, el servicio no está disponible o se
agotó el tiempo. Un estado de error como `404` llega por **then**; lee **status**.
Consulta [llama a una API HTTP](../call-an-http-api/).

<h3 id="a-file-watcher-reports-a-missing-folder">Un observador de archivos indica que falta una carpeta.</h3>

La carpeta observada aún no existe. Corrige la ruta, o activa **wait_for_folder** para
esperar a que aparezca.

<h2 id="packages">Paquetes</h2>

<h3 id="a-saved-node-has-a-missing-provider">Un nodo guardado no encuentra su proveedor.</h3>

El grafo necesita un paquete que no tienes. Revisa su paquete y versión en
**Paquetes** e instala ese paquete exacto. Consulta [crea paquetes](../packages/).

<h3 id="a-plugins-request-is-not-allowed">No se permite la petición de una extensión.</h3>

Le falta el permiso para esa dirección. Revisa sus permisos en **Paquetes**: **http**
llega a direcciones públicas `https://` y **http-local** a este equipo y a la red local.

<h2 id="keys-and-devices">Teclas y dispositivos</h2>

<h3 id="a-key-set-to-anywhere-works-only-while-the-editor-is-focused">Una tecla configurada en cualquier lugar solo funciona con el editor enfocado.</h3>

Otra aplicación o el sistema ya usa esa tecla, o tu escritorio Linux no ofrece atajos
globales. El **Registro** nombra la tecla; elige otra.

<h3 id="an-audio-device-is-missing">Falta un dispositivo de audio.</h3>

Conecta el dispositivo y vuelve a elegirlo en su conexión. El **Registro** indica qué
dispositivo no se pudo abrir.

<h2 id="still-stuck">¿Sigues sin resolverlo?</h2>

Busca en esta documentación, o abre una incidencia en
[GitHub](https://github.com/erinmel/OrangeEditor/issues) con el registro y el grafo, sin
incluir secretos.
