# Glosario

Las palabras que usa esta documentación, en orden alfabético.

<h3 id="activation">Activación</h3>

Una ejecución de un flujo desde una entrada, que empieza cada vez que la entrada se
dispara. Varias activaciones pueden esperar en la cola de la entrada.

<h3 id="channel">Canal</h3>

El nombre que comparten **Despachar** y **Escuchar**. Un **Despachar** en un canal
inicia cada **Escuchar** de ese canal.

<h3 id="data-wire">Cable de datos</h3>

Un cable que lleva un valor tipado de una salida a una entrada. Los valores se leen
cuando se necesitan. Consulta [cables de datos y de ejecución](../data-and-execution/).

<h3 id="execution-wire">Cable de ejecución</h3>

Un cable que indica qué acción se ejecuta a continuación.

<h3 id="connection">Conexión</h3>

Un vínculo configurado con algo fuera del grafo, como un servicio HTTP, una carpeta o
un dispositivo de audio. Guarda la dirección y las referencias a credenciales.

<h3 id="conversion">Conversión</h3>

Un nodo que cambia un valor de un tipo a otro, con sus reglas para los valores que no
caben. Las conversiones nunca están ocultas. Consulta
[tipos y conversiones](../types-and-conversions/).

<h3 id="definition">Definición</h3>

La descripción de un tipo de nodo: su identificador, versión, puertos y ajustes.

<h3 id="entry">Entrada</h3>

Un nodo que inicia activaciones, como **Manual**, un temporizador o **Escuchar**.

<h3 id="workspace-event">Evento del espacio de trabajo</h3>

Un evento que un grafo despacha a los demás grafos en ejecución del mismo host.
Consulta [comparte valores y eventos entre grafos](../share-between-graphs/).

<h3 id="flow">Flujo</h3>

Un grafo mientras se ejecuta, en su propio hilo. Un host ejecuta muchos flujos.

<h3 id="graph">Grafo</h3>

Un documento de nodos y cables, guardado como JSON.

<h3 id="host">Host</h3>

La sesión de `orange-host` que usa una aplicación para editar, comprobar y ejecutar
grafos.

<h3 id="pure-node">Nodo puro</h3>

Un nodo que calcula solo a partir de sus entradas, sin efectos. Su valor se calcula una
vez por cada demanda.

<h3 id="option">Opción</h3>

Un valor que puede faltar.

<h3 id="package">Paquete</h3>

Un conjunto instalable de tipos de nodo, grafos o código, en un archivo `.orangeplug`.
Consulta [crea paquetes](../packages/).

<h3 id="grant">Permiso</h3>

Un acceso que concedes a una extensión Wasm: files, http, http-local o compute.
Consulta el [modelo de seguridad](../security-model/).

<h3 id="provider">Proveedor</h3>

El paquete y la versión exactos que implementan un tipo de nodo en un grafo guardado.

<h3 id="bridge">Puente</h3>

Un par **Puente salida** y **Puente entrada** que comparte un valor sin un cable
visible. Leer el **Puente entrada** obtiene el valor; nunca inicia la ejecución.

<h3 id="credential-reference">Referencia a credencial</h3>

Un enlace `credential://` a un secreto en el almacén de credenciales del sistema
operativo. Los grafos guardan referencias, nunca secretos.

<h3 id="result">Resultado</h3>

Un valor o un error.

<h3 id="node-kind">Tipo de nodo</h3>

Una clase de nodo, como una bifurcación o una petición HTTP, identificada por un ID como
`core/branch`.

<h3 id="shared-variable">Variable compartida</h3>

Una variable que todos los grafos de un host pueden leer y escribir.

<h3 id="run-variable">Variable de ejecución</h3>

Una variable que existe mientras se ejecuta un flujo y vuelve a empezar en la siguiente
ejecución.
