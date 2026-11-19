//! Material Symbols Rounded, from the pinned egui_material_icons 0.9.0 library.
//! Graph toolbar icons use Lucide SVGs bundled by egui-lucide 0.2.0.
//! Codepoints below preserve the editor's public char-based icon API.
use crate::i18n::t;
use egui_lucide::Lucide;

/// The icon family registered by the Material library.
pub const FAMILY: &str = egui_material_icons::FONT_FAMILY_OUTLINED;
pub(crate) const BUTTON_WIDTH: f32 = 32.0;
pub const NEW: char = '\u{e89c}'; // note_add
pub const OPEN: char = '\u{e2c8}'; // folder_open
pub const SAVE: char = '\u{e161}'; // save
pub const SAVE_AS: char = '\u{eb60}'; // save_as
pub const UNDO: char = '\u{e166}'; // undo
pub const REDO: char = '\u{e15a}'; // redo
pub const GROUP: char = '\u{e162}'; // select_all
pub const UNGROUP: char = '\u{f731}'; // ungroup
pub const FUNCTION: char = '\u{e24a}'; // functions
pub const ARRANGE: char = '\u{e97a}'; // account_tree
pub const VARIABLE: char = '\u{ead3}'; // data_object
pub const ADD: char = '\u{e990}'; // add_circle
pub const COPY: char = '\u{e14d}'; // content_copy
pub const CUT: char = '\u{e14e}'; // content_cut
pub const PASTE: char = '\u{e14f}'; // content_paste
pub const DUPLICATE: char = '\u{e03c}'; // library_add
pub const DELETE: char = '\u{e92e}'; // delete
pub const DISCONNECT: char = '\u{e16f}'; // link_off
pub const SELECT_ALL: char = '\u{e6fa}'; // select_window
pub const CHECK: char = '\u{f0be}'; // check_circle
pub const RUN: char = '\u{e037}'; // play_arrow
pub const STOP: char = '\u{e047}'; // stop
pub const WATCH: char = '\u{e8f4}'; // visibility
pub const NODES: char = '\u{f716}'; // left_panel_open
pub const PROPERTIES: char = '\u{e429}'; // tune
pub const PACKAGES: char = '\u{f569}'; // package_2
pub const LOG: char = '\u{eb8e}'; // terminal
pub const SNAP: char = '\u{f016}'; // grid_4x4
pub const GRID: char = '\u{e9b0}'; // grid_view
pub const FIT: char = '\u{ea10}'; // fit_screen
pub const ACTUAL_SIZE: char = '\u{e8ff}'; // zoom_in
pub const LANGUAGE: char = '\u{e8e2}'; // translate
pub const KEYBOARD: char = '\u{e312}'; // keyboard
pub const EDIT: char = '\u{e3c9}'; // edit
pub const EXPAND: char = '\u{e5cc}'; // chevron_right
pub const COLLAPSE: char = '\u{e5cf}'; // expand_more
pub const MOVE_UP: char = '\u{e5d8}'; // arrow_upward
pub const MOVE_DOWN: char = '\u{e5db}'; // arrow_downward
pub const SETTINGS: char = '\u{e8b8}'; // settings
pub const MINIMIZE: char = '\u{e15b}'; // remove
pub const MAXIMIZE: char = '\u{e3c6}'; // crop_square
pub const RESTORE: char = '\u{e3e0}'; // filter_none
pub const CLOSE: char = '\u{e5cd}'; // close
pub const REFRESH: char = '\u{e5d5}'; // refresh
pub const MORE: char = '\u{e5d4}'; // more_vert
pub const DRAG: char = '\u{e945}'; // drag_indicator
pub const LEFT_CLOSE: char = '\u{f717}'; // left_panel_close
pub const RIGHT_OPEN: char = '\u{f704}'; // right_panel_open
pub const RIGHT_CLOSE: char = '\u{f705}'; // right_panel_close
pub const FLOW: char = '\u{e5c8}'; // arrow_forward
pub const ERROR: char = '\u{f8b6}'; // error
pub const WARNING: char = '\u{f083}'; // warning

/// Every icon the editor draws, for the test that each is in the font.
pub const ALL: &[char] = &[
    NEW,
    OPEN,
    SAVE,
    SAVE_AS,
    UNDO,
    REDO,
    GROUP,
    UNGROUP,
    FUNCTION,
    ARRANGE,
    VARIABLE,
    ADD,
    COPY,
    CUT,
    PASTE,
    DUPLICATE,
    DELETE,
    DISCONNECT,
    SELECT_ALL,
    CHECK,
    RUN,
    STOP,
    WATCH,
    NODES,
    PROPERTIES,
    PACKAGES,
    LOG,
    FIT,
    SNAP,
    GRID,
    ACTUAL_SIZE,
    LANGUAGE,
    KEYBOARD,
    EDIT,
    EXPAND,
    COLLAPSE,
    MOVE_UP,
    MOVE_DOWN,
    SETTINGS,
    CLOSE,
    MINIMIZE,
    MAXIMIZE,
    RESTORE,
    REFRESH,
    MORE,
    DRAG,
    LEFT_CLOSE,
    RIGHT_OPEN,
    RIGHT_CLOSE,
    FLOW,
    ERROR,
    WARNING,
];

/// Adds the icon font and SVG loader to `ctx`, once.
pub fn install(ctx: &egui::Context) {
    let first = ctx.data_mut(|data| {
        let id = egui::Id::unique("orange-material-fonts");
        let installed = data.get_temp::<bool>(id).unwrap_or(false);
        data.insert_temp(id, true);
        !installed
    });
    if first {
        egui_material_icons::initialize(ctx);
        egui_extras::install_image_loaders(ctx);
        ctx.request_repaint();
    }
}

/// The family that holds only the icons.
pub fn family() -> egui::FontFamily {
    egui::FontFamily::Name(FAMILY.into())
}

/// An icon as text, from the icon font.
pub fn text(icon: char) -> egui::RichText {
    egui::RichText::new(icon.to_string()).family(family())
}

fn button_face(
    ui: &mut egui::Ui,
    icon: char,
    icon_color: Option<egui::Color32>,
    idle_fill: Option<egui::Color32>,
) -> egui::Response {
    let size = egui::vec2(BUTTON_WIDTH, ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let highlighted = response.hovered() || response.is_pointer_button_down_on();
        let fill = match idle_fill {
            Some(fill) if highlighted => fill.gamma_multiply(1.18),
            Some(fill) => fill,
            None if highlighted => visuals.bg_fill,
            None => egui::Color32::TRANSPARENT,
        };
        let stroke = if response.has_focus() {
            ui.visuals().selection.stroke
        } else {
            egui::Stroke::NONE
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            visuals.corner_radius,
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );

        let color = icon_color.unwrap_or(visuals.fg_stroke.color);
        if let Some(svg) = graph_icon(icon) {
            svg.size(16.0)
                .color(egui::Color32::WHITE)
                .image()
                .tint(color)
                .paint_at(
                    ui,
                    egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(16.0)),
                );
        } else {
            let glyph =
                painter.layout_no_wrap(icon.to_string(), egui::FontId::new(16.0, family()), color);
            // Material symbols have different bearings. Center the visible glyph.
            if glyph.num_vertices > 0 {
                let origin = rect.center() - glyph.mesh_bounds.center().to_vec2();
                painter.galley(origin, glyph, color);
            }
        }
    }
    if let Some(cursor) = ui.visuals().interact_cursor
        && response.hovered()
    {
        ui.ctx().set_cursor_icon(cursor);
    }
    response
}

fn graph_icon(icon: char) -> Option<Lucide> {
    Some(match icon {
        GROUP => Lucide::Group,
        UNGROUP => Lucide::Ungroup,
        ARRANGE => Lucide::Workflow,
        SNAP => Lucide::Magnet,
        _ => return None,
    })
}

#[cfg(test)]
#[test]
fn graph_icons_load_as_svg_textures() {
    let ctx = egui::Context::default();
    install(&ctx);
    ctx.run_ui(egui::RawInput::default(), |ui| {
        for icon in [GROUP, UNGROUP, ARRANGE, SNAP] {
            let image = graph_icon(icon)
                .unwrap()
                .size(16.0)
                .color(egui::Color32::WHITE)
                .image();
            let loaded = image
                .load_for_size(ui.ctx(), egui::Vec2::splat(16.0))
                .unwrap_or_else(|error| panic!("failed to load {icon}: {error:?}"));
            assert!(
                matches!(loaded, egui::load::TexturePoll::Ready { .. }),
                "SVG texture for {icon} was not ready"
            );
        }
    })
    .drop_without_applying_deltas();
}

fn tooltip(label: &str, shortcut: &str) -> String {
    if shortcut.is_empty() || shortcut == t("unassigned") {
        label.to_owned()
    } else {
        format!("{label} ({shortcut})")
    }
}

pub(crate) fn dock_toggle(
    ui: &mut egui::Ui,
    open: bool,
    side: egui::Align,
    label: &str,
    shortcut: &str,
) -> egui::Response {
    let icon = match (side, open) {
        (egui::Align::Min, false) => NODES,
        (egui::Align::Min, true) => LEFT_CLOSE,
        (_, false) => RIGHT_OPEN,
        (_, true) => RIGHT_CLOSE,
    };
    toggle_button(ui, icon, open, label, shortcut)
}

pub(crate) fn toggle_button(
    ui: &mut egui::Ui,
    icon: char,
    active: bool,
    label: &str,
    shortcut: &str,
) -> egui::Response {
    let fill = active.then_some(ui.visuals().selection.bg_fill);
    let response = button_face(ui, icon, None, fill);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
    response.on_hover_text(tooltip(label, shortcut))
}

/// An icon-only button with what it does and its shortcut as its tooltip.
pub(crate) fn button(
    ui: &mut egui::Ui,
    enabled: bool,
    icon: char,
    what: &str,
    shortcut: &str,
) -> egui::Response {
    let tip = tooltip(what, shortcut);
    let response = ui.add_enabled(enabled, |ui: &mut egui::Ui| {
        button_face(ui, icon, None, None)
    });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, what));
    response
        .on_hover_text(tip.clone())
        .on_disabled_hover_text(tip)
}

/// The execution control keeps its state colors while sharing the toolbar layout.
pub(crate) fn run_button(ui: &mut egui::Ui, enabled: bool, running: bool) -> egui::Response {
    let (icon, label) = if running {
        (STOP, t("stop"))
    } else {
        (RUN, t("run"))
    };
    let color = if !enabled {
        ui.visuals().weak_text_color()
    } else if running {
        egui::Color32::from_gray(246)
    } else if ui.visuals().dark_mode {
        egui::Color32::from_rgb(108, 216, 150)
    } else {
        egui::Color32::from_rgb(20, 116, 62)
    };
    let fill = running.then_some(egui::Color32::from_rgb(180, 40, 57));
    let response = ui.add_enabled(enabled, |ui: &mut egui::Ui| {
        button_face(ui, icon, Some(color), fill)
    });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response
}
