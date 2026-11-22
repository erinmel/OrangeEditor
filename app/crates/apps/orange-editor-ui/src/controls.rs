//! Editor defaults for the reusable egui controls.
use crate::{i18n::t, icons};

pub use orange_ui_controls::{
    Action, ButtonGeometry, ButtonSpec, ButtonStyle, ButtonTone, ControlState, Dropdown, Surface,
    button,
};

pub(crate) fn dialog_actions(
    ui: &mut egui::Ui,
    role: Action,
    enabled: bool,
    confirm: egui::Button<'_>,
) -> (egui::Response, egui::Response) {
    orange_ui_controls::dialog_actions(ui, t("cancel"), confirm, role, enabled)
}

/// Draws a search field with the editor's localized clear action and icon set.
/// Returns the text field response, including changes from the clear action.
pub fn search(ui: &mut egui::Ui, salt: &str, query: &mut String, hint: &str) -> egui::Response {
    orange_ui_controls::search(
        ui,
        salt,
        query,
        hint,
        icons::centered(icons::SEARCH),
        icons::centered(icons::CLOSE),
        t("clear-search"),
    )
}

/// Consumes unmodified keys reserved by the focused control.
pub(crate) fn consume_plain_keys<const N: usize>(ui: &egui::Ui, keys: [egui::Key; N]) -> [bool; N] {
    ui.input_mut(|input| keys.map(|key| input.consume_key(egui::Modifiers::NONE, key)))
}
