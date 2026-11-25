//! Editor icon names and command customization, backed by the shared icon assets.
use crate::i18n::t;
use orange_ui_controls::icons::{self as shared, Lucide};
use std::sync::Arc;

pub(crate) use shared::WIDTH as BUTTON_WIDTH;
pub use shared::{
    CHECK, CLOSE, DELETE, EDIT, FAMILY, KEYBOARD, SEARCH, family, install, paint_centered, text,
};
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
pub const ADD: char = '\u{e145}'; // add
pub const COPY: char = '\u{e14d}'; // content_copy
pub const CUT: char = '\u{e14e}'; // content_cut
pub const PASTE: char = '\u{e14f}'; // content_paste
pub const DUPLICATE: char = '\u{e03c}'; // library_add
pub const DISCONNECT: char = '\u{e16f}'; // link_off
pub const SELECT_ALL: char = '\u{e6fa}'; // select_window
pub const RUN: char = '\u{e037}'; // play_arrow
pub const STOP: char = '\u{e047}'; // stop
pub const WATCH: char = '\u{e8f4}'; // visibility
pub const NODES: char = '\u{f716}'; // left_panel_open
pub(crate) const NODE: char = '\u{e87b}'; // extension
pub const PROPERTIES: char = '\u{e429}'; // tune
pub const PACKAGES: char = '\u{f569}'; // package_2
pub const LOG: char = '\u{eb8e}'; // terminal
pub const SNAP: char = '\u{f016}'; // grid_4x4
pub const GRID: char = '\u{e9b0}'; // grid_view
pub const FIT: char = '\u{ea10}'; // fit_screen
pub const ACTUAL_SIZE: char = '\u{e8ff}'; // zoom_in
pub const LANGUAGE: char = '\u{e8e2}'; // translate
pub const DISPLAY: char = '\u{e30c}'; // desktop_windows
pub const EXPAND: char = '\u{e5cc}'; // chevron_right
pub const COLLAPSE: char = '\u{e5cf}'; // expand_more
pub const MOVE_UP: char = '\u{e5d8}'; // arrow_upward
pub const MOVE_DOWN: char = '\u{e5db}'; // arrow_downward
pub const SETTINGS: char = '\u{e8b8}'; // settings
pub const MINIMIZE: char = '\u{e15b}'; // remove
pub const MAXIMIZE: char = '\u{e3c6}'; // crop_square
pub const RESTORE: char = '\u{e3e0}'; // filter_none
pub const REFRESH: char = '\u{e5d5}'; // refresh
pub const MORE: char = '\u{e5d4}'; // more_vert
pub const DRAG: char = '\u{e945}'; // drag_indicator
pub const LEFT_CLOSE: char = '\u{f717}'; // left_panel_close
pub const RIGHT_OPEN: char = '\u{f704}'; // right_panel_open
pub const RIGHT_CLOSE: char = '\u{f705}'; // right_panel_close
pub const FLOW: char = '\u{e5c8}'; // arrow_forward
pub const ERROR: char = '\u{f8b6}'; // error
pub const WARNING: char = '\u{f083}'; // warning
pub const MATH: char = '\u{ea5f}'; // calculate
pub const AUDIO: char = '\u{e050}'; // volume_up
pub const NETWORK: char = '\u{e894}'; // language

pub(crate) const SIGNAL: char = ''; // rss_feed
pub(crate) const TEXT: char = ''; // format_align_left
pub(crate) const CLOCK: char = ''; // schedule
pub(crate) const LOGIC: char = ''; // call_split

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
    NODE,
    PROPERTIES,
    PACKAGES,
    LOG,
    FIT,
    SNAP,
    GRID,
    ACTUAL_SIZE,
    LANGUAGE,
    DISPLAY,
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
    SEARCH,
    MATH,
    AUDIO,
    NETWORK,
    SIGNAL,
    TEXT,
    CLOCK,
    LOGIC,
];

/// Paints a command icon identified by a constant from this module. Return `false`
/// to use the editor's default icon. The rectangle and color include widget state.
pub type CommandPainter =
    dyn Fn(&egui::Painter, egui::Rect, char, egui::Color32) -> bool + Send + Sync;

/// Installs an icon painter for centered command icons in this egui context.
/// Pass `None` to restore the defaults. Text and tooltips are supplied by the editor.
pub fn set_command_painter(ctx: &egui::Context, painter: Option<Arc<CommandPainter>>) {
    ctx.data_mut(|data| {
        let id = command_painter_id();
        if let Some(painter) = painter {
            data.insert_temp(id, painter);
        } else {
            data.remove::<Arc<CommandPainter>>(id);
        }
    });
    ctx.request_repaint();
}

fn command_painter_id() -> egui::Id {
    egui::Id::unique("orange-command-icon-painter")
}

pub(crate) fn centered(icon: char) -> egui::Atom<'static> {
    shared::slot(move |ui, rect, color| paint_command(ui, rect, icon, color))
}

fn paint_command(ui: &egui::Ui, rect: egui::Rect, icon: char, color: egui::Color32) {
    let painter = ui
        .ctx()
        .data(|data| data.get_temp::<Arc<CommandPainter>>(command_painter_id()));
    if painter.is_some_and(|painter| painter(ui.painter(), rect, icon, color)) {
        return;
    }
    let size = rect.width().min(rect.height());
    if let Some(svg) = graph_icon(icon) {
        shared::paint_svg(ui, rect, svg, color);
    } else {
        paint_centered(ui.painter(), rect.center(), icon, size, color);
    }
}

fn button_face(
    ui: &mut egui::Ui,
    icon: char,
    icon_color: Option<egui::Color32>,
    idle_fill: Option<egui::Color32>,
) -> egui::Response {
    orange_ui_controls::icon_button(
        ui,
        BUTTON_WIDTH,
        icon_color,
        idle_fill,
        |ui, rect, color| {
            paint_command(ui, rect, icon, color);
        },
    )
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
