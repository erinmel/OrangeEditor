//! Material Symbols Rounded, from the pinned egui_material_icons 0.9.0 library.
//! Outline icons share one family and size across commands, menus and docks.
//! Codepoints below preserve the editor's public char-based icon API.
use crate::i18n::t;

/// The icon family registered by the Material library.
pub const FAMILY: &str = egui_material_icons::FONT_FAMILY_OUTLINED;
pub const NEW: char = '\u{e89c}'; // note_add
pub const OPEN: char = '\u{e2c8}'; // folder_open
pub const SAVE: char = '\u{e161}'; // save
pub const SAVE_AS: char = '\u{eb60}'; // save_as
pub const UNDO: char = '\u{e166}'; // undo
pub const REDO: char = '\u{e15a}'; // redo
pub const GROUP: char = '\u{e162}'; // select_all
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
pub const FIT: char = '\u{ea10}'; // fit_screen
pub const ACTUAL_SIZE: char = '\u{e8ff}'; // zoom_in
pub const LANGUAGE: char = '\u{e8e2}'; // translate
pub const EXPAND: char = '\u{e5cc}'; // chevron_right
pub const COLLAPSE: char = '\u{e5cf}'; // expand_more
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
    ACTUAL_SIZE,
    LANGUAGE,
    EXPAND,
    COLLAPSE,
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

/// Adds the icon font to `ctx`, once; later calls change nothing.
pub fn install(ctx: &egui::Context) {
    let first = ctx.data_mut(|data| {
        let id = egui::Id::unique("orange-material-fonts");
        let installed = data.get_temp::<bool>(id).unwrap_or(false);
        data.insert_temp(id, true);
        !installed
    });
    if first {
        egui_material_icons::initialize(ctx);
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

pub(crate) fn dock_toggle(
    ui: &mut egui::Ui,
    open: bool,
    side: egui::Align,
    label: &str,
) -> egui::Response {
    let icon = match (side, open) {
        (egui::Align::Min, false) => NODES,
        (egui::Align::Min, true) => LEFT_CLOSE,
        (_, false) => RIGHT_OPEN,
        (_, true) => RIGHT_CLOSE,
    };
    let response = ui.add(
        egui::Button::selectable(open, text(icon).size(16.0))
            .min_size(egui::vec2(32.0, ui.spacing().interact_size.y)),
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, open, label));
    response.on_hover_text(label)
}

/// An icon-only button with what it does and its shortcut as its tooltip.
pub(crate) fn button(
    ui: &mut egui::Ui,
    enabled: bool,
    icon: char,
    what: &str,
    shortcut: &str,
) -> egui::Response {
    let tip = if shortcut.is_empty() {
        what.to_owned()
    } else {
        format!("{what} ({shortcut})")
    };
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(text(icon).size(16.0))
            .frame(false)
            .min_size(egui::vec2(32.0, ui.spacing().interact_size.y)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, what));
    response
        .on_hover_text(tip.clone())
        .on_disabled_hover_text(tip)
}

/// The primary action retains a readable label alongside its icon.
pub(crate) fn run_button(ui: &mut egui::Ui, enabled: bool, running: bool) -> egui::Response {
    let (icon, label) = if running {
        (STOP, t("stop"))
    } else {
        (RUN, t("run"))
    };
    let color = if running {
        egui::Color32::from_rgb(225, 117, 125)
    } else {
        egui::Color32::from_rgb(118, 192, 150)
    };
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(text(icon).size(16.0).color(color))
            .fill(egui::Color32::TRANSPARENT)
            .frame(false)
            .min_size(egui::vec2(32.0, ui.spacing().interact_size.y)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response.on_hover_text(if running {
        t("stop-hint")
    } else {
        t("run-hint")
    })
}
