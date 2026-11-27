export const sections = {
  start: 'Primeros pasos', tutorials: 'Tutoriales', 'how-to': 'Guías prácticas',
  concepts: 'Conceptos', reference: 'Referencia',
};

export const pages = {
  '': ['Inicio', 'Una base en Rust para grafos de nodos tipados con un editor en egui.'],
  'getting-started': ['Primeros pasos', 'Abre el editor y ejecuta tu primer grafo.'],
  'count-button-presses': ['Cuenta las pulsaciones de un botón', 'Construye un contador con disparadores, una variable y acciones.'],
  'repeat-with-a-loop': ['Repite una acción con un bucle', 'Eleva un número a una potencia con Contar, una variable y cables de datos.'],
  'run-a-graph-from-rust': ['Ejecuta un grafo desde Rust', 'Abre un grafo guardado en tu programa, pulsa sus botones y lee sus variables.'],
  editor: ['Usa el editor', 'Coloca y conecta nodos, configura variables y revisa los resultados.'],
  'call-an-http-api': ['Llama a una API HTTP', 'Configura una conexión y distingue respuestas HTTP de fallos de red.'],
  'react-to-file-changes': ['Reacciona a cambios en archivos', 'Ejecuta un flujo cuando cambie un archivo en una carpeta autorizada.'],
  'share-between-graphs': ['Comparte valores y eventos entre grafos', 'Comparte variables y eventos mientras se ejecutan varios grafos.', 'Compartir entre grafos'],
  'custom-nodes': ['Crea nodos personalizados', 'Extrae funciones de grafo o implementa nodos en Wasm y Rust.'],
  packages: ['Crea paquetes', 'Empaqueta, instala y actualiza archivos .orangeplug.'],
  embedding: ['Integra el motor', 'Valida, ejecuta y observa grafos desde una aplicación Rust.'],
  egui: ['Integra y personaliza el editor egui', 'Elige paneles, estilos, fuentes e iconos para tu aplicación.', 'Integrar el editor'],
  'data-and-execution': ['Cables de datos y de ejecución', 'Cuándo se ejecuta una acción y cuándo se lee un valor.', 'Datos y ejecución'],
  'security-model': ['Modelo de seguridad', 'Qué permisos necesitan los grafos y las extensiones.'],
  'types-and-conversions': ['Tipos y conversiones', 'Conecta valores compatibles y convierte tipos de forma explícita.'],
  troubleshooting: ['Solución de problemas', 'Identifica el síntoma y revisa su causa.'],
  nodes: ['Referencia de nodos', 'Busca nodos por nombre, identificador, proveedor o puerto.', 'Nodos'],
  examples: ['Grafos de ejemplo', 'Ejecuta grafos y consulta los nodos que usan.'],
  controls: ['Referencia de controles', 'Elige y personaliza los controles egui de OrangeEditor.', 'Controles de interfaz'],
  cli: ['Referencia de la línea de comandos', 'Valida y ejecuta grafos desde la terminal.', 'Línea de comandos'],
  glossary: ['Glosario', 'Consulta el significado de los términos de OrangeEditor.'],
  sdk: ['Referencia del SDK', 'Consulta Rustdoc, las interfaces Wasm y los contratos.', 'SDK y API'],
};

export const ui = {
  contents: 'En esta página', previous: 'Anterior', next: 'Siguiente', sequence: 'Páginas relacionadas',
  skip: 'Saltar al contenido', search: 'Buscar documentación', theme: 'Tema de color',
  themes: ['Automático', 'Claro', 'Oscuro', 'Alto contraste', 'Confort'],
  docs: 'Documentación', revision: 'Revisión del código', index: 'Índice de texto',
  close: 'Cerrar', placeholder: 'Busca páginas, nodos o identificadores.',
  language: 'Idioma', searchShort: 'Buscar', searchAll: 'Todo', searchGuides: 'Guías', searchNodes: 'Nodos',
};

export const tutorial = {
  'counter-first-step': ['Tarjetas Increment y Add one conectadas en OrangeEditor', 'El cable de ejecución va de Increment a Add one.'],
  'counter-with-reset': ['Grafo del contador en OrangeEditor', 'Increment suma uno, Decrement resta uno y Reset vuelve a dejar Counter en cero.'],
  'json-extraction': ['Grafo de extracción de JSON en OrangeEditor', 'El disparador manual ejecuta la escritura de la variable. Los nodos JSON entregan el valor por los cables de datos.'],
  'power-with-count': ['Grafo de potencia con un bucle en OrangeEditor', 'Contar ejecuta cuerpo cuatro veces. Cada vuelta multiplica la variable por la base.'],
  'editor-counter-properties': ["OrangeEditor con Add one seleccionado y sus Propiedades abiertas", "Selecciona una tarjeta para ver sus ajustes en Propiedades. Add one es un Incrementar contador vinculado a Counter."],
  'editor-counter-variables': ["OrangeEditor ejecutando el contador con Variables abierto", "Tras pulsar Increment tres veces, Variables muestra Counter en 3, y Add one muestra el valor anterior y el nuevo que escribió."],
  'power-loop-start': ["El inicio del grafo del bucle en OrangeEditor", "El disparador pone la variable en 1 y después Contar ejecuta su cuerpo cuatro veces."],
  'http-with-failure-handling': ["Grafo de petición HTTP en OrangeEditor", "Toda respuesta sigue por Salida, donde una Rama lee success. Falló solo se ejecuta si no llegó respuesta."],
  'http-with-bounded-retry': ["Grafo de reintento HTTP en OrangeEditor", "Si la primera petición falla, espera 250 ms y lo intenta una vez más. Un segundo fallo es definitivo."],
  'editor-http-connections': ["OrangeEditor con la pestaña Conexiones abierta", "La conexión HTTP guarda la dirección base. Cada petición la elige y solo añade una ruta."],
  'editor-file-trigger': ["OrangeEditor con Al cambiar un archivo seleccionado", "El disparador vigila la carpeta incoming dentro de la carpeta autorizada. Sus datos indican qué cambió."],
  'file-change-journal': ["Grafo de registro de cambios de archivos en OrangeEditor", "Cada cambio lee la ruta de los datos, añade un salto de línea y la agrega a changes.log."],
  'shared-score': ["Grafo de puntuación compartida en OrangeEditor", "Add a point suma uno a la variable compartida Score y Reset la pone a cero. Todo grafo en ejecución que declare Score ve el cambio."],
  graph: 'Abre el grafo de ejemplo',
  linkedPorts: 'Consulta los puertos',
};

export const examples = {
  'counter-with-reset': ['Cuenta las pulsaciones de un botón', 'Dispara Increment tres veces para ver Counter en 3, Decrement una vez para ver 2 y Reset para volver a 0.'],
  'set-on-trigger': ['Asigna un valor al disparar', 'Activa el disparador manual y comprueba que total vale 7 en Variables.'],
  'idle-watchdog': ['Reinicia una cuenta atrás por inactividad', 'Dispara Heartbeat dos veces en menos de 750 ms. Idle pasa a verdadero al terminar la última cuenta atrás.'],
  'power-with-count': ['Calcula una potencia con un bucle', 'Dispara Calculate power. El resultado empieza en 1 y se multiplica por la base en cada iteración.'],
  'running-total-with-reduce': ['Acumula un total', 'Sigue el valor desde el inicio de la reducción, pasando por la suma, hasta el fin de la reducción.'],
  'json-extraction': ['Lee un campo de JSON', 'Dispara el grafo, sigue el documento hasta el nodo que lee el texto JSON y revisa la variable.'],
  'http-with-failure-handling': ['Dirige el resultado de una petición HTTP', 'Prueba con un servidor accesible. Compara una respuesta exitosa, un error HTTP y un fallo de conexión.'],
  'http-with-bounded-retry': ["Reintenta una petición HTTP una vez", "Elige un servidor de prueba accesible en la conexión y dispara Check endpoint. Detén el servidor para ver el reintento y después el fallo definitivo."],
  'file-change-journal': ["Registra los cambios de archivos", "Apunta la conexión a una carpeta que contenga otra llamada incoming, ejecuta el grafo y añade allí un archivo. Su ruta aparece en changes.log."],
  'shared-score': ["Comparte una puntuación entre grafos", "Ejecuta el grafo y dispara Add a point dos veces. Score vale 2 en Variables y en cada grafo en ejecución que la declare."],
  'quiet-mic-gate': ['Mide audio después de una puerta de ruido', 'Elige un dispositivo y ejecuta el grafo. Observa la señal cuando baje el nivel de entrada. El ejemplo usa dispositivos silenciosos hasta que los cambies.'],
};

export const notes = {
  'core/get-variable': 'Crea una variable y selecciónala en Propiedades. La salida toma el tipo de la variable.',
  'org.orange.collections/enum-payload': 'Elige un enum y una variante que contenga un dato. Ambos determinan los tipos de puerto.',
  'core/branch': 'Conecta un disparador manual a la entrada y un booleano verdadero a Condición. En la ruta Verdadero escribe matched en una variable de texto llamada Route y en Falso escribe skipped. Ejecuta y dispara. Route vale matched. Cambia el booleano a falso y dispara otra vez para ver skipped.',
  'core/bridge-in': 'Elige un canal de Puente salida y conecta el resultado a una entrada de datos. El nodo conectado lee el valor compartido.',
  'core/bridge-out': 'Conecta un valor, ponle nombre al canal y selecciónalo en uno o más nodos Puente entrada.',
  'core/coalesce': 'Deja vacío el número opcional y usa 10 como valor alternativo. El resultado es 10. Si proporcionas un 4, el resultado pasa a 4.',
  'core/emit': 'Elige un evento tipado para que aparezca la entrada de datos. Selecciona el mismo evento en Escuchar y ejecuta Despachar.',
  'core/on-event': 'Elige el evento de Despachar. Cuando se emite, este nodo inicia una activación.',
  'org.orange.math/add': 'Usa 2 en A y 3 en B con el mismo tipo numérico. La suma vale 5 cuando otro nodo la lee.',
  'org.orange.math/multiply': 'Usa 3 en A y 4 en B con el mismo tipo numérico. El producto vale 12.',
};

export const kinds = {
  entry: 'Inicia una activación cuando se dispara. Conecta una salida de ejecución al primer paso.',
  pure: 'Produce datos cuando un nodo conectado solicita el resultado.',
  volatile: 'Entrega un valor que puede cambiar entre evaluaciones.',
  action: 'Actúa cuando le llega la ejecución y sigue por una de sus salidas.',
  async: 'Inicia el trabajo cuando le llega la ejecución y reanuda el flujo por la salida que corresponda.',
  iterator: 'Entrega un elemento cada vez que el bucle lo solicita.',
};

export const commonPorts = {
  in: 'Recibe el cable de ejecución que inicia este paso.',
  then: 'Continúa la ejecución al terminar el paso.',
  failed: 'Sigue esta ruta si falla la operación.',
  cancelled: 'Sigue esta ruta si se cancela el trabajo.',
  body: 'Ejecuta el cuerpo del bucle en esta iteración.',
  completed: 'Continúa después de terminar el bucle.',
};

export const portLabels = {
  in: 'entrada', then: 'luego', condition: 'condición', true: 'verdadero', false: 'falso',
  failed: 'Falló', cancelled: 'Cancelado', error: 'Error',
};
