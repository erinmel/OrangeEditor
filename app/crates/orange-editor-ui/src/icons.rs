//! The editor's icons: one bundled font, Phosphor Regular (MIT, from
//! `@phosphor-icons/web` 2.1.2; see `fonts/Phosphor-LICENSE.txt` and docs/10), so no
//! symbol depends on the fonts of the machine. [`install`] adds it to a context; the
//! editor does so on its first frame.

/// The font family that holds only the icons. Icons are also a last fallback of the
/// proportional family, so one can sit in ordinary text.
pub const FAMILY: &str = "orange-icons";

const FONT: &[u8] = include_bytes!("../fonts/Phosphor.ttf");

// File
pub const NEW: char = '\u{e236}'; // file-plus
pub const OPEN: char = '\u{e256}'; // folder-open
pub const SAVE: char = '\u{e248}'; // floppy-disk
pub const SAVE_AS: char = '\u{eaf4}'; // floppy-disk-back
// Edit
pub const UNDO: char = '\u{e038}'; // arrow-counter-clockwise
pub const REDO: char = '\u{e036}'; // arrow-clockwise
pub const GROUP: char = '\u{e6ce}'; // bounding-box
pub const FUNCTION: char = '\u{ebe4}'; // function
pub const ARRANGE: char = '\u{e67c}'; // tree-structure
pub const VARIABLE: char = '\u{e860}'; // brackets-curly
pub const ADD: char = '\u{e3d6}'; // plus-circle
pub const COPY: char = '\u{e1ca}'; // copy
pub const CUT: char = '\u{eae0}'; // scissors
pub const PASTE: char = '\u{e196}'; // clipboard
pub const DUPLICATE: char = '\u{e1cc}'; // copy-simple
pub const DELETE: char = '\u{e4a6}'; // trash
pub const DISCONNECT: char = '\u{e2e4}'; // link-break
pub const SELECT_ALL: char = '\u{e746}'; // selection-all
// Run
pub const CHECK: char = '\u{e184}'; // check-circle
pub const RUN: char = '\u{e3d0}'; // play
pub const STOP: char = '\u{e46c}'; // stop
pub const WATCH: char = '\u{e220}'; // eye
// View
pub const NODES: char = '\u{ec24}'; // sidebar-simple
pub const PROPERTIES: char = '\u{e434}'; // sliders-horizontal
pub const PACKAGES: char = '\u{e390}'; // package
pub const LOG: char = '\u{eae8}'; // terminal-window
pub const SNAP: char = '\u{e6c8}'; // grid-four
pub const FIT: char = '\u{e626}'; // frame-corners
pub const ACTUAL_SIZE: char = '\u{e310}'; // magnifying-glass-plus
pub const LANGUAGE: char = '\u{e4a2}'; // translate
// Groups
pub const EXPAND: char = '\u{e13a}'; // caret-right
pub const COLLAPSE: char = '\u{e136}'; // caret-down

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
];

/// Adds the icon font to `ctx`, once; later calls change nothing.
pub fn install(ctx: &egui::Context) {
    ctx.add_font(egui::epaint::text::FontInsert::new(
        FAMILY,
        egui::FontData::from_static(FONT),
        vec![
            egui::epaint::text::InsertFontFamily {
                family: family(),
                priority: egui::epaint::text::FontPriority::Highest,
            },
            egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Proportional,
                priority: egui::epaint::text::FontPriority::Lowest,
            },
        ],
    ));
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
    let response = ui.add(
        egui::Button::selectable(open, "").min_size(egui::vec2(32.0, ui.spacing().interact_size.y)),
    );
    let icon = egui::Rect::from_center_size(response.rect.center(), egui::vec2(15.0, 11.0));
    let ink = ui.style().interact_selectable(&response, open).fg_stroke;
    let x = if side == egui::Align::Min {
        icon.left() + 4.5
    } else {
        icon.right() - 4.5
    };
    ui.painter().rect_stroke(icon, 1.5, ink, egui::StrokeKind::Inside);
    ui.painter().vline(x, icon.y_range(), ink);
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
    let response = ui.add_enabled(enabled, egui::Button::new(text(icon).size(16.0)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, what));
    response
        .on_hover_text(tip.clone())
        .on_disabled_hover_text(tip)
}

/// The primary action retains a readable label alongside its icon.
pub(crate) fn run_button(ui: &mut egui::Ui, enabled: bool, running: bool) -> egui::Response {
    let (icon, label) = if running {
        (STOP, "Stop")
    } else {
        (RUN, "Run")
    };
    let fill = if running {
        egui::Color32::from_rgb(152, 40, 40)
    } else {
        egui::Color32::from_rgb(32, 104, 72)
    };
    let response = ui.add_enabled(
        enabled,
        egui::Button::new((
            text(icon).size(16.0).color(egui::Color32::WHITE),
            egui::RichText::new(label)
                .strong()
                .color(egui::Color32::WHITE),
        ))
        .fill(fill)
        .min_size(egui::vec2(72.0, ui.spacing().interact_size.y)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response.on_hover_text(if running {
        "Stop this graph"
    } else {
        "Run this graph with the current values"
    })
}
