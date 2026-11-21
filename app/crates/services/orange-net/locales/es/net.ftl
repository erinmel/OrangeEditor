# Textos del paquete de red en español. Los IDs son las claves de los nodos con
# "/", "." y "_" escritos como "-".

org-orange-net-category-network = Red

org-orange-net-amqp-binary-label = Al mensaje AMQP binario
org-orange-net-amqp-message-label = Al mensaje AMQP
org-orange-net-amqp-publish-label = Publicar mensaje AMQP
org-orange-net-amqp-publish-bytes-label = Publicar mensaje AMQP binario
org-orange-net-amqp-publish-then = Confirmado
org-orange-net-amqp-publish-bytes-then = Confirmado
org-orange-net-amqp-message-invalid = La clave de enrutamiento o el mensaje no es válido o es demasiado grande.
org-orange-net-amqp-confirm-timeout = Venció el plazo para recibir la confirmación de la publicación AMQP.
org-orange-net-amqp-confirm-connection = La conexión AMQP terminó antes de la confirmación.
org-orange-net-amqp-publish-rejected = El intermediario AMQP rechazó la publicación.
org-orange-net-amqp-publish-unroutable = El intercambio AMQP no pudo dirigir el mensaje a una cola.
org-orange-net-amqp-consume-refused = El intermediario AMQP rechazó al consumidor o envió una entrega no válida.
org-orange-net-amqp-delivery-too-large = La entrega AMQP supera el tamaño máximo permitido.
org-orange-net-http-request-label = Petición HTTP
org-orange-net-http-request-keywords = HTTP, HTTPS, REST, API, GET, POST, JSON, cabeceras, encabezados
org-orange-net-http-request-description = Envía una petición por HTTP o HTTPS y devuelve la respuesta como texto. Elige una conexión, escribe una ruta que empiece por / y añade, si hace falta, un tipo de contenido, un cuerpo y las cabeceras (una por línea, en formato Nombre: valor). Todos los estados HTTP se devuelven como resultados. La salida de fallo indica que no se pudo completar la petición.
org-orange-net-http-request-bytes-label = Petición HTTP (binaria)
org-orange-net-http-request-json-label = Petición HTTP JSON
org-orange-net-http-request-json-description = Envía un cuerpo JSON y devuelve el estado HTTP, las cabeceras, el texto de la respuesta y un resultado JSON opcional.
org-orange-net-http-request-json-keywords = http, https, rest, api, post, json, cuerpo
org-orange-net-http-stream-label = Petición HTTP (por partes)
org-orange-net-http-stream-line-label = Al llegar una línea HTTP
org-orange-net-mqtt-binary-label = Al mensaje MQTT binario
org-orange-net-mqtt-message-label = Al mensaje MQTT
org-orange-net-mqtt-publish-label = Publicar mensaje MQTT
org-orange-net-mqtt-publish-bytes-label = Publicar mensaje MQTT binario
org-orange-net-ws-binary-label = Al mensaje WebSocket binario
org-orange-net-ws-connected-label = Al conectar WebSocket
org-orange-net-ws-disconnected-label = Al desconectar WebSocket
org-orange-net-ws-message-label = Al mensaje WebSocket
org-orange-net-ws-send-label = Enviar mensaje WebSocket
org-orange-net-ws-send-bytes-label = Enviar mensaje WebSocket binario
org-orange-net-ws-status-label = Estado de WebSocket

port-method = método
port-content = contenido
port-content-type = tipo de contenido
port-headers = cabeceras
port-status = estado
port-success = correcto
port-response = respuesta
port-response-type = tipo de respuesta
port-lines = líneas
port-topic = tema
port-queue = cola
port-routing-key = clave de enrutamiento
port-retain = retener
port-connected = conectado
port-queued = en cola
