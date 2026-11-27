# Comparte valores y eventos entre grafos

## Comparte una variable

Declara la variable con alcance compartido y conserva el mismo identificador y tipo en cada grafo. Usa un nodo que lea la variable y acciones que asignen o actualicen el valor compartido. Las escrituras se aplican una a la vez y cada grafo ve el cambio en su siguiente turno.

## Envía un evento

[Despachar](../nodes/core/emit/1/) activa [Escuchar](../nodes/core/on-event/1/) en los otros grafos en ejecución que coinciden con el evento de espacio de trabajo. Cada receptor recibe el valor del evento en una activación. Declara estos eventos en los documentos de la aplicación que aloja el motor.

## Declara el espacio de trabajo desde Rust

```rust,ignore
use orange_host::{Host, HostOptions};

let (mut host, events) = Host::new(HostOptions::default())?;
host.share_variables(&workspace_json)?;
let scoring = host.open(&scoring_json)?;
let overlay = host.open(&overlay_json)?;
```

Abre los grafos después de registrar la declaración común. Usa `Host::shared_variable` para leer un valor y `Host::shared_writes` para detectar cambios. Los grafos que usan eventos de espacio de trabajo se turnan. Al editar o detener un grafo se descartan los eventos que tenga pendientes. Consulta la [integración del motor](../embedding/) para el ciclo de vida completo.
