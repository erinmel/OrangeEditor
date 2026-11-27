# Referencia del SDK

## Elige el punto de entrada

- [Motor sin interfaz](../embedding/) para ejecutar grafos desde Rust.
- [Editor egui](../egui/#canvas-only) para integrar el lienzo en una app con su propia interfaz.
- [Controles compartidos](../controls/) para reutilizar los widgets.
- [Nodos personalizados](../custom-nodes/) y [paquetes](../packages/) para ampliar el catálogo.

## API de Rust

Estas páginas de Rustdoc se generan desde esta revisión del código y están en inglés.

| Crate | Punto de entrada |
| --- | --- |
| [orange-host](@shared/api/orange_host/) | `Host`, edición con `Draft`, ejecución y eventos |
| [orange-editor-ui](@shared/api/orange_editor_ui/) | `Editor`, paneles y solicitudes de archivo |
| [orange-ui-controls](@shared/api/orange_ui_controls/) | Controles, temas, medidores e iconos |
| [orange-sdk](@shared/api/orange_sdk/) | `Definition`, tipos y servicios de integración |

## Interfaz Wasm

La [interfaz WIT](@source/sdk/wit/orange-expansion.wit) define el mundo `plugin` y los valores intercambiados con el motor. El [componente de ejemplo](@source/sdk/guests/probe/src/lib.rs) muestra las vinculaciones en Rust. Las bibliotecas nativas son integraciones de confianza que corren con la autoridad del host.

## Contratos y datos

Los contratos versionados se publican en inglés. Consulta el [lenguaje de grafos](@shared/reference/graph-language/), el [formato de archivo](@shared/reference/graph-file-format/), los [paquetes](@shared/reference/packages/) y la [internacionalización](@shared/reference/internationalization/). Para herramientas y agentes, usa el [catálogo JSON](@shared/nodes.json), el [índice de texto](../llms.txt) o el [mapa del sitio](@shared/sitemap.xml).
