# Integra el motor

## Añade el host

Usa la misma revisión del repositorio para cada dependencia de OrangeEditor.

```toml
[dependencies]
orange-host = { git = "https://github.com/erinmel/OrangeEditor.git", rev = "@revision@" }
```

Una copia local puede usar `app/crates/platform/orange-host` como dependencia por ruta.

## Conecta tu aplicación

1. Crea un `Host` y conserva `Events`. Obtén el catálogo y los paquetes con `host.nodes()` y `host.packages()`.
2. Valida JSON guardado con `Host::check`. Usa `Draft` para editar y `GraphView` para presentar el grafo.
3. Abre el flujo con `host.open` y conserva el `FlowId`. Usa `host.observe` para valores activos y `host.trigger` para un disparador manual.
4. Procesa los eventos por flujo sin bloquear la interfaz y ciérralo al terminar.

```sh
cargo run -p orange-host --example attach --locked
```

El [ejemplo completo](@source/app/crates/platform/orange-host/examples/attach.rs) valida, dispara y espera un cambio de variable. Tu aplicación administra ventanas, archivos, credenciales y permisos. Consulta [Rustdoc de orange-host](@shared/api/orange_host/) o [integra el editor egui](../egui/).
