# Integra y personaliza el editor egui

<h2 id="add-editor-ui">Añade el editor</h2>

Fija la versión compatible de egui y la misma revisión para las dependencias de OrangeEditor. Conserva un `Editor`, un `Host` y el receptor `Events`.

```toml
[dependencies]
egui = "=0.37.0"
orange-host = { git = "https://github.com/erinmel/OrangeEditor.git", rev = "@revision@" }
orange-editor-ui = { git = "https://github.com/erinmel/OrangeEditor.git", rev = "@revision@" }
```

<h2 id="canvas-only">Conserva solo el lienzo</h2>

Llama a los paneles que quieras mostrar. Añade `right_panel` para el inspector o `nodes_panel` para el catálogo. Atiende las solicitudes de archivo de `Editor::take_request` desde la aplicación.

```rust,ignore
editor.poll_events(&events);
editor.ui_with_layout(ui, &mut host, |editor, ui, host| {
    egui::CentralPanel::default().show(ui, |ui| {
        editor.canvas_panel(ui, host);
    });
});
```

```sh
cargo run --locked -p orange-desktop --example embed_editor
```

El [ejemplo completo](@source/app/crates/apps/orange-desktop/examples/embed_editor.rs) incluye una cabecera personalizada, inspector opcional y acciones de archivo. Pásale la ruta de una fuente TTF u OTF después de `--` para probarla.

<h2 id="customize-appearance">Colores y espaciado</h2>

Antes del primer fotograma, aplica `orange_editor_ui::apply_theme` si quieres usar el estilo de escritorio como punto de partida. Cambia `style.visuals.panel_fill`, `style.visuals.selection.bg_fill` o `style.spacing.item_spacing`, y guarda el estilo con `ctx.set_style_of`. Configura claro y oscuro si tu aplicación ofrece ambos. Usa `UiTokens::install` con `ThemeMode::Custom` para una paleta explícita de los [controles compartidos](../controls/).

<h2 id="fonts-and-formatting">Fuentes y formato</h2>

Instala las fuentes con `ctx.set_fonts`, inserta la tuya al principio de la familia y conserva las de respaldo. Define tamaños por rol con `Style::text_styles`. Para un control de nodo, `orange_sdk::Control::slider().range(-60.0, 0.0).unit("dB").precision(1)` sugiere el rango y formato del campo. Mantén el valor numérico en el grafo.

<h2 id="icons-and-labels">Iconos y etiquetas</h2>

Registra `icons::set_command_painter` para dibujar iconos de comandos. Devuelve `false` cuando quieras usar el icono predeterminado. Puedes añadir catálogos con `i18n::add_application_catalogs` y elegir el idioma con `Editor::set_language`. Los identificadores guardados permanecen iguales. Consulta [Rustdoc del editor](@shared/api/orange_editor_ui/).
