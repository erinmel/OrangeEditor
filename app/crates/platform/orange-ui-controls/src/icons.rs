//! Bundled icons for controls. Application-specific mappings belong to the host.
pub use egui_lucide::Lucide;

pub const FAMILY: &str = egui_material_icons::FONT_FAMILY_OUTLINED;
pub const SIZE: f32 = 16.0;
pub const WIDTH: f32 = 32.0;
pub const SEARCH: char = '\u{e8b6}';
pub const CLOSE: char = '\u{e5cd}';
pub const CHECK: char = '\u{f0be}';
pub const EDIT: char = '\u{e3c9}';
pub const DELETE: char = '\u{e92e}';
pub const KEYBOARD: char = '\u{e312}';
pub const CHECKED: char = '\u{e9de}';
pub const UNCHECKED: char = '\u{e835}';

/// Registers the bundled icon font and SVG loader once per context.
pub fn install(ctx: &egui::Context) {
    let first = ctx.data_mut(|data| {
        let id = egui::Id::unique("orange-control-icon-assets");
        let first = !data.get_temp::<bool>(id).unwrap_or(false);
        data.insert_temp(id, true);
        first
    });
    if first {
        egui_material_icons::initialize(ctx);
        egui_extras::install_image_loaders(ctx);
    }
}

pub fn family() -> egui::FontFamily {
    egui::FontFamily::Name(FAMILY.into())
}

/// An inline glyph for text runs that mix text and icons.
pub fn text(icon: char) -> egui::RichText {
    egui::RichText::new(icon.to_string()).family(family())
}

/// Paints using the visible glyph bounds, so icons center independently of font metrics.
pub fn paint_centered(
    painter: &egui::Painter,
    center: egui::Pos2,
    icon: char,
    size: f32,
    color: egui::Color32,
) {
    let glyph = painter.layout_no_wrap(icon.to_string(), egui::FontId::new(size, family()), color);
    if glyph.num_vertices > 0 {
        painter.galley(center - glyph.mesh_bounds.center().to_vec2(), glyph, color);
    }
}

/// Reserves a standard icon slot and supplies its centered bounds and state color.
pub fn slot(paint: impl Fn(&egui::Ui, egui::Rect, egui::Color32) + 'static) -> egui::Atom<'static> {
    egui::Atom::paint(egui::Vec2::splat(SIZE), move |ui, args| {
        paint(ui, args.rect, args.fallback_text_color);
    })
}

pub fn centered(icon: char) -> egui::Atom<'static> {
    slot(move |ui, rect, color| paint_centered(ui.painter(), rect.center(), icon, SIZE, color))
}

pub fn paint_svg(ui: &egui::Ui, rect: egui::Rect, icon: Lucide, color: egui::Color32) {
    icon.size(rect.width().min(rect.height()))
        .color(egui::Color32::WHITE)
        .image()
        .tint(color)
        .paint_at(ui, rect);
}

#[cfg(test)]
#[test]
fn bundled_control_icons_are_available_and_center_on_ink() {
    let ctx = egui::Context::default();
    install(&ctx);
    let center = egui::pos2(60.0, 40.0);
    let glyphs = [
        CHECKED, UNCHECKED, SEARCH, CLOSE, CHECK, EDIT, DELETE, KEYBOARD,
    ];
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        for glyph in glyphs {
            assert!(
                ui.fonts_mut(|fonts| fonts.has_glyph(&egui::FontId::new(SIZE, family()), glyph))
            );
            paint_centered(ui.painter(), center, glyph, SIZE, egui::Color32::WHITE);
        }
    });
    let mut rendered = 0;
    for shape in &output.shapes {
        if let egui::Shape::Text(text) = &shape.shape {
            let ink = text.galley.mesh_bounds.translate(text.pos.to_vec2());
            assert!(ink.center().distance(center) <= 0.5);
            rendered += 1;
        }
    }
    assert_eq!(rendered, glyphs.len());
    output.drop_without_applying_deltas();
}
