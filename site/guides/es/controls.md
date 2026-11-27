# Referencia de controles

<h2 id="layout-and-theme">Tema y tamaño</h2>

Instala el estilo de escritorio una vez al crear el contexto. Usa `UiTokens::for_surface(ui, Surface::Card)` para adaptar los controles a la tarjeta.

```rust,ignore
orange_ui_controls::apply_desktop_style(ctx);
```

```sh
cargo run --locked -p orange-ui-controls --example gallery
```

[Tokens en Rustdoc](@shared/api/orange_ui_controls/theme/struct.UiTokens.html) · [Personalización en egui](../egui/#customize-appearance)

<h2 id="sliders-and-dials">Deslizadores y diales</h2>

Guarda `level` como un `f32` con valor inicial `55.0`. Usa la misma referencia mutable en ambos controles.

```rust,ignore
orange_ui_controls::continuous_slider(ui, Surface::Card, level, 0.0..=100.0);
orange_ui_controls::knob(ui, Surface::Card, level, 0.0..=100.0, None, &[]);
```

[Deslizador en Rustdoc](@shared/api/orange_ui_controls/fn.continuous_slider.html) · [Dial en Rustdoc](@shared/api/orange_ui_controls/fn.knob.html)

<h3 id="live-controls">Prueba el deslizador, el dial y la apariencia</h3>

El nivel empieza en 55 %. Mueve el deslizador y el dial sigue su valor. Cambia entre claro y oscuro y comprueba que el valor se conserva.

[Consulta el código Rust de la demo](@source/app/crates/dev/orange-docs-web/src/lib.rs)

<!-- controls-demo -->

<h2 id="buttons-and-choices">Botones y opciones</h2>

Usa un botón para una acción, un interruptor para un booleano y un control segmentado para elegir una sola opción. Al elegir Pico, este ejemplo guarda `1` en `mode`.

```rust,ignore
let tokens = UiTokens::for_surface(ui, Surface::Card);
let caption = ui.label("Modo de medición");
pill_segmented(ui, tokens, &caption, mode, &[(0, "Apagado"), (1, "Pico"), (2, "RMS")]);
```

[Control segmentado en Rustdoc](@shared/api/orange_ui_controls/fn.pill_segmented.html)

<h2 id="text-and-files">Texto y archivos</h2>

Pasa una referencia mutable al texto y asigna una ID estable. Usa el campo de contraseña para secretos y guarda las credenciales en el almacén del sistema. Para seleccionar archivos, elige una ruta desde tu aplicación o activa `native-dialogs` en escritorio.

```rust,ignore
let id = ui.make_persistent_id("graph-name");
orange_ui_controls::text_field(ui, Surface::Card, id, graph_name, "Nombre del grafo");
```

[Campo de texto en Rustdoc](@shared/api/orange_ui_controls/fn.text_field.html) · [Selector de rutas en Rustdoc](@shared/api/orange_ui_controls/components/struct.PathPicker.html)

<h2 id="menus-and-dialogs">Menús y diálogos</h2>

Agrupa acciones en un menú y usa un diálogo modal cuando necesites confirmación. Muestra el diálogo mientras tu aplicación espera una decisión y ocúltalo cuando se confirme o se cierre.

```rust,ignore
let result = orange_ui_controls::modal(ui.ctx(), egui::Id::new("rename-graph"))
    .show(ui.ctx(), |ui| ui.button("Cambiar nombre").clicked());
if result.inner {
    apply_rename();
}
if result.inner || result.should_close() {
    *rename_open = false;
}
```

[Menú en Rustdoc](@shared/api/orange_ui_controls/components/fn.menu.html) · [Modal en Rustdoc](@shared/api/orange_ui_controls/fn.modal.html)

<h2 id="meters-and-charts">Medidores y gráficas</h2>

El medidor dibuja los valores que le proporciona tu aplicación. Usa valores normalizados. Este ejemplo llena el 40 % de la altura y marca el pico en el 70 %.

```rust,ignore
use orange_ui_controls::visualizations::{LevelMeter, MeterColors};
let bounds = egui::Rect::from_min_size(ui.next_widget_position(), egui::vec2(14.0, 72.0));
LevelMeter::vertical(0.4, 0.7).paint(ui, Surface::Card, bounds, MeterColors::standard());
```

[Medidor en Rustdoc](@shared/api/orange_ui_controls/visualizations/struct.LevelMeter.html) · [Gráfica de frecuencia en Rustdoc](@shared/api/orange_ui_controls/visualizations/fn.frequency_chart.html) · [Grafo con una puerta de ruido](../examples/#quiet-mic-gate)
